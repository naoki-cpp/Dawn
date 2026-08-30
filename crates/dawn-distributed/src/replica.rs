//! Receiver side of log-shipping gossip (ADR-0021 decision 1, 8D-2b).
//!
//! [`ReplicaSet`] is the consumer half of the gossip the owner broadcasts via
//! [`crate::ReplicationTransport::broadcast`]. For each *foreign* Sector it
//! holds an ordered, gap-checked, idempotent copy of that Sector's append-only
//! public-event log, advancing a per-Sector public cursor as contiguous batches
//! arrive.
//!
//! ## Scope
//!
//! This realizes the safe, append-only core of ADR-0021: "ship the owner
//! node's append-only log to other nodes and apply it in logical-tick order".
//! When the requested prefix has been compacted, an opaque recovery snapshot
//! replaces that prefix and the retained event suffix resumes at the snapshot's
//! explicit public-event cursor. Authoritative recovery coverage is tracked
//! separately.
//!
//! It deliberately does **not**:
//! - apply foreign events or snapshots into a live `SimulationNode` world —
//!   those records carry another Sector's state and would corrupt the owner's
//!   AoI and collision state, and
//! - perform failover takeover (promoting a replica to owner).
//!
//! Both are separate features that need their own design. Holding the
//! authoritative snapshot plus ordered suffix is the prerequisite for either.

use crate::LogBatch;
use dawn_core::{DomainEvent, SectorId};
use dawn_storage::{PublicEventIndex, RecoveryIndex};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};

/// Opaque recovery snapshot for one foreign Sector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicaSnapshot {
    pub sector_id: SectorId,
    /// Authoritative recovery records below this position are covered by
    /// `bytes`. This is independent of the public-event cursor below.
    pub covered_recovery_index: RecoveryIndex,
    /// The next public event that the replica must request after installing
    /// this snapshot. This is never derived from recovery coverage.
    pub public_event_next_index: PublicEventIndex,
    /// Caller-defined serialized snapshot bytes (normally `StateSnapshot`).
    /// Shared ownership keeps request retries and cached owner snapshots from
    /// duplicating a potentially large allocation.
    pub bytes: Arc<Vec<u8>>,
}

impl ReplicaSnapshot {
    pub fn new(
        sector_id: SectorId,
        covered_recovery_index: RecoveryIndex,
        public_event_next_index: PublicEventIndex,
        bytes: Vec<u8>,
    ) -> Self {
        Self {
            sector_id,
            covered_recovery_index,
            public_event_next_index,
            bytes: Arc::new(bytes),
        }
    }
}

/// A request for the suffix of one Sector's append-only log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissingLogRequest {
    pub sector_id: SectorId,
    pub from_public_event_index: PublicEventIndex,
    pub max_events: usize,
}

