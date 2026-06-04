//! `FileEventStore` — append-only Event Log backed by a binary file.
//!
//! # File format
//!
//! Each record is written as:
//! ```text
//! [4 bytes u32 LE: payload length][N bytes: postcard-encoded DomainEvent]
//! ```
//!
//! Records are never modified or deleted (INV-001).
//! On open, all existing records are read into memory to rebuild the index.
//! Subsequent appends write to the end of the file and flush immediately.
//!
//! # Error handling
//!
//! `EventStore::append` cannot return an error (trait constraint).
//! I/O failures in `append` currently panic.  Production-grade error
//! handling (write-ahead + CRC) is deferred to Phase 6.

use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use dawn_core::DomainEvent;

use crate::{store::EventStore, EventRecord};

// ── FileEventStore ────────────────────────────────────────────────────────────

pub struct FileEventStore {
    /// Path kept for diagnostics / snapshot metadata.
    #[allow(dead_code)]
    path   : PathBuf,
    /// Buffered writer positioned at the end of the file.
    writer : BufWriter<File>,
    /// In-memory mirror — rebuilt from the file on open.
    records: Vec<EventRecord>,
}

impl FileEventStore {
    /// Open (or create) an event log file at `path`.
    ///
    /// If the file already exists, all records are read into memory.
    /// New appends are written to the end of the file.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();

        let records = if path.exists() {
            Self::scan_file(&path)?
        } else {
            Vec::new()
        };

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;

        Ok(Self { path, writer: BufWriter::new(file), records })
    }

    /// Read all records from an existing file.
    fn scan_file(path: &Path) -> io::Result<Vec<EventRecord>> {
        let mut reader = BufReader::new(File::open(path)?);
        let mut records = Vec::new();
        let mut index   = 0u64;

        loop {
            // Length prefix (4 bytes).
            let mut len_bytes = [0u8; 4];
            match reader.read_exact(&mut len_bytes) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e),
            }
            let len = u32::from_le_bytes(len_bytes) as usize;

            // Payload.
            let mut data = vec![0u8; len];
            reader.read_exact(&mut data)?;

            let event = postcard::from_bytes::<DomainEvent>(&data)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

            records.push(EventRecord::new(index, event));
            index += 1;
        }

        Ok(records)
    }

    /// Number of records loaded from the file on open.
    pub fn records_on_disk(&self) -> usize {
        self.records.len()
    }
}

impl EventStore for FileEventStore {
    fn append(&mut self, event: DomainEvent) -> u64 {
        let index = self.records.len() as u64;

        // Serialise.
        let payload = postcard::to_stdvec(&event)
            .expect("DomainEvent serialisation must not fail");

        // Write length prefix + payload and flush.
        self.writer
            .write_all(&(payload.len() as u32).to_le_bytes())
            .expect("event log write failed");
        self.writer
            .write_all(&payload)
            .expect("event log write failed");
        self.writer.flush().expect("event log flush failed");

        // Update in-memory mirror.
        self.records.push(EventRecord::new(index, event));
        index
    }

    fn iter_from(&self, from_index: u64) -> impl Iterator<Item = &EventRecord> {
        let start = (from_index as usize).min(self.records.len());
        self.records[start..].iter()
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
        events::ShipMoved,
        NodeId, Position, ShipId, Tick,
    };

    fn moved_event(n: u64, tick: u64) -> DomainEvent {
        DomainEvent::ShipMoved(ShipMoved {
            ship_id: ShipId::new(NodeId(0), n),
            from   : Position::ORIGIN,
            to     : Position::new(1.0, 0.0, 0.0),
            tick   : Tick(tick),
        })
    }

    #[test]
    fn events_survive_close_and_reopen_of_the_log_file() {
        let dir  = tempfile::tempdir().unwrap();
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
        let dir  = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.log");

        // Session 1: write 3 events (indices 0, 1, 2).
        {
            let mut store = FileEventStore::open(&path).unwrap();
            for i in 0..3 { store.append(moved_event(i, i)); }
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
        let dir  = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.log");

        let mut store = FileEventStore::open(&path).unwrap();
        for i in 0..10 { store.append(moved_event(i, i)); }

        let tail: Vec<_> = store.iter_from(7).collect();
        assert_eq!(tail.len(), 3);
        assert_eq!(tail[0].log_index, 7);
        assert_eq!(tail[2].log_index, 9);
    }

    #[test]
    fn file_store_and_memory_store_produce_identical_event_sequences() {
        use crate::InMemoryEventStore;

        let dir  = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.log");

        let events: Vec<_> = (0..5).map(|i| moved_event(i, i)).collect();

        let mut mem  = InMemoryEventStore::new();
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
}
