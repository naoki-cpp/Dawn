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
//! - `ReplicationTransport`: wire-format-agnostic transport interface.
//! - `InMemoryReplicationBus`: single-process implementation for tests and the
//!   existing multi-node bench. This replaces `dawn_actor::ReplicationBus`.
//! - `AntiEntropy`: request missing events by log index range.
//! - `TcpReplicationTransport`: LAN plaintext transport using length-prefixed
//!   postcard frames.
//!
//! ## (8D-2d)
//!
//! - `SnapshotTransfer`: catch up far-behind replicas via snapshot (raw-bytes
//!   TCP transfer; caller handles postcard serialisation).
//!
//! ## Consumer side
//!
//! - `ReplicaSet`: holds a gap-checked, idempotent, ordered replica of each
//!   foreign Sector's log, fed by gossiped `LogBatch`es via `AntiEntropy`.
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
pub mod outbound;
pub mod replica;
pub mod snapshot;
pub mod tcp;

use dawn_core::{DomainEvent, SectorId};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

pub use anti_entropy::{AntiEntropy, BatchApplyPlan, MissingLogRequest};
pub use bus::{BusMessage, InMemoryReplicationBus};
pub use outbound::OutboundLogPublisher;
pub use replica::{Ingest, ReplicaSet};
pub use snapshot::SnapshotTransfer;
pub use tcp::{TcpReplicationError, TcpReplicationTransport};

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

/// Wire-format-agnostic replication transport.
///
/// TCP gossip (8D-2c) implements this same interface with length-prefixed
/// postcard frames; the in-memory implementation keeps tests single-process.
pub trait ReplicationTransport: Send + Sync {
    /// Send a batch of events to interested peers.
    fn broadcast(&self, batch: LogBatch);

    /// Subscribe to incoming batches from peers.
    fn subscribe(&self) -> broadcast::Receiver<LogBatch>;
}
