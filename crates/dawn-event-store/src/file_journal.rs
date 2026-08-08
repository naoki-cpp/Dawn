//! Versioned file implementation of the generic durable-journal contract.
//!
//! The format is intentionally separate from the legacy `FileEventStore`:
//! callers can migrate to this journal without pretending that public
//! `DomainEvent` bytes are the complete recovery payload.
//!
//! ```text
//! magic[8] once per file
//! batch:
//!   record_count[u32]
//!   first_index[u64]
//!   transition_id[u128]
//!   sector_id[u8]
//!   owner_epoch[u64]
//!   durability_mode[u8]
//!   record: payload_len[u32] + payload[payload_len]
//!   content_hash[u64]
//!   commit_marker[u64]
//! ```
//!
//! A trailing incomplete batch is truncated on open. A complete batch with a
//! bad digest or commit marker is rejected as corruption.

use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Seek, Write};
use std::path::{Path, PathBuf};

use crate::journal::{
    content_hash, validate_batch, AppendReceipt, DurabilityContext, DurabilityMode, DurableJournal,
    JournalBatch, JournalError, JournalIndex, JournalRange, JournalRecord, TransitionId,
    MAX_BATCH_BYTES, MAX_RECORDS_PER_BATCH, MAX_RECORD_BYTES,
};

const MAGIC: &[u8; 8] = b"DAWNJNL1";
const COMMIT_MARKER: u64 = 0x4441_574e_434f_4d4d;

#[derive(Debug)]
pub struct FileJournal {
    path: PathBuf,
    writer: BufWriter<File>,
    next_index: JournalIndex,
    poisoned: bool,
}

impl FileJournal {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, JournalError> {
        let path = path.as_ref().to_path_buf();
        Self::ensure_header(&path)?;
        let next_index = scan_file(&path, true, |_| {})?;
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            path,
            writer: BufWriter::new(file),
            next_index,
            poisoned: false,
        })
    }

    fn ensure_header(path: &Path) -> Result<(), JournalError> {
        if !path.exists() || std::fs::metadata(path)?.len() == 0 {
            let mut file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(path)?;
            file.write_all(MAGIC)?;
            file.sync_all()?;
        }
        Ok(())
    }

    fn rollback(&mut self, start: u64) -> Result<(), JournalError> {
        self.writer.flush()?;
        self.writer.get_ref().set_len(start)?;
        self.writer.get_ref().sync_data()?;
        Ok(())
    }

    fn rollback_after_error(&mut self, start: u64, error: JournalError) -> JournalError {
        match self.rollback(start) {
            Ok(()) => error,
            Err(rollback_error) => {
                self.poisoned = true;
                rollback_error
            }
        }
    }

    fn write_batch(
        &mut self,
        first: JournalIndex,
        batch: &JournalBatch,
    ) -> Result<(), JournalError> {
        self.writer
            .write_all(&(batch.records.len() as u32).to_le_bytes())?;
        self.writer.write_all(&first.0.to_le_bytes())?;
        self.writer
            .write_all(&batch.transition_id.0.to_le_bytes())?;
        self.writer.write_all(&[batch.context.sector_id.0])?;
        self.writer
            .write_all(&batch.context.owner_epoch.to_le_bytes())?;
        self.writer.write_all(&[match batch.durability {
            DurabilityMode::Buffered => 0,
            DurabilityMode::Synced => 1,
        }])?;
        for record in &batch.records {
            self.writer
                .write_all(&(record.len() as u32).to_le_bytes())?;
            self.writer.write_all(record)?;
        }
        self.writer
            .write_all(&content_hash(first, batch).to_le_bytes())?;
        self.writer.write_all(&COMMIT_MARKER.to_le_bytes())?;
        Ok(())
    }
}

