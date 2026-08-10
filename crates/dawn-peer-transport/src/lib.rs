//! Shared peer transport for the sector cluster.
//!
//! This crate owns the physical lifecycle and framing of peer connections. It
//! deliberately does not know about Raft, event logs, or snapshots. Callers
//! provide opaque metadata and payload bytes and select a versioned traffic
//! class. Control and bulk have separate listeners, queues, and TCP
//! connections, so a snapshot cannot consume the queue used by heartbeats.
//!
//! The initial milestone is plaintext LAN transport. The handshake binds a
//! configured node and sector identity to each connection; it is not a crypto
//! authentication mechanism. TLS or another authenticated channel can be
//! inserted below this API later.
//!
//! ```
//! use dawn_core::{NodeId, SectorId};
//! use dawn_peer_transport::{PeerCapabilities, PeerEndpoint, PeerIdentity, PeerTransportConfig};
//!
//! let config = PeerTransportConfig {
//!     local_identity: PeerIdentity { node_id: NodeId(1), sector_id: SectorId(1) },
//!     control_addr: "127.0.0.1:7900".parse().unwrap(),
//!     bulk_addr: "127.0.0.1:7910".parse().unwrap(),
//!     capabilities: PeerCapabilities::NONE,
//!     peers: vec![PeerEndpoint {
//!         identity: PeerIdentity { node_id: NodeId(2), sector_id: SectorId(2) },
//!         control_addr: "127.0.0.1:7901".parse().unwrap(),
//!         bulk_addr: "127.0.0.1:7911".parse().unwrap(),
//!     }],
//! };
//! assert!(config.validate().is_ok());
//! ```

use dawn_core::{NodeId, SectorId};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    io,
    net::SocketAddr,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, RwLock,
    },
    time::Duration,
};
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{broadcast, mpsc},
    time::sleep,
};

pub const PEER_PROTOCOL_VERSION: u16 = 1;
pub const MAX_CONTROL_FRAME_BYTES: usize = 1024 * 1024;
pub const MAX_BULK_FRAME_BYTES: usize = 257 * 1024 * 1024;
const MAX_HANDSHAKE_BYTES: usize = 4096;
const FRAME_CHECKSUM_BYTES: usize = 8;
const CONTROL_QUEUE_CAPACITY: usize = 256;
const BULK_QUEUE_CAPACITY: usize = 2;
const INBOUND_CAPACITY: usize = 10_000;
const RECONNECT_DELAY: Duration = Duration::from_millis(100);

/// Capability bits advertised by a peer during the versioned handshake.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerCapabilities(pub u64);

impl PeerCapabilities {
    pub const NONE: Self = Self(0);
    pub const DURABILITY_STAGING: Self = Self(1 << 0);
    pub const RECOVERY_CATCH_UP: Self = Self(1 << 1);
    pub const REPOSITORY_CATCH_UP: Self = Self(1 << 2);

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }
}

impl std::ops::BitOr for PeerCapabilities {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

/// Identity that a peer must present on every channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PeerIdentity {
    pub node_id: NodeId,
    pub sector_id: SectorId,
}

/// The two independently scheduled physical channels for one peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PeerChannelKind {
    Control,
    Bulk,
}

impl PeerChannelKind {
    fn max_frame_bytes(self) -> usize {
        match self {
            Self::Control => MAX_CONTROL_FRAME_BYTES,
            Self::Bulk => MAX_BULK_FRAME_BYTES,
        }
    }

    fn queue_capacity(self) -> usize {
        match self {
            Self::Control => CONTROL_QUEUE_CAPACITY,
            Self::Bulk => BULK_QUEUE_CAPACITY,
        }
    }
}

/// A configured peer and its separate control/bulk endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerEndpoint {
    pub identity: PeerIdentity,
    pub control_addr: SocketAddr,
    pub bulk_addr: SocketAddr,
}

/// Configuration for one transport instance.
#[derive(Debug, Clone)]
pub struct PeerTransportConfig {
    pub local_identity: PeerIdentity,
    pub control_addr: SocketAddr,
    pub bulk_addr: SocketAddr,
    pub capabilities: PeerCapabilities,
    pub peers: Vec<PeerEndpoint>,
}

