//! # dawn-replication
//!
//! Sector-local event log replication (ADR-0021, ADR-0027).
//!
//! Strategy: gossip-based append-log shipping (not CRDT / LWW).
//! Single-ownership means no concurrent writes, so conflict-free merge is
//! unnecessary. Events are idempotent (INV-004) and ordered by logical tick
//! (INV-005), so delivering them in any order converges to the same state.
//!
//! ## Current scope (8D-2a)
//!
//! - `LogBatch` — the unit of sector-local append-log shipping.
//! - `ReplicationTransport` trait — wire-format-agnostic interface.
//! - `InMemoryReplicationBus` — single-process implementation used by tests
//!   and the existing multi-node bench. This replaces `dawn_actor::ReplicationBus`.
//!
//! ## Planned (8D-2b / 8D-2c / 8D-2d)
//!
//! - `AntiEntropy` — request missing events by log index range.
//! - `TcpReplicationTransport` — LAN plaintext, postcard wire format.
//! - `SnapshotTransfer` — catch up far-behind replicas via snapshot.

pub mod anti_entropy;
pub mod bus;

use dawn_core::{DomainEvent, SectorId};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

pub use anti_entropy::{AntiEntropy, BatchApplyPlan, MissingLogRequest};
pub use bus::{BusMessage, InMemoryReplicationBus};

/// A single gossip payload from a Sector owner's append-only log.
///
/// `from_index` is the first log index represented by `events`. Receivers use
/// `(sector_id, from_index)` to detect gaps and to drop duplicate deliveries
/// idempotently in the anti-entropy step (ADR-0027 / 8D-2b).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogBatch {
    pub sector_id : SectorId,
    pub from_index: u64,
    pub events    : Vec<DomainEvent>,
}

impl LogBatch {
    pub fn new(sector_id: SectorId, from_index: u64, events: Vec<DomainEvent>) -> Self {
        Self { sector_id, from_index, events }
    }

    pub fn next_index(&self) -> u64 {
        self.from_index + self.events.len() as u64
    }
}

/// Wire-format-agnostic replication transport.
///
/// TCP gossip (8D-2c) will implement this same interface with length-prefixed
/// postcard frames; the in-memory implementation keeps tests single-process.
pub trait ReplicationTransport: Send + Sync {
    /// Send a batch of events to interested peers.
    fn broadcast(&self, batch: LogBatch);

    /// Subscribe to incoming batches from peers.
    fn subscribe(&self) -> broadcast::Receiver<LogBatch>;
}
