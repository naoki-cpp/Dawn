//! # dawn-event-store
//!
//! Append-only Event Log for the dawn simulation.
//!
//! ## Invariants (INV-001, INV-002)
//!
//! - The only mutating operation is `append`.
//! - Every event appended receives a monotonically increasing `log_index`.
//! - State can be fully reconstructed by replaying from `log_index == 0`.
//!
//! ## This crate provides
//!
//! - `EventStore` trait — the contract all store implementations must satisfy.
//! - `InMemoryEventStore` — in-process store used by MVP and all tests.
//! - `EventRecord` — a single entry in the log.

pub mod file;
pub mod memory;
pub mod record;
pub mod store;

pub use file::FileEventStore;
pub use memory::InMemoryEventStore;
pub use record::EventRecord;
pub use store::EventStore;
