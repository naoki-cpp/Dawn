//! Authoritative Sector Transit handoff policy.
//!
//! This module owns the Request/Commit/Ack state machine, retry and recovery
//! decisions, destination idempotency, invalid-state handling, and cleanup
//! verification. The surrounding pipeline supplies the node-local transit
//! journal and translates the returned effects into Raft proposals.

use std::collections::BTreeMap;

use crate::node::SimulationNode;
#[cfg(test)]
use crate::persistence::TransitSagaDiagnostics;
use crate::persistence::{
    IncomingTransitReceipt, OutgoingTransitAttempt, TransitAttemptState, TransitSagaSnapshot,
};
#[cfg(test)]
use dawn_core::DomainEvent;
use dawn_core::{
    AbsolutePosition, JumpGateId, SectorId, ShipId, Tick, TransitAttemptId, TransitHandoffState,
};

const TRANSIT_RETRY_INITIAL_TICKS: u64 = 10;
const TRANSIT_RETRY_MAX_TICKS: u64 = 160;
const TRANSIT_RETRY_MAX_ATTEMPTS: u32 = 8;

/// In-memory owner of the durable Transit Saga state.
///
/// Public events remain audit/projection facts. They are not scanned to rebuild
/// this state; the owner is checkpointed and carried by RecoveryDelta.
#[derive(Debug, Clone)]
pub(crate) struct TransitJournal {
    sector_id: SectorId,
    outgoing: BTreeMap<TransitAttemptId, OutgoingTransitAttempt>,
    incoming: BTreeMap<TransitAttemptId, IncomingTransitReceipt>,
}

impl TransitJournal {
    pub(crate) fn new(sector_id: SectorId) -> Self {
        Self {
            sector_id,
            outgoing: BTreeMap::new(),
            incoming: BTreeMap::new(),
        }
    }

    pub(crate) fn from_snapshot(
        sector_id: SectorId,
        snapshot: TransitSagaSnapshot,
    ) -> Result<Self, String> {
        let mut journal = Self::new(sector_id);
        for attempt in snapshot.outgoing {
            if attempt.from != sector_id {
                return Err(format!(
                    "outgoing Transit {:?} belongs to {:?}, not {:?}",
                    attempt.attempt_id, attempt.from, sector_id
                ));
            }
            if attempt.handoff.ship_id != attempt.ship_id {
                return Err(format!(
                    "outgoing Transit {:?} handoff belongs to {:?}, not {:?}",
                    attempt.attempt_id, attempt.handoff.ship_id, attempt.ship_id
                ));
            }
            if let TransitAttemptState::CommitPending { attempts, .. } = attempt.state {
                if attempts == 0 || attempts > TRANSIT_RETRY_MAX_ATTEMPTS {
                    return Err(format!(
                        "outgoing Transit {:?} has invalid retry count {attempts}",
                        attempt.attempt_id
                    ));
                }
            }
            if journal
                .outgoing
                .insert(attempt.attempt_id, attempt)
                .is_some()
            {
                return Err("duplicate outgoing Transit attempt".to_owned());
            }
        }
        for receipt in snapshot.incoming {
            if receipt.to != sector_id {
                return Err(format!(
                    "incoming Transit {:?} targets {:?}, not {:?}",
                    receipt.attempt_id, receipt.to, sector_id
                ));
            }
            if journal.outgoing.contains_key(&receipt.attempt_id) {
                return Err(
                    "Transit attempt appears in both outgoing and incoming state".to_owned(),
                );
            }
            if journal
                .incoming
                .insert(receipt.attempt_id, receipt)
                .is_some()
            {
                return Err("duplicate incoming Transit receipt".to_owned());
            }
        }
        Ok(journal)
    }

    pub(crate) fn snapshot(&self) -> TransitSagaSnapshot {
        TransitSagaSnapshot {
            outgoing: self.outgoing.values().cloned().collect(),
            incoming: self.incoming.values().copied().collect(),
        }
    }

