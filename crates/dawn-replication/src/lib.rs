//! # dawn-replication
//!
//! Sector-local event log replication (ADR-0021, ADR-0027).
//!
//! Strategy: gossip-based append-log shipping, not CRDT / LWW. Single ownership
//! means there are no concurrent writes to merge. Events are idempotent
//! (INV-004) and ordered by logical tick (INV-005), so receivers can detect
//! duplicate, overlapping, or missing log ranges by index.
//!
//! ## Current scope (8D-2a / 8D-2b / 8D-2c)
//!
//! - `LogBatch`: the unit of sector-local append-log shipping.
//! - `ReplicationTransport`: wire-format-agnostic ordinary log transport.
//! - `CatchUpTransport`: bounded suffix/snapshot recovery control traffic.
//! - `InMemoryReplicationBus`: single-process implementation for tests and the
//!   existing multi-node bench. This replaces `dawn_actor::ReplicationBus`.
//! - `AntiEntropy`: request missing events by log index range.
//! - `CatchUpManager`: owns gap detection, suffix requests, compacted-prefix
//!   snapshot fallback, bounded retries, transient-failure classification,
//!   logical-tick cooldowns, and restart from the current replica cursor.
//! - `PeerReplicationTransport`: adapter over `dawn-peer-transport`; recovery
//!   ranges and catch-up control use its bounded control channel, while
//!   snapshots and repository catch-up use its bounded bulk channel.
//!
//! ## Consumer side
//!
//! - `ReplicaSet`: holds a gap-checked, idempotent, ordered recovery replica of
//!   each foreign Sector. A replica may contain an opaque snapshot plus the
//!   retained event suffix after that snapshot boundary.
//! - Foreign replicas are recovery data only. This crate never applies them to
//!   the live local world.
//!
//! ## Example
//!
//! ```
//! use dawn_core::{AbsolutePosition, DomainEvent, NodeId, SectorId, ShipId, ShipTypeId, Tick};
//! use dawn_core::events::ShipSpawned;
//! use dawn_replication::{Ingest, LogBatch, ReplicaSet};
//!
//! let ship_id = ShipId::new(NodeId(1), 1);
//! let event = DomainEvent::ShipSpawned(ShipSpawned {
//!     ship_id,
//!     sector_id: SectorId(0),
//!     initial_position: AbsolutePosition::ORIGIN,
//!     ship_type_id: ShipTypeId(1),
//!     tick: Tick::ZERO,
//! });
//! let batch = LogBatch::new(SectorId(0), 0, vec![event]);
//!
//! let mut replica = ReplicaSet::new(128);
//! assert!(matches!(replica.ingest(&batch), Ingest::Applied { applied: 1, next_index: 1, .. }));
//! assert_eq!(replica.next_index(SectorId(0)), 1);
//! ```

// Rust API Guidelines C-DEBUG: catch new pub types that forget to derive
// Debug at compile time instead of relying on periodic audits (see #83).
#![warn(missing_debug_implementations)]

pub mod anti_entropy;
pub mod bus;
pub mod catch_up;
pub mod outbound;
pub mod peer_transport;
pub mod replica;

use dawn_core::{DomainEvent, SectorId};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

pub use anti_entropy::{AntiEntropy, BatchApplyPlan, MissingLogRequest};
pub use bus::{BusMessage, InMemoryReplicationBus};
pub use catch_up::{
    CatchUpConfig, CatchUpEvent, CatchUpFailure, CatchUpFailureKind, CatchUpManager,
    CatchUpMessage, CatchUpPayload, CatchUpRequest, CatchUpResponse, CatchUpStep, CatchUpTransport,
    CatchUpUnavailable,
};
pub use outbound::OutboundLogPublisher;
pub use peer_transport::{DurabilitySendError, PeerReplicationTransport};
pub use replica::{Ingest, ReplicaSet, ReplicaSnapshot, SnapshotInstall};

/// A single gossip payload from a Sector owner's append-only log.
///
/// `from_index` is the first log index represented by `events`. Receivers use
/// `(sector_id, from_index)` to detect gaps and to drop duplicate deliveries
/// idempotently in the anti-entropy step (ADR-0027 / 8D-2b).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogBatch {
    pub sector_id: SectorId,
    pub from_index: u64,
    pub events: Vec<DomainEvent>,
}

impl LogBatch {
    pub fn new(sector_id: SectorId, from_index: u64, events: Vec<DomainEvent>) -> Self {
        Self {
            sector_id,
            from_index,
            events,
        }
    }

    pub fn next_index(&self) -> u64 {
        self.from_index + self.events.len() as u64
    }
}

/// Wire-format-agnostic ordinary log-gossip transport.
///
/// The peer transport implements this interface with bounded, length-prefixed
/// postcard frames; the in-memory implementation keeps tests single-process.
/// Catch-up control messages use the separate [`CatchUpTransport`] interface.
pub trait ReplicationTransport: Send + Sync {
    /// Send a batch of events to interested peers.
    fn broadcast(&self, batch: LogBatch);

    /// Subscribe to incoming batches from peers.
    fn subscribe(&self) -> broadcast::Receiver<LogBatch>;
}
