//! DurableJournal adapter for prepared Sector transitions.
//!
//! This module is intentionally separate from [`crate::transition`]. The
//! latter is storage-independent; this file is the runtime boundary that
//! encodes its typed transition into the generic journal envelope.

use dawn_event_store::{
    encode_payload, AppendReceipt, DurabilityContext, DurabilityMode, DurableJournal, JournalBatch,
    JournalEntry, JournalError, JournalStream, TransitionId,
};

use crate::transition::{encode_recovery_delta, PreparedSectorTransition};

/// Append one prepared transition as one atomic journal batch.
pub(crate) fn append_prepared_transition<J: DurableJournal>(
    journal: &mut J,
    prepared: &PreparedSectorTransition,
    durability: DurabilityMode,
) -> Result<AppendReceipt, JournalError> {
    append_prepared_transition_with_events(journal, prepared, &[], durability)
}

/// Append a prepared transition together with public events accumulated by
/// command-side adapters before the Tick preparation began. Keeping those
/// events in the same JournalBatch preserves the one-transition visibility
/// boundary while the authoritative delta remains the first entry.
pub(crate) fn append_prepared_transition_with_events<J: DurableJournal>(
    journal: &mut J,
    prepared: &PreparedSectorTransition,
    preceding_public_events: &[dawn_core::DomainEvent],
    durability: DurabilityMode,
) -> Result<AppendReceipt, JournalError> {
    let mut entries = vec![JournalEntry::new(
        JournalStream::RecoveryDelta,
        encode_recovery_delta(&prepared.recovery_delta)
            .map_err(|error| JournalError::Encode(error.to_string()))?,
    )];

    entries.extend(
        preceding_public_events
            .iter()
            .map(|event| {
                encode_payload(event)
                    .map(|payload| JournalEntry::new(JournalStream::PublicEvent, payload))
            })
            .collect::<Result<Vec<_>, _>>()?,
    );
    entries.extend(
        prepared
            .public_events
            .iter()
            .map(|event| {
                encode_payload(event)
                    .map(|payload| JournalEntry::new(JournalStream::PublicEvent, payload))
            })
            .collect::<Result<Vec<_>, _>>()?,
    );
    entries.extend(
        prepared
            .reliable_effects
            .iter()
            .cloned()
            .map(|payload| JournalEntry::new(JournalStream::ReliableEffect, payload)),
    );

    let batch = JournalBatch::with_entries(
        TransitionId(prepared.transition_id.0),
        DurabilityContext {
            sector_id: prepared.context.sector_id,
            owner_epoch: prepared.context.owner_epoch,
        },
        entries,
        durability,
    )?;
    journal.append_batch(batch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transition::{
        SectorEngine, SectorTransitionId, StopCommandState, TransitionContext,
    };
    use dawn_core::{NodeId, SectorId, ShipId};
    use dawn_event_store::{InMemoryJournal, JournalIndex};

    #[test]
    fn prepared_transition_is_encoded_as_one_recovery_batch() {
        let ship_id = ShipId::new(NodeId(0), 1);
        let prepared = SectorEngine::prepare_stop(
            StopCommandState {
                ship_id,
                exists: true,
                is_docked: false,
                is_in_transit: false,
                is_warping: false,
            },
            SectorTransitionId(42),
            TransitionContext {
                sector_id: SectorId(0),
                owner_epoch: 3,
            },
        )
        .unwrap();
        let mut journal = InMemoryJournal::new();

        let receipt =
            append_prepared_transition(&mut journal, &prepared, DurabilityMode::Synced).unwrap();

        assert_eq!(receipt.range.first, JournalIndex::ZERO);
        assert_eq!(receipt.range.len, 1);
        assert_eq!(journal.records().len(), 1);
        let record = &journal.records()[0];
        assert_eq!(record.stream, JournalStream::RecoveryDelta);
        assert_eq!(
            crate::transition::decode_recovery_delta(&record.payload).unwrap(),
            prepared.recovery_delta
        );
    }
}
