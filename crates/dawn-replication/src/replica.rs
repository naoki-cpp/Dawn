//! Receiver side of log-shipping gossip (ADR-0021 decision 1, 8D-2b).
//!
//! [`ReplicaSet`] is the consumer half of the gossip the owner broadcasts via
//! [`crate::ReplicationTransport::broadcast`]. For each *foreign* Sector it
//! holds an ordered, gap-checked, idempotent copy of that Sector's
//! append-only event log, advancing a per-Sector cursor as contiguous
//! batches arrive.
//!
//! ## Scope
//!
//! This realizes the safe, append-only core of ADR-0021: "ship the owner
//! node's append-only log to other nodes and apply it in logical-tick order".
//! It deliberately stops there.
//! It does **not**:
//! - apply foreign events into a live `SimulationNode` world — those events
//!   carry another Sector's coordinates and would corrupt the owner's AoI and
//!   collision state, and
//! - perform failover takeover (promoting a replica to owner).
//!
//! Both are separate features that need their own design. Holding the
//! authoritative ordered log is the prerequisite for either, and is what a
//! future read/failover path consumes.

use crate::{AntiEntropy, BatchApplyPlan, LogBatch, MissingLogRequest};
use dawn_core::{DomainEvent, SectorId};
use std::collections::HashMap;

/// Outcome of ingesting one [`LogBatch`] into a [`ReplicaSet`].
#[derive(Debug, PartialEq)]
pub enum Ingest {
    /// The contiguous suffix was appended to the Sector's replica log.
    Applied {
        sector_id : SectorId,
        /// Number of events newly appended (may be fewer than the batch when
        /// the batch overlapped already-held entries).
        applied   : usize,
        /// The receiver's cursor after applying — the next index it expects.
        next_index: u64,
    },
    /// The batch was entirely below the cursor; dropped idempotently.
    Duplicate,
    /// The batch starts ahead of the cursor, leaving a hole. The replica is
    /// unchanged; the caller should send `request` to the owner (or fall back
    /// to SnapshotTransfer when the prefix has been compacted away).
    Gap(MissingLogRequest),
}

/// One foreign Sector's replicated log.
#[derive(Default)]
struct SectorReplica {
    /// Next log index this replica expects (== number of events held).
    next_index: u64,
    /// The ordered event log, index 0 first.
    events    : Vec<DomainEvent>,
}

/// Holds a gap-checked, idempotent replica of one or more foreign Sectors'
/// append-only logs, fed by gossiped [`LogBatch`]es.
pub struct ReplicaSet {
    /// Cap on the suffix length a gap request may ask for (passed through to
    /// [`AntiEntropy::plan_batch`]).
    max_events: usize,
    sectors   : HashMap<SectorId, SectorReplica>,
}

impl ReplicaSet {
    pub fn new(max_events: usize) -> Self {
        Self { max_events, sectors: HashMap::new() }
    }

    /// Ingest one gossiped batch, returning what happened. Duplicate and
    /// gapped batches leave the replica unchanged (idempotent).
    pub fn ingest(&mut self, batch: &LogBatch) -> Ingest {
        let replica = self.sectors.entry(batch.sector_id).or_default();
        match AntiEntropy::plan_batch(replica.next_index, batch.sector_id, batch, self.max_events) {
            BatchApplyPlan::Duplicate => Ingest::Duplicate,
            BatchApplyPlan::Gap(request) => Ingest::Gap(request),
            BatchApplyPlan::Apply { first_event_offset, next_index } => {
                replica.events.extend_from_slice(&batch.events[first_event_offset..]);
                replica.next_index = next_index;
                Ingest::Applied {
                    sector_id : batch.sector_id,
                    applied   : batch.events.len() - first_event_offset,
                    next_index,
                }
            }
        }
    }

    /// Number of events held for `sector_id` (0 if never seen).
    pub fn replicated_len(&self, sector_id: SectorId) -> usize {
        self.sectors.get(&sector_id).map_or(0, |r| r.events.len())
    }

    /// The next log index expected for `sector_id` (0 if never seen).
    pub fn next_index(&self, sector_id: SectorId) -> u64 {
        self.sectors.get(&sector_id).map_or(0, |r| r.next_index)
    }

    /// The replicated event log for `sector_id`, in index order.
    pub fn events(&self, sector_id: SectorId) -> &[DomainEvent] {
        self.sectors.get(&sector_id).map_or(&[], |r| r.events.as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{events::VelocityChanged, NodeId, ShipId, Tick, Velocity};

    fn event(n: u64) -> DomainEvent {
        DomainEvent::VelocityChanged(VelocityChanged {
            ship_id : ShipId::new(NodeId(1), n),
            velocity: Velocity::new(1.0, 0.0, 0.0),
            tick    : Tick(n),
        })
    }

    fn batch(sector: u8, from: u64, count: u64) -> LogBatch {
        LogBatch::new(SectorId(sector), from, (from..from + count).map(event).collect())
    }

    #[test]
    fn contiguous_batches_are_applied_in_order() {
        let mut set = ReplicaSet::new(128);

        assert_eq!(
            set.ingest(&batch(1, 0, 3)),
            Ingest::Applied { sector_id: SectorId(1), applied: 3, next_index: 3 },
        );
        assert_eq!(
            set.ingest(&batch(1, 3, 2)),
            Ingest::Applied { sector_id: SectorId(1), applied: 2, next_index: 5 },
        );
        assert_eq!(set.replicated_len(SectorId(1)), 5);
        assert_eq!(set.next_index(SectorId(1)), 5);
    }

    #[test]
    fn a_fully_stale_batch_is_dropped_idempotently() {
        let mut set = ReplicaSet::new(128);
        set.ingest(&batch(1, 0, 5));

        assert_eq!(set.ingest(&batch(1, 0, 3)), Ingest::Duplicate);
        assert_eq!(set.replicated_len(SectorId(1)), 5, "duplicate must not grow the log");
    }

    #[test]
    fn an_overlapping_batch_appends_only_the_new_suffix() {
        let mut set = ReplicaSet::new(128);
        set.ingest(&batch(1, 0, 3));

        // Indices 2,3,4 — index 2 already held, 3 and 4 are new.
        assert_eq!(
            set.ingest(&batch(1, 2, 3)),
            Ingest::Applied { sector_id: SectorId(1), applied: 2, next_index: 5 },
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
        assert_eq!(set.replicated_len(SectorId(1)), 2, "gapped batch must not be applied");
    }

    #[test]
    fn distinct_sectors_are_tracked_independently() {
        let mut set = ReplicaSet::new(128);
        set.ingest(&batch(1, 0, 3));
        set.ingest(&batch(2, 0, 1));

        assert_eq!(set.replicated_len(SectorId(1)), 3);
        assert_eq!(set.replicated_len(SectorId(2)), 1);
        assert_eq!(set.next_index(SectorId(99)), 0, "unseen sector reads as empty");
    }
}
