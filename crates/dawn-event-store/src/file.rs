//! `FileEventStore` — append-only Event Log backed by a binary file.
//!
//! # File format
//!
//! ```text
//! [8 bytes u64 LE: base_index]              ← log_index of the first record
//! [4 bytes u32 LE: payload length][payload] ← record 0 (= base_index)
//! [4 bytes u32 LE: payload length][payload] ← record 1 (= base_index + 1)
//! ...
//! ```
//!
//! Records are never modified or deleted (INV-001). The `EventStore` trait stays
//! append-only (FBD-001).
//!
//! # Two-tier log / compaction (ADR-0017)
//!
//! The hot log can be compacted *behind a verified snapshot*: the prefix it
//! covers is moved to an append-only **cold archive** and the hot log is
//! atomically rewritten to hold only the surviving suffix. Global log indices are
//! preserved via the `base_index` header, so a compacted log is indistinguishable
//! from an un-compacted one to operational callers (which only read
//! `iter_from(snapshot.log_index)` with `snapshot.log_index >= base_index`).
//!
//! Compaction is an out-of-band operation ([`FileEventStore::compact`]), NOT an
//! `EventStore` trait method — it never rewrites a record in place, only moves
//! whole prefixes between files. Because `base_index` lives in the hot file
//! header, the atomic rename of the rewritten hot log carries the new base with
//! it: a crash leaves either the old full log or the new suffix, never a
//! mismatched pair.
//!
//! # Error handling
//!
//! `EventStore::append` cannot return an error (trait constraint); I/O failures
//! in `append` panic. `compact` returns `io::Result` (it is called off the hot
//! path).

use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use dawn_core::DomainEvent;

use crate::{store::EventStore, EventRecord};

/// Write one `[u32 len][payload]` record to `w`.
fn write_record(w: &mut impl Write, event: &DomainEvent) -> io::Result<()> {
    let payload = postcard::to_stdvec(event)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    w.write_all(&(payload.len() as u32).to_le_bytes())?;
    w.write_all(&payload)?;
    Ok(())
}

/// Sibling temp path used for the atomic rewrite during compaction.
fn tmp_path_for(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".compact.tmp");
    PathBuf::from(s)
}

// ── FileEventStore ────────────────────────────────────────────────────────────

pub struct FileEventStore {
    /// Path to the hot log file.
    path: PathBuf,
    /// Buffered writer positioned at the end of the file.
    writer: BufWriter<File>,
    /// In-memory mirror of the hot log — rebuilt from the file on open.
    records: Vec<EventRecord>,
    /// `log_index` of `records[0]`. Events before this live in the cold archive.
    base_index: u64,
}

impl FileEventStore {
    /// Open (or create) an event log file at `path`.
    ///
    /// If the file already exists, its header and all records are read into
    /// memory. A fresh file is created with a `base_index = 0` header.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();