    #[cfg(test)]
    pub(crate) fn diagnostics(&self) -> TransitSagaDiagnostics {
        let mut diagnostics = TransitSagaDiagnostics {
            incoming_receipts: self.incoming.len() as u64,
            ..TransitSagaDiagnostics::default()
        };
        for attempt in self.outgoing.values() {
            match attempt.state {
                TransitAttemptState::Prepared => diagnostics.active_attempts += 1,
                TransitAttemptState::CommitPending { .. } => {
                    diagnostics.active_attempts += 1;
                    diagnostics.retrying_attempts += 1;
                }
                TransitAttemptState::Acknowledged => diagnostics.acknowledged_attempts += 1,
                TransitAttemptState::Aborted { .. } => diagnostics.aborted_attempts += 1,
                TransitAttemptState::Quarantined { .. } => diagnostics.quarantined_attempts += 1,
            }
        }
        diagnostics
    }

    pub(crate) fn register_outgoing(&mut self, attempt: OutgoingTransitAttempt) {
        debug_assert_eq!(attempt.from, self.sector_id);
        let previous = self.outgoing.insert(attempt.attempt_id, attempt);
        debug_assert!(previous.is_none(), "Transit attempt IDs must be unique");
    }

    pub(crate) fn register_incoming(&mut self, receipt: IncomingTransitReceipt) -> bool {
        debug_assert_eq!(receipt.to, self.sector_id);
        if self.outgoing.contains_key(&receipt.attempt_id) {
            return false;
        }
        self.incoming.insert(receipt.attempt_id, receipt).is_none()
    }

    pub(crate) fn incoming_receipt(
        &self,
        attempt_id: TransitAttemptId,
    ) -> Option<&IncomingTransitReceipt> {
        self.incoming.get(&attempt_id)
    }

    pub(crate) fn outgoing(&self, attempt_id: TransitAttemptId) -> Option<&OutgoingTransitAttempt> {
        self.outgoing.get(&attempt_id)
    }

    pub(crate) fn outgoing_for_ship(&self, ship_id: ShipId) -> Option<&OutgoingTransitAttempt> {
        self.outgoing
            .values()
            .find(|attempt| attempt.ship_id == ship_id && Self::is_pending(&attempt.state))
    }

    pub(crate) fn pending_outgoing(&self) -> impl Iterator<Item = &OutgoingTransitAttempt> {
        self.outgoing
            .values()
            .filter(|attempt| Self::is_pending(&attempt.state))
    }

    pub(crate) fn mark_commit_proposed(
        &mut self,
        attempt_id: TransitAttemptId,
        current_tick: Tick,
    ) -> bool {
        let Some(attempt) = self.outgoing.get_mut(&attempt_id) else {
            return false;
        };
        if let TransitAttemptState::CommitPending { attempts, .. } = attempt.state {
            if attempts >= TRANSIT_RETRY_MAX_ATTEMPTS {
                attempt.state = TransitAttemptState::Quarantined {
                    reason: format!(
                        "Transit Commit retry limit ({TRANSIT_RETRY_MAX_ATTEMPTS}) exhausted"
                    ),
                };
                return false;
            }
        }
        let (attempts, delay) = match attempt.state {
            TransitAttemptState::Prepared => (1, TRANSIT_RETRY_INITIAL_TICKS),
            TransitAttemptState::CommitPending { attempts, .. } => (
                attempts.saturating_add(1),
                TRANSIT_RETRY_INITIAL_TICKS
                    .saturating_mul(1u64 << attempts.min(4))
                    .min(TRANSIT_RETRY_MAX_TICKS),
            ),
            _ => return false,
        };
        attempt.state = TransitAttemptState::CommitPending {
            attempts,
            next_retry_tick: Tick(current_tick.value().saturating_add(delay)),
        };
        true
    }

    pub(crate) fn mark_acknowledged(&mut self, attempt_id: TransitAttemptId) -> bool {
        let Some(attempt) = self.outgoing.get_mut(&attempt_id) else {
            return false;
        };
        if matches!(
            attempt.state,
            TransitAttemptState::Acknowledged
                | TransitAttemptState::Aborted { .. }
                | TransitAttemptState::Quarantined { .. }
        ) {
            return false;
        }
        attempt.state = TransitAttemptState::Acknowledged;
        true
    }

    pub(crate) fn quarantine(&mut self, attempt_id: TransitAttemptId, reason: String) {
        if let Some(attempt) = self.outgoing.get_mut(&attempt_id) {
            attempt.state = TransitAttemptState::Quarantined { reason };
        }
    }

    fn is_pending(state: &TransitAttemptState) -> bool {
        matches!(
            state,
            TransitAttemptState::Prepared | TransitAttemptState::CommitPending { .. }
        )
    }
}

