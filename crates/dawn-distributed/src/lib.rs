//! Distributed coordination for Dawn.
//!
//! This package is the only boundary for cross-Sector coordination. Its
//! modules have deliberately different responsibilities:
//!
//! - [`peer_transport`] owns versioned control/bulk TCP framing;
//! - Raft modules own logical consensus and its peer adapter;
//! - replication modules own append-log gossip, catch-up, and foreign replica
//!   state.
//!
//! Keeping these modules in one package avoids three crates that only existed
//! to pass opaque transport handles between one another. The module direction
//! remains explicit: consensus and replication depend on the physical
//! transport, while the transport knows nothing about either policy.

#![warn(missing_debug_implementations)]

pub mod actor;
pub mod anti_entropy;
pub mod bus;
pub mod catch_up;
pub mod outbound;
pub mod peer_transport;
pub mod public_event_tail;
pub mod raft_peer_transport;
pub mod replica;
pub mod replication_peer_transport;
pub mod rpc;
pub mod state;
pub mod transport;

use dawn_core::{DomainEvent, SectorId};
use dawn_storage::PublicEventIndex;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

pub use actor::{RaftActor, RaftActorHandle, RaftActorMessage};
pub use anti_entropy::{AntiEntropy, BatchApplyPlan, MissingLogRequest};
pub use bus::{BusMessage, InMemoryReplicationBus};
pub use catch_up::{
    CatchUpConfig, CatchUpEvent, CatchUpFailure, CatchUpFailureKind, CatchUpManager,
    CatchUpMessage, CatchUpPayload, CatchUpRequest, CatchUpResponse, CatchUpStep, CatchUpTransport,
    CatchUpUnavailable,
};
pub use outbound::OutboundLogPublisher;
pub use peer_transport::{
    PeerCapabilities, PeerChannelKind, PeerEndpoint, PeerFrame, PeerIdentity, PeerMessageKind,
    PeerSendError, PeerTransport, PeerTransportConfig,
};
pub use public_event_tail::{PublicEventSuffix, PublicEventTail, PublicEventTailError};
pub use raft_peer_transport::PeerRaftTransport;
pub use replica::{Ingest, ReplicaSet, ReplicaSnapshot, SnapshotInstall};
pub use replication_peer_transport::{DurabilitySendError, PeerReplicationTransport};
pub use rpc::{
    AppendEntries, AppendEntriesResponse, RaftMessage, RequestVote, RequestVoteResponse,
};
pub use state::{LogEntry, RaftState, Role, Term, TickEffect};
pub use transport::{InProcessTransport, PartitionableTransport, RaftTransport};

/// A single gossip payload from a Sector owner's append-only log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogBatch {
    pub sector_id: SectorId,
    pub from_public_event_index: PublicEventIndex,
    pub events: Vec<DomainEvent>,
}

impl LogBatch {
    pub fn new(
        sector_id: SectorId,
        from_public_event_index: impl Into<PublicEventIndex>,
        events: Vec<DomainEvent>,
    ) -> Self {
        Self {
            sector_id,
            from_public_event_index: from_public_event_index.into(),
            events,
        }
    }

    pub fn next_public_event_index(&self) -> PublicEventIndex {
        PublicEventIndex(self.from_public_event_index.0 + self.events.len() as u64)
    }
}

/// Wire-format-agnostic ordinary log-gossip transport.
pub trait ReplicationTransport: Send + Sync {
    fn broadcast(&self, batch: LogBatch);
    fn subscribe(&self) -> broadcast::Receiver<LogBatch>;
}
