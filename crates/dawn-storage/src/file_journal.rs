//! Versioned file implementation of the generic durable-journal contract.
//!
//! The journal stores logical transitions rather than a second public-event
//! file. Public `DomainEvent` bytes are one stream inside the authoritative
//! journal and are projected into a rebuildable replication tail by the
//! distributed runtime.
//!
//! ```text
//! magic[8] once per file (`DAWNJNL3`)
//! base_index[u64] once per file
//! header_checksum[u64] once per file
//! batch:
//!   record_count[u32]
//!   first_index[u64]
//!   transition_id[u128]
//!   sector_id[u8]
//!   owner_epoch[u64]
//!   durability_mode[u8]
//!   record: stream[u8] + payload_len[u32] + payload[payload_len]
//!   content_hash[u64]
//!   commit_marker[u64]
//! ```
//!
//! A trailing incomplete batch is truncated on open. A complete batch with a
//! bad digest or commit marker is rejected as corruption.
//!
//! Reads validate one bounded batch at a time; iteration reports corruption
//! when it reaches that batch, without yielding any of its records. Compaction
//! scans with the same decoder and copies validated ranges in fixed chunks.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::journal::{
    content_hash, validate_batch, AppendReceipt, CompactionReceipt, DurabilityContext,
    DurabilityMode, DurableJournal, JournalBatch, JournalEntry, JournalError, JournalIndex,
    JournalRange, JournalRecord, JournalStream, TransitionId, MAX_BATCH_BYTES,
    MAX_RECORDS_PER_BATCH, MAX_RECORD_BYTES,
};

const MAGIC: &[u8; 8] = b"DAWNJNL3";
const COMMIT_MARKER: u64 = 0x4441_574e_434f_4d4d;
#[cfg(test)]
const HEADER_LEN: u64 = 24;

fn header_checksum(base_index: JournalIndex) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in MAGIC.iter().chain(base_index.0.to_le_bytes().iter()) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

trait JournalWriter: Send {
    fn len(&self) -> io::Result<u64>;
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn flush(&mut self) -> io::Result<()>;
    fn sync_data(&mut self) -> io::Result<()>;
    fn truncate(&mut self, len: u64) -> io::Result<()>;
    fn reopen(&mut self, path: &Path) -> io::Result<()>;
}

#[derive(Debug)]
struct FileWriter {
    path: PathBuf,
    file: Option<File>,
}

impl FileWriter {
    fn open(path: &Path) -> io::Result<Self> {
        Ok(Self {
            path: path.to_path_buf(),
            file: Some(OpenOptions::new().create(true).append(true).open(path)?),
        })
    }

    fn file(&self) -> io::Result<&File> {
        self.file
            .as_ref()
            .ok_or_else(|| io::Error::other("journal writer is closed"))
    }

    fn file_mut(&mut self) -> io::Result<&mut File> {
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("journal writer is closed"))
    }
}

impl JournalWriter for FileWriter {
    fn len(&self) -> io::Result<u64> {
        self.file()?.metadata().map(|metadata| metadata.len())
    }

    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.file_mut()?.write_all(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file_mut()?.flush()
    }

    fn sync_data(&mut self) -> io::Result<()> {
        self.file()?.sync_data()
    }

    fn truncate(&mut self, len: u64) -> io::Result<()> {
        OpenOptions::new()
            .write(true)
            .open(&self.path)?
            .set_len(len)
    }

    fn reopen(&mut self, path: &Path) -> io::Result<()> {
        drop(self.file.take());
        self.path = path.to_path_buf();
        self.file = Some(OpenOptions::new().create(true).append(true).open(path)?);
        Ok(())
    }
}

pub struct FileJournal {
    path: PathBuf,
    writer: Box<dyn JournalWriter>,
    base_index: JournalIndex,
    next_index: JournalIndex,
    poisoned: bool,
}

impl fmt::Debug for FileJournal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileJournal")
            .field("path", &self.path)
            .field("base_index", &self.base_index)
            .field("next_index", &self.next_index)
            .field("poisoned", &self.poisoned)
            .finish()
    }
}

impl FileJournal {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, JournalError> {
        let path = path.as_ref().to_path_buf();
        Self::ensure_header(&path)?;
        let scan = scan_file(&path, true, |_, _, _, _| {})?;
        let writer = Box::new(FileWriter::open(&path)?);
        sync_parent_directory(&path)?;
        Ok(Self {
            path,
            writer,
            base_index: scan.base_index,
            next_index: scan.next_index,
            poisoned: false,
        })
    }

