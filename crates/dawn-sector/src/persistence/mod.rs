//! Snapshot and checkpoint persistence (ADR-0017).
//!
//! - [`snapshot`]: `StateSnapshot` / `ShipSnapshot` — point-in-time captures of
//!   `SimulationNode` state, persisted through a versioned checkpoint envelope.
//! - [`checkpoint`]: `CheckpointScheduler` / `CheckpointConfig` — policy layer
//!   that fires `SimulationNode::checkpoint` on a fixed logical-tick cadence.

pub mod checkpoint;
pub mod recovery;
pub mod snapshot;

pub use checkpoint::{CheckpointConfig, CheckpointScheduler};
pub use snapshot::{CompletedIncomingTransit, ShipSnapshot, StateSnapshot};
