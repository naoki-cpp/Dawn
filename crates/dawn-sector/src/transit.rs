//! Sector Transit commands carried through Raft (ADR-0014).
//!
//! Request freezes the source Ship. Commit materializes the destination. Ack
//! removes the source copy. The source retries pending Requests from its
//! EventStore, so a crash between phases converges without atomic cross-node
//! EventStore writes.

use crate::node::SimulationNode;
use crate::persistence::ShipSnapshot;
use dawn_consensus::RaftActorHandle;
use dawn_core::{AbsolutePosition, DomainEvent, JumpGateId, Position, SectorId, ShipId, Tick};
use dawn_event_store::store::EventStore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransitOp {
    Request {
        ship_id: ShipId,
        to: SectorId,
        gate_id: Option<JumpGateId>,
    },
    Commit {
        ship: Box<ShipSnapshot>,
        from: SectorId,
        to: SectorId,
        entry_pos: Position,
        entry_pos_abs: AbsolutePosition,
        gate_id: Option<JumpGateId>,
        request_tick: Tick,
    },
    Ack {
        ship: Box<ShipSnapshot>,
        from: SectorId,
        to: SectorId,
        entry_pos_abs: AbsolutePosition,
        request_tick: Tick,
    },
}

impl TransitOp {
    pub fn encode(&self) -> Vec<u8> {
        postcard::to_stdvec(self).expect("TransitOp serialization cannot fail")
    }

    pub fn decode(payload: &[u8]) -> Option<Self> {
        postcard::from_bytes(payload).ok()
    }
}

#[derive(Debug, Clone, Copy)]
struct PendingTransit {
    ship_id: ShipId,
    from: SectorId,
    to: SectorId,
    request_tick: Tick,
}

fn pending_outgoing_transits<S: EventStore>(node: &SimulationNode<S>) -> Vec<PendingTransit> {
    let sector_id = node.sector_id();
    let mut pending = HashMap::<ShipId, PendingTransit>::new();
    for record in node.event_store().iter_from(0) {
        match &record.event {
            DomainEvent::SectorTransitRequested(e) if e.from == sector_id => {
                pending.insert(
                    e.ship_id,
                    PendingTransit {
                        ship_id: e.ship_id,
                        from: e.from,
                        to: e.to,
                        request_tick: e.tick,
                    },
                );
            }
            DomainEvent::SectorTransitCompleted(e) if e.from == sector_id => {
                pending.remove(&e.ship_id);
            }
            DomainEvent::SectorTransitAborted(e) if e.from == sector_id => {
                pending.remove(&e.ship_id);
            }
            _ => {}
        }
    }
    pending.into_values().collect()
}

fn inferred_gate_id<S: EventStore>(
    node: &SimulationNode<S>,
    from: SectorId,
    to: SectorId,
) -> Option<JumpGateId> {
    node.galaxy()
        .gates_in_sector(from)
        .into_iter()
        .find(|gate| gate.to_sector == to)
        .map(|gate| gate.id)
}

fn transit_entry<S: EventStore>(
    node: &SimulationNode<S>,
    from: SectorId,
    to: SectorId,
    gate_id: Option<JumpGateId>,
) -> (Position, AbsolutePosition) {
    let gate = gate_id.and_then(|_| {
        node.galaxy()
            .gates_in_sector(to)
            .into_iter()
            .find(|gate| gate.to_sector == from)
    });
    (
        gate.map(|g| g.position).unwrap_or(Position::ORIGIN),
        gate.map(|g| g.abs_m).unwrap_or(AbsolutePosition::ORIGIN),
    )
}

fn snapshot_ship<S: EventStore>(
    node: &mut SimulationNode<S>,
    ship_id: ShipId,
) -> Option<ShipSnapshot> {
    node.take_snapshot()
        .ships
        .into_iter()
        .find(|ship| ship.ship_id == ship_id)
}

#[allow(clippy::too_many_arguments)]
fn propose_commit(
    raft: &RaftActorHandle,
    ship: ShipSnapshot,
    from: SectorId,
    to: SectorId,
    entry_pos: Position,
    entry_pos_abs: AbsolutePosition,
    gate_id: Option<JumpGateId>,
    request_tick: Tick,
) {
    raft.propose(
        TransitOp::Commit {
            ship: Box::new(ship),
            from,
            to,
            entry_pos,
            entry_pos_abs,
            gate_id,
            request_tick,
        }
        .encode(),
    );
}

