//! TCP transport for `RaftActor` RPCs (ADR-0014 / 8D-3).
//!
//! Wire format: `[u32 little-endian payload length][postcard RaftMessage payload]`.
//! Plaintext by design for the Phase 8D LAN milestone (matches 8D-2c wire format).
//!
//! Each peer gets a dedicated outbound task that dials, drains a message queue,
//! and automatically reconnects on failure — matching Raft's best-effort delivery
//! semantics (message loss is tolerated; the algorithm retries on timeout).
//!
//! Usage:
//! ```ignore
//! let (actor_tx, actor_rx) = tokio::sync::mpsc::unbounded_channel();
//! let transport = TcpRaftTransport::bind("0.0.0.0:7001", peers, actor_tx).await?;
//! let actor = RaftActor::new(state, peer_ids, Arc::new(transport), actor_rx, committed_tx);
//! tokio::spawn(actor.run());
//! ```

use crate::actor::RaftActorMessage;
use crate::rpc::RaftMessage;
use crate::transport::RaftTransport;
use dawn_core::NodeId;
use std::{
    collections::HashMap,
    io::{self, ErrorKind},
    net::SocketAddr,
    time::Duration,
};
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::mpsc::{self, UnboundedSender},
    time::sleep,
};

const MAX_FRAME_LEN: usize = 1024 * 1024; // 1 MiB — generous for Raft RPCs
const RECONNECT_DELAY: Duration = Duration::from_millis(100);

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TcpRaftError {
    #[error("tcp raft io error: {0}")]
    Io(#[from] io::Error),
    #[error("tcp raft postcard error: {0}")]
    Postcard(#[from] postcard::Error),
    #[error("tcp raft frame too large: {actual} bytes > {max} bytes")]
    FrameTooLarge { actual: usize, max: usize },
}

/// TCP implementation of [`RaftTransport`].
///
/// `send()` queues a message to a per-peer outbound task; delivery is
/// best-effort (dropped on connection failure, retried by the Raft algorithm).
/// Incoming messages from any peer are deserialized and forwarded to the local
/// `RaftActor` mailbox via the `inbox` channel provided at construction time.
#[derive(Debug)]
pub struct TcpRaftTransport {
    peer_txs: HashMap<NodeId, UnboundedSender<RaftMessage>>,
}

impl TcpRaftTransport {
    /// Bind a listener on `listen_addr`, start per-peer outbound tasks for
    /// every entry in `peers`, and return a transport ready for use.
    ///
    /// Incoming Raft RPCs are deserialized and forwarded to `inbox`.
    pub async fn bind(
        listen_addr: impl tokio::net::ToSocketAddrs,
        peers: HashMap<NodeId, SocketAddr>,
        inbox: UnboundedSender<RaftActorMessage>,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind(listen_addr).await?;
        tokio::spawn(accept_loop(listener, inbox));

        let mut peer_txs = HashMap::new();
        for (node_id, addr) in peers {
            let (tx, rx) = mpsc::unbounded_channel();
            tokio::spawn(outbound_loop(addr, rx));
            peer_txs.insert(node_id, tx);
        }

        Ok(Self { peer_txs })
    }
}

impl RaftTransport for TcpRaftTransport {
    fn send(&self, to: NodeId, msg: RaftMessage) {
        if let Some(tx) = self.peer_txs.get(&to) {
            // Best-effort: if the outbound task's channel is closed (shutdown),
            // drop silently — Raft handles message loss via timeout.
            let _ = tx.send(msg);
        }
    }
}

// ── Inbound (listener) side ───────────────────────────────────────────────

async fn accept_loop(listener: TcpListener, inbox: UnboundedSender<RaftActorMessage>) {
    loop {
        let Ok((stream, _addr)) = listener.accept().await else {
            break;
        };
        let inbox = inbox.clone();
        tokio::spawn(async move {
            let (reader, _) = stream.into_split();
            inbound_loop(reader, inbox).await;
        });
    }
}

async fn inbound_loop<R>(mut reader: R, inbox: UnboundedSender<RaftActorMessage>)
where
    R: AsyncRead + Unpin,
{
    loop {
        match read_frame(&mut reader).await {
            Ok(Some(msg)) => {
                // Inbox closed means the actor has shut down; nothing more to do.
                if inbox.send(RaftActorMessage::Raft(msg)).is_err() {
                    break;
                }
            }
            Ok(None) => break, // peer closed connection
            Err(_) => break,   // framing / IO error; peer will reconnect
        }
    }
}

// ── Outbound (dialer) side ────────────────────────────────────────────────

/// Maintains a persistent TCP connection to `peer_addr`, draining `rx`.
/// Reconnects automatically when the connection drops.
async fn outbound_loop(peer_addr: SocketAddr, mut rx: mpsc::UnboundedReceiver<RaftMessage>) {
    let mut reconnects: u64 = 0;
    loop {
        match TcpStream::connect(peer_addr).await {
            Ok(stream) => {
                let (_, writer) = stream.into_split();
                let closed = drain_outbound(writer, &mut rx).await;
                if closed {
                    return; // sender side dropped → shutdown
                }
                // Connection dropped; reconnect after a short delay. Logged
                // for 8D-5 field observability — reconnect frequency on
                // physical WiFi/USB links is a direct signal of link quality.
                reconnects += 1;
                eprintln!(
                    "[RaftTransport] {peer_addr} dropped, reconnecting (attempt #{reconnects})"
                );
                sleep(RECONNECT_DELAY).await;
            }
            Err(e) => {
                reconnects += 1;
                eprintln!(
                    "[RaftTransport] {peer_addr} connect failed: {e} (attempt #{reconnects})"
                );
                sleep(RECONNECT_DELAY).await;
            }
        }
    }
}

/// Drain `rx` into `writer`.  Returns `true` when `rx` is closed (shutdown),
/// `false` when the TCP connection dropped (caller should reconnect).
async fn drain_outbound<W>(mut writer: W, rx: &mut mpsc::UnboundedReceiver<RaftMessage>) -> bool
where
    W: AsyncWrite + Unpin,
{
    loop {
        let Some(msg) = rx.recv().await else {
            return true; // channel closed → shutdown
        };
        if write_frame(&mut writer, &msg).await.is_err() {
            return false; // connection dropped → reconnect
        }
    }
}

// ── Framing helpers ───────────────────────────────────────────────────────

async fn read_frame<R>(reader: &mut R) -> Result<Option<RaftMessage>, TcpRaftError>
where
    R: AsyncRead + Unpin,
{
    let mut len_buf = [0_u8; 4];
    if let Err(err) = reader.read_exact(&mut len_buf).await {
        if err.kind() == ErrorKind::UnexpectedEof {
            return Ok(None);
        }
        return Err(err.into());
    }

    let len = u32::from_le_bytes(len_buf) as usize;
    if len > MAX_FRAME_LEN {
        return Err(TcpRaftError::FrameTooLarge {
            actual: len,
            max: MAX_FRAME_LEN,
        });
    }

    let mut payload = vec![0_u8; len];
    reader.read_exact(&mut payload).await?;
    Ok(Some(postcard::from_bytes(&payload)?))
}

async fn write_frame<W>(writer: &mut W, msg: &RaftMessage) -> Result<(), TcpRaftError>
where
    W: AsyncWrite + Unpin,
{
    let payload = postcard::to_stdvec(msg)?;
    if payload.len() > MAX_FRAME_LEN {
        return Err(TcpRaftError::FrameTooLarge {
            actual: payload.len(),
            max: MAX_FRAME_LEN,
        });
    }
    writer
        .write_all(&(payload.len() as u32).to_le_bytes())
        .await?;
    writer.write_all(&payload).await?;
    writer.flush().await?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::{AppendEntries, RequestVote};
    use crate::state::Term;
    use tokio::io::duplex;

    fn node(id: u8) -> NodeId {
        NodeId(id)
    }

    fn vote_req() -> RaftMessage {
        RaftMessage::RequestVote(RequestVote {
            term: Term(1),
            candidate_id: node(0),
        })
    }

    fn heartbeat() -> RaftMessage {
        RaftMessage::AppendEntries(AppendEntries::heartbeat(Term(2), node(0)))
    }

    // ── frame round-trip ─────────────────────────────────────────────────

    #[tokio::test]
    async fn frame_round_trips_request_vote() {
        let (mut client, mut server) = duplex(4096);
        let msg = vote_req();
        write_frame(&mut client, &msg).await.unwrap();
        let received = read_frame(&mut server).await.unwrap().unwrap();
        assert_eq!(received, msg);
    }

    #[tokio::test]
    async fn frame_round_trips_append_entries() {
        let (mut client, mut server) = duplex(4096);
        let msg = heartbeat();
        write_frame(&mut client, &msg).await.unwrap();
        let received = read_frame(&mut server).await.unwrap().unwrap();
        assert_eq!(received, msg);
    }

    #[tokio::test]
    async fn oversized_frame_is_rejected() {
        let (mut client, mut server) = duplex(8);
        let writer = tokio::spawn(async move {
            client
                .write_all(&((MAX_FRAME_LEN as u32) + 1).to_le_bytes())
                .await
                .unwrap();
        });
        let err = read_frame(&mut server).await.unwrap_err();
        writer.await.unwrap();
        assert!(matches!(
            err,
            TcpRaftError::FrameTooLarge { actual, max }
                if actual == MAX_FRAME_LEN + 1 && max == MAX_FRAME_LEN
        ));
    }

    #[tokio::test]
    async fn clean_eof_returns_none() {
        let (client, mut server) = duplex(4096);
        drop(client);
        let result = read_frame(&mut server).await.unwrap();
        assert!(result.is_none());
    }

    // ── end-to-end over loopback ──────────────────────────────────────────

    #[tokio::test]
    async fn tcp_transport_delivers_message_to_peer() {
        let (inbox_tx, mut inbox_rx) = mpsc::unbounded_channel();

        // Node B: listen; no outbound peers.
        let b = TcpRaftTransport::bind("127.0.0.1:0", HashMap::new(), inbox_tx)
            .await
            .unwrap();
        let b_addr = {
            // Re-bind to discover the actual port assigned by the OS.
            // We need to find it from our listener, but bind() doesn't expose it.
            // Instead, use a second bind to learn the address — actually we can't.
            // We'll use a fixed approach: bind node A and pass B's addr explicitly.
            //
            // This test spins up an actual listener on a well-known 0.0.0.0:0 port.
            // The workaround: use TcpListener directly for B's address.
            drop(b);
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let (inbox_tx2, inbox_rx2) = mpsc::unbounded_channel::<RaftActorMessage>();
            tokio::spawn(accept_loop(listener, inbox_tx2));
            // Give the background task a moment to start
            tokio::time::sleep(Duration::from_millis(10)).await;
            let _ = inbox_rx; // silence unused warning
            inbox_rx = inbox_rx2;
            addr
        };

        // Node A: no listener (not needed for this test), one peer = B.
        let mut peers_a = HashMap::new();
        peers_a.insert(node(2), b_addr);
        let a = TcpRaftTransport::bind("127.0.0.1:0", peers_a, mpsc::unbounded_channel().0)
            .await
            .unwrap();

        // Allow outbound_loop to connect.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let msg = vote_req();
        a.send(node(2), msg.clone());

        let received = tokio::time::timeout(Duration::from_secs(2), inbox_rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");

        assert!(matches!(received, RaftActorMessage::Raft(m) if m == msg));
    }

    #[tokio::test]
    async fn send_to_unknown_node_is_silently_dropped() {
        let transport = TcpRaftTransport {
            peer_txs: HashMap::new(),
        };
        // Should not panic.
        transport.send(node(99), vote_req());
    }
}
