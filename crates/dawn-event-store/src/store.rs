//! `EventStore` trait — the contract for all Event Log implementations.
//!
//! This is the legacy public-event contract. It remains available while Sector
//! runtime call sites migrate to [`crate::DurableJournal`], which is the
//! storage contract for authoritative recovery batches selected by ADR-0049.
//!
//! # Invariant
//!
//! Implementations **must not** provide methods that update or delete
//! existing records.  If a method with such semantics is needed, raise an ADR
//! for review.  (CLAUDE.md FBD-001)

use crate::EventRecord;
use dawn_core::DomainEvent;

pub trait EventStore {
    /// Append `event` and return the `log_index` assigned to it.
    ///
    /// `log_index` values start at 0 and increase by exactly 1 per call.
    fn append(&mut self, event: DomainEvent) -> u64;

    /// Append multiple events in a single call.
    ///
    /// Equivalent to calling `append` for each event, but may be more
    /// efficient for bulk operations (e.g. end-of-tick batch write).
    fn append_batch(&mut self, events: impl IntoIterator<Item = DomainEvent>) -> Vec<u64> {
        events.into_iter().map(|e| self.append(e)).collect()
    }

    /// Iterate over all records starting from `from_index` (inclusive).
    fn iter_from(&self, from_index: u64) -> impl Iterator<Item = &EventRecord>;

    /// Total number of events stored.
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The `log_index` that will be assigned to the **next** appended event.
    fn next_index(&self) -> u64 {
        self.len() as u64
    }
}
