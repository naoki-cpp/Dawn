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
//!
//! ## Example
//!
//! ```
//! use dawn_core::{DomainEvent, NodeId, Position, SectorId, ShipId, ShipTypeId, Tick};
//! use dawn_core::events::ShipSpawned;
//! use dawn_event_store::{EventStore, InMemoryEventStore};
//!
//! let mut store = InMemoryEventStore::new();
//! let ship_id = ShipId::new(NodeId(1), 7);
//! let index = store.append(DomainEvent::ShipSpawned(ShipSpawned {
//!     ship_id,
//!     sector_id: SectorId(0),
//!     initial_position: Position::ORIGIN,
//!     ship_type_id: ShipTypeId(1),
//!     tick: Tick::ZERO,
//! }));
//!
//! assert_eq!(index, 0);
//! assert_eq!(store.next_index(), 1);
//! ```

// Rust API Guidelines C-DEBUG: catch new pub types that forget to derive
// Debug at compile time instead of relying on periodic audits (see #83).
#![warn(missing_debug_implementations)]

pub mod file;
pub mod memory;
pub mod record;
pub mod store;

pub use file::FileEventStore;
pub use memory::InMemoryEventStore;
pub use record::EventRecord;
pub use store::EventStore;