    #[cfg(test)]
    fn with_writer(
        path: impl AsRef<Path>,
        writer: Box<dyn JournalWriter>,
    ) -> Result<Self, JournalError> {
        let path = path.as_ref().to_path_buf();
        Self::ensure_header(&path)?;
        let scan = scan_file(&path, true, |_, _, _, _| {})?;
        Ok(Self {
            path,
            writer,
            base_index: scan.base_index,
            next_index: scan.next_index,
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
            file.write_all(&0u64.to_le_bytes())?;
            file.write_all(&header_checksum(JournalIndex::ZERO).to_le_bytes())?;
            file.sync_all()?;
            sync_parent_directory(path)?;
        }
        Ok(())
    }

    fn rollback(&mut self, start: u64) -> Result<(), JournalError> {
        self.writer.flush()?;
        self.writer.truncate(start)?;
        self.writer.sync_data()?;
        self.writer.reopen(&self.path)?;
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
            .write_all(&(batch.entries.len() as u32).to_le_bytes())?;
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
        for entry in &batch.entries {
            self.writer.write_all(&[match entry.stream {
                JournalStream::RecoveryDelta => 0,
                JournalStream::PublicEvent => 1,
                JournalStream::ReliableEffect => 2,
            }])?;
            self.writer
                .write_all(&(entry.payload.len() as u32).to_le_bytes())?;
            self.writer.write_all(&entry.payload)?;
        }
        self.writer
            .write_all(&content_hash(first, batch).to_le_bytes())?;
        self.writer.write_all(&COMMIT_MARKER.to_le_bytes())?;
        Ok(())
    }

    pub fn base_index(&self) -> JournalIndex {
        self.base_index
    }

    /// Archive complete transition batches covered by a verified checkpoint.
    ///
    /// The archive is append-only and is made durable before the hot suffix is
    /// atomically replaced. A crash between those steps is recoverable: the
    /// archive already contains the prefix and the old hot file still contains
    /// the full log. `read_from` intentionally serves the retained hot range;
    /// consumers that need archived history read the archive independently.
    pub fn compact(
        &mut self,
        boundary: JournalIndex,
        archive_path: impl AsRef<Path>,
    ) -> Result<CompactionReceipt, JournalError> {
        if self.poisoned {
            return Err(JournalError::Poisoned);
        }
        if boundary < self.base_index || boundary > self.next_index {
            return Err(JournalError::InvalidCompactionBoundary { boundary });
        }
        if boundary == self.base_index {
            return Ok(CompactionReceipt {
                archived: JournalRange {
                    first: boundary,
                    len: 0,
                },
                retained_from: boundary,
                next_index: self.next_index,
            });
        }

        let archive_path = archive_path.as_ref();
        reject_archive_alias(&self.path, archive_path)?;
        let old_base = self.base_index;
        let archived_len =
            u32::try_from(boundary.0 - old_base.0).map_err(|_| JournalError::IndexOverflow)?;
        let archive_scan = ensure_archive(archive_path, self.base_index)?;
        if archive_scan.base_index > self.base_index
            || archive_scan.next_index < self.base_index
            || archive_scan.next_index > boundary
        {
            return Err(JournalError::ArchiveMismatch {
                expected: self.base_index,
                actual: archive_scan.next_index,
            });
        }

        let mut archive_start = None;
        let mut archive_end = None;
        let mut hot_base_start = None;
        let mut hot_overlap_end = None;
        let mut boundary_offset = None;
        scan_file(&self.path, false, |first, start, end, batch| {
            let batch_end = JournalIndex(first.0 + batch.entries.len() as u64);
            if first == self.base_index {
                hot_base_start = Some(start);
            }
            if first == boundary {
                boundary_offset = Some(start);
            }
            if first == archive_scan.next_index {
                archive_start = Some(start);
            }
            if batch_end == boundary {
                archive_end = Some(end);
            }
            if batch_end == archive_scan.next_index {
                hot_overlap_end = Some(end);
            }
            if first < boundary && batch_end > boundary {
                // The closure cannot return an error; the boundary is checked
                // again below using the captured offsets and journal index.
                boundary_offset = None;
            }
        })?;

        if boundary != self.next_index && boundary_offset.is_none() {
            return Err(JournalError::InvalidCompactionBoundary { boundary });
        }
        let boundary_offset = match boundary_offset {
            Some(offset) => offset,
            None if boundary == self.next_index => {
                // The only valid case without a following batch is compacting
                // the complete hot suffix.
                std::fs::metadata(&self.path)?.len()
            }
            None => return Err(JournalError::InvalidCompactionBoundary { boundary }),
        };
        let archive_end =
            archive_end.ok_or(JournalError::InvalidCompactionBoundary { boundary })?;
        let mut hot_reader = File::open(&self.path)?;
        if archive_scan.next_index > self.base_index {
            let mut archive_overlap_start = None;
            let mut archive_overlap_end = None;
            scan_file(archive_path, false, |first, start, end, batch| {
                let batch_end = JournalIndex(first.0 + batch.entries.len() as u64);
                if first == self.base_index {
                    archive_overlap_start = Some(start);
                }
                if batch_end == archive_scan.next_index {
                    archive_overlap_end = Some(end);
                }
            })?;
            let hot_start = hot_base_start.ok_or(JournalError::ArchiveMismatch {
                expected: self.base_index,
                actual: archive_scan.next_index,
            })?;
            let hot_end = hot_overlap_end.ok_or(JournalError::ArchiveMismatch {
                expected: self.base_index,
                actual: archive_scan.next_index,
            })?;
            let archive_start = archive_overlap_start.ok_or(JournalError::ArchiveMismatch {
                expected: self.base_index,
                actual: archive_scan.next_index,
            })?;
            let archive_end = archive_overlap_end.ok_or(JournalError::ArchiveMismatch {
                expected: self.base_index,
                actual: archive_scan.next_index,
            })?;
            if hot_end - hot_start != archive_end - archive_start
                || !archive_range_matches_hot(
                    archive_path,
                    archive_start,
                    archive_end,
                    &mut hot_reader,
                    hot_start,
                    hot_end,
                )?
            {
                return Err(JournalError::ArchiveMismatch {
                    expected: self.base_index,
                    actual: archive_scan.next_index,
                });
            }
        }
        if archive_scan.next_index < boundary && archive_start.is_none() {
            return Err(JournalError::ArchiveMismatch {
                expected: archive_scan.next_index,
                actual: boundary,
            });
        }

        if archive_scan.next_index < boundary {
            let start = match archive_start {
                Some(start) => start,
                None => {
                    return Err(JournalError::ArchiveMismatch {
                        expected: archive_scan.next_index,
                        actual: boundary,
                    })
                }
            };
            let end = archive_end;
            append_archive_range(archive_path, &mut hot_reader, start, end)?;
        }

        let tmp = tmp_path_for(&self.path);
        {
            let file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&tmp)?;
            let mut writer = BufWriter::new(file);
            writer.write_all(MAGIC)?;
            writer.write_all(&boundary.0.to_le_bytes())?;
            writer.write_all(&header_checksum(boundary).to_le_bytes())?;
            let hot_end = hot_reader.metadata()?.len();
            copy_range(&mut hot_reader, &mut writer, boundary_offset, hot_end)?;
            writer.flush()?;
            writer.get_ref().sync_all()?;
        }
        self.writer.flush()?;
        self.finalize_compaction_with(boundary, &tmp, FileWriter::open, sync_parent_directory)?;

        Ok(CompactionReceipt {
            archived: JournalRange {
                first: old_base,
                len: archived_len,
            },
            retained_from: boundary,
            next_index: self.next_index,
        })
    }

