//! Snapshot-cadence orchestration (ADR-0017 8A-7).
//!
//! [`SimulationNode::checkpoint`](crate::node::SimulationNode::checkpoint) is the
//! mechanism: snapshot -> atomically publish -> compact. This module is the
//! cadence adapter; Sector Transit recovery policy, including whether compaction
//! is currently safe, lives in `transit::pipeline`.

use std::io;
use std::path::PathBuf;

use dawn_event_store::FileEventStore;

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

/// Fires [`SimulationNode::checkpoint`] on a fixed logical-tick cadence.
#[derive(Debug)]
pub struct CheckpointScheduler {
    config: CheckpointConfig,
    /// Logical tick of the most recent checkpoint (starts at 0 = node genesis).
    last_checkpoint_tick: u64,
}

impl CheckpointScheduler {
    pub fn new(config: CheckpointConfig) -> Self {
        Self {
            config,
            last_checkpoint_tick: 0,
        }
    }

    /// Checkpoint the node iff at least `interval_ticks` logical ticks have
    /// elapsed since the last checkpoint and Transit recovery says compaction
    /// cannot erase an unresolved durable outbox request.
    pub fn maybe_checkpoint(
        &mut self,
        node: &mut SimulationNode<FileEventStore>,
    ) -> io::Result<Option<StateSnapshot>> {
        let tick = node.current_tick().value();
        if tick.saturating_sub(self.last_checkpoint_tick) < self.config.interval_ticks {
            return Ok(None);
        }
        if crate::transit::pipeline::has_pending_outgoing_transit(node) {
            return Ok(None);
        }
        let snapshot = node.checkpoint(&self.config.snapshot_path, &self.config.cold_path)?;
        self.last_checkpoint_tick = tick;
        Ok(Some(snapshot))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{NodeId, Position, SectorBounds, SectorId, ShipTypeId, Velocity};
    use dawn_event_store::FileEventStore;

    fn file_node(dir: &std::path::Path) -> SimulationNode<FileEventStore> {
        let store = FileEventStore::open(dir.join("events.log")).unwrap();
        SimulationNode::with_test_store(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            store,
        )
    }

    fn cfg(dir: &std::path::Path) -> CheckpointConfig {
        CheckpointConfig {
            interval_ticks: 5,
            snapshot_path: dir.join("snapshot.bin"),
            cold_path: dir.join("cold.log"),
        }
    }

    #[test]
    fn scheduler_does_not_checkpoint_before_the_interval_elapses() {
        let dir = tempfile::tempdir().unwrap();
        let mut node = file_node(dir.path());
        let mut sched = CheckpointScheduler::new(cfg(dir.path()));

        for _ in 0..4 {
            node.tick();
            assert!(sched.maybe_checkpoint(&mut node).unwrap().is_none());
        }
        assert!(!dir.path().join("snapshot.bin").exists());
    }

    #[test]
    fn scheduler_checkpoints_once_the_interval_is_reached_and_compacts_the_log() {
        let dir = tempfile::tempdir().unwrap();
        let mut node = file_node(dir.path());
        node.spawn_ship(
            ShipTypeId(1),
            Position::ORIGIN,
            Velocity::new(30.0, 0.0, 0.0),
        );
        let mut sched = CheckpointScheduler::new(cfg(dir.path()));

        let mut snapshot = None;
        for _ in 0..5 {
            node.tick();
            if let Some(s) = sched.maybe_checkpoint(&mut node).unwrap() {
                snapshot = Some(s);
            }
        }

        let snap = snapshot.expect("a checkpoint fires at tick 5");
        assert_eq!(snap.tick.value(), 5);
        assert!(dir.path().join("snapshot.bin").exists());
        assert_eq!(node.event_store().base_index(), snap.log_index);
        assert_eq!(node.event_store().records_on_disk(), 0);
    }

    #[test]
    fn post_replacement_sync_failure_rolls_back_and_does_not_compact_the_hot_log() {
        let dir = tempfile::tempdir().unwrap();
        let mut node = file_node(dir.path());
        node.spawn_ship(
            ShipTypeId(1),
            Position::ORIGIN,
            Velocity::new(30.0, 0.0, 0.0),
        );

        let snapshot_path = dir.path().join("snapshot.bin");
        let cold_path = dir.path().join("cold.log");
        let previous = node.take_snapshot();
        previous.save(&snapshot_path).unwrap();
        let previous_bytes = std::fs::read(&snapshot_path).unwrap();
        node.tick();

        let base_index_before = node.event_store().base_index();
        let records_before = node.event_store().records_on_disk();
        assert!(records_before > 0, "the test needs a non-empty hot log");

        // Existing-snapshot publication syncs once after preserving the rollback
        // copy, then again after replacing the authoritative path. Fail the
        // second call to exercise replacement-success -> durability-failure.
        crate::persistence::snapshot::inject_directory_sync_failure(&snapshot_path, 2);
        let error = node.checkpoint(&snapshot_path, &cold_path).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(std::fs::read(&snapshot_path).unwrap(), previous_bytes);
        assert_eq!(node.event_store().base_index(), base_index_before);
        assert_eq!(node.event_store().records_on_disk(), records_before);
        assert!(!cold_path.exists());
    }

    #[test]
    fn checkpointed_node_restores_from_snapshot_plus_tail_after_compaction() {
        let dir = tempfile::tempdir().unwrap();
        let mut node = file_node(dir.path());
        node.spawn_ship(
            ShipTypeId(1),
            Position::ORIGIN,
            Velocity::new(40.0, -15.0, 0.0),
        );
        let mut sched = CheckpointScheduler::new(cfg(dir.path()));

        for _ in 0..5 {
            node.tick();
            sched.maybe_checkpoint(&mut node).unwrap();
        }
        for _ in 0..4 {
            node.tick();
        }
        let live_final = postcard::to_stdvec(&node.take_snapshot()).unwrap();

        let snap = StateSnapshot::load(dir.path().join("snapshot.bin")).unwrap();
        let store = FileEventStore::open(dir.path().join("events.log")).unwrap();
        assert!(
            store.base_index() > 0,
            "hot log was compacted behind the snapshot"
        );
        let mut restored = SimulationNode::restore_from_test(
            store,
            &snap,
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            &[],
            &[],
        );
        assert_eq!(restored.current_tick(), snap.tick);
        for _ in 0..4 {
            restored.tick();
        }
        let restored_final = postcard::to_stdvec(&restored.take_snapshot()).unwrap();
        assert_eq!(
            restored_final, live_final,
            "snapshot + tail catch-up == live"
        );
    }
}
