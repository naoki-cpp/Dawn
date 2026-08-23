//! Rebuildable retained public-event read model.
//!
//! `DurableJournal` is the only persistent source. This tail is deliberately
//! an in-memory projection used by replication and catch-up; it can always be
//! rebuilt from a checkpoint cursor and the retained journal range.

use dawn_core::DomainEvent;
use dawn_storage::{DurableJournal, JournalError, JournalIndex, JournalStream, PublicEventIndex};
use std::collections::VecDeque;
use thiserror::Error;

/// A bounded public-event suffix returned for a cursor request.
#[derive(Debug, Clone, PartialEq)]
pub struct PublicEventSuffix {
    pub from_public_event_index: PublicEventIndex,
    pub events: Vec<DomainEvent>,
}

/// Why a tail operation could not produce a public suffix.
#[derive(Debug, Error)]
pub enum PublicEventTailError {
    #[error("public event tail capacity must be non-zero")]
    InvalidCapacity,
    #[error("public event suffix limit must be non-zero, got {max_events}")]
    InvalidReadLimit { max_events: usize },
    #[error("public event cursor {requested} is older than retained base {retained_base}")]
    CursorTooOld {
        requested: PublicEventIndex,
        retained_base: PublicEventIndex,
    },
    #[error("public event cursor {requested} is ahead of retained next index {next_index}")]
    CursorAhead {
        requested: PublicEventIndex,
        next_index: PublicEventIndex,
    },
    #[error("public event index overflow")]
    IndexOverflow,
    #[error("journal read failed: {0}")]
    Journal(#[from] JournalError),
    #[error(
        "public event payload in journal record {journal_index} could not be decoded: {reason}"
    )]
    Decode {
        journal_index: JournalIndex,
        reason: String,
    },
}

/// Rebuildable, bounded public-event projection.
#[derive(Debug, Clone)]
pub struct PublicEventTail {
    retained_base: PublicEventIndex,
    next_index: PublicEventIndex,
    max_events: usize,
    events: VecDeque<DomainEvent>,
}

impl PublicEventTail {
    /// Create an empty tail beginning at an already checkpointed cursor.
    pub fn new(
        checkpoint_next_index: PublicEventIndex,
        max_events: usize,
    ) -> Result<Self, PublicEventTailError> {
        if max_events == 0 {
            return Err(PublicEventTailError::InvalidCapacity);
        }
        Ok(Self {
            retained_base: checkpoint_next_index,
            next_index: checkpoint_next_index,
            max_events,
            events: VecDeque::with_capacity(max_events),
        })
    }

    /// Rebuild public output after a checkpoint without replaying the covered
    /// transition. Recovery and public cursors remain independent: only
    /// `PublicEvent` records advance the public cursor.
    pub fn rebuild<J: DurableJournal>(
        journal: &J,
        recovery_start: JournalIndex,
        checkpoint_next_index: PublicEventIndex,
        max_events: usize,
    ) -> Result<Self, PublicEventTailError> {
        let mut tail = Self::new(checkpoint_next_index, max_events)?;
        for record in journal.read_from(recovery_start)? {
            let record = record?;
            if record.stream != JournalStream::PublicEvent {
                continue;
            }
            let event = postcard::from_bytes::<DomainEvent>(&record.payload).map_err(|error| {
                PublicEventTailError::Decode {
                    journal_index: record.index,
                    reason: error.to_string(),
                }
            })?;
            tail.append_committed(std::slice::from_ref(&event))?;
        }
        Ok(tail)
    }

    pub fn retained_base(&self) -> PublicEventIndex {
        self.retained_base
    }

    pub fn next_index(&self) -> PublicEventIndex {
        self.next_index
    }

    /// Append events after their authoritative transition has committed and
    /// applied. This method never writes durable state.
    pub fn append_committed(&mut self, events: &[DomainEvent]) -> Result<(), PublicEventTailError> {
        if events.is_empty() {
            return Ok(());
        }
        let next = self
            .next_index
            .0
            .checked_add(events.len() as u64)
            .ok_or(PublicEventTailError::IndexOverflow)?;
        self.events.extend(events.iter().cloned());
        self.next_index = PublicEventIndex(next);
        while self.events.len() > self.max_events {
            self.events.pop_front();
            self.retained_base = PublicEventIndex(
                self.retained_base
                    .0
                    .checked_add(1)
                    .ok_or(PublicEventTailError::IndexOverflow)?,
            );
        }
        Ok(())
    }