    fn finalize_compaction_with<Open, Sync>(
        &mut self,
        boundary: JournalIndex,
        tmp: &Path,
        reopen: Open,
        sync_parent: Sync,
    ) -> Result<(), JournalError>
    where
        Open: FnOnce(&Path) -> io::Result<FileWriter>,
        Sync: FnOnce(&Path) -> io::Result<()>,
    {
        std::fs::rename(tmp, &self.path)?;

        // The pathname now points at the replacement. Until the new writer and
        // directory entry are both ready, appending could acknowledge bytes
        // that are not safely attached to the current journal state.
        self.poisoned = true;
        self.base_index = boundary;
        let writer = reopen(&self.path)?;
        self.writer = Box::new(writer);
        sync_parent(&self.path)?;
        self.poisoned = false;
        Ok(())
    }
}

impl DurableJournal for FileJournal {
    fn append_batch(&mut self, batch: JournalBatch) -> Result<AppendReceipt, JournalError> {
        if self.poisoned {
            return Err(JournalError::Poisoned);
        }
        validate_batch(&batch)?;
        let len = u32::try_from(batch.entries.len()).map_err(|_| JournalError::TooManyRecords {
            actual: batch.entries.len(),
            max: MAX_RECORDS_PER_BATCH,
        })?;
        let last = self
            .next_index
            .0
            .checked_add(u64::from(len))
            .ok_or(JournalError::IndexOverflow)?;
        let start = self.writer.len()?;
        if let Err(error) = self.write_batch(self.next_index, &batch) {
            return Err(self.rollback_after_error(start, error));
        }

        if let Err(error) = self.writer.flush() {
            return Err(self.rollback_after_error(start, error.into()));
        }
        if batch.durability == DurabilityMode::Synced {
            if let Err(error) = self.writer.sync_data() {
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
        if index < self.base_index {
            return Err(JournalError::CompactedRange {
                requested: index,
                available_from: self.base_index,
            });
        }
        let (reader, base_index) = open_reader(&self.path, false)?;
        Ok(Box::new(RecordReader {
            reader,
            next_index: base_index,
            requested: index,
            records: Vec::new().into_iter(),
            finished: false,
        }))
    }

    fn next_index(&self) -> Result<JournalIndex, JournalError> {
        Ok(self.next_index)
    }
}

fn tmp_path_for(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(".compact.tmp");
    PathBuf::from(value)
}

fn sync_parent_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        File::open(parent)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

fn reject_archive_alias(hot_path: &Path, archive_path: &Path) -> Result<(), JournalError> {
    let tmp_path = tmp_path_for(hot_path);
    if paths_alias(archive_path, hot_path)?
        || paths_alias(archive_path, &tmp_path)?
        || paths_alias(&tmp_path, hot_path)?
    {
        return Err(JournalError::ArchivePathAlias);
    }
    Ok(())
}

fn paths_alias(left: &Path, right: &Path) -> io::Result<bool> {
    if left == right || !left.exists() || !right.exists() {
        return Ok(left == right);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        let left = std::fs::metadata(left)?;
        let right = std::fs::metadata(right)?;
        Ok(left.dev() == right.dev() && left.ino() == right.ino())
    }
    #[cfg(not(unix))]
    {
        Ok(std::fs::canonicalize(left)? == std::fs::canonicalize(right)?)
    }
}

const COPY_BUFFER_BYTES: usize = 8 * 1024;

fn copy_range(reader: &mut File, writer: &mut impl Write, start: u64, end: u64) -> io::Result<()> {
    reader.seek(SeekFrom::Start(start))?;
    copy_exact(
        reader,
        writer,
        end.checked_sub(start)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "reversed journal range"))?,
        &mut [0; COPY_BUFFER_BYTES],
    )
}

