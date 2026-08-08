//! In-memory implementation of the generic durable-journal contract.

use crate::journal::{
    content_hash, validate_batch, AppendReceipt, DurableJournal, JournalBatch, JournalError,
    JournalIndex, JournalRange, JournalRecord,
};

/// Failure-free journal used by engine tests and as a reference implementation.
#[derive(Debug, Default)]
pub struct InMemoryJournal {
    records: Vec<JournalRecord>,
}

impl InMemoryJournal {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn records(&self) -> &[JournalRecord] {
        &self.records
    }
}

impl DurableJournal for InMemoryJournal {
    fn append_batch(&mut self, batch: JournalBatch) -> Result<AppendReceipt, JournalError> {
        validate_batch(&batch)?;
        let first = JournalIndex(
            self.records
                .len()
                .try_into()
                .map_err(|_| JournalError::IndexOverflow)?,
        );
        let len = batch.records.len() as u32;
        let last = first
            .0
            .checked_add(u64::from(len))
            .ok_or(JournalError::IndexOverflow)?;
        let receipt = AppendReceipt {
            transition_id: batch.transition_id,
            context: batch.context,
            range: JournalRange { first, len },
            content_hash: content_hash(first, &batch),
            durability: batch.durability,
        };

        let transition_id = batch.transition_id;
        let context = batch.context;
        self.records.extend(
            batch
                .records
                .into_iter()
                .enumerate()
                .map(|(ordinal, payload)| JournalRecord {
                    index: JournalIndex(first.0 + ordinal as u64),
                    transition_id,
                    context,
                    ordinal: ordinal as u32,
                    payload,
                }),
        );
        debug_assert_eq!(self.records.len() as u64, last);
        Ok(receipt)
    }

    fn read_from(
        &self,
        index: JournalIndex,
    ) -> Result<Box<dyn Iterator<Item = Result<JournalRecord, JournalError>> + '_>, JournalError>
    {
        Ok(Box::new(
            self.records
                .iter()
                .filter(move |record| record.index >= index)
                .cloned()
                .map(Ok),
        ))
    }

    fn next_index(&self) -> Result<JournalIndex, JournalError> {
        self.records
            .len()
            .try_into()
            .map(JournalIndex)
            .map_err(|_| JournalError::IndexOverflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::{DurabilityContext, DurabilityMode, TransitionId};
    use dawn_core::SectorId;

    fn batch(records: &[&[u8]]) -> JournalBatch {
        JournalBatch::new(
            TransitionId(7),
            DurabilityContext {
                sector_id: SectorId(3),
                owner_epoch: 11,
            },
            records.iter().map(|record| record.to_vec()).collect(),
            DurabilityMode::Synced,
        )
    }

    #[test]
    fn appending_a_batch_assigns_one_contiguous_range_and_receipt() {
        let mut journal = InMemoryJournal::new();
        let receipt = journal.append_batch(batch(&[b"state", b"event"])).unwrap();

        assert_eq!(receipt.range.first, JournalIndex::ZERO);
        assert_eq!(receipt.range.len, 2);
        assert_eq!(
            receipt.range.checked_last_exclusive(),
            Some(JournalIndex(2))
        );
        assert!(receipt.range.contains(JournalIndex(0)));
        assert!(!receipt.range.contains(JournalIndex(2)));
        assert_eq!(receipt.transition_id, TransitionId(7));
        assert_eq!(journal.next_index().unwrap(), JournalIndex(2));
        let encoded = postcard::to_stdvec(&receipt).unwrap();
        let decoded: AppendReceipt = postcard::from_bytes(&encoded).unwrap();
        assert_eq!(decoded, receipt);
        assert!(receipt.matches(
            TransitionId(7),
            receipt.context,
            receipt.range,
            receipt.content_hash,
        ));
        assert!(!receipt.matches(
            TransitionId(8),
            receipt.context,
            receipt.range,
            receipt.content_hash,
        ));
        assert_eq!(JournalIndex(1).checked_next(), Some(JournalIndex(2)));
        assert_eq!(JournalIndex(u64::MAX).checked_next(), None);
    }

    #[test]
    fn an_invalid_batch_does_not_change_the_next_index() {
        let mut journal = InMemoryJournal::new();
        let error = journal
            .append_batch(JournalBatch::new(
                TransitionId(1),
                DurabilityContext {
                    sector_id: SectorId(0),
                    owner_epoch: 0,
                },
                Vec::new(),
                DurabilityMode::Buffered,
            ))
            .unwrap_err();

        assert!(matches!(error, JournalError::EmptyBatch));
        assert_eq!(journal.next_index().unwrap(), JournalIndex::ZERO);
        assert!(journal.records().is_empty());
    }

    #[test]
    fn reading_from_an_index_returns_records_without_requiring_a_domain_event_type() {
        let mut journal = InMemoryJournal::new();
        journal.append_batch(batch(&[b"a", b"b"])).unwrap();
        journal
            .append_batch(JournalBatch::new(
                TransitionId(8),
                DurabilityContext {
                    sector_id: SectorId(3),
                    owner_epoch: 11,
                },
                vec![b"c".to_vec()],
                DurabilityMode::Buffered,
            ))
            .unwrap();

        let payloads: Vec<_> = journal
            .read_from(JournalIndex(1))
            .unwrap()
            .map(|record| record.unwrap().payload)
            .collect();
        assert_eq!(payloads, vec![b"b".to_vec(), b"c".to_vec()]);
    }

    #[test]
    fn invalid_ranges_do_not_wrap_their_exclusive_end() {
        let range = JournalRange {
            first: JournalIndex(u64::MAX),
            len: 1,
        };
        assert_eq!(range.checked_last_exclusive(), None);
        assert!(!range.contains(JournalIndex(u64::MAX)));
    }
}