impl DurableJournal for FileJournal {
    fn append_batch(&mut self, batch: JournalBatch) -> Result<AppendReceipt, JournalError> {
        if self.poisoned {
            return Err(JournalError::Poisoned);
        }
        validate_batch(&batch)?;
        let len = u32::try_from(batch.records.len()).map_err(|_| JournalError::TooManyRecords {
            actual: batch.records.len(),
            max: MAX_RECORDS_PER_BATCH,
        })?;
        let last = self
            .next_index
            .0
            .checked_add(u64::from(len))
            .ok_or(JournalError::IndexOverflow)?;
        let start = self.writer.get_ref().metadata()?.len();
        if let Err(error) = self.write_batch(self.next_index, &batch) {
            return Err(self.rollback_after_error(start, error));
        }

        if let Err(error) = self.writer.flush() {
            return Err(self.rollback_after_error(start, error.into()));
        }
        if batch.durability == DurabilityMode::Synced {
            if let Err(error) = self.writer.get_ref().sync_data() {
                return Err(self.rollback_after_error(start, error.into()));
            }
        }

        let receipt = AppendReceipt {
            transition_id: batch.transition_id,
            context: batch.context,
            range: JournalRange {
                first: self.next_index,
                len,
            },
            content_hash: content_hash(self.next_index, &batch),
            durability: batch.durability,
        };
        self.next_index = JournalIndex(last);
        Ok(receipt)
    }

    fn read_from(
        &self,
        index: JournalIndex,
    ) -> Result<Box<dyn Iterator<Item = Result<JournalRecord, JournalError>> + '_>, JournalError>
    {
        let mut records = Vec::new();
        scan_file(&self.path, false, |record| {
            if record.index >= index {
                records.push(record);
            }
        })?;
        Ok(Box::new(
            records
                .into_iter()
                .filter(move |record| record.index >= index)
                .map(Ok),
        ))
    }

    fn next_index(&self) -> Result<JournalIndex, JournalError> {
        Ok(self.next_index)
    }
}

