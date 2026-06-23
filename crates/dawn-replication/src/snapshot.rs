//! Snapshot transfer for far-behind replicas (ADR-0027 / 8D-2d).
//!
//! `dawn-replication` sits below `dawn-sector` in the dependency DAG (ADR-0027
//! §2) and therefore cannot reference `StateSnapshot` directly. The type is
//! provided by the caller as a generic parameter — postcard serialisation is
//! handled internally so the caller never touches raw bytes.
//!
//! # Wire format
//!
//! ```text
//! [u32 LE payload_length][payload_length bytes of raw snapshot]
//! ```
//!
//! The same 4-byte-prefix framing used by `TcpRaftTransport` and
//! `TcpReplicationTransport`. Maximum payload: 256 MiB.

use serde::{de::DeserializeOwned, Serialize};
use std::{
    io::{self, ErrorKind},
    net::SocketAddr,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

/// 256 MiB — generous for a full ECS snapshot of a busy sector.
const MAX_SNAPSHOT_LEN: usize = 256 * 1024 * 1024;

/// Transfers a single serialised `StateSnapshot` over a TCP connection.
///
/// One `SnapshotTransfer` per node: bind once, call `accept_one` whenever a
/// far-behind replica requests a snapshot catch-up.
pub struct SnapshotTransfer {
    listener: TcpListener,
    local_addr: SocketAddr,
}

impl SnapshotTransfer {
    /// Bind a TCP listener for incoming snapshot requests.
    pub async fn bind(addr: SocketAddr) -> io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;
        Ok(Self {
            listener,
            local_addr,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Accept one incoming snapshot and deserialise it as `S`.
    pub async fn accept_one<S: DeserializeOwned>(&self) -> io::Result<S> {
        let (mut stream, _peer) = self.listener.accept().await?;
        let bytes = read_snapshot(&mut stream).await?;
        postcard::from_bytes(&bytes)
            .map_err(|e| io::Error::new(ErrorKind::InvalidData, e.to_string()))
    }

    /// Serialise `snapshot` and send it to `addr`.
    pub async fn send<S: Serialize>(addr: SocketAddr, snapshot: &S) -> io::Result<()> {
        let bytes = postcard::to_stdvec(snapshot)
            .map_err(|e| io::Error::new(ErrorKind::InvalidInput, e.to_string()))?;
        let mut stream = TcpStream::connect(addr).await?;
        write_snapshot(&mut stream, &bytes).await
    }
}

// ── Frame I/O ─────────────────────────────────────────────────────────────────

async fn write_snapshot(stream: &mut TcpStream, bytes: &[u8]) -> io::Result<()> {
    if bytes.len() > MAX_SNAPSHOT_LEN {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            format!(
                "snapshot too large: {} bytes > {} bytes",
                bytes.len(),
                MAX_SNAPSHOT_LEN
            ),
        ));
    }
    stream
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .await?;
    stream.write_all(bytes).await?;
    stream.flush().await
}

async fn read_snapshot(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut len_buf = [0_u8; 4];
    stream.read_exact(&mut len_buf).await?;

    let len = u32::from_le_bytes(len_buf) as usize;
    if len > MAX_SNAPSHOT_LEN {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            format!(
                "snapshot too large: {} bytes > {} bytes",
                len, MAX_SNAPSHOT_LEN
            ),
        ));
    }

    let mut payload = vec![0_u8; len];
    stream.read_exact(&mut payload).await?;
    Ok(payload)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct FakeSnapshot {
        id: u32,
        label: String,
        data: Vec<u8>,
    }

    fn sample() -> FakeSnapshot {
        FakeSnapshot {
            id: 42,
            label: "sector-0".into(),
            data: (0..128u8).collect(),
        }
    }

    // ── End-to-end round-trip ─────────────────────────────────────────────────

    #[tokio::test]
    async fn send_and_accept_one_round_trip() {
        let transfer = SnapshotTransfer::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = transfer.local_addr();
        let snap = sample();

        let server =
            tokio::spawn(async move { transfer.accept_one::<FakeSnapshot>().await.unwrap() });

        SnapshotTransfer::send(addr, &snap).await.unwrap();

        assert_eq!(server.await.unwrap(), snap);
    }

    // ── Oversized frame is rejected by receiver ───────────────────────────────

    #[tokio::test]
    async fn oversized_frame_rejected_by_receiver() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_snapshot(&mut stream).await
        });

        let mut client = TcpStream::connect(addr).await.unwrap();
        client
            .write_all(&((MAX_SNAPSHOT_LEN as u32) + 1).to_le_bytes())
            .await
            .unwrap();
        client.flush().await.unwrap();
        drop(client);

        let err = server.await.unwrap().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);
    }
}
