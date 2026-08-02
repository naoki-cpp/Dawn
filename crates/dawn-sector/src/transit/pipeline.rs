//! Durable Sector Transit recovery policy (ADR-0014).
//!
//! This is the deep module behind the thin Raft, replay, and checkpoint
//! adapters. It owns the durable-outbox scan, retry scheduling decisions,
//! destination idempotency, source Ack validation, and classification of the
//! Transit events consumed during snapshot-plus-tail replay. ECS mutation
//! remains encapsulated by `SimulationNode`; adapters only execute the decision
//! returned here.

use std::collections::HashMap;

use crate::node::SimulationNode;
use dawn_core::{
    AbsolutePosition, DomainEvent, JumpGateId, SectorId, ShipId, Tick, TransitHandoffState,
};
use dawn_event_store::store::EventStore;

#[derive(Debug)]
pub(crate) struct CommitProposal {
    pub handoff: TransitHandoffState,
    pub from: SectorId,
    pub to: SectorId,
    pub entry_pos: AbsolutePosition,
    pub gate_id: Option<JumpGateId>,
    pub request_tick: Tick,
}

#[derive(Debug)]
pub(crate) struct AckProposal {
    pub ship_id: ShipId,
    pub from: SectorId,
    pub to: SectorId,
    pub request_tick: Tick,
}