fn scan_file<F>(
    path: &Path,
    repair_trailing_batch: bool,
    mut on_record: F,
) -> Result<JournalIndex, JournalError>
where
    F: FnMut(JournalRecord),
{
    let file = OpenOptions::new()
        .read(true)
        .write(repair_trailing_batch)
        .open(path)?;
    let mut reader = BufReader::new(file);
    let mut magic = [0u8; 8];
    reader.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(JournalError::UnsupportedFormat);
    }

    let mut next_index = JournalIndex::ZERO;
    let mut truncate_at = None;

    loop {
        let batch_start = reader.stream_position()?;
        let record_count = match read_u32_or_eof(&mut reader) {
            Ok(Some(value)) => value,
            Ok(None) => break,
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
                truncate_at = Some(batch_start);
                break;
            }
            Err(error) => return Err(error.into()),
        };
        let Some(first_index) = read_u64_or_incomplete(&mut reader)? else {
            truncate_at = Some(batch_start);
            break;
        };
        let first_index = JournalIndex(first_index);
        if first_index != next_index {
            return Err(JournalError::NonContiguousBatch {
                expected: next_index,
                actual: first_index,
            });
        }
        let Some(transition_id) = read_u128_or_incomplete(&mut reader)? else {
            truncate_at = Some(batch_start);
            break;
        };
        let Some(sector_id) = read_u8_or_incomplete(&mut reader)? else {
            truncate_at = Some(batch_start);
            break;
        };
        let Some(owner_epoch) = read_u64_or_incomplete(&mut reader)? else {
            truncate_at = Some(batch_start);
            break;
        };
        let Some(mode) = read_u8_or_incomplete(&mut reader)? else {
            truncate_at = Some(batch_start);
            break;
        };
        let durability = match mode {
            0 => DurabilityMode::Buffered,
            1 => DurabilityMode::Synced,
            _ => {
                return Err(JournalError::InvalidFormat(
                    "unknown durability mode".into(),
                ))
            }
        };
        let record_count = record_count as usize;
        if record_count == 0 || record_count > MAX_RECORDS_PER_BATCH {
            return Err(JournalError::InvalidFormat(
                "invalid record count in committed batch".into(),
            ));
        }

        let context = DurabilityContext {
            sector_id: dawn_core::SectorId(sector_id),
            owner_epoch,
        };
        let mut payloads = Vec::with_capacity(record_count);
        let mut total_bytes = 0usize;
        let mut incomplete = false;
        for _ in 0..record_count {
            let Some(len) = read_u32_or_incomplete(&mut reader)? else {
                incomplete = true;
                break;
            };
            let len = len as usize;
            if len > MAX_RECORD_BYTES {
                return Err(JournalError::RecordTooLarge {
                    actual: len,
                    max: MAX_RECORD_BYTES,
                });
            }
            total_bytes = total_bytes
                .checked_add(len)
                .ok_or(JournalError::BatchTooLarge {
                    actual: usize::MAX,
                    max: MAX_BATCH_BYTES,
                })?;
            if total_bytes > MAX_BATCH_BYTES {
                return Err(JournalError::BatchTooLarge {
                    actual: total_bytes,
                    max: MAX_BATCH_BYTES,
                });
            }
            let Some(payload) = read_bytes_or_incomplete(&mut reader, len)? else {
                incomplete = true;
                break;
            };
            payloads.push(payload);
        }
        if incomplete {
            truncate_at = Some(batch_start);
            break;
        }
        let Some(expected_hash) = read_u64_or_incomplete(&mut reader)? else {
            truncate_at = Some(batch_start);
            break;
        };
        let Some(marker) = read_u64_or_incomplete(&mut reader)? else {
            truncate_at = Some(batch_start);
            break;
        };
        if marker != COMMIT_MARKER {
            return Err(JournalError::InvalidCommitMarker);
        }

        let batch = JournalBatch::new(TransitionId(transition_id), context, payloads, durability);
        if content_hash(first_index, &batch) != expected_hash {
            return Err(JournalError::ContentHashMismatch);
        }
        for (ordinal, payload) in batch.records.into_iter().enumerate() {
            on_record(JournalRecord {
                index: JournalIndex(first_index.0 + ordinal as u64),
                transition_id: batch.transition_id,
                context: batch.context,
                ordinal: ordinal as u32,
                payload,
            });
        }
        next_index = next_index
            .0
            .checked_add(record_count as u64)
            .map(JournalIndex)
            .ok_or(JournalError::IndexOverflow)?;
    }

    if let Some(offset) = truncate_at {
        if !repair_trailing_batch {
            return Err(JournalError::IncompleteBatch);
        }
        let file = reader.into_inner();
        file.set_len(offset)?;
        file.sync_all()?;
    }
    Ok(next_index)
}

fn read_u32_or_eof(reader: &mut BufReader<File>) -> Result<Option<u32>, io::Error> {
    let mut bytes = [0u8; 4];
    let mut filled = 0;
    while filled < bytes.len() {
        let read = reader.read(&mut bytes[filled..])?;
        if read == 0 {
            return if filled == 0 {
                Ok(None)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "partial journal batch header",
                ))
            };
        }
        filled += read;
    }
    Ok(Some(u32::from_le_bytes(bytes)))
}

fn read_u8_or_incomplete(reader: &mut BufReader<File>) -> Result<Option<u8>, io::Error> {
    let mut bytes = [0u8; 1];
    match reader.read_exact(&mut bytes) {
        Ok(()) => Ok(Some(bytes[0])),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
        Err(error) => Err(error),
    }
}

fn read_u32_or_incomplete(reader: &mut BufReader<File>) -> Result<Option<u32>, io::Error> {
    let mut bytes = [0u8; 4];
    match reader.read_exact(&mut bytes) {
        Ok(()) => Ok(Some(u32::from_le_bytes(bytes))),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
        Err(error) => Err(error),
    }
}

fn read_u64_or_incomplete(reader: &mut BufReader<File>) -> Result<Option<u64>, io::Error> {
    let mut bytes = [0u8; 8];
    match reader.read_exact(&mut bytes) {
        Ok(()) => Ok(Some(u64::from_le_bytes(bytes))),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
        Err(error) => Err(error),
    }
}

