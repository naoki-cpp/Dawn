//! Snapshot cadence policy owned by the runtime.
//!
//! The authoritative engine only captures state. This adapter owns the
//! external journal, snapshot publication, and compaction ordering.

use std::io;
use std::path::{Path, PathBuf};

use dawn_storage::{DurableJournal, EventStore, FileEventStore, FileJournal, JournalIndex};

use super::snapshot::StateSnapshot;
use crate::node::SimulationNode;

/// Where checkpoints are written and how often.
#[derive(Debug, Clone)]
pub struct CheckpointConfig {
    /// Number of logical ticks between checkpoints.
    pub interval_ticks: u64,
    /// Path for the atomically replaced latest authoritative snapshot.
    pub snapshot_path: PathBuf,
    /// Path for the append-only cold archive that receives compacted prefixes.
    pub cold_path: PathBuf,
}

/// Fires a runtime-owned checkpoint on a fixed logical-tick cadence.
#[derive(Debug)]
pub struct CheckpointScheduler {
    config: CheckpointConfig,
    last_checkpoint_tick: u64,
}

/// Runtime-owned journal operations required by checkpoint scheduling.
///
/// The engine does not depend on this trait. It exists at the persistence
/// adapter boundary so the public-event projection log and the authoritative
/// recovery journal remain independently owned.
pub trait CheckpointJournal {
    fn next_recovery_index(&self) -> io::Result<u64>;
    fn compact_checkpoint(&mut self, boundary: u64, cold_path: &Path) -> io::Result<()>;
}

impl CheckpointJournal for FileJournal {
    fn next_recovery_index(&self) -> io::Result<u64> {
        self.next_index()
            .map(|index| index.0)
            .map_err(|error| io::Error::other(error.to_string()))
    }

    fn compact_checkpoint(&mut self, boundary: u64, cold_path: &Path) -> io::Result<()> {
        self.compact(JournalIndex(boundary), cold_path)
            .map(|_| ())
            .map_err(|error| io::Error::other(error.to_string()))
    }
}

impl CheckpointJournal for FileEventStore {
    fn next_recovery_index(&self) -> io::Result<u64> {
        Ok(self.next_index())
    }

    fn compact_checkpoint(&mut self, boundary: u64, cold_path: &Path) -> io::Result<()> {
        self.compact(boundary, cold_path)
    }
}

impl CheckpointScheduler {
    pub fn new(config: CheckpointConfig) -> Self {
        Self {
            config,
            last_checkpoint_tick: 0,
        }
    }

    /// Snapshot the engine at the journal's next global position, then compact
    /// that same journal only after the snapshot has been published.
    pub fn maybe_checkpoint<J: CheckpointJournal>(
        &mut self,
        node: &mut SimulationNode,
        journal: &mut J,
    ) -> io::Result<Option<StateSnapshot>> {
        let tick = node.current_tick().value();
        if tick.saturating_sub(self.last_checkpoint_tick) < self.config.interval_ticks {
            return Ok(None);
        }

        let covered_recovery_index = journal.next_recovery_index()?;
        let snapshot = node.take_snapshot_at(covered_recovery_index);
        snapshot.save(&self.config.snapshot_path)?;
        journal.compact_checkpoint(covered_recovery_index, &self.config.cold_path)?;
        self.last_checkpoint_tick = tick;
        Ok(Some(snapshot))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{NodeId, SectorBounds, SectorId, ShipTypeId};
    use dawn_storage::{
        DurabilityContext, DurabilityMode, DurableJournal, JournalBatch, JournalEntry,
        JournalStream, TransitionId,
    };

    fn node() -> SimulationNode {
        SimulationNode::new_test(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        )
    }

    fn cfg(dir: &std::path::Path) -> CheckpointConfig {
        CheckpointConfig {
            interval_ticks: 5,
            snapshot_path: dir.join("snapshot.bin"),
            cold_path: dir.join("cold.log"),
        }
    }

    fn seed_journal(journal: &mut FileJournal) {
        let batch = JournalBatch::with_entries(
            TransitionId(1),
            DurabilityContext {
                sector_id: SectorId(0),
                owner_epoch: 0,
            },
            vec![JournalEntry::new(
                JournalStream::RecoveryDelta,
                vec![1, 2, 3],
            )],
            DurabilityMode::Synced,
        )
        .unwrap();
        journal.append_batch(batch).unwrap();
    }

    #[test]
    fn scheduler_waits_for_the_logical_tick_interval() {
        let dir = tempfile::tempdir().unwrap();
        let mut node = node();
        let mut journal = FileJournal::open(dir.path().join("hot.log")).unwrap();
        let mut scheduler = CheckpointScheduler::new(cfg(dir.path()));

        for _ in 0..4 {
            node.tick();
            assert!(scheduler
                .maybe_checkpoint(&mut node, &mut journal)
                .unwrap()
                .is_none());
        }
        assert!(!dir.path().join("snapshot.bin").exists());
    }

    #[test]
    fn scheduler_records_external_journal_position_and_compacts_after_publish() {
        let dir = tempfile::tempdir().unwrap();
        let mut node = node();
        node.spawn_ship(
            ShipTypeId(1),
            dawn_core::Position::ORIGIN,
            dawn_core::Velocity::ZERO,
        );
        let mut journal = FileJournal::open(dir.path().join("hot.log")).unwrap();
        seed_journal(&mut journal);
        let mut scheduler = CheckpointScheduler::new(cfg(dir.path()));

        for _ in 0..5 {
            node.tick();
        }
        let snapshot = scheduler
            .maybe_checkpoint(&mut node, &mut journal)
            .unwrap()
            .expect("checkpoint at tick five");

        assert_eq!(snapshot.covered_recovery_index, 1);
        assert_eq!(journal.next_recovery_index().unwrap(), 1);
        assert!(dir.path().join("snapshot.bin").exists());
        assert!(dir.path().join("cold.log").exists());
    }

    #[test]
    fn scheduler_checkpoints_pending_transit_saga_without_waiting_for_ack() {
        let dir = tempfile::tempdir().unwrap();
        let mut node = node();
        let ship_id = node.spawn_ship(
            ShipTypeId(1),
            dawn_core::Position::ORIGIN,
            dawn_core::Velocity::ZERO,
        );
        node.prepare_transit_commit(ship_id, SectorId(1), None)
            .expect("Transit request must create a durable Saga attempt");
        let mut journal = FileJournal::open(dir.path().join("hot.log")).unwrap();
        seed_journal(&mut journal);
        let mut scheduler = CheckpointScheduler::new(cfg(dir.path()));

        for _ in 0..5 {
            node.tick();
        }
        let snapshot = scheduler
            .maybe_checkpoint(&mut node, &mut journal)
            .unwrap()
            .expect("pending Transit must not block a checkpoint");

        assert_eq!(snapshot.transit_saga.outgoing.len(), 1);
        assert_eq!(snapshot.transit_saga.outgoing[0].ship_id, ship_id);
        assert!(matches!(
            snapshot.transit_saga.outgoing[0].state,
            crate::persistence::TransitAttemptState::Prepared
        ));
    }
}
