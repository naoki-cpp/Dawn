//! Authoritative Sector Transit handoff policy.
//!
//! This module owns the Request/Commit/Ack state machine, retry and recovery
//! decisions, destination idempotency, invalid-state handling, and cleanup
//! verification. The surrounding pipeline only reconstructs durable facts from
//! the EventStore and translates the returned effects into Raft proposals.

use std::collections::HashMap;

use crate::node::SimulationNode;
use dawn_core::{
    AbsolutePosition, DomainEvent, JumpGateId, SectorId, ShipId, Tick, TransitHandoffState,
};
use dawn_event_store::store::EventStore;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TransferIdentity {
    ship_id: ShipId,
    from: SectorId,
    to: SectorId,
    request_tick: Tick,
}

impl TransferIdentity {
    fn new(ship_id: ShipId, from: SectorId, to: SectorId, request_tick: Tick) -> Self {
        Self {
            ship_id,
            from,
            to,
            request_tick,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PendingTransit {
    identity: TransferIdentity,
    gate_id: Option<JumpGateId>,
    entry_pos: AbsolutePosition,
}

/// Transit facts reconstructed by the EventStore adapter.
///
/// Event ordering and identity matching are interpreted here so callers cannot
/// accidentally clear a newer request with an older completion or treat an
/// unproven destination collision as an idempotent replay.
#[derive(Debug)]
pub(super) struct TransitJournal {
    sector_id: SectorId,
    pending_outgoing: HashMap<ShipId, PendingTransit>,
    incoming_markers: Vec<TransferIdentity>,
    completed_incoming: Vec<TransferIdentity>,
}

impl TransitJournal {
    pub(super) fn new(sector_id: SectorId) -> Self {
        Self {
            sector_id,
            pending_outgoing: HashMap::new(),
            incoming_markers: Vec::new(),
            completed_incoming: Vec::new(),
        }
    }

    pub(super) fn observe(&mut self, event: &DomainEvent) {
        match event {
            DomainEvent::SectorTransitRequested(event) => {
                let identity =
                    TransferIdentity::new(event.ship_id, event.from, event.to, event.request_tick);
                if event.from == self.sector_id {
                    self.pending_outgoing.insert(
                        event.ship_id,
                        PendingTransit {
                            identity,
                            gate_id: event.gate_id,
                            entry_pos: event.entry_pos,
                        },
                    );
                }
                if event.to == self.sector_id && !self.incoming_markers.contains(&identity) {
                    self.incoming_markers.push(identity);
                }
            }
            DomainEvent::SectorTransitCompleted(event) => {
                let identity = TransferIdentity::new(
                    event.handoff.ship_id,
                    event.from,
                    event.to,
                    event.request_tick,
                );
                if event.from == self.sector_id
                    && self
                        .pending_outgoing
                        .get(&event.handoff.ship_id)
                        .is_some_and(|pending| pending.identity == identity)
                {
                    self.pending_outgoing.remove(&event.handoff.ship_id);
                }
                if event.to == self.sector_id
                    && self.incoming_markers.contains(&identity)
                    && !self.completed_incoming.contains(&identity)
                {
                    self.completed_incoming.push(identity);
                }
            }
            DomainEvent::SectorTransitAborted(event)
                if event.from == self.sector_id
                    && self
                        .pending_outgoing
                        .get(&event.ship_id)
                        .is_some_and(|pending| {
                            pending.identity.from == event.from && pending.identity.to == event.to
                        }) =>
            {
                // SectorTransitAborted predates request_tick in its payload, so
                // route identity is the strongest safe match available. Never
                // let an old A -> B abort clear a newer A -> C request.
                self.pending_outgoing.remove(&event.ship_id);
            }
            _ => {}
        }
    }

    fn has_pending_outgoing(&self) -> bool {
        !self.pending_outgoing.is_empty()
    }

    fn pending_outgoing(&self) -> impl Iterator<Item = PendingTransit> + '_ {
        self.pending_outgoing.values().copied()
    }

    fn pending_for(
        &self,
        ship_id: ShipId,
        from: SectorId,
        to: SectorId,
        request_tick: Tick,
    ) -> Option<PendingTransit> {
        let identity = TransferIdentity::new(ship_id, from, to, request_tick);
        self.pending_outgoing
            .get(&ship_id)
            .copied()
            .filter(|pending| pending.identity == identity)
    }

    fn completed_incoming(&self, identity: TransferIdentity) -> bool {
        self.completed_incoming.contains(&identity)
    }
}

#[derive(Debug)]
pub(super) struct CommitEffect {
    pub handoff: TransitHandoffState,
    pub from: SectorId,
    pub to: SectorId,
    pub entry_pos: AbsolutePosition,
    pub gate_id: Option<JumpGateId>,
    pub request_tick: Tick,
}

#[derive(Debug)]
pub(super) struct AckEffect {
    pub ship_id: ShipId,
    pub from: SectorId,
    pub to: SectorId,
    pub request_tick: Tick,
}

/// Transit-specific action for the generic EventStore replay adapter.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ReplayDirective<'a> {
    Requested(&'a dawn_core::events::SectorTransitRequested),
    Completed(&'a dawn_core::events::SectorTransitCompleted),
    Aborted(&'a dawn_core::events::SectorTransitAborted),
}

pub(super) fn replay_directive(event: &DomainEvent) -> Option<ReplayDirective<'_>> {
    match event {
        DomainEvent::SectorTransitRequested(event) => Some(ReplayDirective::Requested(event)),
        DomainEvent::SectorTransitCompleted(event) => Some(ReplayDirective::Completed(event)),
        DomainEvent::SectorTransitAborted(event) => Some(ReplayDirective::Aborted(event)),
        _ => None,
    }
}

pub(super) fn has_pending_outgoing_transit(journal: &TransitJournal) -> bool {
    journal.has_pending_outgoing()
}

/// Apply a committed Request and return the exact Commit effect needed by the
/// consensus adapter. Validation, freezing, snapshotting, and retry scheduling
/// are resolved before the effect escapes this boundary.
pub(super) fn apply_request<S: EventStore>(
    node: &mut SimulationNode<S>,
    ship_id: ShipId,
    to: SectorId,
    gate_id: Option<JumpGateId>,
) -> Option<CommitEffect> {
    let data = node.prepare_transit_commit(ship_id, to, gate_id)?;
    let request_tick = data.request_tick;
    node.note_transit_commit_proposed(ship_id, request_tick);
    Some(CommitEffect {
        handoff: *data.handoff,
        from: node.sector_id(),
        to,
        entry_pos: data.entry_pos,
        gate_id,
        request_tick,
    })
}

fn destination_completed_transfer<S: EventStore>(
    node: &SimulationNode<S>,
    journal: &TransitJournal,
    identity: TransferIdentity,
) -> bool {
    node.has_completed_incoming_transit(
        identity.ship_id,
        identity.from,
        identity.to,
        identity.request_tick,
    ) || journal.completed_incoming(identity)
}

/// Close a still-pending outgoing attempt before accepting the same Ship back
/// into this Sector. Success means cleanup actually removed the frozen copy;
/// an EventStore mismatch or incomplete handoff snapshot is an invalid state,
/// not permission to acknowledge data that was never materialized.
fn complete_superseded_outgoing_transit<S: EventStore>(
    node: &mut SimulationNode<S>,
    journal: &TransitJournal,
    ship_id: ShipId,
) -> bool {
    if !node.is_ship_in_transit(ship_id) {
        return false;
    }

    let Some(pending) = journal.pending_outgoing.get(&ship_id).copied() else {
        return false;
    };
    node.complete_outgoing_transit(
        ship_id,
        pending.identity.to,
        pending.entry_pos,
        pending.identity.request_tick,
    );
    node.get_ship_position(ship_id).is_none()
}

/// Apply a committed destination Commit idempotently and return the Ack effect.
///
/// Existing destination state is acknowledged only when a durable completion
/// receipt proves the same transfer identity. An unproven active-Ship collision
/// or a failed superseded-outgoing cleanup returns no Ack, preventing the source
/// from deleting its recovery copy.
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_commit<S: EventStore>(
    node: &mut SimulationNode<S>,
    journal: &TransitJournal,
    handoff: &TransitHandoffState,
    from: SectorId,
    to: SectorId,
    entry_pos: AbsolutePosition,
    gate_id: Option<JumpGateId>,
    request_tick: Tick,
) -> Option<AckEffect> {
    if to != node.sector_id() {
        return None;
    }

    let identity = TransferIdentity::new(handoff.ship_id, from, to, request_tick);
    if !destination_completed_transfer(node, journal, identity) {
        if node.get_ship_position(handoff.ship_id).is_some()
            && (!node.is_ship_in_transit(handoff.ship_id)
                || !complete_superseded_outgoing_transit(node, journal, handoff.ship_id))
        {
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
    }

    Some(AckEffect {
        ship_id: handoff.ship_id,
        from,
        to,
        request_tick,
    })
}

/// Validate a committed Ack against the durable request and confirm that source
/// cleanup actually removed the frozen recovery copy.
pub(super) fn apply_ack<S: EventStore>(
    node: &mut SimulationNode<S>,
    journal: &TransitJournal,
    ship_id: ShipId,
    from: SectorId,
    to: SectorId,
    request_tick: Tick,
) -> bool {
    if from != node.sector_id()
        || !node.is_ship_in_transit(ship_id)
        || node.get_ship_position(ship_id).is_none()
    {
        return false;
    }
    let Some(pending) = journal.pending_for(ship_id, from, to, request_tick) else {
        return false;
    };
    node.complete_outgoing_transit(
        ship_id,
        pending.identity.to,
        pending.entry_pos,
        pending.identity.request_tick,
    );
    node.get_ship_position(ship_id).is_none()
}

/// Return only retry Commit effects whose bounded backoff deadline is due.
/// Durable route facts come from the journal; canonical handoff state comes
/// from the frozen source entity.
pub(super) fn due_retries<S: EventStore>(
    node: &mut SimulationNode<S>,
    journal: &TransitJournal,
) -> Vec<CommitEffect> {
    let mut effects = Vec::new();
    for transit in journal.pending_outgoing() {
        if !node.transit_commit_retry_due(transit.identity.ship_id, transit.identity.request_tick) {
            continue;
        }
        let Some(handoff) = node.handoff_for_transit(transit.identity.ship_id) else {
            continue;
        };
        node.note_transit_commit_proposed(transit.identity.ship_id, transit.identity.request_tick);
        effects.push(CommitEffect {
            handoff,
            from: transit.identity.from,
            to: transit.identity.to,
            entry_pos: transit.entry_pos,
            gate_id: transit.gate_id,
            request_tick: transit.identity.request_tick,
        });
    }
    effects
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::events::{SectorTransitAborted, SectorTransitCompleted, SectorTransitRequested};
    use dawn_core::{NodeId, Position, SectorBounds, ShipTypeId, Velocity};
    use dawn_event_store::InMemoryEventStore;

    fn node(sector: u8) -> SimulationNode {
        SimulationNode::new_test(
            NodeId(sector),
            SectorId(sector),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        )
    }

    fn journal<S: EventStore>(node: &SimulationNode<S>) -> TransitJournal {
        let mut journal = TransitJournal::new(node.sector_id());
        for record in node.event_store().iter_from(0) {
            journal.observe(&record.event);
        }
        journal
    }

    #[test]
    fn pending_outbox_is_the_checkpoint_gate() {
        let mut source = node(0);
        let ship_id = source.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let effect = apply_request(&mut source, ship_id, SectorId(1), None).unwrap();
        assert!(has_pending_outgoing_transit(&journal(&source)));

        source.complete_outgoing_transit(
            effect.handoff.ship_id,
            effect.to,
            effect.entry_pos,
            effect.request_tick,
        );
        assert!(!has_pending_outgoing_transit(&journal(&source)));
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
        assert!(!has_pending_outgoing_transit(&journal(&sector_a)));

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
        assert!(!has_pending_outgoing_transit(&journal(&sector_a)));
        assert!(!has_pending_outgoing_transit(&journal(&sector_b)));
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

    fn requested(
        ship_id: ShipId,
        from: SectorId,
        to: SectorId,
        request_tick: u64,
        event_tick: u64,
    ) -> DomainEvent {
        DomainEvent::SectorTransitRequested(SectorTransitRequested {
            ship_id,
            from,
            to,
            request_tick: Tick(request_tick),
            gate_id: None,
            entry_pos: AbsolutePosition::ORIGIN,
            tick: Tick(event_tick),
        })
    }

    fn completed(
        ship_id: ShipId,
        from: SectorId,
        to: SectorId,
        request_tick: u64,
        event_tick: u64,
    ) -> DomainEvent {
        DomainEvent::SectorTransitCompleted(SectorTransitCompleted {
            handoff: test_handoff(ship_id),
            from,
            to,
            request_tick: Tick(request_tick),
            entry_pos: AbsolutePosition::ORIGIN,
            tick: Tick(event_tick),
        })
    }

    fn aborted(ship_id: ShipId, from: SectorId, to: SectorId, event_tick: u64) -> DomainEvent {
        DomainEvent::SectorTransitAborted(SectorTransitAborted {
            ship_id,
            from,
            to,
            tick: Tick(event_tick),
        })
    }

    #[test]
    fn stale_abort_for_an_old_route_does_not_clear_the_current_request() {
        let ship_id = ShipId::new(NodeId(0), 7);
        let mut current = TransitJournal::new(SectorId(0));

        current.observe(&requested(ship_id, SectorId(0), SectorId(1), 10, 1));
        current.observe(&requested(ship_id, SectorId(0), SectorId(2), 20, 2));
        current.observe(&aborted(ship_id, SectorId(0), SectorId(1), 3));

        assert!(current
            .pending_for(ship_id, SectorId(0), SectorId(2), Tick(20))
            .is_some());
        assert!(current.has_pending_outgoing());
    }

    #[test]
    fn matching_route_abort_clears_the_current_request() {
        let ship_id = ShipId::new(NodeId(0), 7);
        let mut current = TransitJournal::new(SectorId(0));

        current.observe(&requested(ship_id, SectorId(0), SectorId(1), 10, 1));
        current.observe(&aborted(ship_id, SectorId(0), SectorId(1), 2));

        assert!(!current.has_pending_outgoing());
    }

    #[test]
    fn repeated_same_route_replay_preserves_each_attempt_receipt_after_checkpoint() {
        let destination = node(1);
        let snapshot_before = destination.take_snapshot();
        let ship_id = ShipId::new(NodeId(0), 7);
        let mut store = InMemoryEventStore::new();

        for event in [
            requested(ship_id, SectorId(0), SectorId(1), 10, 1),
            completed(ship_id, SectorId(0), SectorId(1), 10, 1),
            requested(ship_id, SectorId(1), SectorId(0), 20, 2),
            completed(ship_id, SectorId(1), SectorId(0), 20, 2),
            requested(ship_id, SectorId(0), SectorId(1), 30, 3),
            completed(ship_id, SectorId(0), SectorId(1), 30, 3),
            requested(ship_id, SectorId(1), SectorId(0), 40, 4),
            completed(ship_id, SectorId(1), SectorId(0), 40, 4),
        ] {
            store.append(event);
        }

        let restored = SimulationNode::restore_from_test(
            store,
            &snapshot_before,
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            &[],
            &[],
        );
        assert!(restored.get_ship_position(ship_id).is_none());

        let checkpoint = restored.take_snapshot();
        for request_tick in [Tick(10), Tick(30)] {
            assert!(
                checkpoint.completed_incoming_transits.contains(
                    &crate::persistence::CompletedIncomingTransit {
                        ship_id,
                        from: SectorId(0),
                        to: SectorId(1),
                        request_tick,
                    }
                ),
                "missing durable receipt for A -> B attempt {request_tick:?}"
            );
        }

        let mut compacted = SimulationNode::restore_from_test(
            InMemoryEventStore::new(),
            &checkpoint,
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            &[],
            &[],
        );
        let events_before = compacted.total_event_count();
        let current = journal(&compacted);
        let ack = apply_commit(
            &mut compacted,
            &current,
            &test_handoff(ship_id),
            SectorId(0),
            SectorId(1),
            AbsolutePosition::ORIGIN,
            None,
            Tick(10),
        );

        assert!(ack.is_some());
        assert!(compacted.get_ship_position(ship_id).is_none());
        assert_eq!(compacted.total_event_count(), events_before);
    }
}