fn read_u128_or_incomplete(reader: &mut BufReader<File>) -> Result<Option<u128>, io::Error> {
    let mut bytes = [0u8; 16];
    match reader.read_exact(&mut bytes) {
        Ok(()) => Ok(Some(u128::from_le_bytes(bytes))),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
        Err(error) => Err(error),
    }
}

fn read_bytes_or_incomplete(
    reader: &mut BufReader<File>,
    len: usize,
) -> Result<Option<Vec<u8>>, io::Error> {
    let mut bytes = vec![0u8; len];
    match reader.read_exact(&mut bytes) {
        Ok(()) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::{DurabilityContext, DurabilityMode, TransitionId};

    fn batch(id: u128, payloads: &[&[u8]]) -> JournalBatch {
        JournalBatch::new(
            TransitionId(id),
            DurabilityContext {
                sector_id: dawn_core::SectorId(4),
                owner_epoch: 9,
            },
            payloads.iter().map(|payload| payload.to_vec()).collect(),
            DurabilityMode::Synced,
        )
    }

    #[test]
    fn synced_batches_survive_close_and_reopen_with_receipt_bound_records() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("journal.bin");
        let receipt = {
            let mut journal = FileJournal::open(&path).unwrap();
            journal
                .append_batch(batch(17, &[b"delta", b"event"]))
                .unwrap()
        };

        let journal = FileJournal::open(&path).unwrap();
        let records: Vec<_> = journal
            .read_from(JournalIndex::ZERO)
            .unwrap()
            .map(|record| record.unwrap())
            .collect();
        assert_eq!(journal.next_index().unwrap(), JournalIndex(2));
        assert_eq!(
            receipt.range,
            JournalRange {
                first: JournalIndex::ZERO,
                len: 2
            }
        );
        assert_eq!(records[0].transition_id, receipt.transition_id);
        assert_eq!(records[1].payload, b"event");
    }

    #[test]
    fn an_incomplete_trailing_batch_is_removed_on_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("journal.bin");
        {
            let mut journal = FileJournal::open(&path).unwrap();
            journal.append_batch(batch(1, &[b"complete"])).unwrap();
        }
        let complete_len = std::fs::metadata(&path).unwrap().len();
        {
            let mut file = OpenOptions::new().append(true).open(&path).unwrap();
            file.write_all(&2u32.to_le_bytes()).unwrap();
        }

        let journal = FileJournal::open(&path).unwrap();
        assert_eq!(journal.next_index().unwrap(), JournalIndex(1));
        assert_eq!(std::fs::metadata(&path).unwrap().len(), complete_len);
    }

    #[test]
    fn a_corrupted_committed_payload_is_rejected_instead_of_replayed() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("journal.bin");
        {
            let mut journal = FileJournal::open(&path).unwrap();
            journal.append_batch(batch(1, &[b"payload"])).unwrap();
        }
        let mut bytes = std::fs::read(&path).unwrap();
        let payload_offset = 8 + 4 + 8 + 16 + 1 + 8 + 1 + 4;
        bytes[payload_offset] ^= 0xff;
        std::fs::write(&path, bytes).unwrap();

        assert!(matches!(
            FileJournal::open(&path),
            Err(JournalError::ContentHashMismatch)
        ));
    }

    #[test]
    fn a_non_contiguous_batch_is_rejected_instead_of_reindexed() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("journal.bin");
        {
            let mut journal = FileJournal::open(&path).unwrap();
            journal.append_batch(batch(1, &[b"first"])).unwrap();
            journal.append_batch(batch(2, &[b"second"])).unwrap();
        }

        let first_batch_bytes = 4 + 8 + 16 + 1 + 8 + 1 + 4 + 5 + 8 + 8;
        let second_first_index_offset = 8 + first_batch_bytes + 4;
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[second_first_index_offset..second_first_index_offset + 8]
            .copy_from_slice(&0u64.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();

        assert!(matches!(
            FileJournal::open(&path),
            Err(JournalError::NonContiguousBatch {
                expected: JournalIndex(1),
                actual: JournalIndex(0),
            })
        ));
    }
}