#[derive(Debug)]
pub(super) struct CommitEffect {
    pub attempt_id: TransitAttemptId,
    pub handoff: TransitHandoffState,
    pub from: SectorId,
    pub to: SectorId,
    pub entry_pos: AbsolutePosition,
    pub gate_id: Option<JumpGateId>,
    pub request_tick: Tick,
}

#[derive(Debug)]
pub(super) struct AckEffect {
    pub attempt_id: TransitAttemptId,
    pub ship_id: ShipId,
    pub from: SectorId,
    pub to: SectorId,
    pub request_tick: Tick,
}

/// Transit-specific action for the legacy public-event replay fixture.
#[derive(Debug, Clone, Copy)]
#[cfg(test)]
pub(crate) enum ReplayDirective<'a> {
    Requested(&'a dawn_core::events::SectorTransitRequested),
    Completed(&'a dawn_core::events::SectorTransitCompleted),
    Aborted(&'a dawn_core::events::SectorTransitAborted),
}

#[cfg(test)]
pub(super) fn replay_directive(event: &DomainEvent) -> Option<ReplayDirective<'_>> {
    match event {
        DomainEvent::SectorTransitRequested(event) => Some(ReplayDirective::Requested(event)),
        DomainEvent::SectorTransitCompleted(event) => Some(ReplayDirective::Completed(event)),
        DomainEvent::SectorTransitAborted(event) => Some(ReplayDirective::Aborted(event)),
        _ => None,
    }
}

/// Apply a committed Request and return the exact Commit effect needed by the
/// consensus adapter. Validation, freezing, snapshotting, and retry scheduling
/// are resolved before the effect escapes this boundary.
pub(super) fn apply_request(
    node: &mut SimulationNode,
    ship_id: ShipId,
    to: SectorId,
    gate_id: Option<JumpGateId>,
) -> Option<CommitEffect> {
    let data = node.prepare_transit_commit(ship_id, to, gate_id)?;
    let request_tick = data.request_tick;
    node.note_transit_commit_proposed(data.attempt_id);
    Some(CommitEffect {
        attempt_id: data.attempt_id,
        handoff: *data.handoff,
        from: node.sector_id(),
        to,
        entry_pos: data.entry_pos,
        gate_id,
        request_tick,
    })
}

/// Close a still-pending outgoing attempt before accepting the same Ship back
/// into this Sector. Success means cleanup actually removed the frozen copy;
/// an incomplete handoff snapshot or journal identity mismatch is an invalid state,
/// not permission to acknowledge data that was never materialized.
fn complete_superseded_outgoing_transit(
    node: &mut SimulationNode,
    journal: &TransitJournal,
    ship_id: ShipId,
) -> bool {
    if !node.is_ship_in_transit(ship_id) {
        return false;
    }

    let Some(pending) = journal.outgoing_for_ship(ship_id).cloned() else {
        return false;
    };
    node.complete_outgoing_transit_for_attempt(
        pending.attempt_id,
        pending.to,
        pending.entry_pos,
        pending.request_tick,
    );
    node.get_ship_position(ship_id).is_none()
}

/// Apply a committed destination Commit idempotently and return the Ack effect.
///
/// Existing destination state is acknowledged only when a durable completion
/// receipt proves the same transfer identity. An unproven active-Ship collision
/// or a failed superseded-outgoing cleanup returns no Ack, preventing the source
/// from deleting its recovery copy.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_commit(
    node: &mut SimulationNode,
    journal: &TransitJournal,
    handoff: &TransitHandoffState,
    from: SectorId,
    to: SectorId,
    entry_pos: AbsolutePosition,
    gate_id: Option<JumpGateId>,
    request_tick: Tick,
) -> Option<AckEffect> {
    apply_commit_with_attempt(
        node,
        journal,
        handoff,
        from,
        to,
        entry_pos,
        gate_id,
        request_tick,
        TransitAttemptId::new(from, handoff.ship_id, request_tick.value()),
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn apply_commit_with_attempt(
    node: &mut SimulationNode,
    journal: &TransitJournal,
    handoff: &TransitHandoffState,
    from: SectorId,
    to: SectorId,
    entry_pos: AbsolutePosition,
    gate_id: Option<JumpGateId>,
    request_tick: Tick,
    attempt_id: TransitAttemptId,
) -> Option<AckEffect> {
    if to != node.sector_id() {
        return None;
    }

    if let Some(receipt) = journal.incoming_receipt(attempt_id) {
        if receipt.ship_id != handoff.ship_id
            || receipt.from != from
            || receipt.to != to
            || receipt.request_tick != request_tick
        {
            return None;
        }
        return Some(AckEffect {
            attempt_id,
            ship_id: receipt.ship_id,
            from: receipt.from,
            to: receipt.to,
            request_tick: receipt.request_tick,
        });
    }

    if node.get_ship_position(handoff.ship_id).is_some()
        && (!node.is_ship_in_transit(handoff.ship_id)
            || !complete_superseded_outgoing_transit(node, journal, handoff.ship_id))
    {
        return None;
    }

    if !node.register_incoming_transit_receipt(IncomingTransitReceipt {
        attempt_id,
        ship_id: handoff.ship_id,
        from,
        to,
        request_tick,
        materialized_at: node.current_tick(),
    }) {
        return None;
    }
    node.append_incoming_transit_marker(
        handoff.ship_id,
        from,
        to,
        request_tick,
        gate_id,
        entry_pos,
    );
    node.handle_transit_commit(handoff, from, entry_pos, gate_id, request_tick);

    Some(AckEffect {
        attempt_id,
        ship_id: handoff.ship_id,
        from,
        to,
        request_tick,
    })
}

/// Validate a committed Ack against the durable request and confirm that source
/// cleanup actually removed the frozen recovery copy.
#[cfg(test)]
pub(super) fn apply_ack(
    node: &mut SimulationNode,
    journal: &TransitJournal,
    ship_id: ShipId,
    from: SectorId,
    to: SectorId,
    request_tick: Tick,
) -> bool {
    apply_ack_with_attempt(
        node,
        journal,
        ship_id,
        from,
        to,
        request_tick,
        TransitAttemptId::new(from, ship_id, request_tick.value()),
    )
}

pub(super) fn apply_ack_with_attempt(
    node: &mut SimulationNode,
    journal: &TransitJournal,
    ship_id: ShipId,
    from: SectorId,
    to: SectorId,
    request_tick: Tick,
    attempt_id: TransitAttemptId,
) -> bool {
    if from != node.sector_id() {
        return false;
    }
    let Some(pending) = journal.outgoing(attempt_id) else {
        return false;
    };
    if pending.ship_id != ship_id
        || pending.from != from
        || pending.to != to
        || pending.request_tick != request_tick
    {
        return false;
    }
    if matches!(
        pending.state,
        TransitAttemptState::Acknowledged
            | TransitAttemptState::Aborted { .. }
            | TransitAttemptState::Quarantined { .. }
    ) {
        return false;
    }
    if !node.is_ship_in_transit(ship_id) || node.get_ship_position(ship_id).is_none() {
        node.quarantine_transit_attempt(
            attempt_id,
            if node.get_ship_position(ship_id).is_none() {
                "pending Transit source Ship is missing during Ack validation"
            } else {
                "pending Transit source Ship is not frozen in Transit state"
            }
            .to_owned(),
        );
        return false;
    }
    node.complete_outgoing_transit_for_attempt(
        attempt_id,
        pending.to,
        pending.entry_pos,
        pending.request_tick,
    );
    node.get_ship_position(ship_id).is_none() && node.transit_attempt_acknowledged(attempt_id)
}

/// Return only retry Commit effects whose bounded backoff deadline is due.
/// Durable route facts and canonical handoff state both come from the Saga.
pub(super) fn due_retries(
    node: &mut SimulationNode,
    journal: &TransitJournal,
) -> Vec<CommitEffect> {
    let mut effects = Vec::new();
    let attempts: Vec<_> = journal.pending_outgoing().cloned().collect();
    for transit in attempts {
        let due = match transit.state {
            TransitAttemptState::Prepared => true,
            TransitAttemptState::CommitPending {
                next_retry_tick, ..
            } => node.current_tick() >= next_retry_tick,
            _ => false,
        };
        if !due {
            continue;
        }
        if !node.note_transit_commit_proposed(transit.attempt_id) {
            continue;
        }
        effects.push(CommitEffect {
            attempt_id: transit.attempt_id,
            handoff: transit.handoff.clone(),
            from: transit.from,
            to: transit.to,
            entry_pos: transit.entry_pos,
            gate_id: transit.gate_id,
            request_tick: transit.request_tick,
        });
    }
    effects
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{NodeId, Position, SectorBounds, ShipTypeId, Velocity};

    fn node(sector: u8) -> SimulationNode {
        SimulationNode::new_test(
            NodeId(sector),
            SectorId(sector),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        )
    }

    fn journal(node: &SimulationNode) -> TransitJournal {
        node.transit_journal().clone()
    }

    #[test]
    fn pending_outbox_is_owned_by_the_checkpointed_saga() {
        let mut source = node(0);
        let ship_id = source.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let effect = apply_request(&mut source, ship_id, SectorId(1), None).unwrap();
        assert!(journal(&source).pending_outgoing().next().is_some());

        source.complete_outgoing_transit(
            effect.handoff.ship_id,
            effect.to,
            effect.entry_pos,
            effect.request_tick,
        );
        assert!(journal(&source).pending_outgoing().next().is_none());
    }

    #[test]
    fn retry_backoff_emits_only_when_due() {
        let mut source = node(0);
        let ship_id = source.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        apply_request(&mut source, ship_id, SectorId(1), None).unwrap();

        let current = journal(&source);
        assert!(due_retries(&mut source, &current).is_empty());

        for _ in 0..10 {
            source.tick();
        }
        let current = journal(&source);
        assert_eq!(due_retries(&mut source, &current).len(), 1);

        let current = journal(&source);
        assert!(due_retries(&mut source, &current).is_empty());
        for _ in 0..20 {
            source.tick();
        }
        let current = journal(&source);
        assert_eq!(due_retries(&mut source, &current).len(), 1);
    }

    #[test]
    fn retry_limit_quarantines_the_attempt_and_reports_diagnostics() {
        let mut source = node(0);
        let ship_id = source.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let effect = apply_request(&mut source, ship_id, SectorId(1), None).unwrap();

        for _ in 1..TRANSIT_RETRY_MAX_ATTEMPTS {
            assert!(source.note_transit_commit_proposed(effect.attempt_id));
        }
        assert!(!source.note_transit_commit_proposed(effect.attempt_id));
        assert!(matches!(
            journal(&source)
                .outgoing(effect.attempt_id)
                .expect("attempt remains inspectable after quarantine")
                .state,
            TransitAttemptState::Quarantined { .. }
        ));

        let diagnostics = source.transit_saga_diagnostics();
        assert_eq!(diagnostics.active_attempts, 0);
        assert_eq!(diagnostics.retrying_attempts, 0);
        assert_eq!(diagnostics.quarantined_attempts, 1);

        let event_count = source.total_event_count();
        let current = journal(&source);
        assert!(!apply_ack(
            &mut source,
            &current,
            effect.handoff.ship_id,
            effect.from,
            effect.to,
            effect.request_tick,
        ));
        assert!(source.is_ship_in_transit(ship_id));
        assert_eq!(source.total_event_count(), event_count);
    }

    #[test]
    fn restore_rejects_an_attempt_present_in_both_saga_owners() {
        let ship_id = ShipId::new(NodeId(1), 7);
        let attempt_id = TransitAttemptId::new(SectorId(0), ship_id, 11);
        let snapshot = TransitSagaSnapshot {
            outgoing: vec![OutgoingTransitAttempt {
                attempt_id,
                ship_id,
                from: SectorId(0),
                to: SectorId(1),
                handoff: test_handoff(ship_id),
                gate_id: None,
                entry_pos: AbsolutePosition::ORIGIN,
                request_tick: Tick(11),
                state: TransitAttemptState::Prepared,
            }],
            incoming: vec![IncomingTransitReceipt {
                attempt_id,
                ship_id,
                from: SectorId(1),
                to: SectorId(0),
                request_tick: Tick(11),
                materialized_at: Tick(12),
            }],
        };

        let error = TransitJournal::from_snapshot(SectorId(0), snapshot).unwrap_err();
        assert_eq!(
            error,
            "Transit attempt appears in both outgoing and incoming state"
        );
    }

    #[test]
    fn restore_rejects_inconsistent_outgoing_retry_state() {
        let ship_id = ShipId::new(NodeId(1), 7);
        let attempt_id = TransitAttemptId::new(SectorId(0), ship_id, 11);
        let snapshot = TransitSagaSnapshot {
            outgoing: vec![OutgoingTransitAttempt {
                attempt_id,
                ship_id,
                from: SectorId(0),
                to: SectorId(1),
                handoff: test_handoff(ShipId::new(NodeId(1), 8)),
                gate_id: None,
                entry_pos: AbsolutePosition::ORIGIN,
                request_tick: Tick(11),
                state: TransitAttemptState::CommitPending {
                    attempts: 0,
                    next_retry_tick: Tick(12),
                },
            }],
            incoming: Vec::new(),
        };

        let error = TransitJournal::from_snapshot(SectorId(0), snapshot).unwrap_err();
        assert!(error.contains("handoff belongs to"));
    }

    #[test]
    fn restore_rejects_zero_retry_count() {
        let ship_id = ShipId::new(NodeId(1), 7);
        let attempt_id = TransitAttemptId::new(SectorId(0), ship_id, 11);
        let snapshot = TransitSagaSnapshot {
            outgoing: vec![OutgoingTransitAttempt {
                attempt_id,
                ship_id,
                from: SectorId(0),
                to: SectorId(1),
                handoff: test_handoff(ship_id),
                gate_id: None,
                entry_pos: AbsolutePosition::ORIGIN,
                request_tick: Tick(11),
                state: TransitAttemptState::CommitPending {
                    attempts: 0,
                    next_retry_tick: Tick(12),
                },
            }],
            incoming: Vec::new(),
        };

        let error = TransitJournal::from_snapshot(SectorId(0), snapshot).unwrap_err();
        assert!(error.contains("invalid retry count"));
    }

    #[test]
    fn duplicate_destination_commit_is_ack_only() {
        let mut source = node(0);
        let mut destination = node(1);
        let ship_id = source.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let effect = apply_request(&mut source, ship_id, SectorId(1), None).unwrap();

        let current = journal(&destination);
        let first = apply_commit(
            &mut destination,
            &current,
            &effect.handoff,
            effect.from,
            effect.to,
            effect.entry_pos,
            effect.gate_id,
            effect.request_tick,
        );
        let event_count = destination.total_event_count();
        let current = journal(&destination);
        let second = apply_commit(
            &mut destination,
            &current,
            &effect.handoff,
            effect.from,
            effect.to,
            effect.entry_pos,
            effect.gate_id,
            effect.request_tick,
        );

        assert!(first.is_some() && second.is_some());
        assert_eq!(destination.total_event_count(), event_count);
        assert_eq!(destination.ship_count(), 1);
    }

    #[test]
    fn duplicate_commit_with_mismatched_identity_is_rejected() {
        let mut source = node(0);
        let mut destination = node(1);
        let ship_id = source.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let effect = apply_request(&mut source, ship_id, SectorId(1), None).unwrap();

        let current = journal(&destination);
        assert!(apply_commit(
            &mut destination,
            &current,
            &effect.handoff,
            effect.from,
            effect.to,
            effect.entry_pos,
            effect.gate_id,
            effect.request_tick,
        )
        .is_some());

        let mut mismatched = effect.handoff.clone();
        mismatched.ship_id = ShipId::new(NodeId(9), 99);
        let current = journal(&destination);
        let events_before = destination.total_event_count();
        assert!(apply_commit_with_attempt(
            &mut destination,
            &current,
            &mismatched,
            effect.from,
            effect.to,
            effect.entry_pos,
            effect.gate_id,
            effect.request_tick,
            effect.attempt_id,
        )
        .is_none());
        assert_eq!(destination.total_event_count(), events_before);
        assert_eq!(destination.ship_count(), 1);
    }

    #[test]
    fn active_ship_collision_without_receipt_is_not_acknowledged() {
        let mut destination = node(1);
        let ship_id = destination.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let handoff = test_handoff(ship_id);
        let current = journal(&destination);
        let event_count = destination.total_event_count();

        let ack = apply_commit(
            &mut destination,
            &current,
            &handoff,
            SectorId(0),
            SectorId(1),
            AbsolutePosition::ORIGIN,
            None,
            Tick(5),
        );

        assert!(ack.is_none());
        assert_eq!(destination.ship_count(), 1);
        assert_eq!(destination.total_event_count(), event_count);
    }

    #[test]
    fn failed_superseded_cleanup_is_not_acknowledged() {
        let mut destination = node(1);
        let ship_id = destination.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let outbound = destination
            .prepare_transit_commit(ship_id, SectorId(2), None)
            .expect("outgoing Transit should begin");
        let handoff = *outbound.handoff;
        let empty_journal = TransitJournal::new(destination.sector_id());
        let event_count = destination.total_event_count();

        let ack = apply_commit(
            &mut destination,
            &empty_journal,
            &handoff,
            SectorId(0),
            SectorId(1),
            AbsolutePosition::ORIGIN,
            None,
            Tick(7),
        );

        assert!(ack.is_none());
        assert!(destination.is_ship_in_transit(ship_id));
        assert_eq!(destination.total_event_count(), event_count);
    }

    #[test]
    fn mismatched_ack_cannot_cleanup_the_source() {
        let mut source = node(0);
        let ship_id = source.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let effect = apply_request(&mut source, ship_id, SectorId(1), None).unwrap();
        let current = journal(&source);

        assert!(!apply_ack(
            &mut source,
            &current,
            ship_id,
            SectorId(0),
            SectorId(1),
            Tick(effect.request_tick.value() + 1),
        ));
        assert!(source.is_ship_in_transit(ship_id));
        assert_eq!(source.ship_count(), 1);
    }

    #[test]
    fn missing_source_ship_during_ack_quarantines_the_attempt() {
        let mut source = node(0);
        let ship_id = source.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let effect = apply_request(&mut source, ship_id, SectorId(1), None).unwrap();
        source.remove_ship(ship_id);
        let current = journal(&source);
        let event_count = source.total_event_count();

        assert!(!apply_ack(
            &mut source,
            &current,
            ship_id,
            effect.from,
            effect.to,
            effect.request_tick,
        ));
        assert_eq!(source.total_event_count(), event_count);
        assert!(matches!(
            journal(&source)
                .outgoing(effect.attempt_id)
                .expect("missing source remains diagnosable")
                .state,
            TransitAttemptState::Quarantined { .. }
        ));
    }

    #[test]
    fn incoming_return_replaces_unacked_frozen_copy() {
        let mut sector_a = node(0);
        let mut sector_b = node(1);
        let ship_id = sector_a.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);

        let outbound = apply_request(&mut sector_a, ship_id, SectorId(1), None).unwrap();
        let current = journal(&sector_b);
        let delayed_outbound_ack = apply_commit(
            &mut sector_b,
            &current,
            &outbound.handoff,
            outbound.from,
            outbound.to,
            outbound.entry_pos,
            outbound.gate_id,
            outbound.request_tick,
        )
        .unwrap();

        assert!(sector_a.is_ship_in_transit(ship_id));
        assert!(!sector_b.is_ship_in_transit(ship_id));

        let returning = apply_request(&mut sector_b, ship_id, SectorId(0), None).unwrap();
        let current = journal(&sector_a);
        let return_ack = apply_commit(
            &mut sector_a,
            &current,
            &returning.handoff,
            returning.from,
            returning.to,
            returning.entry_pos,
            returning.gate_id,
            returning.request_tick,
        )
        .unwrap();

        assert_eq!(sector_a.ship_count(), 1);
        assert!(!sector_a.is_ship_in_transit(ship_id));
        assert!(journal(&sector_a).pending_outgoing().next().is_none());

        let current = journal(&sector_b);
        assert!(apply_ack(
            &mut sector_b,
            &current,
            return_ack.ship_id,
            return_ack.from,
            return_ack.to,
            return_ack.request_tick,
        ));
        assert_eq!(sector_b.ship_count(), 0);

        let current = journal(&sector_a);
        assert!(!apply_ack(
            &mut sector_a,
            &current,
            delayed_outbound_ack.ship_id,
            delayed_outbound_ack.from,
            delayed_outbound_ack.to,
            delayed_outbound_ack.request_tick,
        ));
        assert_eq!(sector_a.ship_count(), 1);
        assert!(!sector_a.is_ship_in_transit(ship_id));
        assert!(journal(&sector_a).pending_outgoing().next().is_none());
        assert!(journal(&sector_b).pending_outgoing().next().is_none());
    }

    fn test_handoff(ship_id: ShipId) -> TransitHandoffState {
        TransitHandoffState {
            ship_id,
            owner_player_id: None,
            resume_ticket: None,
            pending_resume_ticket: None,
            ship_type_id: ShipTypeId(1),
            velocity: Velocity::ZERO,
            current_shield: 100.0,
            current_armor: 100.0,
            current_hull: 100.0,
            is_destroyed: false,
            capacitor: Some(100.0),
            fitting: dawn_core::fitting::FittingSnapshot::empty(),
            inventory: std::collections::BTreeMap::new(),
        }
    }
}
