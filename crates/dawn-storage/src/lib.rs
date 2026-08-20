//! # dawn-storage
//!
//! Append-only journal storage for the Dawn simulation.
//!
//! ## Invariants (INV-001, INV-002)
//!
//! - The public-fact `EventStore` appends `DomainEvent` values with a
//!   monotonically increasing `log_index`.
//! - `DurableJournal` appends encoded logical-transition batches with explicit
//!   errors, receipts, and durability modes.
//! - Exact authoritative recovery uses the versioned `DurableJournal` payload;
//!   public-event replay is a projection/audit path, not an implicit recovery
//!   guarantee.
//!
//! ## This crate provides
//!
//! - `DurableJournal` — the generic recovery-journal contract.
//! - `EventStore` — the legacy public-event store still used by the migration
//!   path.
//! - `RecoveryIndex` / `PublicEventIndex` — non-interchangeable positions for
//!   authoritative recovery and public replication.
//! - `InMemoryEventStore` — in-process store used by MVP and all tests.
//! - `EventRecord` — a single entry in the log.
//!
//! ## Example
//!
//! ```
//! use dawn_core::{AbsolutePosition, DomainEvent, NodeId, SectorId, ShipId, ShipTypeId, Tick};
//! use dawn_core::events::ShipSpawned;
//! use dawn_storage::{EventStore, InMemoryEventStore};
//!
//! let mut store = InMemoryEventStore::new();
//! let ship_id = ShipId::new(NodeId(1), 7);
//! let index = store.append(DomainEvent::ShipSpawned(ShipSpawned {
//!     ship_id,
//!     sector_id: SectorId(0),
//!     initial_position: AbsolutePosition::ORIGIN,
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

pub mod cursor;
pub mod file;
pub mod file_journal;
pub mod journal;
pub mod memory;
pub mod memory_journal;
pub mod record;
pub mod store;

pub use cursor::{PublicEventIndex, RecoveryIndex};
pub use file::FileEventStore;
pub use file_journal::FileJournal;
pub use journal::{
    encode_payload, AppendReceipt, CompactionReceipt, DurabilityContext, DurabilityEvidence,
    DurabilityEvidenceSource, DurabilityMode, DurabilityTransportContext,
    DurabilityTransportMessage, DurableJournal, JournalBatch, JournalEntry, JournalError,
    JournalIndex, JournalRange, JournalRecord, JournalStream, TransitionId,
};
pub use memory::InMemoryEventStore;
pub use memory_journal::InMemoryJournal;
pub use record::EventRecord;
pub use store::EventStore;
