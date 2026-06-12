//! Self-implemented Raft consensus for Sector Transit (ADR-0014).
//!
//! Scope: Leader election + Log Replication + failover for a fixed
//! 3-node cluster. Out of scope: Membership Change, Log Compaction,
//! Pre-Vote/Learner extensions.
//!
//! Timers are driven by logical Tick counts, never physical time
//! (INV-005, FBD-003).

pub mod state;

pub use state::{Role, RaftState, TickEffect, Term};