impl PeerTransportConfig {
    pub fn validate(&self) -> Result<(), PeerTransportError> {
        if self.control_addr == self.bulk_addr && self.control_addr.port() != 0 {
            return Err(PeerTransportError::InvalidConfig(
                "control and bulk listeners must be different",
            ));
        }

        let mut ids = HashSet::new();
        let mut sectors = HashSet::new();
        for peer in &self.peers {
            if peer.identity.node_id == self.local_identity.node_id
                || peer.identity == self.local_identity
                || peer.identity.sector_id == self.local_identity.sector_id
            {
                return Err(PeerTransportError::InvalidConfig(
                    "local node and sector identities must not be listed as peers",
                ));
            }
            if !ids.insert(peer.identity.node_id) {
                return Err(PeerTransportError::InvalidConfig(
                    "peer node identities must be unique",
                ));
            }
            if peer.control_addr == peer.bulk_addr {
                return Err(PeerTransportError::InvalidConfig(
                    "peer control and bulk endpoints must be different",
                ));
            }
            if !sectors.insert(peer.identity.sector_id) {
                return Err(PeerTransportError::InvalidConfig(
                    "peer sector identities must be unique",
                ));
            }
        }
        Ok(())
    }
}

/// Message class carried by a frame. Domain crates own the bytes inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum PeerMessageKind {
    RaftControl = 1,
    DurabilityControl = 2,
    CatchUpControl = 3,
    RecoveryRange = 4,
    Snapshot = 5,
    RepositoryCatchUp = 6,
}

impl PeerMessageKind {
    fn channel(self) -> PeerChannelKind {
        match self {
            Self::RecoveryRange | Self::Snapshot | Self::RepositoryCatchUp => PeerChannelKind::Bulk,
            Self::RaftControl | Self::DurabilityControl | Self::CatchUpControl => {
                PeerChannelKind::Control
            }
        }
    }

    fn from_byte(value: u8) -> Result<Self, PeerTransportError> {
        match value {
            1 => Ok(Self::RaftControl),
            2 => Ok(Self::DurabilityControl),
            3 => Ok(Self::CatchUpControl),
            4 => Ok(Self::RecoveryRange),
            5 => Ok(Self::Snapshot),
            6 => Ok(Self::RepositoryCatchUp),
            _ => Err(PeerTransportError::MalformedFrame("unknown message kind")),
        }
    }
}

/// A decoded frame delivered by the shared transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerFrame {
    pub from: PeerIdentity,
    pub kind: PeerMessageKind,
    pub metadata: Vec<u8>,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum HandshakeChannel {
    Control,
    Bulk,
}