        let (base_index, records) = if path.exists() && std::fs::metadata(&path)?.len() > 0 {
            Self::scan_file(&path)?
        } else {
            // Fresh log: write the base-index header (base = 0).
            let mut f = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&path)?;
            f.write_all(&0u64.to_le_bytes())?;
            f.sync_all()?;
            (0u64, Vec::new())
        };

        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            path,
            writer: BufWriter::new(file),
            records,
            base_index,
        })
    }

    /// Read the header and all records from an existing file.
    fn scan_file(path: &Path) -> io::Result<(u64, Vec<EventRecord>)> {
        let mut reader = BufReader::new(File::open(path)?);

        let mut header = [0u8; 8];
        reader.read_exact(&mut header)?;
        let base_index = u64::from_le_bytes(header);

        let mut records = Vec::new();
        let mut index = base_index;

        loop {
            let mut len_bytes = [0u8; 4];
            match reader.read_exact(&mut len_bytes) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e),
            }
            let len = u32::from_le_bytes(len_bytes) as usize;

            let mut data = vec![0u8; len];
            reader.read_exact(&mut data)?;

            let event = postcard::from_bytes::<DomainEvent>(&data)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

            records.push(EventRecord::new(index, event));
            index += 1;
        }

        Ok((base_index, records))
    }

    /// Number of records physically present in the hot log (excludes archived).
    pub fn records_on_disk(&self) -> usize {
        self.records.len()
    }

    /// `log_index` of the first record still in the hot log. Events with a lower
    /// index have been moved to the cold archive.
    pub fn base_index(&self) -> u64 {
        self.base_index
    }

    /// Compact the hot log behind a verified snapshot boundary (ADR-0017).
    ///
    /// Moves every event with `log_index < boundary` into the append-only cold
    /// archive at `cold_path`, then atomically rewrites the hot log to contain
    /// only `[boundary, ..)`. Global log indices are preserved.
    ///
    /// `boundary` must satisfy `base_index() <= boundary <= len()` and should be
    /// the `log_index` of a durable, verified snapshot (so operational restores,
    /// which read `iter_from(snapshot.log_index)`, never need the archived prefix).
    ///
    /// Crash safety: the prefix is made durable in the cold archive *before* the
    /// hot log is rewritten, and the rewrite is an atomic rename. A crash leaves
    /// either the old full hot log or the new suffix — never a lossy state. (A
    /// crash between the cold append and the rename may duplicate the prefix in
    /// the cold archive on retry; the archive is off-path audit data and tolerates
    /// this.)
    pub fn compact(&mut self, boundary: u64, cold_path: impl AsRef<Path>) -> io::Result<()> {
        assert!(
            boundary >= self.base_index,
            "compact boundary {boundary} below base {}",
            self.base_index
        );
        let cut = (boundary - self.base_index) as usize;
        assert!(
            cut <= self.records.len(),
            "compact boundary {boundary} beyond end {}",
            self.base_index + self.records.len() as u64
        );
        if cut == 0 {
            return Ok(());
        }

        // 1. Persist the prefix to the append-only cold archive first.
        Self::append_to_cold(cold_path.as_ref(), &self.records[..cut])?;

        // 2. Write the surviving suffix (with the new base header) to a temp file,
        //    fsync, then atomically replace the hot log.
        let tmp = tmp_path_for(&self.path);
        {
            let f = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&tmp)?;
            let mut w = BufWriter::new(f);
            w.write_all(&boundary.to_le_bytes())?;
            for rec in &self.records[cut..] {
                write_record(&mut w, &rec.event)?;
            }
            w.flush()?;
            w.get_ref().sync_all()?;
        }
        self.writer.flush()?;
        std::fs::rename(&tmp, &self.path)?;

        // 3. Update in-memory state and reopen the append writer on the new file.
        self.base_index = boundary;
        self.records.drain(..cut);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        self.writer = BufWriter::new(file);
        Ok(())
    }

    /// Append `records` to the cold archive (append-only, header-less stream).
    fn append_to_cold(cold_path: &Path, records: &[EventRecord]) -> io::Result<()> {
        let f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(cold_path)?;
        let mut w = BufWriter::new(f);
        for rec in records {
            write_record(&mut w, &rec.event)?;
        }
        w.flush()?;
        w.get_ref().sync_all()?;
        Ok(())
    }
}

impl EventStore for FileEventStore {
    fn append(&mut self, event: DomainEvent) -> u64 {
        let index = self.base_index + self.records.len() as u64;

        write_record(&mut self.writer, &event).expect("event log write failed");
        self.writer.flush().expect("event log flush failed");

        self.records.push(EventRecord::new(index, event));
        index
    }

    fn iter_from(&self, from_index: u64) -> impl Iterator<Item = &EventRecord> {
        // Indices below `base_index` live in the cold archive (off-path); clamp
        // to the start of the hot log.
        let start = (from_index.saturating_sub(self.base_index) as usize).min(self.records.len());
        self.records[start..].iter()
    }

    fn len(&self) -> usize {
        (self.base_index + self.records.len() as u64) as usize
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{events::VelocityChanged, NodeId, ShipId, Tick, Velocity};

    fn moved_event(n: u64, tick: u64) -> DomainEvent {
        DomainEvent::VelocityChanged(VelocityChanged {
            ship_id: ShipId::new(NodeId(0), n),
            velocity: Velocity::new(1.0, 0.0, 0.0),
            tick: Tick(tick),
        })
    }

    /// Count records in a header-less archive stream (test helper).
    fn count_archive_records(bytes: &[u8]) -> usize {
        let mut off = 0;
        let mut count = 0;
        while off + 4 <= bytes.len() {
            let len = u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap()) as usize;
            off += 4 + len;
            count += 1;
        }
        count
    }

    #[test]
    fn events_survive_close_and_reopen_of_the_log_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.log");

        // Write 5 events.
        {
            let mut store = FileEventStore::open(&path).unwrap();
            for i in 0..5 {
                store.append(moved_event(i, i));
            }
        }

