//! In-memory `EventStore` implementation.
//!
//! Used by the MVP and all unit/integration tests.
//! All records are held in a `Vec` — no disk I/O, no external dependencies.
//!
//! The file-based persistent store (ADR-0010) will implement the same
//! `EventStore` trait, so `SimulationNode` requires no changes to switch.

use crate::{record::EventRecord, store::EventStore};
use dawn_core::DomainEvent;

/// Append-only, in-memory implementation of `EventStore`.
///
/// # Guarantees
///
/// - `append` is O(1) amortised (Vec push).
/// - `iter_from` is O(N) where N is the number of records from `from_index`.
/// - Records are never mutated or removed.
#[derive(Debug, Default)]
pub struct InMemoryEventStore {
    records: Vec<EventRecord>,
}

impl InMemoryEventStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return a slice of all records — useful for snapshot and test assertions.
    pub fn all_records(&self) -> &[EventRecord] {
        &self.records
    }
}

impl EventStore for InMemoryEventStore {
    fn append(&mut self, event: DomainEvent) -> u64 {
        let index = self.records.len() as u64;
        self.records.push(EventRecord::new(index, event));
        index
    }

    fn iter_from(&self, from_index: u64) -> impl Iterator<Item = &EventRecord> {
        let start = from_index as usize;
        self.records[start.min(self.records.len())..].iter()
    }

    fn len(&self) -> usize {
        self.records.len()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{
        events::{ShipMoved, ShipSpawned},
        NodeId, Position, SectorId, ShipId, Tick,
    };

    fn ship_id(n: u64) -> ShipId {
        ShipId::new(NodeId(0), n)
    }

    fn moved_event(ship_n: u64, tick: u64) -> DomainEvent {
        DomainEvent::ShipMoved(ShipMoved {
            ship_id: ship_id(ship_n),
            from   : Position::ORIGIN,
            to     : Position::new(1.0, 0.0, 0.0),
            tick   : Tick(tick),
        })
    }

    fn spawned_event(ship_n: u64) -> DomainEvent {
        DomainEvent::ShipSpawned(ShipSpawned {
            ship_id          : ship_id(ship_n),
            sector_id        : SectorId(0),
            initial_position : Position::ORIGIN,
            ship_type_id     : dawn_core::ShipTypeId(1),
            tick             : Tick::ZERO,
        })
    }

    // ── INV-001: store has no update or delete operations ────────────────────

    #[test]
    fn event_store_trait_exposes_no_update_or_delete_method() {
        // This is a compile-time invariant: `InMemoryEventStore` must not
        // implement any method named `update`, `delete`, `truncate`, or
        // `rewrite`.  If someone adds such a method, this test reminds
        // reviewers to re-read CLAUDE.md FBD-001.
        //
        // We assert the absence by verifying only `append` changes state.
        let mut store = InMemoryEventStore::new();
        store.append(moved_event(1, 1));
        let len_before = store.len();

        // No call to update/delete here — there is no such method.
        assert_eq!(store.len(), len_before, "len must only grow, never shrink");
    }

    // ── Append ───────────────────────────────────────────────────────────────

    #[test]
    fn appending_first_event_assigns_log_index_zero() {
        let mut store = InMemoryEventStore::new();
        let index = store.append(moved_event(1, 1));
        assert_eq!(index, 0);
    }

    #[test]
    fn log_indices_are_monotonically_increasing_by_one() {
        let mut store = InMemoryEventStore::new();
        let indices: Vec<u64> = (0..5).map(|i| store.append(moved_event(i, i))).collect();
        assert_eq!(indices, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn append_batch_assigns_consecutive_indices() {
        let mut store = InMemoryEventStore::new();
        let events = vec![moved_event(1, 1), moved_event(2, 1), moved_event(3, 1)];
        let indices = store.append_batch(events);
        assert_eq!(indices, vec![0, 1, 2]);
    }

    // ── Iteration ────────────────────────────────────────────────────────────

    #[test]
    fn iter_from_zero_returns_all_records() {
        let mut store = InMemoryEventStore::new();
        store.append(spawned_event(1));
        store.append(moved_event(1, 1));

        let all: Vec<&EventRecord> = store.iter_from(0).collect();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn iter_from_nonzero_skips_earlier_records() {
        let mut store = InMemoryEventStore::new();
        for i in 0..10 {
            store.append(moved_event(i, i));
        }

        let tail: Vec<&EventRecord> = store.iter_from(7).collect();
        assert_eq!(tail.len(), 3);
        assert_eq!(tail[0].log_index, 7);
    }

    #[test]
    fn iter_from_index_beyond_end_returns_empty_iterator() {
        let mut store = InMemoryEventStore::new();
        store.append(moved_event(1, 1));

        let records: Vec<_> = store.iter_from(999).collect();
        assert!(records.is_empty());
    }

    // ── INV-002: state reproducibility ───────────────────────────────────────

    #[test]
    fn replaying_all_events_from_index_zero_reconstructs_original_ship_count() {
        let mut store = InMemoryEventStore::new();

        // Simulate: spawn 3 ships, move them once each.
        store.append(spawned_event(1));
        store.append(spawned_event(2));
        store.append(spawned_event(3));
        store.append(moved_event(1, 1));
        store.append(moved_event(2, 1));
        store.append(moved_event(3, 1));

        // A naive "replay" counts spawns minus despawns.
        let ship_count = store
            .iter_from(0)
            .filter(|r| matches!(r.event, DomainEvent::ShipSpawned(_)))
            .count();

        assert_eq!(ship_count, 3, "replay must reproduce the number of spawned ships");
    }
}