impl From<PeerChannelKind> for HandshakeChannel {
    fn from(value: PeerChannelKind) -> Self {
        match value {
            PeerChannelKind::Control => Self::Control,
            PeerChannelKind::Bulk => Self::Bulk,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Handshake {
    version: u16,
    identity: PeerIdentity,
    channel: HandshakeChannel,
    capabilities: PeerCapabilities,
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PeerTransportError {
    #[error("peer transport I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("peer transport postcard error: {0}")]
    Postcard(#[from] postcard::Error),
    #[error("peer transport frame is too large: {actual} bytes > {max} bytes")]
    FrameTooLarge { actual: usize, max: usize },
    #[error("peer transport frame is malformed: {0}")]
    MalformedFrame(&'static str),
    #[error("invalid peer transport configuration: {0}")]
    InvalidConfig(&'static str),
    #[error("peer identity mismatch: expected {expected:?}, received {received:?}")]
    IdentityMismatch {
        expected: PeerIdentity,
        received: PeerIdentity,
    },
    #[error("unsupported peer protocol version {0}")]
    UnsupportedVersion(u16),
    #[error("peer channel mismatch")]
    ChannelMismatch,
}

/// Error returned when a bounded outbound queue cannot accept a frame.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum PeerSendError {
    #[error("peer {0:?} is not configured")]
    UnknownPeer(NodeId),
    #[error("peer {0:?} is not connected")]
    NotConnected(NodeId),
    #[error("{channel:?} queue for peer {peer:?} is full")]
    QueueFull {
        peer: NodeId,
        channel: PeerChannelKind,
    },
    #[error("frame is too large: {actual} bytes > {max} bytes")]
    FrameTooLarge { actual: usize, max: usize },
}

#[derive(Debug, Default)]
pub struct PeerTransportMetrics {
    frames_sent: AtomicU64,
    frames_received: AtomicU64,
    rejected_handshakes: AtomicU64,
    queue_full: AtomicU64,
    reconnects: AtomicU64,
}

impl PeerTransportMetrics {
    pub fn frames_sent(&self) -> u64 {
        self.frames_sent.load(Ordering::Relaxed)
    }
    pub fn frames_received(&self) -> u64 {
        self.frames_received.load(Ordering::Relaxed)
    }
    pub fn rejected_handshakes(&self) -> u64 {
        self.rejected_handshakes.load(Ordering::Relaxed)
    }
    pub fn queue_full(&self) -> u64 {
        self.queue_full.load(Ordering::Relaxed)
    }
    pub fn reconnects(&self) -> u64 {
        self.reconnects.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Clone)]
struct OutboundFrame {
    kind: PeerMessageKind,
    metadata: Vec<u8>,
    payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ConnectionKey {
    node_id: NodeId,
    channel: PeerChannelKind,
}

#[derive(Debug)]
struct Inner {
    local: PeerIdentity,
    capabilities: PeerCapabilities,
    peers: HashMap<NodeId, PeerEndpoint>,
    senders: RwLock<HashMap<ConnectionKey, mpsc::Sender<OutboundFrame>>>,
    inbound: broadcast::Sender<PeerFrame>,
    metrics: PeerTransportMetrics,
}

/// Shared peer transport. Cloning it only clones handles to the same listeners
/// and bounded connection queues.
#[derive(Debug, Clone)]
pub struct PeerTransport {
    inner: Arc<Inner>,
    control_addr: SocketAddr,
    bulk_addr: SocketAddr,
}

impl PeerTransport {
    pub async fn bind(config: PeerTransportConfig) -> Result<Self, PeerTransportError> {
        config.validate()?;
        let control_listener = TcpListener::bind(config.control_addr).await?;
        let bulk_listener = TcpListener::bind(config.bulk_addr).await?;
        let control_addr = control_listener.local_addr()?;
        let bulk_addr = bulk_listener.local_addr()?;
        let (inbound, _) = broadcast::channel(INBOUND_CAPACITY);
        let peers = config
            .peers
            .iter()
            .copied()
            .map(|peer| (peer.identity.node_id, peer))
            .collect::<HashMap<_, _>>();
        let transport = Self {
            inner: Arc::new(Inner {
                local: config.local_identity,
                capabilities: config.capabilities,
                peers,
                senders: RwLock::new(HashMap::new()),
                inbound,
                metrics: PeerTransportMetrics::default(),
            }),
            control_addr,
            bulk_addr,
        };

        spawn_accept_loop(
            control_listener,
            PeerChannelKind::Control,
            transport.inner.clone(),
        );
        spawn_accept_loop(
            bulk_listener,
            PeerChannelKind::Bulk,
            transport.inner.clone(),
        );

        for peer in config.peers {
            // The lower node owns dialing. The higher node only accepts. This
            // removes duplicate connections without an out-of-band election.
            if config.local_identity.node_id.0 < peer.identity.node_id.0 {
                for channel in [PeerChannelKind::Control, PeerChannelKind::Bulk] {
                    spawn_outbound_loop(peer, channel, transport.inner.clone());
                }
            }
        }
        Ok(transport)
    }

    pub fn local_identity(&self) -> PeerIdentity {
        self.inner.local
    }
    pub fn control_addr(&self) -> SocketAddr {
        self.control_addr
    }
    pub fn bulk_addr(&self) -> SocketAddr {
        self.bulk_addr
    }
    pub fn subscribe(&self) -> broadcast::Receiver<PeerFrame> {
        self.inner.inbound.subscribe()
    }
    pub fn metrics(&self) -> &PeerTransportMetrics {
        &self.inner.metrics
    }
    pub fn peer_identities(&self) -> impl Iterator<Item = PeerIdentity> + '_ {
        self.inner.peers.values().map(|peer| peer.identity)
    }
    pub fn peer_for_sector(&self, sector_id: SectorId) -> Option<NodeId> {
        self.inner
            .peers
            .values()
            .find(|peer| peer.identity.sector_id == sector_id)
            .map(|peer| peer.identity.node_id)
    }

    pub fn send(
        &self,
        peer: NodeId,
        kind: PeerMessageKind,
        payload: Vec<u8>,
    ) -> Result<(), PeerSendError> {
        self.send_split(peer, kind, Vec::new(), payload)
    }

    pub fn send_split(
        &self,
        peer: NodeId,
        kind: PeerMessageKind,
        metadata: Vec<u8>,
        payload: Vec<u8>,
    ) -> Result<(), PeerSendError> {
        if !self.inner.peers.contains_key(&peer) {
            return Err(PeerSendError::UnknownPeer(peer));
        }
        let channel = kind.channel();
        let actual = 1usize
            .saturating_add(4)
            .saturating_add(metadata.len())
            .saturating_add(payload.len())
            .saturating_add(FRAME_CHECKSUM_BYTES);
        let max = channel.max_frame_bytes();
        if actual > max {
            return Err(PeerSendError::FrameTooLarge { actual, max });
        }
        let key = ConnectionKey {
            node_id: peer,
            channel,
        };
        let sender = self
            .inner
            .senders
            .read()
            .expect("peer sender map poisoned")
            .get(&key)
            .cloned()
            .ok_or(PeerSendError::NotConnected(peer))?;
        sender
            .try_send(OutboundFrame {
                kind,
                metadata,
                payload,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => {
                    self.inner
                        .metrics
                        .queue_full
                        .fetch_add(1, Ordering::Relaxed);
                    PeerSendError::QueueFull { peer, channel }
                }
                mpsc::error::TrySendError::Closed(_) => PeerSendError::NotConnected(peer),
            })
    }
}

fn spawn_accept_loop(listener: TcpListener, channel: PeerChannelKind, inner: Arc<Inner>) {
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let inner = inner.clone();
            tokio::spawn(async move {
                if run_inbound(stream, channel, inner.clone()).await.is_err() {
                    inner
                        .metrics
                        .rejected_handshakes
                        .fetch_add(1, Ordering::Relaxed);
                }
            });
        }
    });
}

fn spawn_outbound_loop(peer: PeerEndpoint, channel: PeerChannelKind, inner: Arc<Inner>) {
    tokio::spawn(async move {
        let (sender, mut receiver) = mpsc::channel(channel.queue_capacity());
        let key = ConnectionKey {
            node_id: peer.identity.node_id,
            channel,
        };
        inner
            .senders
            .write()
            .expect("peer sender map poisoned")
            .insert(key, sender.clone());
        loop {
            let addr = match channel {
                PeerChannelKind::Control => peer.control_addr,
                PeerChannelKind::Bulk => peer.bulk_addr,
            };
            match TcpStream::connect(addr).await {
                Ok(stream) => {
                    let mut stream = stream;
                    let handshake_ok = async {
                        write_handshake(&mut stream, channel, &inner).await?;
                        let remote = read_handshake(&mut stream, channel).await?;
                        if remote.identity != peer.identity {
                            return Err(PeerTransportError::IdentityMismatch {
                                expected: peer.identity,
                                received: remote.identity,
                            });
                        }
                        Ok::<(), PeerTransportError>(())
                    }
                    .await
                    .is_ok();
                    if !handshake_ok {
                        sleep(RECONNECT_DELAY).await;
                        continue;
                    }
                    let (reader, writer) = stream.into_split();
                    let completed = tokio::select! {
                        result = read_loop(reader, channel, peer.identity, inner.clone()) => result,
                        result = write_loop(writer, &mut receiver, channel, &inner.metrics) => result,
                    };
                    if completed.is_err() {
                        inner.metrics.reconnects.fetch_add(1, Ordering::Relaxed);
                    }
                    sleep(RECONNECT_DELAY).await;
                }
                Err(_) => {
                    inner.metrics.reconnects.fetch_add(1, Ordering::Relaxed);
                    sleep(RECONNECT_DELAY).await;
                }
            }
        }
    });
}

async fn run_inbound(
    stream: TcpStream,
    channel: PeerChannelKind,
    inner: Arc<Inner>,
) -> Result<(), PeerTransportError> {
    let mut stream = stream;
    let handshake = read_handshake(&mut stream, channel).await?;
    let expected = inner
        .peers
        .get(&handshake.identity.node_id)
        .ok_or(PeerTransportError::IdentityMismatch {
            expected: inner.local,
            received: handshake.identity,
        })?
        .identity;
    if expected != handshake.identity {
        return Err(PeerTransportError::IdentityMismatch {
            expected,
            received: handshake.identity,
        });
    }
    write_handshake(&mut stream, channel, &inner).await?;
    let (reader, writer) = stream.into_split();
    let (sender, mut receiver) = mpsc::channel(channel.queue_capacity());
    let key = ConnectionKey {
        node_id: handshake.identity.node_id,
        channel,
    };
    inner
        .senders
        .write()
        .expect("peer sender map poisoned")
        .insert(key, sender.clone());
    let result = tokio::select! {
        result = read_loop(reader, channel, handshake.identity, inner.clone()) => result,
        result = write_loop(writer, &mut receiver, channel, &inner.metrics) => result,
    };
    remove_sender(&inner, key, &sender);
    result
}

fn remove_sender(inner: &Inner, key: ConnectionKey, sender: &mpsc::Sender<OutboundFrame>) {
    let mut senders = inner.senders.write().expect("peer sender map poisoned");
    if senders
        .get(&key)
        .is_some_and(|current| current.same_channel(sender))
    {
        senders.remove(&key);
    }
}

async fn read_loop<R>(
    mut reader: R,
    channel: PeerChannelKind,
    from: PeerIdentity,
    inner: Arc<Inner>,
) -> Result<(), PeerTransportError>
where
    R: AsyncRead + Unpin,
{
    loop {
        let Some((kind, metadata, payload)) = read_frame(&mut reader, channel).await? else {
            return Ok(());
        };
        let _ = inner.inbound.send(PeerFrame {
            from,
            kind,
            metadata,
            payload,
        });
        inner
            .metrics
            .frames_received
            .fetch_add(1, Ordering::Relaxed);
    }
}

async fn write_loop<W>(
    mut writer: W,
    receiver: &mut mpsc::Receiver<OutboundFrame>,
    channel: PeerChannelKind,
    metrics: &PeerTransportMetrics,
) -> Result<(), PeerTransportError>
where
    W: AsyncWrite + Unpin,
{
    while let Some(frame) = receiver.recv().await {
        write_frame(&mut writer, channel, &frame).await?;
        metrics.frames_sent.fetch_add(1, Ordering::Relaxed);
    }
    Ok(())
}

async fn read_handshake<R>(
    reader: &mut R,
    channel: PeerChannelKind,
) -> Result<Handshake, PeerTransportError>
where
    R: AsyncRead + Unpin,
{
    let Some(bytes) = read_length_prefixed(reader, MAX_HANDSHAKE_BYTES).await? else {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "handshake ended").into());
    };
    let handshake: Handshake = postcard::from_bytes(&bytes)?;
    if handshake.version != PEER_PROTOCOL_VERSION {
        return Err(PeerTransportError::UnsupportedVersion(handshake.version));
    }
    if handshake.channel != channel.into() {
        return Err(PeerTransportError::ChannelMismatch);
    }
    Ok(handshake)
}

async fn write_handshake<W>(
    writer: &mut W,
    channel: PeerChannelKind,
    inner: &Inner,
) -> Result<(), PeerTransportError>
where
    W: AsyncWrite + Unpin,
{
    let handshake = Handshake {
        version: PEER_PROTOCOL_VERSION,
        identity: inner.local,
        channel: channel.into(),
        capabilities: inner.capabilities,
    };
    let bytes = postcard::to_stdvec(&handshake)?;
    write_length_prefixed(writer, &bytes, MAX_HANDSHAKE_BYTES).await
}

async fn read_frame<R>(
    reader: &mut R,
    channel: PeerChannelKind,
) -> Result<Option<(PeerMessageKind, Vec<u8>, Vec<u8>)>, PeerTransportError>
where
    R: AsyncRead + Unpin,
{
    let Some(body) = read_length_prefixed(reader, channel.max_frame_bytes()).await? else {
        return Ok(None);
    };
    if body.len() < 5 + FRAME_CHECKSUM_BYTES {
        return Err(PeerTransportError::MalformedFrame(
            "frame header is truncated",
        ));
    }
    let checksum_offset = body.len() - FRAME_CHECKSUM_BYTES;
    let expected_checksum = u64::from_le_bytes(
        body[checksum_offset..]
            .try_into()
            .expect("checksum has a fixed width"),
    );
    if checksum(&body[..checksum_offset]) != expected_checksum {
        return Err(PeerTransportError::MalformedFrame(
            "frame checksum mismatch",
        ));
    }
    let body = &body[..checksum_offset];
    let kind = PeerMessageKind::from_byte(body[0])?;
    if kind.channel() != channel {
        return Err(PeerTransportError::ChannelMismatch);
    }
    let metadata_len = u32::from_le_bytes(body[1..5].try_into().unwrap()) as usize;
    if metadata_len > body.len() - 5 {
        return Err(PeerTransportError::MalformedFrame(
            "metadata length exceeds frame",
        ));
    }
    let metadata_end = 5 + metadata_len;
    Ok(Some((
        kind,
        body[5..metadata_end].to_vec(),
        body[metadata_end..].to_vec(),
    )))
}

async fn write_frame<W>(
    writer: &mut W,
    channel: PeerChannelKind,
    frame: &OutboundFrame,
) -> Result<(), PeerTransportError>
where
    W: AsyncWrite + Unpin,
{
    let len = 1usize
        .saturating_add(4)
        .saturating_add(frame.metadata.len())
        .saturating_add(frame.payload.len())
        .saturating_add(FRAME_CHECKSUM_BYTES);
    if len > channel.max_frame_bytes() || len > u32::MAX as usize {
        return Err(PeerTransportError::FrameTooLarge {
            actual: len,
            max: channel.max_frame_bytes(),
        });
    }
    let mut body = Vec::with_capacity(len);
    body.push(frame.kind as u8);
    body.extend_from_slice(&(frame.metadata.len() as u32).to_le_bytes());
    body.extend_from_slice(&frame.metadata);
    body.extend_from_slice(&frame.payload);
    body.extend_from_slice(&checksum(&body).to_le_bytes());
    writer.write_all(&(len as u32).to_le_bytes()).await?;
    writer.write_all(&body).await?;
    writer.flush().await?;
    Ok(())
}

fn checksum(bytes: &[u8]) -> u64 {
    // FNV-1a is used only for corruption detection, not authentication.
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

async fn read_length_prefixed<R>(
    reader: &mut R,
    max: usize,
) -> Result<Option<Vec<u8>>, PeerTransportError>
where
    R: AsyncRead + Unpin,
{
    let mut len_buf = [0_u8; 4];
    if let Err(error) = reader.read_exact(&mut len_buf).await {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            return Ok(None);
        }
        return Err(error.into());
    }
    let len = u32::from_le_bytes(len_buf) as usize;
    if len == 0 {
        return Err(PeerTransportError::MalformedFrame(
            "empty length-prefixed payload",
        ));
    }
    if len > max {
        return Err(PeerTransportError::FrameTooLarge { actual: len, max });
    }
    let mut bytes = vec![0_u8; len];
    reader.read_exact(&mut bytes).await?;
    Ok(Some(bytes))
}

async fn write_length_prefixed<W>(
    writer: &mut W,
    bytes: &[u8],
    max: usize,
) -> Result<(), PeerTransportError>
where
    W: AsyncWrite + Unpin,
{
    if bytes.is_empty() || bytes.len() > max || bytes.len() > u32::MAX as usize {
        return Err(PeerTransportError::FrameTooLarge {
            actual: bytes.len(),
            max,
        });
    }
    writer
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .await?;
    writer.write_all(bytes).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::time::{sleep, timeout};

    fn identity(node: u8, sector: u8) -> PeerIdentity {
        PeerIdentity {
            node_id: NodeId(node),
            sector_id: SectorId(sector),
        }
    }

    async fn pair() -> (PeerTransport, PeerTransport) {
        let b = PeerTransport::bind(PeerTransportConfig {
            local_identity: identity(2, 2),
            control_addr: "127.0.0.1:0".parse().unwrap(),
            bulk_addr: "127.0.0.1:0".parse().unwrap(),
            capabilities: PeerCapabilities::RECOVERY_CATCH_UP,
            peers: vec![PeerEndpoint {
                identity: identity(1, 1),
                control_addr: "127.0.0.1:9".parse().unwrap(),
                bulk_addr: "127.0.0.1:10".parse().unwrap(),
            }],
        })
        .await
        .unwrap();
        let a = PeerTransport::bind(PeerTransportConfig {
            local_identity: identity(1, 1),
            control_addr: "127.0.0.1:0".parse().unwrap(),
            bulk_addr: "127.0.0.1:0".parse().unwrap(),
            capabilities: PeerCapabilities::RECOVERY_CATCH_UP,
            peers: vec![PeerEndpoint {
                identity: identity(2, 2),
                control_addr: b.control_addr(),
                bulk_addr: b.bulk_addr(),
            }],
        })
        .await
        .unwrap();
        (a, b)
    }

    async fn send_when_connected(
        transport: &PeerTransport,
        kind: PeerMessageKind,
        metadata: Vec<u8>,
        payload: Vec<u8>,
    ) {
        for _ in 0..100 {
            if transport
                .send_split(NodeId(2), kind, metadata.clone(), payload.clone())
                .is_ok()
            {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
        panic!("peer connection did not become ready");
    }

    #[tokio::test]
    async fn separate_channels_deliver_typed_frames() {
        let (a, b) = pair().await;
        let mut rx = b.subscribe();
        send_when_connected(&a, PeerMessageKind::RaftControl, vec![], vec![1, 2, 3]).await;
        send_when_connected(&a, PeerMessageKind::Snapshot, vec![9, 8], vec![7; 4096]).await;
        let first = timeout(Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.kind, PeerMessageKind::RaftControl);
        assert_eq!(first.payload, vec![1, 2, 3]);
        let second = timeout(Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.kind, PeerMessageKind::Snapshot);
        assert_eq!(second.metadata, vec![9, 8]);
        assert_eq!(second.payload.len(), 4096);
        assert_ne!(a.control_addr(), a.bulk_addr());
    }

    #[tokio::test]
    async fn corrupted_frame_checksum_is_rejected() {
        let (mut client, mut server) = tokio::io::duplex(256);
        let mut body = vec![PeerMessageKind::RaftControl as u8, 0, 0, 0, 0, 1, 2, 3];
        body.extend_from_slice(&0_u64.to_le_bytes());
        client
            .write_all(&(body.len() as u32).to_le_bytes())
            .await
            .unwrap();
        client.write_all(&body).await.unwrap();
        assert!(matches!(
            read_frame(&mut server, PeerChannelKind::Control).await,
            Err(PeerTransportError::MalformedFrame(
                "frame checksum mismatch"
            ))
        ));
    }

    #[test]
    fn validation_rejects_duplicate_peer_nodes() {
        let config = PeerTransportConfig {
            local_identity: identity(1, 1),
            control_addr: "127.0.0.1:1".parse().unwrap(),
            bulk_addr: "127.0.0.1:2".parse().unwrap(),
            capabilities: PeerCapabilities::NONE,
            peers: vec![
                PeerEndpoint {
                    identity: identity(2, 2),
                    control_addr: "127.0.0.1:3".parse().unwrap(),
                    bulk_addr: "127.0.0.1:4".parse().unwrap(),
                },
                PeerEndpoint {
                    identity: identity(2, 3),
                    control_addr: "127.0.0.1:5".parse().unwrap(),
                    bulk_addr: "127.0.0.1:6".parse().unwrap(),
                },
            ],
        };
        assert!(matches!(
            config.validate(),
            Err(PeerTransportError::InvalidConfig(_))
        ));
    }

    #[test]
    fn validation_rejects_reusing_local_node_or_sector_identity() {
        let config = PeerTransportConfig {
            local_identity: identity(1, 1),
            control_addr: "127.0.0.1:1".parse().unwrap(),
            bulk_addr: "127.0.0.1:2".parse().unwrap(),
            capabilities: PeerCapabilities::NONE,
            peers: vec![PeerEndpoint {
                identity: identity(1, 2),
                control_addr: "127.0.0.1:3".parse().unwrap(),
                bulk_addr: "127.0.0.1:4".parse().unwrap(),
            }],
        };
        assert!(matches!(
            config.validate(),
            Err(PeerTransportError::InvalidConfig(_))
        ));
    }
}
