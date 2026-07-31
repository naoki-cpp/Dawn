//! Receiver side of log-shipping gossip (ADR-0021 decision 1, 8D-2b).
//!
//! [`ReplicaSet`] holds foreign Sector recovery data only. It never applies
//! replicated events or snapshots to the live local world.

use crate::{AntiEntropy, BatchApplyPlan, LogBatch, MissingLogRequest};
use dawn_core::{DomainEvent, SectorId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Opaque recovery snapshot for one foreign Sector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicaSnapshot {
    pub sector_id: SectorId,
    /// All owner log entries below this index are covered by `bytes`.
    pub log_index: u64,
    /// Caller-defined serialized snapshot bytes (normally `StateSnapshot`).
    pub bytes: Vec<u8>,
}

impl ReplicaSnapshot {
    pub fn new(sector_id: SectorId, log_index: u64, bytes: Vec<u8>) -> Self {
        Self {
            sector_id,
            log_index,
            bytes,
        }
    }
}

/// Result of installing an opaque recovery snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotInstall {
    Installed { next_index: u64 },
    Duplicate { next_index: u64 },
}

/// Outcome of ingesting one [`LogBatch`] into a [`ReplicaSet`].
#[derive(Debug, PartialEq)]
pub enum Ingest {
    Applied {
        sector_id: SectorId,
        applied: usize,
        next_index: u64,
    },
    Duplicate,
    Gap(MissingLogRequest),
}

/// One foreign Sector's recovery replica.
#[derive(Debug, Default)]
struct SectorReplica {
    /// First index represented by `events`. Zero before snapshot fallback.
    base_index: u64,
    next_index: u64,
    /// Retained suffix after `base_index`; a snapshot covers the earlier prefix.
    events: Vec<DomainEvent>,
    snapshot: Option<ReplicaSnapshot>,
}

/// Gap-checked, idempotent recovery replicas for foreign Sectors.
#[derive(Debug)]
pub struct ReplicaSet {
    max_events: usize,
    sectors: HashMap<SectorId, SectorReplica>,
}

impl ReplicaSet {
    pub fn new(max_events: usize) -> Self {
        Self {
            max_events,
            sectors: HashMap::new(),
        }
    }

    /// Ingest one batch. Duplicate and gapped batches leave recovery data
    /// unchanged; overlapping batches append only their new suffix.
    pub fn ingest(&mut self, batch: &LogBatch) -> Ingest {
        let replica = self.sectors.entry(batch.sector_id).or_default();
        match AntiEntropy::plan_batch(replica.next_index, batch.sector_id, batch, self.max_events) {
            BatchApplyPlan::Duplicate => Ingest::Duplicate,
            BatchApplyPlan::Gap(request) => Ingest::Gap(request),
            BatchApplyPlan::Apply {
                first_event_offset,
                next_index,
            } => {
                replica
                    .events
                    .extend_from_slice(&batch.events[first_event_offset..]);
                replica.next_index = next_index;
                Ingest::Applied {
                    sector_id: batch.sector_id,
                    applied: batch.events.len() - first_event_offset,
                    next_index,
                }
            }
        }
    }

    /// Install a newer snapshot atomically as recovery data and reset the
    /// retained suffix cursor to its `log_index`. Older/repeated snapshots are
    /// idempotent no-ops.
    pub fn install_snapshot(&mut self, snapshot: ReplicaSnapshot) -> SnapshotInstall {
        let replica = self.sectors.entry(snapshot.sector_id).or_default();
        if snapshot.log_index < replica.next_index
            || (snapshot.log_index == replica.next_index && replica.snapshot.is_some())
        {
            return SnapshotInstall::Duplicate {
                next_index: replica.next_index,
            };
        }

        replica.base_index = snapshot.log_index;
        replica.next_index = snapshot.log_index;
        replica.events.clear();
        replica.snapshot = Some(snapshot);
        SnapshotInstall::Installed {
            next_index: replica.next_index,
        }
    }

    /// Number of retained suffix events after the current snapshot boundary.
    pub fn replicated_len(&self, sector_id: SectorId) -> usize {
        self.sectors.get(&sector_id).map_or(0, |r| r.events.len())
    }