fn copy_exact(
    reader: &mut impl Read,
    writer: &mut impl Write,
    mut remaining: u64,
    buffer: &mut [u8],
) -> io::Result<()> {
    if buffer.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "empty copy buffer",
        ));
    }
    while remaining > 0 {
        let len = remaining.min(buffer.len() as u64) as usize;
        reader.read_exact(&mut buffer[..len])?;
        writer.write_all(&buffer[..len])?;
        remaining -= len as u64;
    }
    Ok(())
}

fn archive_range_matches_hot(
    archive_path: &Path,
    archive_start: u64,
    archive_end: u64,
    hot_reader: &mut File,
    hot_start: u64,
    hot_end: u64,
) -> Result<bool, JournalError> {
    if archive_end < archive_start
        || hot_end < hot_start
        || archive_end - archive_start != hot_end - hot_start
    {
        return Ok(false);
    }
    let mut archive = File::open(archive_path)?;
    archive.seek(SeekFrom::Start(archive_start))?;
    hot_reader.seek(SeekFrom::Start(hot_start))?;
    ranges_match(
        &mut archive,
        hot_reader,
        hot_end - hot_start,
        &mut [0; COPY_BUFFER_BYTES],
        &mut [0; COPY_BUFFER_BYTES],
    )
    .map_err(Into::into)
}

fn ranges_match(
    left: &mut impl Read,
    right: &mut impl Read,
    mut remaining: u64,
    left_buffer: &mut [u8],
    right_buffer: &mut [u8],
) -> io::Result<bool> {
    let capacity = left_buffer.len().min(right_buffer.len());
    if capacity == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "empty comparison buffer",
        ));
    }
    while remaining > 0 {
        let len = remaining.min(capacity as u64) as usize;
        left.read_exact(&mut left_buffer[..len])?;
        right.read_exact(&mut right_buffer[..len])?;
        if left_buffer[..len] != right_buffer[..len] {
            return Ok(false);
        }
        remaining -= len as u64;
    }
    Ok(true)
}

fn ensure_archive(path: &Path, base_index: JournalIndex) -> Result<JournalScan, JournalError> {
    if !path.exists() || std::fs::metadata(path)?.len() == 0 {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        file.write_all(MAGIC)?;
        file.write_all(&base_index.0.to_le_bytes())?;
        file.write_all(&header_checksum(base_index).to_le_bytes())?;
        file.sync_all()?;
    }
    let scan = scan_file(path, true, |_, _, _, _| {})?;
    sync_parent_directory(path)?;
    Ok(scan)
}

fn append_archive_range(
    path: &Path,
    reader: &mut File,
    start: u64,
    end: u64,
) -> Result<(), JournalError> {
    let file = OpenOptions::new().append(true).open(path)?;
    let mut writer = BufWriter::new(file);
    copy_range(reader, &mut writer, start, end)?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct JournalScan {
    base_index: JournalIndex,
    next_index: JournalIndex,
}

fn open_reader(
    path: &Path,
    writable: bool,
) -> Result<(BufReader<File>, JournalIndex), JournalError> {
    let file = OpenOptions::new().read(true).write(writable).open(path)?;
    let mut reader = BufReader::new(file);
    let mut magic = [0u8; 8];
    reader.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(JournalError::UnsupportedFormat);
    }

    let mut base_bytes = [0u8; 8];
    reader.read_exact(&mut base_bytes)?;
    let base_index = JournalIndex(u64::from_le_bytes(base_bytes));
    let mut checksum_bytes = [0u8; 8];
    reader.read_exact(&mut checksum_bytes)?;
    if u64::from_le_bytes(checksum_bytes) != header_checksum(base_index) {
        return Err(JournalError::HeaderChecksumMismatch);
    }
    Ok((reader, base_index))
}

