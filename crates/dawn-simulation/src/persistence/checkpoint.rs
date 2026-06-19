//! Snapshot-cadence orchestration (ADR-0017 8A-7).
//!
//! [`SimulationNode::checkpoint`](crate::node::SimulationNode::checkpoint) is the
//! mechanism: snapshot -> persist -> compact. This module is the *policy* around
//! it: a [`CheckpointScheduler`] that fires a checkpoint every `interval_ticks`
//! logical ticks.
//!
//! The cadence is driven by the **logical tick** (INV-005 / FBD-003), never by
//! wall-clock time, so checkpointing is deterministic and replay-stable. The
//! scheduler holds no state beyond the last checkpoint tick and its paths; it
//! does no work until [`CheckpointScheduler::maybe_checkpoint`] is called with a
//! node whose tick has advanced past the interval.

use std::io;
use std::path::PathBuf;

use dawn_event_store::FileEventStore;

use crate::node::SimulationNode;
use super::snapshot::StateSnapshot;

/// Where checkpoints are written and how often.
#[derive(Debug, Clone)]
pub struct CheckpointConfig {
    /// Number of logical ticks between checkpoints.
    pub interval_ticks: u64,
    /// Path for the (overwritten) latest authoritative snapshot.
    pub snapshot_path: PathBuf,
    /// Path for the append-only cold archive that receives compacted prefixes.
    pub cold_path: PathBuf,
}

/// Fires [`SimulationNode::checkpoint`] on a fixed logical-tick cadence.
pub struct CheckpointScheduler {
    config: CheckpointConfig,
    /// Logical tick of the most recent checkpoint (starts at 0 = node genesis).
    last_checkpoint_tick: u64,
}

impl CheckpointScheduler {
    pub fn new(config: CheckpointConfig) -> Self {
        Self { config, last_checkpoint_tick: 0 }
    }

    /// Checkpoint the node iff at least `interval_ticks` logical ticks have
    /// elapsed since the last checkpoint.
    ///
    /// Returns `Ok(Some(snapshot))` when a checkpoint was taken, `Ok(None)` when
    /// the interval has not yet elapsed. Call this once per tick (e.g. right after
    /// `node.tick()`); it is cheap when no checkpoint is due.
    pub fn maybe_checkpoint(
        &mut self,
        node: &mut SimulationNode<FileEventStore>,
    ) -> io::Result<Option<StateSnapshot>> {
        let tick = node.current_tick().value();
        if tick.saturating_sub(self.last_checkpoint_tick) < self.config.interval_ticks {
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
    use crate::node::SimulationNode;
    use dawn_core::{NodeId, Position, SectorBounds, SectorId, ShipTypeId, Velocity};

    fn file_node(dir: &std::path::Path) -> SimulationNode<FileEventStore> {
        let store = FileEventStore::open(dir.join("events.log")).unwrap();
        SimulationNode::with_store(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
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
        node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::new(30.0, 0.0, 0.0));
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
        // Snapshot is persisted and the hot log is compacted behind it.
        assert!(dir.path().join("snapshot.bin").exists());
        assert_eq!(node.event_store().base_index(), snap.log_index);
        assert_eq!(node.event_store().records_on_disk(), 0);
    }

    #[test]
    fn checkpointed_node_restores_from_snapshot_plus_tail_after_compaction() {
        let dir = tempfile::tempdir().unwrap();
        let mut node = file_node(dir.path());
        node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::new(40.0, -15.0, 0.0));
        let mut sched = CheckpointScheduler::new(cfg(dir.path()));

        // Run past one checkpoint, then a few more ticks (the post-snapshot tail).
        for _ in 0..5 {
            node.tick();
            sched.maybe_checkpoint(&mut node).unwrap();
        }
        for _ in 0..4 {
            node.tick();
        }
        let live_final = postcard::to_stdvec(&node.take_snapshot()).unwrap();

        // Recover from the persisted snapshot + the compacted hot log's tail.
        let snap = StateSnapshot::load(dir.path().join("snapshot.bin")).unwrap();
        let store = FileEventStore::open(dir.path().join("events.log")).unwrap();
        assert!(store.base_index() > 0, "hot log was compacted behind the snapshot");
        let mut restored = SimulationNode::restore_from(store, &snap, &[], &[]);
        assert_eq!(restored.current_tick(), snap.tick);
        // Re-run the same post-snapshot tail. Transient state (position) is not
        // event-sourced, so it is recomputed by stepping the sim forward.
        for _ in 0..4 {
            restored.tick();
        }
        let restored_final = postcard::to_stdvec(&restored.take_snapshot()).unwrap();
        assert_eq!(restored_final, live_final, "snapshot + tail catch-up == live");
    }
}
