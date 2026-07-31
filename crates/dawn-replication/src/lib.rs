//! # dawn-replication
//!
//! Sector-local event-log gossip and bounded replica recovery (ADR-0021,
//! ADR-0027). Foreign replicas are recovery data only and are never applied to
//! a live local world.

#![warn(missing_debug_implementations)]

pub mod anti_entropy;
pub mod bus;
pub mod catch_up;
pub mod outbound;
pub mod replica;
pub mod snapshot;
pub mod tcp;

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
pub use replica::{Ingest, ReplicaSet, ReplicaSnapshot, SnapshotInstall};
pub use snapshot::SnapshotTransfer;
pub use tcp::{TcpReplicationError, TcpReplicationTransport};

/// A single gossip payload from a Sector owner's append-only log.
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
pub trait ReplicationTransport: Send + Sync {
    fn broadcast(&self, batch: LogBatch);
    fn subscribe(&self) -> broadcast::Receiver<LogBatch>;
}
