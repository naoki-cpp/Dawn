//! Authoritative checkpoint-plus-journal recovery (ADR-0049 / issue #284).
//!
//! The recovery reader consumes the journal's logical-batch framing rather
//! than treating public events as a reducer. Every committed batch must start
//! with exactly one versioned `RecoveryDelta`; the remaining records in that
//! batch are public projections or reliable effects and are deliberately not
//! applied to the authoritative world.

use dawn_storage::{DurableJournal, JournalError, JournalIndex, JournalStream, TransitionId};
use std::collections::HashSet;
use thiserror::Error;

use crate::{
    node::SimulationNode,
    transition::{
        decode_recovery_delta, SectorRecoveryDelta, TransitionApplyError, TransitionCodecError,
        TransitionContext,
    },
};

#[derive(Debug, Error)]
pub enum RecoveryError {
    #[error("recovery journal read failed: {0}")]
    Journal(#[from] JournalError),
    #[error("recovery record index is not contiguous: expected {expected}, found {actual}")]
    NonContiguous {
        expected: JournalIndex,
        actual: JournalIndex,
    },
    #[error("recovery batch {transition_id:?} does not start with a RecoveryDelta")]
    MissingDelta { transition_id: TransitionId },
    #[error("recovery batch {transition_id:?} contains a second RecoveryDelta")]
    DuplicateDelta { transition_id: TransitionId },
    #[error("recovery transition {transition_id:?} appears more than once")]
    DuplicateTransition { transition_id: TransitionId },
    #[error("recovery delta decode failed: {0}")]
    Decode(#[from] TransitionCodecError),
    #[error("recovery delta apply failed: {0}")]
    Apply(#[from] TransitionApplyError),
    #[error("recovery record belongs to sector {actual:?}; expected {expected:?}")]
    SectorMismatch {
        expected: dawn_core::SectorId,
        actual: dawn_core::SectorId,
    },
    #[error("recovery journal ended at {actual}; expected {expected}")]
    UnexpectedEnd {
        actual: JournalIndex,
        expected: JournalIndex,
    },
}

/// Apply every authoritative transition at and after `covered_index`.
///
/// `covered_index` must be the next record after a complete journal batch,
/// which is exactly what `CheckpointScheduler` records. The reader rejects a
/// tail that starts in the middle of a batch, has a gap, or lacks the
/// RecoveryDelta first entry. This makes duplicate/missing/out-of-order tail
/// failures visible instead of silently producing a plausible but wrong
/// world.
///
/// The node is consumed deliberately. A semantic apply failure can occur
/// after an earlier delta has already been applied; returning the partially
/// mutated node would invite a caller to keep serving from an untrusted
/// state. Callers must fence/discard the node on error and only receive it
/// back after the complete tail has applied successfully.
pub fn apply_tail<J: DurableJournal>(
    mut node: SimulationNode,
    journal: &J,
    covered_index: u64,
) -> Result<(SimulationNode, JournalIndex), RecoveryError> {
    let start = JournalIndex(covered_index);
    let expected_end = journal.next_index()?;
    let records = journal.read_from(start)?;
    let mut expected = start;
    let mut current_transition = None;
    let mut validated_delta = false;
    let mut seen_transitions = HashSet::new();
    let mut pending_deltas: Vec<(SectorRecoveryDelta, TransitionContext)> = Vec::new();

    for record in records {
        let record = record?;
        if record.index != expected {
            return Err(RecoveryError::NonContiguous {
                expected,
                actual: record.index,
            });
        }
        expected = expected.checked_next().ok_or(JournalError::IndexOverflow)?;

        if current_transition == Some(record.transition_id) && record.ordinal == 0 {
            return Err(RecoveryError::DuplicateTransition {
                transition_id: record.transition_id,
            });
        }
        if current_transition != Some(record.transition_id) {
            current_transition = Some(record.transition_id);
            validated_delta = false;
            if !seen_transitions.insert(record.transition_id) {
                return Err(RecoveryError::DuplicateTransition {
                    transition_id: record.transition_id,
                });
            }
            if record.ordinal != 0 || record.stream != JournalStream::RecoveryDelta {
                return Err(RecoveryError::MissingDelta {
                    transition_id: record.transition_id,
                });
            }
        }

        if record.stream != JournalStream::RecoveryDelta {
            continue;
        }
        if validated_delta {
            return Err(RecoveryError::DuplicateDelta {
                transition_id: record.transition_id,
            });
        }
        validated_delta = true;

        if record.context.sector_id != node.sector_id() {
            return Err(RecoveryError::SectorMismatch {
                expected: node.sector_id(),
                actual: record.context.sector_id,
            });
        }
        let delta = decode_recovery_delta(&record.payload)?;
        pending_deltas.push((
            delta,
            TransitionContext {
                sector_id: record.context.sector_id,
                owner_epoch: record.context.owner_epoch,
            },
        ));
    }

    if expected != expected_end {
        return Err(RecoveryError::UnexpectedEnd {
            actual: expected,
            expected: expected_end,
        });
    }
    for (delta, context) in pending_deltas {
        node.apply_recovery_delta(delta, context)?;
    }
    Ok((node, expected))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        galaxy::Galaxy, game_data::test_catalog_arc, node::SimulationNode,
        transition::SectorTransitionId,
    };
    use dawn_core::{NodeId, SectorBounds, SectorId};
    use dawn_storage::{
        DurabilityContext, DurabilityMode, InMemoryJournal, JournalBatch, JournalEntry,
    };
    use std::sync::Arc;

    fn node() -> SimulationNode {
        SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            Arc::new(Galaxy::demo()),
            test_catalog_arc(),
        )
    }

    #[test]
    fn eventless_tick_tail_is_applied() {
        let source = node();
        let prepared = source
            .prepare_tick_transition(SectorTransitionId(1), 0)
            .unwrap();
        let mut journal = InMemoryJournal::new();
        crate::transition_journal::append_prepared_transition(
            &mut journal,
            &prepared,
            DurabilityMode::Synced,
        )
        .unwrap();

        let restored = node();
        let (restored, _) = apply_tail(restored, &journal, 0).unwrap();
        assert_eq!(restored.current_tick(), dawn_core::Tick(1));
    }

    #[test]
    fn checkpoint_plus_tail_reproduces_the_authoritative_snapshot() {
        let mut source = node();
        let ship_id = source.spawn_ship(
            dawn_core::ShipTypeId(1),
            dawn_core::Position::new(10.0, 20.0, 30.0),
            dawn_core::Velocity::new(2.0, 0.0, 0.0),
        );
        let checkpoint = source.take_snapshot_at(0);
        let mut journal = InMemoryJournal::new();

        crate::transit::commit_tick_state_transition(
            &mut source,
            &mut journal,
            crate::transition::FrameInput::lock_only(&[]),
            SectorTransitionId(2),
            0,
            DurabilityMode::Synced,
        )
        .unwrap();
        let expected = source.take_snapshot_at(journal.next_index().unwrap().0);

        let restored =
            SimulationNode::restore_from(&checkpoint, Arc::new(Galaxy::demo()), test_catalog_arc());
        let (restored, recovered_to) =
            apply_tail(restored, &journal, checkpoint.covered_recovery_index)
                .expect("checkpoint tail should apply");

        assert_eq!(recovered_to, journal.next_index().unwrap());
        assert_eq!(restored.current_tick(), dawn_core::Tick(1));
        assert_eq!(restored.ship_count(), 1);
        assert_eq!(
            restored.get_ship_position(ship_id),
            source.get_ship_position(ship_id)
        );
        assert_eq!(
            restored.take_snapshot_at(recovered_to.0).tick,
            expected.tick
        );
        assert_eq!(
            restored.take_snapshot_at(recovered_to.0).ships,
            expected.ships
        );
    }

    #[test]
    fn public_only_batch_is_rejected_without_mutating_the_node() {
        let mut journal = InMemoryJournal::new();
        let batch = JournalBatch::with_entries(
            TransitionId(99),
            DurabilityContext {
                sector_id: SectorId(0),
                owner_epoch: 0,
            },
            vec![JournalEntry::new(
                JournalStream::PublicEvent,
                b"projection".to_vec(),
            )],
            DurabilityMode::Synced,
        )
        .unwrap();
        journal.append_batch(batch).unwrap();

        let restored = node();
        let error = apply_tail(restored, &journal, 0).unwrap_err();

        assert!(matches!(error, RecoveryError::MissingDelta { .. }));
    }

    #[test]
    fn a_transition_id_cannot_reappear_in_a_later_batch() {
        let mut journal = InMemoryJournal::new();
        for (from, to) in [
            (dawn_core::Tick::ZERO, dawn_core::Tick(1)),
            (dawn_core::Tick(1), dawn_core::Tick(2)),
        ] {
            let delta = SectorRecoveryDelta::Tick(Box::new(
                crate::transition::TickRecoveryDelta::logical(from, to),
            ));
            let batch = JournalBatch::with_entries(
                TransitionId(7),
                DurabilityContext {
                    sector_id: SectorId(0),
                    owner_epoch: 0,
                },
                vec![JournalEntry::new(
                    JournalStream::RecoveryDelta,
                    crate::transition::encode_recovery_delta(&delta).unwrap(),
                )],
                DurabilityMode::Synced,
            )
            .unwrap();
            journal.append_batch(batch).unwrap();
        }

        let error = apply_tail(node(), &journal, 0).unwrap_err();
        assert!(matches!(
            error,
            RecoveryError::DuplicateTransition {
                transition_id: TransitionId(7)
            }
        ));
    }
}