    pub fn base_index(&self, sector_id: SectorId) -> u64 {
        self.sectors.get(&sector_id).map_or(0, |r| r.base_index)
    }

    pub fn next_index(&self, sector_id: SectorId) -> u64 {
        self.sectors.get(&sector_id).map_or(0, |r| r.next_index)
    }

    /// Retained suffix events in global index order.
    pub fn events(&self, sector_id: SectorId) -> &[DomainEvent] {
        self.sectors
            .get(&sector_id)
            .map_or(&[], |r| r.events.as_slice())
    }

    /// Opaque recovery snapshot, if catch-up crossed a compacted prefix.
    pub fn snapshot(&self, sector_id: SectorId) -> Option<&ReplicaSnapshot> {
        self.sectors
            .get(&sector_id)
            .and_then(|replica| replica.snapshot.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{events::VelocityChanged, NodeId, ShipId, Tick, Velocity};

    fn event(n: u64) -> DomainEvent {
        DomainEvent::VelocityChanged(VelocityChanged {
            ship_id: ShipId::new(NodeId(1), n),
            velocity: Velocity::new(1.0, 0.0, 0.0),
            tick: Tick(n),
        })
    }

    fn batch(sector: u8, from: u64, count: u64) -> LogBatch {
        LogBatch::new(
            SectorId(sector),
            from,
            (from..from + count).map(event).collect(),
        )
    }

    #[test]
    fn contiguous_batches_are_applied_in_order() {
        let mut set = ReplicaSet::new(128);
        set.ingest(&batch(1, 0, 3));
        set.ingest(&batch(1, 3, 2));
        assert_eq!(set.replicated_len(SectorId(1)), 5);
        assert_eq!(set.next_index(SectorId(1)), 5);
    }

    #[test]
    fn stale_and_overlapping_batches_are_idempotent() {
        let mut set = ReplicaSet::new(128);
        set.ingest(&batch(1, 0, 3));
        assert_eq!(set.ingest(&batch(1, 0, 3)), Ingest::Duplicate);
        set.ingest(&batch(1, 2, 3));
        assert_eq!(set.replicated_len(SectorId(1)), 5);
    }

    #[test]
    fn a_gap_leaves_the_replica_unchanged() {
        let mut set = ReplicaSet::new(64);
        set.ingest(&batch(1, 0, 2));
        assert!(matches!(set.ingest(&batch(1, 4, 2)), Ingest::Gap(_)));
        assert_eq!(set.replicated_len(SectorId(1)), 2);
    }

    #[test]
    fn snapshot_replaces_the_prefix_and_suffix_resumes_at_its_log_index() {
        let mut set = ReplicaSet::new(64);
        set.ingest(&batch(1, 0, 3));
        let snapshot = ReplicaSnapshot::new(SectorId(1), 6, vec![1, 2, 3]);

        assert_eq!(
            set.install_snapshot(snapshot.clone()),
            SnapshotInstall::Installed { next_index: 6 }
        );
        assert_eq!(set.base_index(SectorId(1)), 6);
        assert_eq!(set.replicated_len(SectorId(1)), 0);
        assert_eq!(set.snapshot(SectorId(1)), Some(&snapshot));

        set.ingest(&batch(1, 6, 2));
        assert_eq!(set.next_index(SectorId(1)), 8);
        assert_eq!(set.replicated_len(SectorId(1)), 2);
        assert_eq!(
            set.install_snapshot(snapshot),
            SnapshotInstall::Duplicate { next_index: 8 }
        );
        assert_eq!(set.replicated_len(SectorId(1)), 2);
    }

    #[test]
    fn distinct_sectors_are_tracked_independently() {
        let mut set = ReplicaSet::new(128);
        set.ingest(&batch(1, 0, 3));
        set.ingest(&batch(2, 0, 1));
        assert_eq!(set.next_index(SectorId(1)), 3);
        assert_eq!(set.next_index(SectorId(2)), 1);
        assert_eq!(set.next_index(SectorId(99)), 0);
    }
}