        // Reopen and verify.
        let store = FileEventStore::open(&path).unwrap();
        assert_eq!(store.len(), 5);
        assert_eq!(store.records_on_disk(), 5);
    }

    #[test]
    fn log_indices_are_consecutive_across_two_open_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.log");

        // Session 1: write 3 events (indices 0, 1, 2).
        {
            let mut store = FileEventStore::open(&path).unwrap();
            for i in 0..3 {
                store.append(moved_event(i, i));
            }
        }

        // Session 2: append 2 more (indices 3, 4).
        {
            let mut store = FileEventStore::open(&path).unwrap();
            let idx3 = store.append(moved_event(3, 3));
            let idx4 = store.append(moved_event(4, 4));
            assert_eq!(idx3, 3);
            assert_eq!(idx4, 4);
        }

        let store = FileEventStore::open(&path).unwrap();
        assert_eq!(store.len(), 5);
    }

    #[test]
    fn iter_from_returns_only_records_at_or_after_given_index() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.log");

        let mut store = FileEventStore::open(&path).unwrap();
        for i in 0..10 {
            store.append(moved_event(i, i));
        }

        let tail: Vec<_> = store.iter_from(7).collect();
        assert_eq!(tail.len(), 3);
        assert_eq!(tail[0].log_index, 7);
        assert_eq!(tail[2].log_index, 9);
    }

    #[test]
    fn file_store_and_memory_store_produce_identical_event_sequences() {
        use crate::InMemoryEventStore;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.log");

        let events: Vec<_> = (0..5).map(|i| moved_event(i, i)).collect();

        let mut mem = InMemoryEventStore::new();
        let mut file = FileEventStore::open(&path).unwrap();

        for e in &events {
            mem.append(e.clone());
            file.append(e.clone());
        }

        // Both must produce the same sequence.
        for (m, f) in mem.iter_from(0).zip(file.iter_from(0)) {
            assert_eq!(m.log_index, f.log_index);
            assert_eq!(m.event, f.event);
        }
    }

    // ── Compaction (ADR-0017 8A-2/8A-3) ──────────────────────────────────────

    #[test]
    fn compaction_preserves_global_indices_and_serves_the_tail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.log");
        let cold = dir.path().join("cold.log");

        let mut store = FileEventStore::open(&path).unwrap();
        for i in 0..10 {
            store.append(moved_event(i, i));
        } // indices 0..=9

        store.compact(6, &cold).unwrap(); // archive [0,6); hot = [6,9]

        assert_eq!(store.base_index(), 6);
        assert_eq!(store.len(), 10, "global length is unchanged by compaction");
        assert_eq!(store.records_on_disk(), 4, "hot log holds 6,7,8,9");

        let tail: Vec<_> = store.iter_from(6).collect();
        assert_eq!(tail.len(), 4);
        assert_eq!(tail[0].log_index, 6);
        assert_eq!(tail[3].log_index, 9);

        // Appends continue at the global next index.
        assert_eq!(store.append(moved_event(10, 10)), 10);
    }

    #[test]
    fn compacted_hot_log_reopens_with_preserved_base_index() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.log");
        let cold = dir.path().join("cold.log");

        {
            let mut s = FileEventStore::open(&path).unwrap();
            for i in 0..8 {
                s.append(moved_event(i, i));
            } // 0..=7
            s.compact(5, &cold).unwrap(); // hot = 5,6,7
            s.append(moved_event(8, 8)); // hot = 5,6,7,8
        }

        let s = FileEventStore::open(&path).unwrap();
        assert_eq!(
            s.base_index(),
            5,
            "base_index survives reopen (file header)"
        );
        assert_eq!(s.len(), 9, "global length 0..=8");
        let tail: Vec<_> = s.iter_from(5).collect();
        assert_eq!(tail[0].log_index, 5);
        assert_eq!(tail.last().unwrap().log_index, 8);
    }

    #[test]
    fn cold_archive_receives_the_compacted_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.log");
        let cold = dir.path().join("cold.log");

        let mut store = FileEventStore::open(&path).unwrap();
        for i in 0..10 {
            store.append(moved_event(i, i));
        }
        store.compact(4, &cold).unwrap();

        let bytes = std::fs::read(&cold).unwrap();
        assert_eq!(
            count_archive_records(&bytes),
            4,
            "cold archive holds the [0,4) prefix"
        );
    }

    #[test]
    fn compacting_the_entire_hot_log_leaves_an_empty_suffix_that_still_appends() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.log");
        let cold = dir.path().join("cold.log");

        let mut store = FileEventStore::open(&path).unwrap();
        for i in 0..5 {
            store.append(moved_event(i, i));
        }
        store.compact(5, &cold).unwrap(); // archive everything

        assert_eq!(store.base_index(), 5);
        assert_eq!(store.records_on_disk(), 0);
        assert_eq!(store.len(), 5);
        assert_eq!(store.iter_from(5).count(), 0);
        assert_eq!(
            store.append(moved_event(5, 5)),
            5,
            "next append is still global index 5"
        );
    }
}
