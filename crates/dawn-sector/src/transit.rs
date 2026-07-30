//! Sector Transit Raft adapter (ADR-0014).
//!
//! The durable recovery and idempotency policy lives in [`pipeline`]. This
//! module owns only the wire payload and the translation between committed Raft
//! entries and pipeline proposals.

mod pipeline;

use crate::node::SimulationNode;
use crate::persistence::ShipSnapshot;
use dawn_consensus::RaftActorHandle;
use dawn_core::{AbsolutePosition, DomainEvent, JumpGateId, Position, SectorId, ShipId, Tick};
use dawn_event_store::store::EventStore;
use serde::{Deserialize, Serialize};
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

fn propose_commit(raft: &RaftActorHandle, proposal: pipeline::CommitProposal) {
    raft.propose(
        TransitOp::Commit {
            ship: Box::new(proposal.ship),
            from: proposal.from,
            to: proposal.to,
            entry_pos: proposal.entry_pos,
            entry_pos_abs: proposal.entry_pos_abs,
            gate_id: proposal.gate_id,
            request_tick: proposal.request_tick,
        }
        .encode(),
    );
}

fn propose_ack(raft: &RaftActorHandle, proposal: pipeline::AckProposal) {
    raft.propose(
        TransitOp::Ack {
            ship: Box::new(proposal.ship),
            from: proposal.from,
            to: proposal.to,
            entry_pos_abs: proposal.entry_pos_abs,
            request_tick: proposal.request_tick,
        }
        .encode(),
    );
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
                if let Some(proposal) = pipeline::apply_request(node, ship_id, to, gate_id) {
                    propose_commit(raft, proposal);
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
                if let Some(proposal) = pipeline::apply_commit(
                    node,
                    &ship,
                    from,
                    to,
                    entry_pos,
                    entry_pos_abs,
                    gate_id,
                    request_tick,
                ) {
                    propose_ack(raft, proposal);
                }
            }
            TransitOp::Ack {
                ship,
                from,
                to,
                entry_pos_abs,
                request_tick,
            } => {
                pipeline::apply_ack(node, &ship, from, to, entry_pos_abs, request_tick);
            }
        }
    }

    for proposal in pipeline::due_retries(node) {
        propose_commit(raft, proposal);
    }
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

    fn node(node_id: u8, sector_id: u8) -> SimulationNode {
        SimulationNode::new(
            NodeId(node_id),
            SectorId(sector_id),
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
        TransitOp::decode(&payload).expect("payload must decode as TransitOp")
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
    fn transit_payloads_round_trip() {
        let request = TransitOp::Request {
            ship_id: sample_ship().ship_id,
            to: SectorId(1),
            gate_id: Some(JumpGateId(0)),
        };
        assert!(matches!(TransitOp::decode(&request.encode()), Some(TransitOp::Request { .. })));

        let commit = TransitOp::Commit {
            ship: Box::new(sample_ship()),
            from: SectorId(0),
            to: SectorId(1),
            entry_pos: Position::ORIGIN,
            entry_pos_abs: AbsolutePosition::ORIGIN,
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
    }

    #[test]
    fn request_adapter_proposes_one_commit() {
        let mut source = node(0, 0);
        let ship_id = source.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let (raft, mut proposals) = raft_handle();
        let (tx, mut committed) = mpsc::unbounded_channel();
        tx.send(
            TransitOp::Request {
                ship_id,
                to: SectorId(1),
                gate_id: None,
            }
            .encode(),
        )
        .unwrap();

        apply_committed_raft_entries(&mut source, &raft, &mut committed);
        assert!(matches!(
            decode_proposed_transit(&mut proposals),
            TransitOp::Commit { .. }
        ));
        assert!(proposals.try_recv().is_err());
    }

    #[test]
    fn commit_then_ack_moves_ownership() {
        let mut source = node(0, 0);
        let mut destination = node(1, 1);
        let ship_id = source.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);

        let (source_raft, mut source_proposals) = raft_handle();
        let (request_tx, mut request_rx) = mpsc::unbounded_channel();
        request_tx
            .send(
                TransitOp::Request {
                    ship_id,
                    to: SectorId(1),
                    gate_id: None,
                }
                .encode(),
            )
            .unwrap();
        apply_committed_raft_entries(&mut source, &source_raft, &mut request_rx);
        let commit = decode_proposed_transit(&mut source_proposals);

        let (destination_raft, mut destination_proposals) = raft_handle();
        let (commit_tx, mut commit_rx) = mpsc::unbounded_channel();
        commit_tx.send(commit.encode()).unwrap();
        apply_committed_raft_entries(&mut destination, &destination_raft, &mut commit_rx);
        let ack = decode_proposed_transit(&mut destination_proposals);

        let (noop_raft, _noop_proposals) = raft_handle();
        let (ack_tx, mut ack_rx) = mpsc::unbounded_channel();
        ack_tx.send(ack.encode()).unwrap();
        apply_committed_raft_entries(&mut source, &noop_raft, &mut ack_rx);

        assert!(source.get_ship_position(ship_id).is_none());
        assert!(destination.get_ship_position(ship_id).is_some());
    }
}