/// Transit-specific action for the generic EventStore replay adapter.
///
/// Keeping this classification beside Request/Commit/Ack policy prevents
/// `node::apply_event` from knowing the Transit event catalog. The node-side
/// methods named by these variants remain the mechanism because they require
/// private ECS state.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ReplayDirective<'a> {
    Requested(&'a dawn_core::events::SectorTransitRequested),
    Completed(&'a dawn_core::events::SectorTransitCompleted),
    Aborted(&'a dawn_core::events::SectorTransitAborted),
}

pub(crate) fn replay_directive(event: &DomainEvent) -> Option<ReplayDirective<'_>> {
    match event {
        DomainEvent::SectorTransitRequested(event) => Some(ReplayDirective::Requested(event)),
        DomainEvent::SectorTransitCompleted(event) => Some(ReplayDirective::Completed(event)),
        DomainEvent::SectorTransitAborted(event) => Some(ReplayDirective::Aborted(event)),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
struct PendingTransit {
    ship_id: ShipId,
    from: SectorId,
    to: SectorId,
    request_tick: Tick,
    gate_id: Option<JumpGateId>,
    entry_pos: AbsolutePosition,
}

fn pending_outgoing_transits<S: EventStore>(node: &SimulationNode<S>) -> Vec<PendingTransit> {
    let sector_id = node.sector_id();
    let mut pending = HashMap::<ShipId, PendingTransit>::new();
    for record in node.event_store().iter_from(0) {
        match &record.event {
            DomainEvent::SectorTransitRequested(event) if event.from == sector_id => {
                pending.insert(
                    event.ship_id,
                    PendingTransit {
                        ship_id: event.ship_id,
                        from: event.from,
                        to: event.to,
                        request_tick: event.request_tick,
                        gate_id: event.gate_id,
                        entry_pos: event.entry_pos,
                    },
                );
            }
            DomainEvent::SectorTransitCompleted(event) if event.from == sector_id => {
                pending.remove(&event.handoff.ship_id);
            }
            DomainEvent::SectorTransitAborted(event) if event.from == sector_id => {
                pending.remove(&event.ship_id);
            }
            _ => {}
        }
    }
    pending.into_values().collect()
}

/// Whether checkpoint compaction must be deferred to preserve the durable
/// outgoing request used for restart retry.
pub(crate) fn has_pending_outgoing_transit<S: EventStore>(node: &SimulationNode<S>) -> bool {
    !pending_outgoing_transits(node).is_empty()
}

fn destination_completed_transfer<S: EventStore>(
    node: &SimulationNode<S>,
    ship_id: ShipId,
    from: SectorId,
    to: SectorId,
    request_tick: Tick,
) -> bool {
    if node.has_completed_incoming_transit(ship_id, from, to, request_tick) {
        return true;
    }

    // Before a checkpoint, the hot EventStore pair is also a valid receipt.
    // After compaction, `completed_incoming_transits` in the snapshot is the
    // durable authority instead.
    let mut marker_seen = false;
    for record in node.event_store().iter_from(0) {
        match &record.event {
            DomainEvent::SectorTransitRequested(event)
                if event.ship_id == ship_id
                    && event.from == from
                    && event.to == to
                    && event.request_tick == request_tick =>
            {
                marker_seen = true;
            }
            DomainEvent::SectorTransitCompleted(event)
                if marker_seen
                    && event.handoff.ship_id == ship_id
                    && event.from == from
                    && event.to == to
                    && event.request_tick == request_tick =>
            {
                return true;
            }
            _ => {}
        }
    }
    false
}

/// Close a still-pending outgoing attempt before accepting the same Ship
/// back into this Sector. This occurs when the Ship returns before the Ack
/// for its previous departure reaches the source: the local ECS entry is a
/// frozen recovery copy, not an active destination-side materialization.
fn complete_superseded_outgoing_transit<S: EventStore>(
    node: &mut SimulationNode<S>,
    ship_id: ShipId,
) -> bool {
    if !node.is_ship_in_transit(ship_id) {
        return false;
    }

    let Some(pending) = pending_outgoing_transits(node)
        .into_iter()
        .find(|pending| pending.ship_id == ship_id)
    else {
        return false;
    };
    node.complete_outgoing_transit(ship_id, pending.to, pending.entry_pos, pending.request_tick);
    true
}

fn matching_request<S: EventStore>(
    node: &SimulationNode<S>,
    ship_id: ShipId,
    from: SectorId,
    to: SectorId,
    request_tick: Tick,
) -> Option<PendingTransit> {
    node.get_ship_position(ship_id)?;
    pending_outgoing_transits(node).into_iter().find(|pending| {
        pending.ship_id == ship_id
            && pending.from == from
            && pending.to == to
            && pending.request_tick == request_tick
    })
}

/// Apply a committed Request and return the one Commit proposal the Raft
/// adapter should send. Registering the retry deadline happens before the
/// proposal leaves this function, preventing a same-step duplicate.
pub(crate) fn apply_request<S: EventStore>(
    node: &mut SimulationNode<S>,
    ship_id: ShipId,
    to: SectorId,
    gate_id: Option<JumpGateId>,
) -> Option<CommitProposal> {
    let data = node.prepare_transit_commit(ship_id, to, gate_id)?;
    let request_tick = data.request_tick;
    node.note_transit_commit_proposed(ship_id, request_tick);
    Some(CommitProposal {
        handoff: *data.handoff,
        from: node.sector_id(),
        to,
        entry_pos: data.entry_pos,
        gate_id,
        request_tick,
    })
}

/// Apply a committed destination Commit idempotently and return the Ack the
/// Raft adapter should send. A checkpointed, already-materialized Ship is a
/// durable dedupe fact even when the original event pair moved to cold storage.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_commit<S: EventStore>(
    node: &mut SimulationNode<S>,
    handoff: &TransitHandoffState,
    from: SectorId,
    to: SectorId,
    entry_pos: AbsolutePosition,
    gate_id: Option<JumpGateId>,
    request_tick: Tick,
) -> Option<AckProposal> {
    if to != node.sector_id() {
        return None;
    }

    let completed = destination_completed_transfer(node, handoff.ship_id, from, to, request_tick);
    if !completed {
        // A Ship can return before the Ack for its previous departure reaches
        // this Sector. In that case the existing entity is an InTransit frozen
        // recovery copy. Close that older outbox first so the delayed Ack cannot
        // later delete the newly returned active Ship.
        if node.get_ship_position(handoff.ship_id).is_some()
            && node.is_ship_in_transit(handoff.ship_id)
        {
            complete_superseded_outgoing_transit(node, handoff.ship_id);
        }

        // A non-InTransit Ship with this ID is already the active materialization
        // for this attempt (or a duplicate delivery), so it remains Ack-only.
        if node.get_ship_position(handoff.ship_id).is_none() {
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
    }

    Some(AckProposal {
        ship_id: handoff.ship_id,
        from,
        to,
        request_tick,
    })
}

/// Validate a committed Ack against the durable source request before removing
/// the frozen source copy. Returns whether the Ack completed the handoff.
pub(crate) fn apply_ack<S: EventStore>(
    node: &mut SimulationNode<S>,
    ship_id: ShipId,
    from: SectorId,
    to: SectorId,
    request_tick: Tick,
) -> bool {
    if from != node.sector_id() {
        return false;
    }
    let Some(pending) = matching_request(node, ship_id, from, to, request_tick) else {
        return false;
    };
    node.complete_outgoing_transit(ship_id, pending.to, pending.entry_pos, pending.request_tick);
    true
}

/// Return only retry Commit proposals whose bounded backoff deadline is due.
/// The durable route and canonical Transit handoff state are reconstructed from
/// the source EventStore and frozen ECS state, respectively.
pub(crate) fn due_retries<S: EventStore>(node: &mut SimulationNode<S>) -> Vec<CommitProposal> {
    let mut proposals = Vec::new();
    for transit in pending_outgoing_transits(node) {
        if !node.transit_commit_retry_due(transit.ship_id, transit.request_tick) {
            continue;
        }
        let Some(handoff) = node.handoff_for_transit(transit.ship_id) else {
            continue;
        };
        node.note_transit_commit_proposed(transit.ship_id, transit.request_tick);
        proposals.push(CommitProposal {
            handoff,
            from: transit.from,
            to: transit.to,
            entry_pos: transit.entry_pos,
            gate_id: transit.gate_id,
            request_tick: transit.request_tick,
        });
    }
    proposals
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::events::{SectorTransitCompleted, SectorTransitRequested};
    use dawn_core::{NodeId, Position, SectorBounds, ShipTypeId, Velocity};
    use dawn_event_store::InMemoryEventStore;

    fn node(sector: u8) -> SimulationNode {
        SimulationNode::new(
            NodeId(sector),
            SectorId(sector),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        )
    }

    #[test]
    fn pending_outbox_is_the_checkpoint_gate() {
        let mut source = node(0);
        let ship_id = source.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let proposal = apply_request(&mut source, ship_id, SectorId(1), None).unwrap();
        assert!(has_pending_outgoing_transit(&source));

        source.complete_outgoing_transit(
            proposal.handoff.ship_id,
            proposal.to,
            proposal.entry_pos,
            proposal.request_tick,
        );
        assert!(!has_pending_outgoing_transit(&source));
    }

    #[test]
    fn duplicate_destination_commit_is_ack_only() {
        let mut source = node(0);
        let mut destination = node(1);
        let ship_id = source.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let proposal = apply_request(&mut source, ship_id, SectorId(1), None).unwrap();

        let first = apply_commit(
            &mut destination,
            &proposal.handoff,
            proposal.from,
            proposal.to,
            proposal.entry_pos,
            proposal.gate_id,
            proposal.request_tick,
        );
        let event_count = destination.total_event_count();
        let second = apply_commit(
            &mut destination,
            &proposal.handoff,
            proposal.from,
            proposal.to,
            proposal.entry_pos,
            proposal.gate_id,
            proposal.request_tick,
        );

        assert!(first.is_some() && second.is_some());
        assert_eq!(destination.total_event_count(), event_count);
        assert_eq!(destination.ship_count(), 1);
    }

    #[test]
    fn incoming_return_replaces_unacked_frozen_copy() {
        let mut sector_a = node(0);
        let mut sector_b = node(1);
        let ship_id = sector_a.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);

        let outbound = apply_request(&mut sector_a, ship_id, SectorId(1), None).unwrap();
        let delayed_outbound_ack = apply_commit(
            &mut sector_b,
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
        let return_ack = apply_commit(
            &mut sector_a,
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
        assert!(!has_pending_outgoing_transit(&sector_a));

        assert!(apply_ack(
            &mut sector_b,
            return_ack.ship_id,
            return_ack.from,
            return_ack.to,
            return_ack.request_tick,
        ));
        assert_eq!(sector_b.ship_count(), 0);

        // The late Ack for A -> B must no longer match an outgoing request
        // and therefore cannot delete the active Ship that just returned to A.
        assert!(!apply_ack(
            &mut sector_a,
            delayed_outbound_ack.ship_id,
            delayed_outbound_ack.from,
            delayed_outbound_ack.to,
            delayed_outbound_ack.request_tick,
        ));
        assert_eq!(sector_a.ship_count(), 1);
        assert!(!sector_a.is_ship_in_transit(ship_id));
        assert!(!has_pending_outgoing_transit(&sector_a));
        assert!(!has_pending_outgoing_transit(&sector_b));
    }

    fn test_handoff(ship_id: ShipId) -> TransitHandoffState {
        TransitHandoffState {
            ship_id,
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

    #[test]
    fn repeated_same_route_replay_preserves_each_attempt_receipt_after_checkpoint() {
        let destination = node(1);
        let snapshot_before = destination.take_snapshot();
        let ship_id = ShipId::new(NodeId(0), 7);
        let mut store = InMemoryEventStore::new();

        // A -> B -> A -> B -> A in one post-snapshot tail. The two
        // A -> B attempts must retain distinct source-local identities.
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

        let restored = SimulationNode::restore_from(
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

        // Simulate compaction: only the checkpoint survives. A delayed
        // Commit from the first attempt must produce Ack only, never
        // resurrecting the Ship after it has already left B again.
        let mut compacted = SimulationNode::restore_from(
            InMemoryEventStore::new(),
            &checkpoint,
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            &[],
            &[],
        );
        let events_before = compacted.total_event_count();
        let ack = apply_commit(
            &mut compacted,
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
