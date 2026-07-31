//! TCP transport for Sector-local log shipping and catch-up control traffic
//! (ADR-0027 / 8D-2c).
//!
//! Wire format: `[u32 little-endian payload length][postcard ReplicationFrame]`.
//! The transport is plaintext by design for the Phase 8D LAN milestone.

use crate::{CatchUpMessage, CatchUpTransport, LogBatch, ReplicationTransport};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, ErrorKind},
    net::SocketAddr,
};
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream, ToSocketAddrs},
    sync::broadcast,
};

const CHANNEL_CAPACITY: usize = 10_000;
/// Snapshot fallback shares this framed connection. Keep the allocation bound
/// aligned with `SnapshotTransfer` plus a small postcard envelope allowance.
const MAX_FRAME_LEN: usize = 257 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum ReplicationFrame {
    Batch(LogBatch),
    CatchUp(CatchUpMessage),
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TcpReplicationError {
    #[error("tcp replication io error: {0}")]
    Io(#[from] io::Error),
    #[error("tcp replication postcard error: {0}")]
    Postcard(#[from] postcard::Error),
    #[error("tcp replication frame is too large: {actual} bytes > {max} bytes")]
    FrameTooLarge { actual: usize, max: usize },
}

/// Plain TCP implementation of [`ReplicationTransport`] and
/// [`CatchUpTransport`].
///
/// Ordinary batches and catch-up control messages share one framed connection
/// but are delivered to separate bounded inbound channels. Outbound frames are
/// published to every currently connected peer; the catch-up protocol carries
/// requester/owner Sector IDs and request IDs so unrelated peers safely ignore
/// directed control messages.
#[derive(Debug, Clone)]
pub struct TcpReplicationTransport {
    local_addr: SocketAddr,
    inbound_batch_tx: broadcast::Sender<LogBatch>,
    inbound_catch_up_tx: broadcast::Sender<CatchUpMessage>,
    outbound_tx: broadcast::Sender<ReplicationFrame>,
}

impl TcpReplicationTransport {
    /// Bind a listener and start accepting peer connections.
    pub async fn bind<A: ToSocketAddrs>(addr: A) -> Result<Self, TcpReplicationError> {
        let listener = TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;
        let (inbound_batch_tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        let (inbound_catch_up_tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        let (outbound_tx, _) = broadcast::channel(CHANNEL_CAPACITY);

        tokio::spawn(accept_loop(
            listener,
            inbound_batch_tx.clone(),
            inbound_catch_up_tx.clone(),
            outbound_tx.clone(),
        ));

        Ok(Self {
            local_addr,
            inbound_batch_tx,
            inbound_catch_up_tx,
            outbound_tx,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Connect to one peer and start bidirectional frame forwarding.
    pub async fn connect_peer(&self, addr: SocketAddr) -> Result<(), TcpReplicationError> {
        let stream = TcpStream::connect(addr).await?;
        spawn_connection(
            stream,
            self.inbound_batch_tx.clone(),
            self.inbound_catch_up_tx.clone(),
            self.outbound_tx.subscribe(),
        );
        Ok(())
    }
}

impl ReplicationTransport for TcpReplicationTransport {
    fn broadcast(&self, batch: LogBatch) {
        let _ = self.outbound_tx.send(ReplicationFrame::Batch(batch));
    }

    fn subscribe(&self) -> broadcast::Receiver<LogBatch> {
        self.inbound_batch_tx.subscribe()
    }
}

impl CatchUpTransport for TcpReplicationTransport {
    fn send_catch_up(&self, message: CatchUpMessage) {
        let _ = self.outbound_tx.send(ReplicationFrame::CatchUp(message));
    }

    fn subscribe_catch_up(&self) -> broadcast::Receiver<CatchUpMessage> {
        self.inbound_catch_up_tx.subscribe()
    }
}

async fn accept_loop(
    listener: TcpListener,
    inbound_batch_tx: broadcast::Sender<LogBatch>,
    inbound_catch_up_tx: broadcast::Sender<CatchUpMessage>,
    outbound_tx: broadcast::Sender<ReplicationFrame>,
) {
    loop {
        let Ok((stream, _addr)) = listener.accept().await else {
            break;
        };
        spawn_connection(
            stream,
            inbound_batch_tx.clone(),
            inbound_catch_up_tx.clone(),
            outbound_tx.subscribe(),
        );
    }
}

fn spawn_connection(
    stream: TcpStream,
    inbound_batch_tx: broadcast::Sender<LogBatch>,
    inbound_catch_up_tx: broadcast::Sender<CatchUpMessage>,
    outbound_rx: broadcast::Receiver<ReplicationFrame>,
) {
    tokio::spawn(async move {
        let (reader, writer) = stream.into_split();
        tokio::select! {
            _ = read_loop(reader, inbound_batch_tx, inbound_catch_up_tx) => {}
            _ = write_loop(writer, outbound_rx) => {}
        }
    });
}

async fn read_loop<R>(
    mut reader: R,
    inbound_batch_tx: broadcast::Sender<LogBatch>,
    inbound_catch_up_tx: broadcast::Sender<CatchUpMessage>,
) where
    R: AsyncRead + Unpin,
{
    loop {
        match read_frame(&mut reader).await {
            Ok(Some(ReplicationFrame::Batch(batch))) => {
                let _ = inbound_batch_tx.send(batch);
            }
            Ok(Some(ReplicationFrame::CatchUp(message))) => {
                let _ = inbound_catch_up_tx.send(message);
            }
            Ok(None) | Err(_) => break,
        }
    }
}

async fn write_loop<W>(mut writer: W, mut outbound_rx: broadcast::Receiver<ReplicationFrame>)
where
    W: AsyncWrite + Unpin,
{
    loop {
        match outbound_rx.recv().await {
            Ok(frame) => {
                if write_frame(&mut writer, &frame).await.is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}

async fn read_frame<R>(reader: &mut R) -> Result<Option<ReplicationFrame>, TcpReplicationError>
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
        return Err(TcpReplicationError::FrameTooLarge {
            actual: len,
            max: MAX_FRAME_LEN,
        });
    }

    let mut payload = vec![0_u8; len];
    reader.read_exact(&mut payload).await?;
    Ok(Some(postcard::from_bytes(&payload)?))
}

async fn write_frame<W>(writer: &mut W, frame: &ReplicationFrame) -> Result<(), TcpReplicationError>
where
    W: AsyncWrite + Unpin,
{
    let payload = postcard::to_stdvec(frame)?;
    if payload.len() > MAX_FRAME_LEN {
        return Err(TcpReplicationError::FrameTooLarge {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CatchUpRequest;
    use dawn_core::{events::VelocityChanged, NodeId, SectorId, ShipId, Tick, Velocity};
    use tokio::io::duplex;

    fn make_batch(sector_id: SectorId, from_index: u64, count: usize) -> LogBatch {
        let events = (0..count)
            .map(|i| {
                dawn_core::DomainEvent::VelocityChanged(VelocityChanged {
                    ship_id: ShipId::new(NodeId(0), i as u64),
                    velocity: Velocity::new(1.0, 0.0, 0.0),
                    tick: Tick(1),
                })
            })
            .collect();
        LogBatch::new(sector_id, from_index, events)
    }

    #[tokio::test]
    async fn frame_round_trips_log_batch() {
        let (mut client, mut server) = duplex(4096);
        let frame = ReplicationFrame::Batch(make_batch(SectorId(7), 42, 2));
        let sent = frame.clone();
        let writer = tokio::spawn(async move {
            write_frame(&mut client, &sent).await.unwrap();
        });

        let received = read_frame(&mut server).await.unwrap().unwrap();
        writer.await.unwrap();

        assert_eq!(received, frame);
    }

    #[tokio::test]
    async fn frame_round_trips_catch_up_control() {
        let (mut client, mut server) = duplex(4096);
        let frame = ReplicationFrame::CatchUp(CatchUpMessage::Request(CatchUpRequest {
            request_id: 9,
            requester_sector_id: SectorId(2),
            owner_sector_id: SectorId(1),
            from_index: 5,
            max_events: 32,
        }));
        let sent = frame.clone();
        let writer = tokio::spawn(async move {
            write_frame(&mut client, &sent).await.unwrap();
        });

        let received = read_frame(&mut server).await.unwrap().unwrap();
        writer.await.unwrap();

        assert_eq!(received, frame);
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
            TcpReplicationError::FrameTooLarge { actual, max }
                if actual == MAX_FRAME_LEN + 1 && max == MAX_FRAME_LEN
        ));
    }

    #[tokio::test]
    async fn tcp_transport_delivers_batches_and_catch_up_messages() {
        let a = TcpReplicationTransport::bind("127.0.0.1:0").await.unwrap();
        let b = TcpReplicationTransport::bind("127.0.0.1:0").await.unwrap();
        let mut batch_rx = b.subscribe();
        let mut catch_up_rx = b.subscribe_catch_up();

        a.connect_peer(b.local_addr()).await.unwrap();

        let batch = make_batch(SectorId(1), 5, 3);
        a.broadcast(batch.clone());
        assert_eq!(batch_rx.recv().await.unwrap(), batch);

        let message = CatchUpMessage::Request(CatchUpRequest {
            request_id: 9,
            requester_sector_id: SectorId(2),
            owner_sector_id: SectorId(1),
            from_index: 5,
            max_events: 32,
        });
        a.send_catch_up(message.clone());
        assert_eq!(catch_up_rx.recv().await.unwrap(), message);
    }

    #[tokio::test]
    async fn a_batch_received_over_the_wire_is_ingested_into_a_replica() {
        // End-to-end consumer side: owner broadcasts, peer receives over TCP
        // and feeds the batch into its ReplicaSet, which retains the log.
        let owner = TcpReplicationTransport::bind("127.0.0.1:0").await.unwrap();
        let peer = TcpReplicationTransport::bind("127.0.0.1:0").await.unwrap();
        let mut rx = peer.subscribe();
        owner.connect_peer(peer.local_addr()).await.unwrap();
        let mut replicas = crate::ReplicaSet::new(1024);

        owner.broadcast(make_batch(SectorId(1), 0, 3));
        let batch = rx.recv().await.unwrap();
        assert_eq!(
            replicas.ingest(&batch),
            crate::Ingest::Applied {
                sector_id: SectorId(1),
                applied: 3,
                next_index: 3
            },
        );
        assert_eq!(replicas.replicated_len(SectorId(1)), 3);
    }
}
