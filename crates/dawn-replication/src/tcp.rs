//! TCP transport for Sector-local log shipping and catch-up control traffic
//! (ADR-0027 / 8D-2c).
//!
//! Wire format: `[u32 little-endian payload length][tag][payload]`.
//! Ordinary frames use postcard after the tag. Snapshot responses use a small
//! postcard header followed by raw snapshot bytes so the sender does not build
//! a second snapshot-sized serialization buffer.
//! The transport is plaintext by design for the Phase 8D LAN milestone.

use crate::{
    CatchUpMessage, CatchUpPayload, CatchUpResponse, CatchUpTransport, LogBatch, ReplicaSnapshot,
    ReplicationTransport,
};
use dawn_core::SectorId;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::{self, ErrorKind},
    net::SocketAddr,
    sync::{Arc, RwLock},
};
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream, ToSocketAddrs},
    sync::{broadcast, mpsc},
};

const CHANNEL_CAPACITY: usize = 10_000;
/// At most one unsent frame per peer. Dropped gossip is repaired by catch-up;
/// bounding this queue prevents multiple snapshot-sized responses accumulating.
const PEER_CHANNEL_CAPACITY: usize = 1;
const FRAME_KIND_POSTCARD: u8 = 0;
const FRAME_KIND_SNAPSHOT: u8 = 1;
/// Snapshot fallback shares this framed connection. Keep the allocation bound
/// aligned with `SnapshotTransfer` plus a small header allowance.
const MAX_FRAME_LEN: usize = 257 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum ReplicationFrame {
    Batch(LogBatch),
    CatchUp(CatchUpMessage),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SnapshotFrameHeader {
    request_id: u64,
    requester_sector_id: SectorId,
    owner_sector_id: SectorId,
    snapshot_sector_id: SectorId,
    snapshot_log_index: u64,
    owner_next_index: u64,
    snapshot_len: u32,
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
    #[error("tcp replication frame is malformed: {0}")]
    MalformedFrame(&'static str),
}

/// Plain TCP implementation of [`ReplicationTransport`] and
/// [`CatchUpTransport`].
///
/// Ordinary batches are published to every connected peer. Directed catch-up
/// requests and responses are queued only for the owning/requesting Sector, so
/// a large snapshot response is never cloned and deserialised by unrelated
/// peers.
#[derive(Debug, Clone)]
pub struct TcpReplicationTransport {
    local_addr: SocketAddr,
    inbound_batch_tx: broadcast::Sender<LogBatch>,
    inbound_catch_up_tx: broadcast::Sender<CatchUpMessage>,
    outbound_peers: Arc<RwLock<HashMap<SectorId, mpsc::Sender<ReplicationFrame>>>>,
}

impl TcpReplicationTransport {
    /// Bind a listener and start accepting peer connections.
    pub async fn bind<A: ToSocketAddrs>(addr: A) -> Result<Self, TcpReplicationError> {
        let listener = TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;
        let (inbound_batch_tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        let (inbound_catch_up_tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        let outbound_peers = Arc::new(RwLock::new(HashMap::new()));

        tokio::spawn(accept_loop(
            listener,
            inbound_batch_tx.clone(),
            inbound_catch_up_tx.clone(),
        ));

        Ok(Self {
            local_addr,
            inbound_batch_tx,
            inbound_catch_up_tx,
            outbound_peers,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Connect to one peer and register the connection for directed catch-up
    /// traffic to that peer's Sector.
    pub async fn connect_peer(
        &self,
        peer_sector_id: SectorId,
        addr: SocketAddr,
    ) -> Result<(), TcpReplicationError> {
        let stream = TcpStream::connect(addr).await?;
        let (outbound_tx, outbound_rx) = mpsc::channel(PEER_CHANNEL_CAPACITY);
        self.outbound_peers
            .write()
            .expect("replication peer map poisoned")
            .insert(peer_sector_id, outbound_tx.clone());
        spawn_connection(
            stream,
            self.inbound_batch_tx.clone(),
            self.inbound_catch_up_tx.clone(),
            outbound_rx,
            peer_sector_id,
            outbound_tx,
            self.outbound_peers.clone(),
        );
        Ok(())
    }

    fn send_to_peer(&self, peer_sector_id: SectorId, frame: ReplicationFrame) {
        let sender = self
            .outbound_peers
            .read()
            .expect("replication peer map poisoned")
            .get(&peer_sector_id)
            .cloned();
        if let Some(sender) = sender {
            let _ = sender.try_send(frame);
        }
    }
}

impl ReplicationTransport for TcpReplicationTransport {
    fn broadcast(&self, batch: LogBatch) {
        let frame = ReplicationFrame::Batch(batch);
        let senders: Vec<_> = self
            .outbound_peers
            .read()
            .expect("replication peer map poisoned")
            .values()
            .cloned()
            .collect();
        for sender in senders {
            let _ = sender.try_send(frame.clone());
        }
    }

    fn subscribe(&self) -> broadcast::Receiver<LogBatch> {
        self.inbound_batch_tx.subscribe()
    }
}

impl CatchUpTransport for TcpReplicationTransport {
    fn send_catch_up(&self, message: CatchUpMessage) {
        let target = match &message {
            CatchUpMessage::Request(request) => request.owner_sector_id,
            CatchUpMessage::Response(response) => response.requester_sector_id,
        };
        self.send_to_peer(target, ReplicationFrame::CatchUp(message));
    }

    fn subscribe_catch_up(&self) -> broadcast::Receiver<CatchUpMessage> {
        self.inbound_catch_up_tx.subscribe()
    }
}

async fn accept_loop(
    listener: TcpListener,
    inbound_batch_tx: broadcast::Sender<LogBatch>,
    inbound_catch_up_tx: broadcast::Sender<CatchUpMessage>,
) {
    loop {
        let Ok((stream, _addr)) = listener.accept().await else {
            break;
        };
        tokio::spawn(read_loop(
            stream,
            inbound_batch_tx.clone(),
            inbound_catch_up_tx.clone(),
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_connection(
    stream: TcpStream,
    inbound_batch_tx: broadcast::Sender<LogBatch>,
    inbound_catch_up_tx: broadcast::Sender<CatchUpMessage>,
    outbound_rx: mpsc::Receiver<ReplicationFrame>,
    peer_sector_id: SectorId,
    registered_sender: mpsc::Sender<ReplicationFrame>,
    outbound_peers: Arc<RwLock<HashMap<SectorId, mpsc::Sender<ReplicationFrame>>>>,
) {
    tokio::spawn(async move {
        let (reader, writer) = stream.into_split();
        tokio::select! {
            _ = read_loop(reader, inbound_batch_tx, inbound_catch_up_tx) => {}
            _ = write_loop(writer, outbound_rx) => {}
        }

        let mut peers = outbound_peers
            .write()
            .expect("replication peer map poisoned");
        if peers
            .get(&peer_sector_id)
            .is_some_and(|current| current.same_channel(&registered_sender))
        {
            peers.remove(&peer_sector_id);
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

async fn write_loop<W>(mut writer: W, mut outbound_rx: mpsc::Receiver<ReplicationFrame>)
where
    W: AsyncWrite + Unpin,
{
    while let Some(frame) = outbound_rx.recv().await {
        if write_frame(&mut writer, &frame).await.is_err() {
            break;
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
    if len == 0 {
        return Err(TcpReplicationError::MalformedFrame("empty frame"));
    }

    let kind = reader.read_u8().await?;
    let remaining = len - 1;
    match kind {
        FRAME_KIND_POSTCARD => {
            let mut payload = vec![0_u8; remaining];
            reader.read_exact(&mut payload).await?;
            Ok(Some(postcard::from_bytes(&payload)?))
        }
        FRAME_KIND_SNAPSHOT => read_snapshot_frame(reader, remaining).await.map(Some),
        _ => Err(TcpReplicationError::MalformedFrame("unknown frame kind")),
    }
}

async fn read_snapshot_frame<R>(
    reader: &mut R,
    remaining: usize,
) -> Result<ReplicationFrame, TcpReplicationError>
where
    R: AsyncRead + Unpin,
{
    if remaining < 4 {
        return Err(TcpReplicationError::MalformedFrame(
            "snapshot frame missing header length",
        ));
    }
    let header_len = reader.read_u32_le().await? as usize;
    let remaining = remaining - 4;
    if header_len > remaining {
        return Err(TcpReplicationError::MalformedFrame(
            "snapshot header exceeds frame",
        ));
    }

    let mut header_bytes = vec![0_u8; header_len];
    reader.read_exact(&mut header_bytes).await?;
    let header: SnapshotFrameHeader = postcard::from_bytes(&header_bytes)?;
    let snapshot_len = remaining - header_len;
    if snapshot_len != header.snapshot_len as usize {
        return Err(TcpReplicationError::MalformedFrame(
            "snapshot length does not match header",
        ));
    }

    let mut bytes = vec![0_u8; snapshot_len];
    reader.read_exact(&mut bytes).await?;
    Ok(ReplicationFrame::CatchUp(CatchUpMessage::Response(
        CatchUpResponse {
            request_id: header.request_id,
            requester_sector_id: header.requester_sector_id,
            owner_sector_id: header.owner_sector_id,
            payload: CatchUpPayload::Snapshot {
                snapshot: ReplicaSnapshot::new(
                    header.snapshot_sector_id,
                    header.snapshot_log_index,
                    bytes,
                ),
                owner_next_index: header.owner_next_index,
            },
        },
    )))
}

async fn write_frame<W>(writer: &mut W, frame: &ReplicationFrame) -> Result<(), TcpReplicationError>
where
    W: AsyncWrite + Unpin,
{
    if let ReplicationFrame::CatchUp(CatchUpMessage::Response(CatchUpResponse {
        request_id,
        requester_sector_id,
        owner_sector_id,
        payload:
            CatchUpPayload::Snapshot {
                snapshot,
                owner_next_index,
            },
    })) = frame
    {
        return write_snapshot_frame(
            writer,
            SnapshotFrameHeader {
                request_id: *request_id,
                requester_sector_id: *requester_sector_id,
                owner_sector_id: *owner_sector_id,
                snapshot_sector_id: snapshot.sector_id,
                snapshot_log_index: snapshot.log_index,
                owner_next_index: *owner_next_index,
                snapshot_len: snapshot.bytes.len().try_into().map_err(|_| {
                    TcpReplicationError::FrameTooLarge {
                        actual: snapshot.bytes.len(),
                        max: u32::MAX as usize,
                    }
                })?,
            },
            snapshot.bytes.as_slice(),
        )
        .await;
    }

    let payload = postcard::to_stdvec(frame)?;
    let len = 1 + payload.len();
    if len > MAX_FRAME_LEN {
        return Err(TcpReplicationError::FrameTooLarge {
            actual: len,
            max: MAX_FRAME_LEN,
        });
    }

    writer.write_all(&(len as u32).to_le_bytes()).await?;
    writer.write_u8(FRAME_KIND_POSTCARD).await?;
    writer.write_all(&payload).await?;
    writer.flush().await?;
    Ok(())
}

async fn write_snapshot_frame<W>(
    writer: &mut W,
    header: SnapshotFrameHeader,
    snapshot_bytes: &[u8],
) -> Result<(), TcpReplicationError>
where
    W: AsyncWrite + Unpin,
{
    let header_bytes = postcard::to_stdvec(&header)?;
    let len = 1 + 4 + header_bytes.len() + snapshot_bytes.len();
    if len > MAX_FRAME_LEN || len > u32::MAX as usize {
        return Err(TcpReplicationError::FrameTooLarge {
            actual: len,
            max: MAX_FRAME_LEN.min(u32::MAX as usize),
        });
    }

    writer.write_all(&(len as u32).to_le_bytes()).await?;
    writer.write_u8(FRAME_KIND_SNAPSHOT).await?;
    writer
        .write_all(&(header_bytes.len() as u32).to_le_bytes())
        .await?;
    writer.write_all(&header_bytes).await?;
    writer.write_all(snapshot_bytes).await?;
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
    async fn snapshot_frame_streams_raw_bytes_and_round_trips() {
        let bytes = vec![7; 64 * 1024];
        let frame = ReplicationFrame::CatchUp(CatchUpMessage::Response(CatchUpResponse {
            request_id: 11,
            requester_sector_id: SectorId(2),
            owner_sector_id: SectorId(1),
            payload: CatchUpPayload::Snapshot {
                snapshot: ReplicaSnapshot::new(SectorId(1), 40, bytes),
                owner_next_index: 44,
            },
        }));
        let (mut client, mut server) = duplex(128 * 1024);
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
    async fn tcp_transport_delivers_batches_and_directed_catch_up_messages() {
        let a = TcpReplicationTransport::bind("127.0.0.1:0").await.unwrap();
        let b = TcpReplicationTransport::bind("127.0.0.1:0").await.unwrap();
        let mut batch_rx = b.subscribe();
        let mut catch_up_rx = b.subscribe_catch_up();

        a.connect_peer(SectorId(2), b.local_addr()).await.unwrap();

        let batch = make_batch(SectorId(1), 5, 3);
        a.broadcast(batch.clone());
        assert_eq!(batch_rx.recv().await.unwrap(), batch);

        let message = CatchUpMessage::Request(CatchUpRequest {
            request_id: 9,
            requester_sector_id: SectorId(1),
            owner_sector_id: SectorId(2),
            from_index: 5,
            max_events: 32,
        });
        a.send_catch_up(message.clone());
        assert_eq!(catch_up_rx.recv().await.unwrap(), message);
    }

    #[tokio::test]
    async fn directed_catch_up_is_not_sent_to_unrelated_peers() {
        let a = TcpReplicationTransport::bind("127.0.0.1:0").await.unwrap();
        let b = TcpReplicationTransport::bind("127.0.0.1:0").await.unwrap();
        let c = TcpReplicationTransport::bind("127.0.0.1:0").await.unwrap();
        let mut b_rx = b.subscribe_catch_up();
        let mut c_rx = c.subscribe_catch_up();

        a.connect_peer(SectorId(2), b.local_addr()).await.unwrap();
        a.connect_peer(SectorId(3), c.local_addr()).await.unwrap();

        a.send_catch_up(CatchUpMessage::Request(CatchUpRequest {
            request_id: 10,
            requester_sector_id: SectorId(1),
            owner_sector_id: SectorId(2),
            from_index: 0,
            max_events: 32,
        }));

        assert!(matches!(
            b_rx.recv().await.unwrap(),
            CatchUpMessage::Request(_)
        ));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(25), c_rx.recv())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_batch_received_over_the_wire_is_ingested_into_a_replica() {
        let owner = TcpReplicationTransport::bind("127.0.0.1:0").await.unwrap();
        let peer = TcpReplicationTransport::bind("127.0.0.1:0").await.unwrap();
        let mut rx = peer.subscribe();
        owner
            .connect_peer(SectorId(2), peer.local_addr())
            .await
            .unwrap();
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