impl MissingLogRequest {
    pub fn new(
        sector_id: SectorId,
        from_public_event_index: impl Into<PublicEventIndex>,
        max_events: usize,
    ) -> Self {
        Self {
            sector_id,
            from_public_event_index: from_public_event_index.into(),
            max_events,
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
    /// The contiguous suffix was appended to the Sector's recovery log.
    Applied {
        sector_id: SectorId,
        /// Number of events newly appended (may be fewer than the batch when
        /// the batch overlapped already-held entries).
        applied: usize,
        /// The receiver's cursor after applying — the next index it expects.
        next_index: u64,
    },
    /// The batch was entirely below the cursor; dropped idempotently.
    Duplicate,
    /// The batch starts ahead of the cursor, leaving a hole. The replica is
    /// unchanged; the caller should send `request` to the owner and may need a
    /// snapshot when the requested prefix has been compacted.
    Gap(MissingLogRequest),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BatchApplyPlan {
    Duplicate,
    Gap(MissingLogRequest),
    Apply {
        first_event_offset: usize,
        next_index: u64,
    },
}

fn plan_batch(
    expected_next_index: u64,
    sector_id: SectorId,
    batch: &LogBatch,
    max_events: usize,
) -> BatchApplyPlan {
    if batch.next_public_event_index().0 <= expected_next_index {
        return BatchApplyPlan::Duplicate;
    }

    if batch.from_public_event_index.0 > expected_next_index {
        return BatchApplyPlan::Gap(MissingLogRequest::new(
            sector_id,
            expected_next_index,
            max_events,
        ));
    }

    BatchApplyPlan::Apply {
        first_event_offset: (expected_next_index - batch.from_public_event_index.0) as usize,
        next_index: batch.next_public_event_index().0,
    }
}

/// One foreign Sector's recovery replica.
#[derive(Debug, Default)]
struct SectorReplica {
    /// First public-event index represented by `events`. Zero before snapshot
    /// fallback; equal to the installed snapshot boundary afterwards.
    base_public_event_index: PublicEventIndex,
    /// Next public-event index this replica expects.
    next_public_event_index: PublicEventIndex,
    /// Ordered retained suffix after `base_public_event_index`.
    events: Vec<DomainEvent>,
    /// Opaque snapshot covering the state before `base_public_event_index`.
    snapshot: Option<ReplicaSnapshot>,
}

/// Holds a gap-checked, idempotent recovery replica of one or more foreign
/// Sectors, fed by gossiped [`LogBatch`]es and optional snapshots.
#[derive(Debug)]
pub struct ReplicaSet {
    /// Cap on the suffix length a gap request may ask for.
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

    /// Ingest one gossiped batch, returning what happened. Duplicate and
    /// gapped batches leave the replica unchanged; overlapping batches append
    /// only their new suffix.
    pub fn ingest(&mut self, batch: &LogBatch) -> Ingest {
        let replica = self.sectors.entry(batch.sector_id).or_default();
        match plan_batch(
            replica.next_public_event_index.0,
            batch.sector_id,
            batch,
            self.max_events,
        ) {
            BatchApplyPlan::Duplicate => Ingest::Duplicate,
            BatchApplyPlan::Gap(request) => Ingest::Gap(request),
            BatchApplyPlan::Apply {
                first_event_offset,
                next_index,
            } => {
                replica
                    .events
                    .extend_from_slice(&batch.events[first_event_offset..]);
                replica.next_public_event_index = PublicEventIndex(next_index);
                Ingest::Applied {
                    sector_id: batch.sector_id,
                    applied: batch.events.len() - first_event_offset,
                    next_index,
                }
            }
        }
    }

    /// Install a newer snapshot atomically as recovery data and reset the
    /// retained suffix cursor to its public-event position. A snapshot must
    /// advance authoritative recovery coverage without moving the already
    /// retained public-event cursor backwards. Older/repeated snapshots are
    /// idempotent no-ops.
    pub fn install_snapshot(&mut self, snapshot: ReplicaSnapshot) -> SnapshotInstall {
        let replica = self.sectors.entry(snapshot.sector_id).or_default();
        let recovery_is_not_newer = replica.snapshot.as_ref().is_some_and(|current| {
            snapshot.covered_recovery_index <= current.covered_recovery_index
        });
        if snapshot.public_event_next_index < replica.next_public_event_index
            || recovery_is_not_newer
        {
            return SnapshotInstall::Duplicate {
                next_index: replica.next_public_event_index.0,
            };
        }

        replica.base_public_event_index = snapshot.public_event_next_index;
        replica.next_public_event_index = snapshot.public_event_next_index;
        replica.events.clear();
        replica.snapshot = Some(snapshot);
        SnapshotInstall::Installed {
            next_index: replica.next_public_event_index.0,
        }
    }

    /// Number of retained suffix events after the current snapshot boundary.
    pub fn replicated_len(&self, sector_id: SectorId) -> usize {
        self.sectors.get(&sector_id).map_or(0, |r| r.events.len())
    }

    /// First public-event index represented by [`Self::events`].
    pub fn base_public_event_index(&self, sector_id: SectorId) -> u64 {
        self.sectors
            .get(&sector_id)
            .map_or(0, |r| r.base_public_event_index.0)
    }

    /// The next public-event index expected for `sector_id` (0 if never seen).
    pub fn next_public_event_index(&self, sector_id: SectorId) -> u64 {
        self.sectors
            .get(&sector_id)
            .map_or(0, |r| r.next_public_event_index.0)
    }

    /// The retained suffix for `sector_id`, in global index order.
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

        assert_eq!(
            set.ingest(&batch(1, 0, 3)),
            Ingest::Applied {
                sector_id: SectorId(1),
                applied: 3,
                next_index: 3
            },
        );
        assert_eq!(
            set.ingest(&batch(1, 3, 2)),
            Ingest::Applied {
                sector_id: SectorId(1),
                applied: 2,
                next_index: 5
            },
        );
        assert_eq!(set.replicated_len(SectorId(1)), 5);
        assert_eq!(set.next_public_event_index(SectorId(1)), 5);
    }

    #[test]
    fn a_fully_stale_batch_is_dropped_idempotently() {
        let mut set = ReplicaSet::new(128);
        set.ingest(&batch(1, 0, 5));

        assert_eq!(set.ingest(&batch(1, 0, 3)), Ingest::Duplicate);
        assert_eq!(
            set.replicated_len(SectorId(1)),
            5,
            "duplicate must not grow the log"
        );
    }

    #[test]
    fn an_overlapping_batch_appends_only_the_new_suffix() {
        let mut set = ReplicaSet::new(128);
        set.ingest(&batch(1, 0, 3));

        // Indices 2,3,4 — index 2 already held, 3 and 4 are new.
        assert_eq!(
            set.ingest(&batch(1, 2, 3)),
            Ingest::Applied {
                sector_id: SectorId(1),
                applied: 2,
                next_index: 5
            },
        );
        assert_eq!(set.replicated_len(SectorId(1)), 5);
    }

    #[test]
    fn a_gap_leaves_the_replica_unchanged_and_requests_the_missing_suffix() {
        let mut set = ReplicaSet::new(64);
        set.ingest(&batch(1, 0, 2));

        // Cursor is at 2, but this batch starts at 4 → hole at 2,3.
        match set.ingest(&batch(1, 4, 2)) {
            Ingest::Gap(req) => {
                assert_eq!(req, MissingLogRequest::new(SectorId(1), 2, 64));
            }
            other => panic!("expected Gap, got {other:?}"),
        }
        assert_eq!(
            set.replicated_len(SectorId(1)),
            2,
            "gapped batch must not be applied"
        );
    }

    #[test]
    fn snapshot_replaces_the_prefix_and_suffix_resumes_at_its_public_event_index() {
        let mut set = ReplicaSet::new(64);
        set.ingest(&batch(1, 0, 3));
        let snapshot = ReplicaSnapshot::new(
            SectorId(1),
            RecoveryIndex(12),
            PublicEventIndex(6),
            vec![1, 2, 3],
        );

        assert_eq!(
            set.install_snapshot(snapshot.clone()),
            SnapshotInstall::Installed { next_index: 6 }
        );
        assert_eq!(set.base_public_event_index(SectorId(1)), 6);
        assert_eq!(set.replicated_len(SectorId(1)), 0);
        assert_eq!(set.snapshot(SectorId(1)), Some(&snapshot));

        set.ingest(&batch(1, 6, 2));
        assert_eq!(set.next_public_event_index(SectorId(1)), 8);
        assert_eq!(set.replicated_len(SectorId(1)), 2);
        assert_eq!(
            set.install_snapshot(snapshot),
            SnapshotInstall::Duplicate { next_index: 8 }
        );
        assert_eq!(set.replicated_len(SectorId(1)), 2);
    }

    #[test]
    fn newer_recovery_snapshot_installs_when_public_cursor_is_unchanged() {
        let mut set = ReplicaSet::new(64);
        let previous =
            ReplicaSnapshot::new(SectorId(1), RecoveryIndex(12), PublicEventIndex(6), vec![1]);
        assert!(matches!(
            set.install_snapshot(previous),
            SnapshotInstall::Installed { next_index: 6 }
        ));

        let replacement =
            ReplicaSnapshot::new(SectorId(1), RecoveryIndex(13), PublicEventIndex(6), vec![2]);
        assert!(matches!(
            set.install_snapshot(replacement.clone()),
            SnapshotInstall::Installed { next_index: 6 }
        ));
        assert_eq!(set.snapshot(SectorId(1)), Some(&replacement));
    }

    #[test]
    fn stale_recovery_snapshot_cannot_skip_public_events() {
        let mut set = ReplicaSet::new(64);
        let current =
            ReplicaSnapshot::new(SectorId(1), RecoveryIndex(12), PublicEventIndex(6), vec![1]);
        set.install_snapshot(current.clone());

        let stale =
            ReplicaSnapshot::new(SectorId(1), RecoveryIndex(11), PublicEventIndex(9), vec![2]);
        assert!(matches!(
            set.install_snapshot(stale),
            SnapshotInstall::Duplicate { next_index: 6 }
        ));
        assert_eq!(set.snapshot(SectorId(1)), Some(&current));
        assert_eq!(set.next_public_event_index(SectorId(1)), 6);
    }

    #[test]
    fn snapshot_clones_share_the_payload() {
        let snapshot = ReplicaSnapshot::new(
            SectorId(1),
            RecoveryIndex(12),
            PublicEventIndex(6),
            vec![1, 2, 3],
        );
        let clone = snapshot.clone();

        assert!(Arc::ptr_eq(&snapshot.bytes, &clone.bytes));
    }

    #[test]
    fn distinct_sectors_are_tracked_independently() {
        let mut set = ReplicaSet::new(128);
        set.ingest(&batch(1, 0, 3));
        set.ingest(&batch(2, 0, 1));

        assert_eq!(set.replicated_len(SectorId(1)), 3);
        assert_eq!(set.replicated_len(SectorId(2)), 1);
        assert_eq!(
            set.next_public_event_index(SectorId(99)),
            0,
            "unseen sector reads as empty"
        );
    }
}
