//! # dawn-storage
//!
//! Append-only journal storage for the Dawn simulation.
//!
//! ## Invariants (INV-001, INV-002)
//!
//! - `DurableJournal` appends encoded logical-transition batches with explicit
//!   errors, receipts, and durability modes.
//! - Exact authoritative recovery uses the versioned `DurableJournal` payload;
//!   public-event replay is a projection/audit path, not an implicit recovery
//!   guarantee.
//!
//! ## This crate provides
//!
//! - `DurableJournal` — the generic recovery-journal contract.
//! - `RecoveryIndex` / `PublicEventIndex` — non-interchangeable positions for
//!   authoritative recovery and public replication.
//!
//! ## Example
//!
//! ```
//! use dawn_core::SectorId;
//! use dawn_storage::{
//!     DurabilityContext, DurabilityMode, DurableJournal, InMemoryJournal,
//!     JournalBatch, JournalEntry, JournalIndex, JournalStream, TransitionId,
//! };
//!
//! let mut journal = InMemoryJournal::new();
//! let batch = JournalBatch::with_entries(
//!     TransitionId(1),
//!     DurabilityContext {
//!         sector_id: SectorId(0),
//!         owner_epoch: 0,
//!     },
//!     vec![JournalEntry::new(
//!         JournalStream::PublicEvent,
//!         b"encoded-domain-event".to_vec(),
//!     )],
//!     DurabilityMode::Synced,
//! )
//! .expect("one non-empty transition");
//!
//! let receipt = journal.append_batch(batch).expect("durable append");
//! assert_eq!(receipt.range.first, JournalIndex::ZERO);
//! assert_eq!(journal.next_index().unwrap(), JournalIndex(1));
//! ```
//!
// Rust API Guidelines C-DEBUG: catch new pub types that forget to derive
// Debug at compile time instead of relying on periodic audits (see #83).
#![warn(missing_debug_implementations)]

pub mod cursor;
pub mod file_journal;
pub mod journal;
pub mod memory_journal;

pub use cursor::{PublicEventIndex, RecoveryIndex};
pub use file_journal::FileJournal;
pub use journal::{
    encode_payload, AppendReceipt, CompactionReceipt, DurabilityContext, DurabilityEvidence,
    DurabilityEvidenceSource, DurabilityMode, DurabilityTransportContext,
    DurabilityTransportMessage, DurableJournal, JournalBatch, JournalEntry, JournalError,
    JournalIndex, JournalRange, JournalRecord, JournalStream, TransitionId,
};
pub use memory_journal::InMemoryJournal;
