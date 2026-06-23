//! TCP gossip transport for Sector-local log shipping (ADR-0027 / 8D-2c).
//!
//! Wire format: `[u32 little-endian payload length][postcard LogBatch payload]`.
//! The transport is plaintext by design for the Phase 8D LAN milestone.

use crate::{LogBatch, ReplicationTransport};
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
const MAX_FRAME_LEN: usize = 16 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum TcpReplicationError {
    #[error("tcp replication io error: {0}")]
    Io(#[from] io::Error),
    #[error("tcp replication postcard error: {0}")]
    Postcard(#[from] postcard::Error),
    #[error("tcp replication frame is too large: {actual} bytes > {max} bytes")]
    FrameTooLarge { actual: usize, max: usize },
}

/// Plain TCP implementation of [`ReplicationTransport`].
///
/// `broadcast()` publishes a `LogBatch` to every currently connected peer.
/// Incoming frames from peers are exposed through `subscribe()`.
#[derive(Clone)]
pub struct TcpReplicationTransport {
    local_addr: SocketAddr,
    inbound_tx: broadcast::Sender<LogBatch>,
    outbound_tx: broadcast::Sender<LogBatch>,
}

impl TcpReplicationTransport {
    /// Bind a listener and start accepting peer connections.
    pub async fn bind<A: ToSocketAddrs>(addr: A) -> Result<Self, TcpReplicationError> {
        let listener = TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;
        let (inbound_tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        let (outbound_tx, _) = broadcast::channel(CHANNEL_CAPACITY);

        tokio::spawn(accept_loop(
            listener,
            inbound_tx.clone(),
            outbound_tx.clone(),
        ));

        Ok(Self {
            local_addr,
            inbound_tx,
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
            self.inbound_tx.clone(),
            self.outbound_tx.subscribe(),
        );
        Ok(())
    }
}

impl ReplicationTransport for TcpReplicationTransport {
    fn broadcast(&self, batch: LogBatch) {
        let _ = self.outbound_tx.send(batch);
    }

    fn subscribe(&self) -> broadcast::Receiver<LogBatch> {
        self.inbound_tx.subscribe()
    }
}

async fn accept_loop(
    listener: TcpListener,
    inbound_tx: broadcast::Sender<LogBatch>,
    outbound_tx: broadcast::Sender<LogBatch>,
) {
    loop {
        let Ok((stream, _addr)) = listener.accept().await else {
            break;
        };
        spawn_connection(stream, inbound_tx.clone(), outbound_tx.subscribe());
    }
}

fn spawn_connection(
    stream: TcpStream,
    inbound_tx: broadcast::Sender<LogBatch>,
    outbound_rx: broadcast::Receiver<LogBatch>,
) {
    tokio::spawn(async move {
        let (reader, writer) = stream.into_split();
        tokio::select! {
            _ = read_loop(reader, inbound_tx) => {}
            _ = write_loop(writer, outbound_rx) => {}
        }
    });
}

async fn read_loop<R>(mut reader: R, inbound_tx: broadcast::Sender<LogBatch>)
where
    R: AsyncRead + Unpin,
{
    loop {
        match read_frame(&mut reader).await {
            Ok(Some(batch)) => {
                let _ = inbound_tx.send(batch);
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }
}

async fn write_loop<W>(mut writer: W, mut outbound_rx: broadcast::Receiver<LogBatch>)
where
    W: AsyncWrite + Unpin,
{
    loop {
        match outbound_rx.recv().await {
            Ok(batch) => {
                if write_frame(&mut writer, &batch).await.is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}

async fn read_frame<R>(reader: &mut R) -> Result<Option<LogBatch>, TcpReplicationError>
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

async fn write_frame<W>(writer: &mut W, batch: &LogBatch) -> Result<(), TcpReplicationError>
where
    W: AsyncWrite + Unpin,
{
    let payload = postcard::to_stdvec(batch)?;
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
        let batch = make_batch(SectorId(7), 42, 2);

        let write_batch = batch.clone();
        let writer = tokio::spawn(async move {
            write_frame(&mut client, &write_batch).await.unwrap();
        });

        let received = read_frame(&mut server).await.unwrap().unwrap();
        writer.await.unwrap();

        assert_eq!(received, batch);
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
    async fn tcp_transport_delivers_batch_to_connected_peer() {
        let a = TcpReplicationTransport::bind("127.0.0.1:0").await.unwrap();
        let b = TcpReplicationTransport::bind("127.0.0.1:0").await.unwrap();
        let mut rx = b.subscribe();

        a.connect_peer(b.local_addr()).await.unwrap();

        let batch = make_batch(SectorId(1), 5, 3);
        a.broadcast(batch.clone());

        let received = rx.recv().await.unwrap();
        assert_eq!(received, batch);
    }

    #[tokio::test]
    async fn a_batch_received_over_the_wire_is_ingested_into_a_replica() {
        // End-to-end consumer side (M-5): owner broadcasts, peer receives over
        // TCP and feeds the batch into its ReplicaSet, which retains the log.
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
