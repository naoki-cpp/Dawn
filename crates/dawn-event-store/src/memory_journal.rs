//! In-memory implementation of the generic durable-journal contract.

use crate::journal::{
    content_hash, validate_batch, AppendReceipt, CompactionReceipt, DurableJournal, JournalBatch,
    JournalError, JournalIndex, JournalRange, JournalRecord,
};

/// Failure-free journal used by engine tests and as a reference implementation.
#[derive(Debug, Default)]
pub struct InMemoryJournal {
    records: Vec<JournalRecord>,
    base_index: JournalIndex,
}

impl InMemoryJournal {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn records(&self) -> &[JournalRecord] {
        &self.records
    }

    pub fn base_index(&self) -> JournalIndex {
        self.base_index
    }

    /// Drop a complete prefix after its checkpoint has been verified.
    pub fn compact(&mut self, boundary: JournalIndex) -> Result<CompactionReceipt, JournalError> {
        if boundary < self.base_index || boundary > self.next_index()? {
            return Err(JournalError::InvalidCompactionBoundary { boundary });
        }
        if boundary < self.next_index()? {
            let cut = (boundary.0 - self.base_index.0) as usize;
            if cut > 0 && self.records[cut - 1].transition_id == self.records[cut].transition_id {
                return Err(JournalError::InvalidCompactionBoundary { boundary });
            }
        }
        let archived = JournalRange {
            first: self.base_index,
            len: (boundary.0 - self.base_index.0)
                .try_into()
                .map_err(|_| JournalError::IndexOverflow)?,
        };
        let cut = archived.len as usize;
        self.records.drain(..cut);
        self.base_index = boundary;
        Ok(CompactionReceipt {
            archived,
            retained_from: boundary,
            next_index: self.next_index()?,
        })
    }
}

impl DurableJournal for InMemoryJournal {
    fn append_batch(&mut self, batch: JournalBatch) -> Result<AppendReceipt, JournalError> {
        validate_batch(&batch)?;
        let first = JournalIndex(
            self.base_index
                .0
                .checked_add(self.records.len() as u64)
                .ok_or(JournalError::IndexOverflow)?,
        );
        let len = batch.entries.len() as u32;
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
                .entries
                .into_iter()
                .enumerate()
                .map(|(ordinal, entry)| JournalRecord {
                    index: JournalIndex(first.0 + ordinal as u64),
                    transition_id,
                    context,
                    stream: entry.stream,
                    ordinal: ordinal as u32,
                    payload: entry.payload,
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
        if index < self.base_index {
            return Err(JournalError::CompactedRange {
                requested: index,
                available_from: self.base_index,
            });
        }
        Ok(Box::new(
            self.records
                .iter()
                .filter(move |record| record.index >= index)
                .cloned()
                .map(Ok),
        ))
    }

    fn next_index(&self) -> Result<JournalIndex, JournalError> {
        self.base_index
            .0
            .checked_add(self.records.len() as u64)
            .map(JournalIndex)
            .ok_or(JournalError::IndexOverflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::{
        encode_payload, DurabilityContext, DurabilityEvidence, DurabilityEvidenceSource,
        DurabilityMode, JournalEntry, JournalStream, TransitionId,
    };
    use dawn_core::SectorId;
    use serde::ser::Error as _;

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
        .unwrap()
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
        let evidence = DurabilityEvidence {
            receipt,
            source: DurabilityEvidenceSource::Remote,
        };
        let encoded = postcard::to_stdvec(&evidence).unwrap();
        let decoded: DurabilityEvidence = postcard::from_bytes(&encoded).unwrap();
        assert_eq!(decoded, evidence);
        assert_eq!(JournalIndex(1).checked_next(), Some(JournalIndex(2)));
        assert_eq!(JournalIndex(u64::MAX).checked_next(), None);
    }

    #[test]
    fn an_invalid_batch_does_not_change_the_next_index() {
        let journal = InMemoryJournal::new();
        let error = JournalBatch::new(
            TransitionId(1),
            DurabilityContext {
                sector_id: SectorId(0),
                owner_epoch: 0,
            },
            Vec::new(),
            DurabilityMode::Buffered,
        )
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
            .append_batch(
                JournalBatch::new(
                    TransitionId(8),
                    DurabilityContext {
                        sector_id: SectorId(3),
                        owner_epoch: 11,
                    },
                    vec![b"c".to_vec()],
                    DurabilityMode::Buffered,
                )
                .unwrap(),
            )
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

    #[test]
    fn receipt_rejects_wrong_owner_sector_range_or_payload_evidence() {
        let mut journal = InMemoryJournal::new();
        let batch = batch(&[b"delta"]);
        let receipt = journal.append_batch(batch.clone()).unwrap();

        assert!(receipt.validate_batch(&batch, receipt.range).is_ok());
        assert!(receipt
            .validate_batch(
                &JournalBatch::with_entries(
                    receipt.transition_id,
                    DurabilityContext {
                        sector_id: SectorId(99),
                        owner_epoch: receipt.context.owner_epoch,
                    },
                    vec![JournalEntry::new(
                        JournalStream::RecoveryDelta,
                        b"delta".to_vec()
                    )],
                    receipt.durability,
                )
                .unwrap(),
                receipt.range,
            )
            .is_err());
        assert!(receipt
            .validate_batch(
                &batch,
                JournalRange {
                    first: JournalIndex(1),
                    len: 1
                }
            )
            .is_err());
    }

    #[test]
    fn encoding_failure_is_reported_before_a_batch_can_be_appended() {
        struct FailingValue;

        impl serde::Serialize for FailingValue {
            fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                Err(S::Error::custom("intentional encoding failure"))
            }
        }

        assert!(matches!(
            encode_payload(&FailingValue),
            Err(JournalError::Encode(_))
        ));
    }

    #[test]
    fn compaction_preserves_global_indices_and_rejects_partial_transitions() {
        let mut journal = InMemoryJournal::new();
        journal.append_batch(batch(&[b"delta", b"event"])).unwrap();
        assert!(matches!(
            journal.compact(JournalIndex(1)),
            Err(JournalError::InvalidCompactionBoundary {
                boundary: JournalIndex(1)
            })
        ));

        let mut journal = InMemoryJournal::new();
        journal.append_batch(batch(&[b"first"])).unwrap();
        journal
            .append_batch(
                JournalBatch::new(
                    TransitionId(8),
                    DurabilityContext {
                        sector_id: SectorId(3),
                        owner_epoch: 11,
                    },
                    vec![b"second".to_vec()],
                    DurabilityMode::Synced,
                )
                .unwrap(),
            )
            .unwrap();
        let receipt = journal.compact(JournalIndex(1)).unwrap();
        assert_eq!(receipt.archived.first, JournalIndex(0));
        assert_eq!(receipt.archived.len, 1);
        assert_eq!(journal.base_index(), JournalIndex(1));
        assert_eq!(journal.next_index().unwrap(), JournalIndex(2));
        assert!(matches!(
            journal.read_from(JournalIndex(0)),
            Err(JournalError::CompactedRange { .. })
        ));
    }
}
