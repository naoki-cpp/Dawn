//! # dawn-actor
//!
//! Actor infrastructure for the dawn simulation platform.
//!
//! ## Invariants (CLAUDE.md §5, FBD-004)
//!
//! - Actors own their data exclusively. No `Arc<Mutex<T>>` across Actor boundaries.
//! - All inter-Actor communication is via Mailbox (`tokio::mpsc`) only.
//! - Callers interact only with `*Handle` types, never with Actor internals.
//!
//! ## Crates that may depend on dawn-actor
//!
//! dawn-simulation (and future dawn-sector-node).
//! dawn-actor must never depend on dawn-ecs or dawn-simulation.

pub mod event_store_actor;
pub mod replication_bus;

pub use event_store_actor::EventStoreHandle;
pub use replication_bus::{BusMessage, ReplicationBusHandle};