fn retry_pending_transits<S: EventStore>(node: &mut SimulationNode<S>, raft: &RaftActorHandle) {
    let pending = pending_outgoing_transits(node);
    if pending.is_empty() {
        return;
    }
    let snapshot = node.take_snapshot();

    for transit in pending {
        let Some(ship) = snapshot
            .ships
            .iter()
            .find(|ship| ship.ship_id == transit.ship_id)
            .cloned()
        else {
            continue;
        };
        let gate_id = inferred_gate_id(node, transit.from, transit.to);

        // A checkpoint may cover the Requested event while ShipSnapshot does
        // not carry TransitComp. Re-issuing Request restores the frozen marker.
        if node.can_propose_transit(transit.ship_id) {
            raft.propose(
                TransitOp::Request {
                    ship_id: transit.ship_id,
                    to: transit.to,
                    gate_id,
                }
                .encode(),
            );
            continue;
        }

        let (entry_pos, entry_pos_abs) = transit_entry(node, transit.from, transit.to, gate_id);
        propose_commit(
            raft,
            ship,
            transit.from,
            transit.to,
            entry_pos,
            entry_pos_abs,
            gate_id,
            transit.request_tick,
        );
    }
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

pub fn apply_committed_raft_entries<S: EventStore>(
    node: &mut SimulationNode<S>,
    raft: &RaftActorHandle,
    committed_rx: &mut mpsc::UnboundedReceiver<Vec<u8>>,
) {
    while let Ok(payload) = committed_rx.try_recv() {
        let Some(op) = TransitOp::decode(&payload) else {
            continue;
        };
        match op {
            TransitOp::Request {
                ship_id,
                to,
                gate_id,
            } => {
                if let Some(data) = node.prepare_transit_commit(ship_id, to, gate_id) {
                    propose_commit(
                        raft,
                        *data.ship,
                        node.sector_id(),
                        to,
                        data.entry_pos,
                        data.entry_pos_abs,
                        gate_id,
                        node.current_tick(),
                    );
                }
            }
            TransitOp::Commit {
                ship,
                from,
                to,
                entry_pos,
                entry_pos_abs,
                gate_id,
                request_tick,
            } => {
                if to == node.sector_id() {
                    if node.get_ship_position(ship.ship_id).is_none() {
                        node.handle_transit_commit(
                            &ship,
                            from,
                            entry_pos,
                            entry_pos_abs,
                            gate_id,
                        );
                    }
                    let ack_ship =
                        snapshot_ship(node, ship.ship_id).unwrap_or_else(|| (*ship).clone());
                    raft.propose(
                        TransitOp::Ack {
                            ship: Box::new(ack_ship),
                            from,
                            to,
                            entry_pos_abs,
                            request_tick,
                        }
                        .encode(),
                    );
                }
            }
            TransitOp::Ack {
                ship,
                from,
                to,
                entry_pos_abs,
                request_tick,
            } => {
                if from == node.sector_id()
                    && request_matches(node, ship.ship_id, from, to, request_tick)
                {
                    node.complete_outgoing_transit(&ship, to, entry_pos_abs);
                }
            }
        }
    }
    retry_pending_transits(node, raft);
}

#[derive(Debug)]
pub struct RuntimeTickOutput {
    pub tick_result: crate::node::TickResult,
    pub events: Vec<DomainEvent>,
    pub pending_auto_jumps: Vec<(ShipId, JumpGateId)>,
    pub completed_warps: Vec<ShipId>,
}

pub fn run_runtime_tick<S, F>(
    node: &mut SimulationNode<S>,
    raft: &RaftActorHandle,
    committed_rx: &mut mpsc::UnboundedReceiver<Vec<u8>>,
    lock_commands: &[dawn_core::LockOnCommand],
    after_events_appended: F,
) -> RuntimeTickOutput
where
    S: EventStore,
    F: FnOnce(&mut SimulationNode<S>, &crate::node::TickResult),
{
    let events_before = node.total_event_count() as u64;
    apply_committed_raft_entries(node, raft, committed_rx);
    let result = node.tick_with_lock_commands(lock_commands);
    after_events_appended(node, &result);
    raft.tick();
    let pending_auto_jumps = node.drain_pending_auto_jumps();
    let completed_warps = node.drain_completed_warps();
    let events = node
        .event_store()
        .iter_from(events_before)
        .map(|record| record.event.clone())
        .collect();

    RuntimeTickOutput {
        tick_result: result,
        events,
        pending_auto_jumps,
        completed_warps,
    }
}

pub fn propose_jump<S: EventStore>(
    node: &mut SimulationNode<S>,
    raft: &RaftActorHandle,
    ship_id: ShipId,
    gate_id: JumpGateId,
) -> crate::node::JumpOutcome {
    let outcome = node.apply_jump_with_fallback(ship_id, gate_id);
    if let crate::node::JumpOutcome::NeedsTransitProposal { to } = outcome {
        propose_transit_request(raft, ship_id, to, gate_id);
    }
    outcome
}

pub fn propose_auto_jump<S: EventStore>(
    node: &mut SimulationNode<S>,
    raft: &RaftActorHandle,
    ship_id: ShipId,
    gate_id: JumpGateId,
) -> Option<SectorId> {
    let to = node.resolve_auto_jump(ship_id, gate_id)?;
    propose_transit_request(raft, ship_id, to, gate_id);
    Some(to)
}

fn propose_transit_request(
    raft: &RaftActorHandle,
    ship_id: ShipId,
    to: SectorId,
    gate_id: JumpGateId,
) {
    raft.propose(
        TransitOp::Request {
            ship_id,
            to,
            gate_id: Some(gate_id),
        }
        .encode(),
    );
}

pub fn step_cluster_node<S: EventStore>(
    node: &mut SimulationNode<S>,
    raft: &RaftActorHandle,
    committed_rx: &mut mpsc::UnboundedReceiver<Vec<u8>>,
    lock_commands: &[dawn_core::LockOnCommand],
) -> crate::node::TickResult {
    apply_committed_raft_entries(node, raft, committed_rx);
    let result = node.tick_with_lock_commands(lock_commands);
    raft.tick();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::fitting::FittingSnapshot;
    use dawn_core::{NodeId, SectorBounds, ShipTypeId, Velocity};

    fn mem_node() -> SimulationNode {
        SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        )
    }

    fn raft_handle() -> (
        RaftActorHandle,
        mpsc::UnboundedReceiver<dawn_consensus::RaftActorMessage>,
    ) {
        let (tx, rx) = mpsc::unbounded_channel();
        (RaftActorHandle::new(tx), rx)
    }

    fn decode_proposed_transit(
        rx: &mut mpsc::UnboundedReceiver<dawn_consensus::RaftActorMessage>,
    ) -> TransitOp {
        let msg = rx.try_recv().expect("a proposal must have been sent");
        let payload = match msg {
            dawn_consensus::RaftActorMessage::Propose(payload) => payload,
            other => panic!("expected Propose, got {other:?}"),
        };
        TransitOp::decode(&payload).expect("payload must decode as a TransitOp")
    }

    #[test]
    fn propose_jump_proposes_a_transit_request_when_the_ship_is_in_range() {
        let mut node = mem_node();
        let (raft, mut rx) = raft_handle();
        let gate = *node.jump_gate(JumpGateId(0)).expect("Sector 0 has Gate 0");
        let near_gate_abs = [
            gate.abs_m[0] - (gate.activation_radius * 0.5),
            gate.abs_m[1],
            gate.abs_m[2],
        ];
        let player_id = node.next_player_id();
        let ship = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);
        node.set_spawn_anchor_abs(ship, near_gate_abs);

        let outcome = propose_jump(&mut node, &raft, ship, JumpGateId(0));
        assert_eq!(
            outcome,
            crate::node::JumpOutcome::NeedsTransitProposal { to: gate.to_sector }
        );
        assert!(matches!(
            decode_proposed_transit(&mut rx),
            TransitOp::Request {
                ship_id,
                gate_id: Some(JumpGateId(0)),
                ..
            } if ship_id == ship
        ));
    }

    #[test]
    fn propose_jump_does_not_propose_when_the_ship_is_out_of_range() {
        let mut node = mem_node();
        let (raft, mut rx) = raft_handle();
        let player_id = node.next_player_id();
        let ship = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);
        let outcome = propose_jump(&mut node, &raft, ship, JumpGateId(0));
        assert!(!matches!(
            outcome,
            crate::node::JumpOutcome::NeedsTransitProposal { .. }
        ));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn request_op_round_trips() {
        let op = TransitOp::Request {
            ship_id: ShipId::new(NodeId(0), 42),
            to: SectorId(1),
            gate_id: Some(JumpGateId(0)),
        };
        assert!(matches!(
            TransitOp::decode(&op.encode()),
            Some(TransitOp::Request {
                gate_id: Some(JumpGateId(0)),
                ..
            })
        ));
    }

    fn sample_ship() -> ShipSnapshot {
        ShipSnapshot {
            ship_id: ShipId::new(NodeId(0), 7),
            ship_type_id: ShipTypeId(1),
            absolute_position: None,
            position: Position::new(1.0, 2.0, 3.0),
            anchor: dawn_core::AnchorId(0),
            velocity: Velocity::new(4.0, 5.0, 6.0),
            current_shield: 10.0,
            current_armor: 20.0,
            current_hull: 30.0,
            is_destroyed: false,
            capacitor: Some(50.0),
            fitting: FittingSnapshot::empty(),
            tackled_by: vec![],
            inventory: std::collections::BTreeMap::new(),
        }
    }

    #[test]
    fn commit_and_ack_round_trip() {
        let commit = TransitOp::Commit {
            ship: Box::new(sample_ship()),
            from: SectorId(0),
            to: SectorId(1),
            entry_pos: Position::new(500.0, 0.0, 0.0),
            entry_pos_abs: AbsolutePosition::new(500.0, 0.0, 0.0),
            gate_id: None,
            request_tick: Tick(12),
        };
        assert!(matches!(
            TransitOp::decode(&commit.encode()),
            Some(TransitOp::Commit {
                request_tick: Tick(12),
                ..
            })
        ));

        let ack = TransitOp::Ack {
            ship: Box::new(sample_ship()),
            from: SectorId(0),
            to: SectorId(1),
            entry_pos_abs: AbsolutePosition::new(500.0, 0.0, 0.0),
            request_tick: Tick(12),
        };
        assert!(matches!(
            TransitOp::decode(&ack.encode()),
            Some(TransitOp::Ack {
                request_tick: Tick(12),
                ..
            })
        ));
    }

    #[test]
    fn decode_returns_none_for_garbage_payload() {
        assert!(TransitOp::decode(&[0xFF, 0xFE, 0xFD]).is_none());
    }
}