fn read_batch(
    reader: &mut BufReader<File>,
    expected: JournalIndex,
) -> Result<Option<JournalBatch>, JournalError> {
    let record_count = match read_u32_or_eof(reader) {
        Ok(Some(value)) => value,
        Ok(None) => return Ok(None),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
            return Err(JournalError::IncompleteBatch);
        }
        Err(error) => return Err(error.into()),
    };
    let Some(first_index) = read_u64_or_incomplete(reader)? else {
        return Err(JournalError::IncompleteBatch);
    };
    let first_index = JournalIndex(first_index);
    if first_index != expected {
        return Err(JournalError::NonContiguousBatch {
            expected,
            actual: first_index,
        });
    }
    let Some(transition_id) = read_u128_or_incomplete(reader)? else {
        return Err(JournalError::IncompleteBatch);
    };
    let Some(sector_id) = read_u8_or_incomplete(reader)? else {
        return Err(JournalError::IncompleteBatch);
    };
    let Some(owner_epoch) = read_u64_or_incomplete(reader)? else {
        return Err(JournalError::IncompleteBatch);
    };
    let Some(mode) = read_u8_or_incomplete(reader)? else {
        return Err(JournalError::IncompleteBatch);
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
    let mut entries = Vec::with_capacity(record_count);
    let mut total_bytes = 0usize;
    for _ in 0..record_count {
        let Some(stream) = read_u8_or_incomplete(reader)? else {
            return Err(JournalError::IncompleteBatch);
        };
        let stream = match stream {
            0 => JournalStream::RecoveryDelta,
            1 => JournalStream::PublicEvent,
            2 => JournalStream::ReliableEffect,
            _ => return Err(JournalError::InvalidFormat("unknown journal stream".into())),
        };
        let Some(len) = read_u32_or_incomplete(reader)? else {
            return Err(JournalError::IncompleteBatch);
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
        let Some(payload) = read_bytes_or_incomplete(reader, len)? else {
            return Err(JournalError::IncompleteBatch);
        };
        entries.push(JournalEntry::new(stream, payload));
    }
    let Some(expected_hash) = read_u64_or_incomplete(reader)? else {
        return Err(JournalError::IncompleteBatch);
    };
    let Some(marker) = read_u64_or_incomplete(reader)? else {
        return Err(JournalError::IncompleteBatch);
    };
    if marker != COMMIT_MARKER {
        return Err(JournalError::InvalidCommitMarker);
    }

    let batch =
        JournalBatch::with_entries(TransitionId(transition_id), context, entries, durability)?;
    if content_hash(first_index, &batch) != expected_hash {
        return Err(JournalError::ContentHashMismatch);
    }
    Ok(Some(batch))
}

fn scan_file<F>(
    path: &Path,
    repair_trailing_batch: bool,
    mut on_batch: F,
) -> Result<JournalScan, JournalError>
where
    F: FnMut(JournalIndex, u64, u64, &JournalBatch),
{
    let (mut reader, base_index) = open_reader(path, repair_trailing_batch)?;
    let mut next_index = base_index;
    loop {
        let start = reader.stream_position()?;
        let batch = match read_batch(&mut reader, next_index) {
            Ok(Some(batch)) => batch,
            Ok(None) => break,
            Err(JournalError::IncompleteBatch) if repair_trailing_batch => {
                let file = reader.into_inner();
                file.set_len(start)?;
                file.sync_all()?;
                break;
            }
            Err(error) => return Err(error),
        };
        let end = reader.stream_position()?;
        let following = next_index
            .0
            .checked_add(batch.entries.len() as u64)
            .map(JournalIndex)
            .ok_or(JournalError::IndexOverflow)?;
        on_batch(next_index, start, end, &batch);
        next_index = following;
    }
    Ok(JournalScan {
        base_index,
        next_index,
    })
}

// Only one fully validated batch is retained. Payloads move into records;
// callers never observe a prefix of a corrupt transition.
struct RecordReader {
    reader: BufReader<File>,
    next_index: JournalIndex,
    requested: JournalIndex,
    records: std::vec::IntoIter<JournalRecord>,
    finished: bool,
}

impl Iterator for RecordReader {
    type Item = Result<JournalRecord, JournalError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(record) = self.records.next() {
                if record.index >= self.requested {
                    return Some(Ok(record));
                }
                continue;
            }
            if self.finished {
                return None;
            }
            // Release the previous batch before allocating its successor.
            self.records = Vec::new().into_iter();
            let batch = match read_batch(&mut self.reader, self.next_index) {
                Ok(Some(batch)) => batch,
                Ok(None) => {
                    self.finished = true;
                    return None;
                }
                Err(error) => {
                    self.finished = true;
                    return Some(Err(error));
                }
            };
            let first = self.next_index;
            self.next_index = match first.0.checked_add(batch.entries.len() as u64) {
                Some(next) => JournalIndex(next),
                None => {
                    self.finished = true;
                    return Some(Err(JournalError::IndexOverflow));
                }
            };
            self.records = batch
                .entries
                .into_iter()
                .enumerate()
                .map(|(ordinal, entry)| JournalRecord {
                    index: JournalIndex(first.0 + ordinal as u64),
                    transition_id: batch.transition_id,
                    context: batch.context,
                    stream: entry.stream,
                    ordinal: ordinal as u32,
                    payload: entry.payload,
                })
                .collect::<Vec<_>>()
                .into_iter();
        }
    }
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
    use crate::journal::{
        DurabilityContext, DurabilityMode, JournalEntry, JournalStream, TransitionId,
    };

    #[test]
    fn suffix_reports_later_corruption_only_when_reached_without_emitting_its_prefix() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("journal.bin");
        let mut journal = FileJournal::open(&path).unwrap();
        journal.append_batch(batch(1, &[b"first"])).unwrap();
        journal
            .append_batch(batch(2, &[b"second", b"third"]))
            .unwrap();
        let len = std::fs::metadata(&path).unwrap().len();
        let mut file = OpenOptions::new().write(true).open(&path).unwrap();
        file.seek(SeekFrom::Start(len - 16)).unwrap();
        file.write_all(&0u64.to_le_bytes()).unwrap();
        drop(file);

        let mut records = journal.read_from(JournalIndex::ZERO).unwrap();
        assert_eq!(records.next().unwrap().unwrap().payload, b"first");
        assert!(matches!(
            records.next(),
            Some(Err(JournalError::ContentHashMismatch))
        ));
        assert!(records.next().is_none());
        assert!(records.next().is_none());
    }

    #[test]
    fn lazy_reader_retains_only_one_batch_and_preserves_requested_ordinals() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("journal.bin");
        let mut journal = FileJournal::open(&path).unwrap();
        for id in 0..100 {
            journal
                .append_batch(batch(id, &[b"a", b"b", b"c"]))
                .unwrap();
        }
        let (reader, base) = open_reader(&path, false).unwrap();
        let mut records = RecordReader {
            reader,
            next_index: base,
            requested: JournalIndex(1),
            records: Vec::new().into_iter(),
            finished: false,
        };
        let record = records.next().unwrap().unwrap();
        assert_eq!(record.index, JournalIndex(1));
        assert_eq!(record.ordinal, 1);
        assert_eq!(records.next_index, JournalIndex(3));
        assert_eq!(records.records.len(), 1);
        assert_eq!(records.count(), 298);
    }

    #[test]
    fn range_copy_and_comparison_use_small_buffers_and_reject_truncation() {
        let bytes: Vec<u8> = (0..101).collect();
        let mut output = Vec::new();
        copy_exact(&mut io::Cursor::new(&bytes), &mut output, 101, &mut [0; 3]).unwrap();
        assert_eq!(output, bytes);
        assert!(ranges_match(
            &mut io::Cursor::new(&bytes),
            &mut io::Cursor::new(&output),
            101,
            &mut [0; 3],
            &mut [0; 5]
        )
        .unwrap());
        output[99] ^= 1;
        assert!(!ranges_match(
            &mut io::Cursor::new(&bytes),
            &mut io::Cursor::new(&output),
            101,
            &mut [0; 3],
            &mut [0; 5]
        )
        .unwrap());
        assert_eq!(
            copy_exact(
                &mut io::Cursor::new([1, 2]),
                &mut Vec::new(),
                3,
                &mut [0; 1]
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::UnexpectedEof
        );
    }

    #[test]
    fn compaction_copies_multiple_chunks_and_validates_archived_overlap() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("journal.bin");
        let archive = directory.path().join("archive.bin");
        let mut journal = FileJournal::open(&path).unwrap();
        let payload = vec![42; COPY_BUFFER_BYTES * 3 + 7];
        for id in 0..3 {
            journal.append_batch(batch(id, &[&payload])).unwrap();
        }
        // Stage the archive prefix without replacing hot, as in a failed compaction.
        let mut staged = FileJournal::open(&archive).unwrap();
        staged.append_batch(batch(0, &[&payload])).unwrap();
        drop(staged);
        journal.compact(JournalIndex(2), &archive).unwrap();
        journal.append_batch(batch(3, &[&payload])).unwrap();
        journal.compact(JournalIndex(3), &archive).unwrap();
        let archived = FileJournal::open(&archive).unwrap();
        let records = archived
            .read_from(JournalIndex::ZERO)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(records.len(), 3);
        assert!(records.iter().all(|record| record.payload == payload));
        let retained = journal
            .read_from(JournalIndex(3))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(retained.len(), 1);
        assert_eq!(retained[0].payload, payload);
    }

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
        .unwrap()
    }

    fn stream_batch(id: u128) -> JournalBatch {
        JournalBatch::with_entries(
            TransitionId(id),
            DurabilityContext {
                sector_id: dawn_core::SectorId(4),
                owner_epoch: 9,
            },
            vec![
                JournalEntry::new(JournalStream::RecoveryDelta, b"delta".to_vec()),
                JournalEntry::new(JournalStream::PublicEvent, b"event".to_vec()),
                JournalEntry::new(JournalStream::ReliableEffect, b"outbox".to_vec()),
            ],
            DurabilityMode::Synced,
        )
        .unwrap()
    }

    #[derive(Debug, Clone, Copy)]
    enum FailurePoint {
        Write,
        Flush,
        Sync,
    }

    struct FailingWriter {
        inner: FileWriter,
        point: FailurePoint,
        failed: bool,
    }

    impl JournalWriter for FailingWriter {
        fn len(&self) -> io::Result<u64> {
            self.inner.len()
        }

        fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
            if matches!(self.point, FailurePoint::Write) && !self.failed {
                self.failed = true;
                let split = bytes.len() / 2;
                self.inner.write_all(&bytes[..split])?;
                return Err(io::Error::other("injected write failure"));
            }
            self.inner.write_all(bytes)
        }

        fn flush(&mut self) -> io::Result<()> {
            if matches!(self.point, FailurePoint::Flush) && !self.failed {
                self.failed = true;
                return Err(io::Error::other("injected flush failure"));
            }
            self.inner.flush()
        }

        fn sync_data(&mut self) -> io::Result<()> {
            if matches!(self.point, FailurePoint::Sync) && !self.failed {
                self.failed = true;
                return Err(io::Error::other("injected sync failure"));
            }
            self.inner.sync_data()
        }

        fn truncate(&mut self, len: u64) -> io::Result<()> {
            self.inner.truncate(len)
        }

        fn reopen(&mut self, path: &Path) -> io::Result<()> {
            self.inner.reopen(path)
        }
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
    fn one_reopened_batch_keeps_recovery_public_and_reliable_streams_together() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("journal.bin");
        let mut journal = FileJournal::open(&path).unwrap();
        journal.append_batch(stream_batch(22)).unwrap();
        drop(journal);

        let journal = FileJournal::open(&path).unwrap();
        let records: Vec<_> = journal
            .read_from(JournalIndex::ZERO)
            .unwrap()
            .map(|record| record.unwrap())
            .collect();
        assert_eq!(
            records
                .iter()
                .map(|record| record.stream)
                .collect::<Vec<_>>(),
            vec![
                JournalStream::RecoveryDelta,
                JournalStream::PublicEvent,
                JournalStream::ReliableEffect,
            ]
        );
        assert!(records
            .iter()
            .all(|record| record.transition_id == TransitionId(22)));
    }

    #[test]
    fn compaction_archives_a_complete_prefix_and_keeps_global_ranges() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("journal.bin");
        let archive = directory.path().join("journal.archive");
        {
            let mut journal = FileJournal::open(&path).unwrap();
            journal.append_batch(batch(1, &[b"first"])).unwrap();
            journal.append_batch(batch(2, &[b"second"])).unwrap();
            let receipt = journal.compact(JournalIndex(1), &archive).unwrap();
            assert_eq!(receipt.archived.first, JournalIndex(0));
            assert_eq!(receipt.archived.len, 1);
            assert_eq!(journal.base_index(), JournalIndex(1));
            assert_eq!(journal.next_index().unwrap(), JournalIndex(2));
            assert!(matches!(
                journal.read_from(JournalIndex::ZERO),
                Err(JournalError::CompactedRange { .. })
            ));
            assert_eq!(
                journal
                    .read_from(JournalIndex(1))
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap()
                    .payload,
                b"second"
            );
        }

        let hot = FileJournal::open(&path).unwrap();
        assert_eq!(hot.base_index(), JournalIndex(1));
        assert_eq!(hot.next_index().unwrap(), JournalIndex(2));
        let cold = FileJournal::open(&archive).unwrap();
        assert_eq!(cold.base_index(), JournalIndex(0));
        assert_eq!(cold.next_index().unwrap(), JournalIndex(1));
        assert_eq!(
            cold.read_from(JournalIndex::ZERO)
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .payload,
            b"first"
        );
    }

    #[test]
    fn repeated_compaction_appends_to_the_same_archive() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("journal.bin");
        let archive = directory.path().join("journal.archive");
        let mut journal = FileJournal::open(&path).unwrap();
        journal.append_batch(batch(1, &[b"first"])).unwrap();
        journal.append_batch(batch(2, &[b"second"])).unwrap();
        journal.compact(JournalIndex(1), &archive).unwrap();

        journal.append_batch(batch(3, &[b"third"])).unwrap();
        let receipt = journal.compact(JournalIndex(2), &archive).unwrap();

        assert_eq!(receipt.archived.first, JournalIndex(1));
        assert_eq!(receipt.archived.len, 1);
        assert_eq!(journal.base_index(), JournalIndex(2));
        assert_eq!(journal.next_index().unwrap(), JournalIndex(3));

        let archive = FileJournal::open(&archive).unwrap();
        assert_eq!(archive.base_index(), JournalIndex::ZERO);
        assert_eq!(archive.next_index().unwrap(), JournalIndex(2));
        let records: Vec<_> = archive
            .read_from(JournalIndex::ZERO)
            .unwrap()
            .map(|record| record.unwrap().payload)
            .collect();
        assert_eq!(records, vec![b"first".to_vec(), b"second".to_vec()]);
    }

    #[test]
    fn compaction_can_retry_after_archive_append_before_hot_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("journal.bin");
        let archive = directory.path().join("journal.archive");
        let mut journal = FileJournal::open(&path).unwrap();
        journal.append_batch(batch(1, &[b"first"])).unwrap();
        journal.append_batch(batch(2, &[b"second"])).unwrap();

        let tmp = tmp_path_for(&path);
        std::fs::create_dir(&tmp).unwrap();
        assert!(journal.compact(JournalIndex(1), &archive).is_err());
        std::fs::remove_dir(&tmp).unwrap();

        let receipt = journal.compact(JournalIndex(1), &archive).unwrap();
        assert_eq!(receipt.archived.first, JournalIndex(0));
        assert_eq!(receipt.archived.len, 1);
        assert_eq!(journal.base_index(), JournalIndex(1));
        assert_eq!(journal.next_index().unwrap(), JournalIndex(2));
        let archive = FileJournal::open(&archive).unwrap();
        assert_eq!(archive.next_index().unwrap(), JournalIndex(1));
    }

    #[test]
    fn post_rename_reopen_failure_poisoned_journal_cannot_append() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("journal.bin");
        let mut journal = FileJournal::open(&path).unwrap();
        journal.append_batch(batch(1, &[b"first"])).unwrap();

        let tmp = tmp_path_for(&path);
        let mut replacement = MAGIC.to_vec();
        let base_index = JournalIndex(1);
        replacement.extend_from_slice(&base_index.0.to_le_bytes());
        replacement.extend_from_slice(&header_checksum(base_index).to_le_bytes());
        std::fs::write(&tmp, replacement).unwrap();
        let result = journal.finalize_compaction_with(
            JournalIndex(1),
            &tmp,
            |_path| Err(io::Error::other("injected reopen failure")),
            sync_parent_directory,
        );
        assert!(result.is_err());
        assert!(matches!(
            journal.append_batch(batch(2, &[b"must not append"])),
            Err(JournalError::Poisoned)
        ));

        let reopened = FileJournal::open(&path).unwrap();
        assert_eq!(reopened.base_index(), JournalIndex(1));
        assert_eq!(reopened.next_index().unwrap(), JournalIndex(1));
    }

    #[test]
    fn post_rename_directory_sync_failure_poisoned_journal_cannot_append() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("journal.bin");
        let mut journal = FileJournal::open(&path).unwrap();
        journal.append_batch(batch(1, &[b"first"])).unwrap();

        let tmp = tmp_path_for(&path);
        let mut replacement = MAGIC.to_vec();
        let base_index = JournalIndex(1);
        replacement.extend_from_slice(&base_index.0.to_le_bytes());
        replacement.extend_from_slice(&header_checksum(base_index).to_le_bytes());
        std::fs::write(&tmp, replacement).unwrap();
        let result =
            journal.finalize_compaction_with(JournalIndex(1), &tmp, FileWriter::open, |_path| {
                Err(io::Error::other("injected directory sync failure"))
            });
        assert!(result.is_err());
        assert!(matches!(
            journal.append_batch(batch(2, &[b"must not append"])),
            Err(JournalError::Poisoned)
        ));

        let reopened = FileJournal::open(&path).unwrap();
        assert_eq!(reopened.base_index(), JournalIndex(1));
        assert_eq!(reopened.next_index().unwrap(), JournalIndex(1));
    }

    #[test]
    fn compaction_rejects_archive_alias_before_mutating_the_hot_journal() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("journal.bin");
        let mut journal = FileJournal::open(&path).unwrap();
        journal.append_batch(batch(1, &[b"first"])).unwrap();

        for archive in [path.clone(), tmp_path_for(&path)] {
            assert!(matches!(
                journal.compact(JournalIndex(1), archive),
                Err(JournalError::ArchivePathAlias)
            ));
        }

        let record = journal
            .read_from(JournalIndex::ZERO)
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(record.payload, b"first");
        assert_eq!(journal.base_index(), JournalIndex::ZERO);
        assert_eq!(journal.next_index().unwrap(), JournalIndex(1));
    }

    #[test]
    fn bare_relative_paths_sync_the_current_directory() {
        assert!(sync_parent_directory(Path::new("journal.bin")).is_ok());
    }

    #[test]
    fn compaction_rejects_a_boundary_inside_one_logical_transition() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("journal.bin");
        let archive = directory.path().join("journal.archive");
        let mut journal = FileJournal::open(&path).unwrap();
        journal
            .append_batch(batch(1, &[b"first", b"second"]))
            .unwrap();
        assert!(matches!(
            journal.compact(JournalIndex(1), &archive),
            Err(JournalError::InvalidCompactionBoundary {
                boundary: JournalIndex(1)
            })
        ));
    }

    #[test]
    fn injected_write_flush_and_sync_failures_leave_no_committed_batch() {
        for point in [FailurePoint::Write, FailurePoint::Flush, FailurePoint::Sync] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("journal.bin");
            let writer = FailingWriter {
                inner: FileWriter::open(&path).unwrap(),
                point,
                failed: false,
            };
            let mut journal = FileJournal::with_writer(&path, Box::new(writer)).unwrap();
            assert!(journal
                .append_batch(batch(31, &[b"not committed"]))
                .is_err());
            drop(journal);

            let journal = FileJournal::open(&path).unwrap();
            assert_eq!(
                journal.next_index().unwrap(),
                JournalIndex::ZERO,
                "failure point: {point:?}"
            );
            assert!(journal
                .read_from(JournalIndex::ZERO)
                .unwrap()
                .next()
                .is_none());
        }
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
        let payload_offset = HEADER_LEN as usize + 4 + 8 + 16 + 1 + 8 + 1 + 1 + 4;
        bytes[payload_offset] ^= 0xff;
        std::fs::write(&path, bytes).unwrap();

        assert!(matches!(
            FileJournal::open(&path),
            Err(JournalError::ContentHashMismatch)
        ));
    }

    #[test]
    fn a_corrupted_header_is_rejected_even_for_an_empty_hot_suffix() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("journal.bin");
        let archive = directory.path().join("journal.archive");
        {
            let mut journal = FileJournal::open(&path).unwrap();
            journal.append_batch(batch(1, &[b"first"])).unwrap();
            journal.compact(JournalIndex(1), &archive).unwrap();
        }

        let mut bytes = std::fs::read(&path).unwrap();
        bytes[8] ^= 0xff;
        std::fs::write(&path, bytes).unwrap();

        assert!(matches!(
            FileJournal::open(&path),
            Err(JournalError::HeaderChecksumMismatch)
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

        let first_batch_bytes = 4 + 8 + 16 + 1 + 8 + 1 + 1 + 4 + 5 + 8 + 8;
        let second_first_index_offset = HEADER_LEN as usize + first_batch_bytes + 4;
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
