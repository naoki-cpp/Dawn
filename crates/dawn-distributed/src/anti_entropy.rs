//! Anti-entropy helpers for Sector-local log shipping (ADR-0027 / 8D-2b).
//!
//! The transport layer can drop or reorder gossip batches. Anti-entropy keeps
//! the recovery rule simple: peers compare the next log index they expect for a
//! Sector, then ask the owner for the missing suffix via [`PublicEventTail`].

use crate::LogBatch;
use dawn_core::SectorId;
use dawn_storage::PublicEventIndex;

use crate::PublicEventTail;

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

/// How a receiver should handle an incoming [`LogBatch`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchApplyPlan {
    /// The entire batch is at or before the receiver cursor.
    Duplicate,
    /// The batch starts after the receiver cursor; request the missing range.
    Gap(MissingLogRequest),
    /// Apply events starting at `first_event_offset`, then advance to
    /// `next_index`. Offset is non-zero for an overlapping retry batch.
    Apply {
        first_event_offset: usize,
        next_index: u64,
    },
}

/// Stateless anti-entropy operations.
#[derive(Debug, Clone, Copy, Default)]
pub struct AntiEntropy;

impl AntiEntropy {
    /// Build a request for the missing suffix starting at `expected_next_index`.
    pub fn request_missing(
        sector_id: SectorId,
        expected_next_index: u64,
        max_events: usize,
    ) -> MissingLogRequest {
        MissingLogRequest::new(sector_id, expected_next_index, max_events)
    }

    /// Serve a missing-log request from the retained, rebuildable public tail.
    ///
    /// A cursor before the retained base is returned as an error so the caller
    /// must select snapshot fallback rather than emit a truncated suffix.
    pub fn batch_from_tail(
        tail: &PublicEventTail,
        request: MissingLogRequest,
    ) -> Result<Option<LogBatch>, crate::PublicEventTailError> {
        let suffix = tail.read_from(request.from_public_event_index, request.max_events)?;
        if suffix.events.is_empty() {
            return Ok(None);
        }
        Ok(Some(LogBatch::new(
            request.sector_id,
            suffix.from_public_event_index,
            suffix.events,
        )))
    }

    /// Decide whether an incoming batch is duplicate, contiguous, overlapping,
    /// or ahead of the receiver's cursor.
    pub fn plan_batch(
        expected_next_index: u64,
        sector_id: SectorId,
        batch: &LogBatch,
        max_events: usize,
    ) -> BatchApplyPlan {
        if batch.next_public_event_index().0 <= expected_next_index {
            return BatchApplyPlan::Duplicate;
        }

        if batch.from_public_event_index.0 > expected_next_index {
            return BatchApplyPlan::Gap(Self::request_missing(
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PublicEventTail;
    use dawn_core::{events::VelocityChanged, DomainEvent, NodeId, ShipId, Tick, Velocity};

    fn event(n: u64) -> DomainEvent {
        DomainEvent::VelocityChanged(VelocityChanged {
            ship_id: ShipId::new(NodeId(0), n),
            velocity: Velocity::new(1.0, 0.0, 0.0),
            tick: Tick(n),
        })
    }

    fn tail_with(count: u64) -> PublicEventTail {
        let mut tail = PublicEventTail::new(0.into(), 32).unwrap();
        for i in 0..count {
            tail.append_committed(&[event(i)]).unwrap();
        }
        tail
    }

    #[test]
    fn request_missing_uses_the_receivers_expected_next_index() {
        let request = AntiEntropy::request_missing(SectorId(7), 42, 128);
        assert_eq!(request, MissingLogRequest::new(SectorId(7), 42, 128));
    }

    #[test]
    fn batch_from_tail_returns_the_requested_suffix() {
        let tail = tail_with(5);
        let request = MissingLogRequest::new(SectorId(1), 2, 10);

        let batch = AntiEntropy::batch_from_tail(&tail, request)
            .unwrap()
            .expect("suffix exists");

        assert_eq!(batch.sector_id, SectorId(1));
        assert_eq!(batch.from_public_event_index, 2);
        assert_eq!(batch.events.len(), 3);
        assert_eq!(batch.next_public_event_index(), 5);
    }

    #[test]
    fn batch_from_tail_respects_max_events() {
        let tail = tail_with(10);
        let request = MissingLogRequest::new(SectorId(1), 3, 4);

        let batch = AntiEntropy::batch_from_tail(&tail, request)
            .unwrap()
            .expect("suffix exists");

        assert_eq!(batch.from_public_event_index, 3);
        assert_eq!(batch.events.len(), 4);
        assert_eq!(batch.next_public_event_index(), 7);
    }

    #[test]
    fn batch_from_tail_returns_none_when_peer_is_caught_up() {
        let tail = tail_with(3);
        let request = MissingLogRequest::new(SectorId(1), 3, 10);

        assert!(AntiEntropy::batch_from_tail(&tail, request)
            .unwrap()
            .is_none());
    }

    #[test]
    fn batch_from_tail_rejects_a_cursor_before_the_retained_base() {
        let mut tail = PublicEventTail::new(0.into(), 2).unwrap();
        tail.append_committed(&[event(0), event(1), event(2)])
            .unwrap();
        let request = MissingLogRequest::new(SectorId(1), 0, 10);

        assert!(matches!(
            AntiEntropy::batch_from_tail(&tail, request),
            Err(crate::PublicEventTailError::CursorTooOld { .. })
        ));
    }

    #[test]
    fn contiguous_batch_can_be_applied_from_the_start() {
        let batch = LogBatch::new(SectorId(1), 5, vec![event(5), event(6)]);

        let plan = AntiEntropy::plan_batch(5, SectorId(1), &batch, 10);

        assert_eq!(
            plan,
            BatchApplyPlan::Apply {
                first_event_offset: 0,
                next_index: 7,
            },
        );
    }

    #[test]
    fn overlapping_retry_batch_applies_only_the_missing_suffix() {
        let batch = LogBatch::new(SectorId(1), 3, vec![event(3), event(4), event(5)]);

        let plan = AntiEntropy::plan_batch(5, SectorId(1), &batch, 10);

        assert_eq!(
            plan,
            BatchApplyPlan::Apply {
                first_event_offset: 2,
                next_index: 6,
            },
        );
    }

    #[test]
    fn fully_old_batch_is_duplicate() {
        let batch = LogBatch::new(SectorId(1), 1, vec![event(1), event(2)]);

        let plan = AntiEntropy::plan_batch(3, SectorId(1), &batch, 10);

        assert_eq!(plan, BatchApplyPlan::Duplicate);
    }

    #[test]
    fn future_batch_reports_gap_request() {
        let batch = LogBatch::new(SectorId(1), 8, vec![event(8)]);

        let plan = AntiEntropy::plan_batch(5, SectorId(1), &batch, 10);

        assert_eq!(
            plan,
            BatchApplyPlan::Gap(MissingLogRequest::new(SectorId(1), 5, 10)),
        );
    }
}
