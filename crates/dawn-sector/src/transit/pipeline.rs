//! Durable Sector Transit recovery policy (ADR-0014).
//!
//! This is the deep module behind the thin Raft and checkpoint adapters. It
//! owns the durable-outbox scan, retry scheduling decisions, destination
//! idempotency, and source Ack validation. ECS mutation remains encapsulated by
//! `SimulationNode`; transport code only turns the returned proposals into Raft
//! payloads.

use std::collections::HashMap;

use crate::node::SimulationNode;
use crate::persistence::ShipSnapshot;
use dawn_core::{AbsolutePosition, DomainEvent, JumpGateId, Position, SectorId, ShipId, Tick};
use dawn_event_store::store::EventStore;

#[derive(Debug)]
pub(crate) struct CommitProposal {
    pub ship: ShipSnapshot,
    pub from: SectorId,
    pub to: SectorId,
    pub entry_pos: Position,
    pub entry_pos_abs: AbsolutePosition,
    pub gate_id: Option<JumpGateId>,
    pub request_tick: Tick,
}

#[derive(Debug)]
pub(crate) struct AckProposal {
    pub ship: ShipSnapshot,
    pub from: SectorId,
    pub to: SectorId,
    pub entry_pos_abs: AbsolutePosition,
    pub request_tick: Tick,
}

#[derive(Debug, Clone, Copy)]
struct PendingTransit {
    ship_id: ShipId,
    from: SectorId,
    to: SectorId,
    request_tick: Tick,
    gate_id: Option<JumpGateId>,
    entry_pos: Position,
    entry_pos_abs: AbsolutePosition,
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
                        entry_pos_abs: event.entry_pos_abs,
                    },
                );
            }
            DomainEvent::SectorTransitCompleted(event) if event.from == sector_id => {
                pending.remove(&event.ship_id);
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
                    && event.ship_id == ship_id
                    && event.from == from
                    && event.to == to =>
            {
                return true;
            }
            _ => {}
        }
    }
    false
}

fn snapshot_ship<S: EventStore>(node: &SimulationNode<S>, ship_id: ShipId) -> Option<ShipSnapshot> {
    node.take_snapshot()
        .ships
        .into_iter()
        .find(|ship| ship.ship_id == ship_id)
}

fn request_matches<S: EventStore>(
    node: &SimulationNode<S>,
    ship_id: ShipId,
    from: SectorId,
    to: SectorId,
    request_tick: Tick,
) -> bool {
    node.get_ship_position(ship_id).is_some()
        && pending_outgoing_transits(node).iter().any(|pending| {
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
        ship: *data.ship,
        from: node.sector_id(),
        to,
        entry_pos: data.entry_pos,
        entry_pos_abs: data.entry_pos_abs,
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
    ship: &ShipSnapshot,
    from: SectorId,
    to: SectorId,
    entry_pos: Position,
    entry_pos_abs: AbsolutePosition,
    gate_id: Option<JumpGateId>,
    request_tick: Tick,
) -> Option<AckProposal> {
    if to != node.sector_id() {
        return None;
    }

    let ship_present = node.get_ship_position(ship.ship_id).is_some();
    let completed = destination_completed_transfer(node, ship.ship_id, from, to, request_tick);
    if !completed && !ship_present {
        node.append_incoming_transit_marker(
            ship.ship_id,
            from,
            to,
            request_tick,
            gate_id,
            entry_pos,
            entry_pos_abs,
        );
        node.handle_transit_commit(ship, from, entry_pos, entry_pos_abs, gate_id, request_tick);
    }

    Some(AckProposal {
        ship: snapshot_ship(node, ship.ship_id).unwrap_or_else(|| ship.clone()),
        from,
        to,
        entry_pos_abs,
        request_tick,
    })
}

/// Validate a committed Ack against the durable source request before removing
/// the frozen source copy. Returns whether the Ack completed the handoff.
pub(crate) fn apply_ack<S: EventStore>(
    node: &mut SimulationNode<S>,
    ship: &ShipSnapshot,
    from: SectorId,
    to: SectorId,
    entry_pos_abs: AbsolutePosition,
    request_tick: Tick,
) -> bool {
    if from != node.sector_id() || !request_matches(node, ship.ship_id, from, to, request_tick) {
        return false;
    }
    node.complete_outgoing_transit(ship, to, entry_pos_abs);
    true
}

/// Return only retry Commit proposals whose bounded backoff deadline is due.
/// The durable route and canonical transit snapshot are reconstructed from the
/// source EventStore and frozen ECS state, respectively.
pub(crate) fn due_retries<S: EventStore>(node: &mut SimulationNode<S>) -> Vec<CommitProposal> {
    let mut proposals = Vec::new();
    for transit in pending_outgoing_transits(node) {
        if !node.transit_commit_retry_due(transit.ship_id, transit.request_tick) {
            continue;
        }
        let Some(ship) = node.snapshot_for_transit(transit.ship_id) else {
            continue;
        };
        node.note_transit_commit_proposed(transit.ship_id, transit.request_tick);
        proposals.push(CommitProposal {
            ship,
            from: transit.from,
            to: transit.to,
            entry_pos: transit.entry_pos,
            entry_pos_abs: transit.entry_pos_abs,
            gate_id: transit.gate_id,
            request_tick: transit.request_tick,
        });
    }
    proposals
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{NodeId, SectorBounds, ShipTypeId, Velocity};

    fn node(sector: u8) -> SimulationNode {
        SimulationNode::new(
            NodeId(sector),
            SectorId(sector),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        )
    }

    #[test]
    fn pending_outbox_is_the_checkpoint_gate() {
        let mut source = node(0);
        let ship_id = source.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let proposal = apply_request(&mut source, ship_id, SectorId(1), None).unwrap();
        assert!(has_pending_outgoing_transit(&source));

        source.complete_outgoing_transit(&proposal.ship, proposal.to, proposal.entry_pos_abs);
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
            &proposal.ship,
            proposal.from,
            proposal.to,
            proposal.entry_pos,
            proposal.entry_pos_abs,
            proposal.gate_id,
            proposal.request_tick,
        );
        let event_count = destination.total_event_count();
        let second = apply_commit(
            &mut destination,
            &proposal.ship,
            proposal.from,
            proposal.to,
            proposal.entry_pos,
            proposal.entry_pos_abs,
            proposal.gate_id,
            proposal.request_tick,
        );

        assert!(first.is_some() && second.is_some());
        assert_eq!(destination.total_event_count(), event_count);
        assert_eq!(destination.ship_count(), 1);
    }
}