    /// Read a bounded suffix. A cursor before the retained base is explicit
    /// so callers must choose snapshot fallback rather than ship a truncated
    /// stream that appears contiguous.
    pub fn read_from(
        &self,
        requested: PublicEventIndex,
        max_events: usize,
    ) -> Result<PublicEventSuffix, PublicEventTailError> {
        if max_events == 0 {
            return Err(PublicEventTailError::InvalidReadLimit { max_events });
        }
        if requested < self.retained_base {
            return Err(PublicEventTailError::CursorTooOld {
                requested,
                retained_base: self.retained_base,
            });
        }
        if requested > self.next_index {
            return Err(PublicEventTailError::CursorAhead {
                requested,
                next_index: self.next_index,
            });
        }
        let offset = (requested.0 - self.retained_base.0) as usize;
        let events = self
            .events
            .iter()
            .skip(offset)
            .take(max_events)
            .cloned()
            .collect();
        Ok(PublicEventSuffix {
            from_public_event_index: requested,
            events,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{events::VelocityChanged, NodeId, SectorId, ShipId, Tick, Velocity};
    use dawn_storage::{
        DurabilityContext, DurabilityMode, InMemoryJournal, JournalBatch, JournalEntry,
        TransitionId,
    };

    fn event(n: u64) -> DomainEvent {
        DomainEvent::VelocityChanged(VelocityChanged {
            ship_id: ShipId::new(NodeId(0), n),
            velocity: Velocity::new(n as f64, 0.0, 0.0),
            tick: Tick(n),
        })
    }

    fn context() -> DurabilityContext {
        DurabilityContext {
            sector_id: SectorId(0),
            owner_epoch: 0,
        }
    }

    #[test]
    fn zero_capacity_is_rejected() {
        assert!(matches!(
            PublicEventTail::new(PublicEventIndex::ZERO, 0),
            Err(PublicEventTailError::InvalidCapacity)
        ));
    }

    #[test]
    fn bounded_reads_preserve_global_cursor_and_reject_expired_prefix() {
        let mut tail = PublicEventTail::new(PublicEventIndex(3), 2).unwrap();
        tail.append_committed(&[event(3), event(4), event(5)])
            .unwrap();

        assert_eq!(tail.retained_base(), PublicEventIndex(4));
        assert_eq!(tail.next_index(), PublicEventIndex(6));
        assert!(matches!(
            tail.read_from(PublicEventIndex(3), 2),
            Err(PublicEventTailError::CursorTooOld { .. })
        ));
        let suffix = tail.read_from(PublicEventIndex(4), 1).unwrap();
        assert_eq!(suffix.events, vec![event(4)]);
        assert!(matches!(
            tail.read_from(PublicEventIndex(7), 1),
            Err(PublicEventTailError::CursorAhead { .. })
        ));
        assert!(matches!(
            tail.read_from(PublicEventIndex(4), 0),
            Err(PublicEventTailError::InvalidReadLimit { max_events: 0 })
        ));
    }

    #[test]
    fn empty_committed_output_does_not_advance_the_public_cursor() {
        let mut tail = PublicEventTail::new(PublicEventIndex(7), 2).unwrap();

        tail.append_committed(&[]).unwrap();

        assert_eq!(tail.retained_base(), PublicEventIndex(7));
        assert_eq!(tail.next_index(), PublicEventIndex(7));
    }

    #[test]
    fn cursor_overflow_rejects_the_append_without_changing_the_tail() {
        let mut tail = PublicEventTail::new(PublicEventIndex(u64::MAX), 1).unwrap();

        assert!(matches!(
            tail.append_committed(&[event(1)]),
            Err(PublicEventTailError::IndexOverflow)
        ));
        assert_eq!(tail.retained_base(), PublicEventIndex(u64::MAX));
        assert_eq!(tail.next_index(), PublicEventIndex(u64::MAX));
        assert!(tail
            .read_from(PublicEventIndex(u64::MAX), 1)
            .unwrap()
            .events
            .is_empty());
    }

    #[test]
    fn rebuild_counts_only_public_events_in_mixed_journal_stream() {
        let mut journal = InMemoryJournal::new();
        let entries = vec![
            JournalEntry::new(JournalStream::RecoveryDelta, vec![1]),
            JournalEntry::new(
                JournalStream::PublicEvent,
                dawn_storage::encode_payload(&event(7)).unwrap(),
            ),
            JournalEntry::new(JournalStream::ReliableEffect, vec![2]),
            JournalEntry::new(
                JournalStream::PublicEvent,
                dawn_storage::encode_payload(&event(8)).unwrap(),
            ),
        ];
        journal
            .append_batch(
                JournalBatch::with_entries(
                    TransitionId(1),
                    context(),
                    entries,
                    DurabilityMode::Synced,
                )
                .unwrap(),
            )
            .unwrap();

        let tail =
            PublicEventTail::rebuild(&journal, JournalIndex::ZERO, PublicEventIndex::ZERO, 8)
                .unwrap();
        assert_eq!(tail.next_index(), PublicEventIndex(2));
        assert_eq!(
            tail.read_from(PublicEventIndex::ZERO, 8).unwrap().events,
            vec![event(7), event(8)]
        );
    }

    #[test]
    fn recovery_then_live_append_does_not_duplicate_events() {
        let mut journal = InMemoryJournal::new();
        journal
            .append_batch(
                JournalBatch::with_entries(
                    TransitionId(1),
                    context(),
                    vec![
                        JournalEntry::new(JournalStream::RecoveryDelta, vec![1]),
                        JournalEntry::new(
                            JournalStream::PublicEvent,
                            dawn_storage::encode_payload(&event(1)).unwrap(),
                        ),
                    ],
                    DurabilityMode::Synced,
                )
                .unwrap(),
            )
            .unwrap();
        let mut tail = PublicEventTail::rebuild(&journal, JournalIndex::ZERO, 0.into(), 8).unwrap();
        tail.append_committed(&[event(2)]).unwrap();

        assert_eq!(tail.next_index(), PublicEventIndex(2));
        assert_eq!(
            tail.read_from(0.into(), 8).unwrap().events,
            vec![event(1), event(2)]
        );
    }

    #[test]
    fn rebuild_resumes_from_checkpoint_recovery_and_public_cursors() {
        let mut journal = InMemoryJournal::new();
        let covered = journal
            .append_batch(
                JournalBatch::with_entries(
                    TransitionId(1),
                    context(),
                    vec![
                        JournalEntry::new(JournalStream::RecoveryDelta, vec![1]),
                        JournalEntry::new(
                            JournalStream::PublicEvent,
                            dawn_storage::encode_payload(&event(40)).unwrap(),
                        ),
                    ],
                    DurabilityMode::Synced,
                )
                .unwrap(),
            )
            .unwrap()
            .range
            .checked_last_exclusive()
            .unwrap();
        journal
            .append_batch(
                JournalBatch::with_entries(
                    TransitionId(2),
                    context(),
                    vec![
                        JournalEntry::new(JournalStream::RecoveryDelta, vec![2]),
                        JournalEntry::new(JournalStream::ReliableEffect, vec![3]),
                        JournalEntry::new(
                            JournalStream::PublicEvent,
                            dawn_storage::encode_payload(&event(41)).unwrap(),
                        ),
                        JournalEntry::new(
                            JournalStream::PublicEvent,
                            dawn_storage::encode_payload(&event(42)).unwrap(),
                        ),
                    ],
                    DurabilityMode::Synced,
                )
                .unwrap(),
            )
            .unwrap();

        let tail = PublicEventTail::rebuild(&journal, covered, PublicEventIndex(41), 8).unwrap();

        assert_eq!(tail.retained_base(), PublicEventIndex(41));
        assert_eq!(tail.next_index(), PublicEventIndex(43));
        assert_eq!(
            tail.read_from(PublicEventIndex(41), 8).unwrap().events,
            vec![event(41), event(42)]
        );
    }
}
