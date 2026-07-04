//! Self-implemented Raft consensus for Sector Transit (ADR-0014).
//!
//! Scope: Leader election + Log Replication + failover for a fixed
//! 3-node cluster. Out of scope: Membership Change, Log Compaction,
//! Pre-Vote/Learner extensions.
//!
//! Timers are driven by logical Tick counts, never physical time
//! (INV-005, FBD-003).
//!
//! ## Example
//!
//! ```
//! use dawn_consensus::{RaftState, Role};
//! use dawn_core::NodeId;
//!
//! let state = RaftState::new(NodeId(0), vec![NodeId(1), NodeId(2)], 10, 3);
//! assert_eq!(state.role, Role::Follower);
//! ```

// Rust API Guidelines C-DEBUG: catch new pub types that forget to derive
// Debug at compile time instead of relying on periodic audits (see #83).
#![warn(missing_debug_implementations)]

pub mod actor;
pub mod rpc;
pub mod state;
pub mod tcp_transport;
pub mod transport;

pub use actor::{RaftActor, RaftActorHandle, RaftActorMessage};
pub use rpc::{
    AppendEntries, AppendEntriesResponse, RaftMessage, RequestVote, RequestVoteResponse,
};
pub use state::{LogEntry, RaftState, Role, Term, TickEffect};
pub use tcp_transport::{TcpRaftError, TcpRaftTransport};
pub use transport::{InProcessTransport, PartitionableTransport, RaftTransport};
