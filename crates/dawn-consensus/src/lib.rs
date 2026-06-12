//! Self-implemented Raft consensus for Sector Transit (ADR-0014).
//!
//! Scope: Leader election + Log Replication + failover for a fixed
//! 3-node cluster. Out of scope: Membership Change, Log Compaction,
//! Pre-Vote/Learner extensions.
//!
//! Timers are driven by logical Tick counts, never physical time
//! (INV-005, FBD-003).

pub mod actor;
pub mod rpc;
pub mod state;
pub mod transport;

pub use actor::{RaftActor, RaftActorHandle, RaftActorMessage};
pub use rpc::{AppendEntries, AppendEntriesResponse, RaftMessage, RequestVote, RequestVoteResponse};
pub use state::{Role, RaftState, TickEffect, Term};
pub use transport::{InProcessTransport, PartitionableTransport, RaftTransport};
