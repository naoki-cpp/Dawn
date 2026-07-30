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

fn destination_completed_transfer<S: EventStore>(
    node: &SimulationNode<S>,
    ship_id: ShipId,
    from: SectorId,
    to: SectorId,
    request_tick: Tick,
) -> bool {
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
    for transit in pending {
        if !node.transit_commit_retry_due(transit.ship_id, transit.request_tick) {
            continue;
        }
        let Some(ship) = node.snapshot_for_transit(transit.ship_id) else {
            continue;
        };
        propose_commit(
            raft,
            ship,
            transit.from,
            transit.to,
            transit.entry_pos,
            transit.entry_pos_abs,
            transit.gate_id,
            transit.request_tick,
        );
        node.note_transit_commit_proposed(transit.ship_id, transit.request_tick);
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
                    let request_tick = data.request_tick;
                    node.note_transit_commit_proposed(ship_id, request_tick);
                    propose_commit(
                        raft,
                        *data.ship,
                        node.sector_id(),
                        to,
                        data.entry_pos,
                        data.entry_pos_abs,
                        gate_id,
                        request_tick,
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
                    let ship_present = node.get_ship_position(ship.ship_id).is_some();
                    let completed =
                        destination_completed_transfer(node, ship.ship_id, from, to, request_tick);
                    // A checkpointed destination can retain the materialized Ship while
                    // its incoming Requested/Completed pair has moved to the cold archive.
                    // In that case the Ship itself is the durable dedupe fact: do not append
                    // a fresh Requested marker that replay could misread as a pending source.
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
                        node.handle_transit_commit(&ship, from, entry_pos, entry_pos_abs, gate_id);
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
    use dawn_event_store::InMemoryEventStore;

    fn node(node_id: u8, sector_id: u8) -> SimulationNode {
        SimulationNode::new(
            NodeId(node_id),
            SectorId(sector_id),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        )
    }

    fn mem_node() -> SimulationNode {
        node(0, 0)
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
    fn destination_commit_then_source_ack_moves_ownership_without_a_zero_owner_window() {
        let mut source = node(0, 0);
        let mut destination = node(1, 1);
        let ship_id = source.spawn_ship(
            ShipTypeId(1),
            Position::ORIGIN,
            Velocity::new(1.0, 0.0, 0.0),
        );
        let data = source
            .prepare_transit_commit(ship_id, SectorId(1), None)
            .unwrap();
        let request_tick = source.current_tick();
        let commit = TransitOp::Commit {
            ship: data.ship,
            from: SectorId(0),
            to: SectorId(1),
            entry_pos: data.entry_pos,
            entry_pos_abs: data.entry_pos_abs,
            gate_id: None,
            request_tick,
        };

        let (ack_raft, mut ack_proposals) = raft_handle();
        let (commit_tx, mut commit_rx) = mpsc::unbounded_channel();
        commit_tx.send(commit.encode()).unwrap();
        apply_committed_raft_entries(&mut destination, &ack_raft, &mut commit_rx);

        assert!(source.get_ship_position(ship_id).is_some());
        assert!(destination.get_ship_position(ship_id).is_some());
        let ack = decode_proposed_transit(&mut ack_proposals);
        assert!(matches!(ack, TransitOp::Ack { .. }));

        let (noop_raft, _noop_rx) = raft_handle();
        let (ack_tx, mut ack_rx) = mpsc::unbounded_channel();
        ack_tx.send(ack.encode()).unwrap();
        apply_committed_raft_entries(&mut source, &noop_raft, &mut ack_rx);

        assert!(source.get_ship_position(ship_id).is_none());
        assert!(destination.get_ship_position(ship_id).is_some());
    }

    #[test]
    fn duplicate_destination_commit_is_idempotent_and_reissues_ack() {
        let mut source = node(0, 0);
        let mut destination = node(1, 1);
        let ship_id = source.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let data = source
            .prepare_transit_commit(ship_id, SectorId(1), None)
            .unwrap();
        let commit = TransitOp::Commit {
            ship: data.ship,
            from: SectorId(0),
            to: SectorId(1),
            entry_pos: data.entry_pos,
            entry_pos_abs: data.entry_pos_abs,
            gate_id: None,
            request_tick: source.current_tick(),
        };
        let completed_before = destination
            .event_store()
            .iter_from(0)
            .filter(|record| matches!(record.event, DomainEvent::SectorTransitCompleted(_)))
            .count();
        let (raft, mut proposals) = raft_handle();

        for _ in 0..2 {
            let (tx, mut rx) = mpsc::unbounded_channel();
            tx.send(commit.encode()).unwrap();
            apply_committed_raft_entries(&mut destination, &raft, &mut rx);
            assert!(matches!(
                decode_proposed_transit(&mut proposals),
                TransitOp::Ack { .. }
            ));
        }

        let completed_after = destination
            .event_store()
            .iter_from(0)
            .filter(|record| matches!(record.event, DomainEvent::SectorTransitCompleted(_)))
            .count();
        assert_eq!(destination.ship_count(), 1);
        assert_eq!(completed_after, completed_before + 1);
    }

    #[test]
    fn restored_requested_transit_reproposes_commit_with_the_durable_route() {
        let mut source = node(0, 0);
        let ship_id = source.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let snapshot_before = source.take_snapshot();
        source
            .prepare_transit_commit(ship_id, SectorId(1), None)
            .unwrap();

        let mut store = InMemoryEventStore::new();
        for record in source.event_store().iter_from(0) {
            store.append(record.event.clone());
        }
        let mut restored = SimulationNode::restore_from(store, &snapshot_before, &[], &[]);
        let (raft, mut proposals) = raft_handle();
        let (_tx, mut committed_rx) = mpsc::unbounded_channel();
        apply_committed_raft_entries(&mut restored, &raft, &mut committed_rx);

        match decode_proposed_transit(&mut proposals) {
            TransitOp::Commit {
                ship,
                gate_id,
                entry_pos,
                entry_pos_abs,
                request_tick,
                ..
            } => {
                assert_eq!(ship.ship_id, ship_id);
                assert_eq!(gate_id, None);
                assert_eq!(entry_pos, Position::ORIGIN);
                assert_eq!(entry_pos_abs, AbsolutePosition::ORIGIN);
                assert_eq!(request_tick, Tick::ZERO);
            }
            other => panic!("expected Commit, got {other:?}"),
        }
    }

    #[test]
    fn initial_request_proposes_one_commit_then_waits_for_the_retry_deadline() {
        let mut source = node(0, 0);
        let ship_id = source.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let (raft, mut proposals) = raft_handle();
        let (tx, mut committed_rx) = mpsc::unbounded_channel();
        tx.send(
            TransitOp::Request {
                ship_id,
                to: SectorId(1),
                gate_id: None,
            }
            .encode(),
        )
        .unwrap();
        apply_committed_raft_entries(&mut source, &raft, &mut committed_rx);
        assert!(matches!(
            decode_proposed_transit(&mut proposals),
            TransitOp::Commit { .. }
        ));
        assert!(
            proposals.try_recv().is_err(),
            "initial apply proposed Commit twice"
        );

        let (_empty_tx, mut empty_rx) = mpsc::unbounded_channel();
        for _ in 0..9 {
            source.tick();
            apply_committed_raft_entries(&mut source, &raft, &mut empty_rx);
            assert!(
                proposals.try_recv().is_err(),
                "Commit retried before the ten-Tick deadline"
            );
        }
        source.tick();
        apply_committed_raft_entries(&mut source, &raft, &mut empty_rx);
        assert!(matches!(
            decode_proposed_transit(&mut proposals),
            TransitOp::Commit { .. }
        ));
        assert!(
            proposals.try_recv().is_err(),
            "retry emitted more than one Commit"
        );
    }

    #[test]
    fn destination_marker_keeps_destination_local_tick() {
        let mut destination = node(1, 1);
        let (raft, _proposals) = raft_handle();
        let (tx, mut committed_rx) = mpsc::unbounded_channel();
        tx.send(
            TransitOp::Commit {
                ship: Box::new(sample_ship()),
                from: SectorId(0),
                to: SectorId(1),
                entry_pos: Position::ORIGIN,
                entry_pos_abs: AbsolutePosition::ORIGIN,
                gate_id: None,
                request_tick: Tick(99),
            }
            .encode(),
        )
        .unwrap();
        apply_committed_raft_entries(&mut destination, &raft, &mut committed_rx);

        let marker = destination
            .event_store()
            .iter_from(0)
            .find_map(|record| match &record.event {
                DomainEvent::SectorTransitRequested(event) => Some(event),
                _ => None,
            })
            .expect("destination marker");
        assert_eq!(marker.request_tick, Tick(99));
        assert_eq!(marker.tick, Tick::ZERO);
        assert_eq!(destination.current_tick(), Tick::ZERO);
    }

    #[test]
    fn retry_commit_uses_the_canonical_transit_snapshot_without_tackle_state() {
        let mut source = node(0, 0);
        let ship_id = source.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        source.set_tackled_by_for_test(ship_id, vec![ShipId::new(NodeId(9), 1)]);
        source
            .prepare_transit_commit(ship_id, SectorId(1), None)
            .expect("request must be durable");

        let (raft, mut proposals) = raft_handle();
        let (_tx, mut committed_rx) = mpsc::unbounded_channel();
        apply_committed_raft_entries(&mut source, &raft, &mut committed_rx);

        match decode_proposed_transit(&mut proposals) {
            TransitOp::Commit { ship, .. } => assert!(
                ship.tackled_by.is_empty(),
                "Sector-local tackle state must not cross the boundary on retry"
            ),
            other => panic!("expected Commit, got {other:?}"),
        }
    }

    #[test]
    fn duplicate_commit_after_destination_checkpoint_does_not_append_a_pending_marker() {
        let mut source = node(0, 0);
        let mut destination = node(1, 1);
        let ship_id = source.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let data = source
            .prepare_transit_commit(ship_id, SectorId(1), None)
            .unwrap();
        let commit = TransitOp::Commit {
            ship: data.ship,
            from: SectorId(0),
            to: SectorId(1),
            entry_pos: data.entry_pos,
            entry_pos_abs: data.entry_pos_abs,
            gate_id: None,
            request_tick: data.request_tick,
        };

        let (raft, _proposals) = raft_handle();
        let (tx, mut rx) = mpsc::unbounded_channel();
        tx.send(commit.encode()).unwrap();
        apply_committed_raft_entries(&mut destination, &raft, &mut rx);

        // Snapshot + empty hot tail models a destination checkpoint that
        // compacted the incoming Requested/Completed pair to cold storage.
        let checkpoint = destination.take_snapshot();
        let mut restored =
            SimulationNode::restore_from(InMemoryEventStore::new(), &checkpoint, &[], &[]);
        let (dup_tx, mut dup_rx) = mpsc::unbounded_channel();
        dup_tx.send(commit.encode()).unwrap();
        apply_committed_raft_entries(&mut restored, &raft, &mut dup_rx);

        assert!(restored.can_propose_transit(ship_id));
        assert_eq!(
            restored
                .event_store()
                .iter_from(0)
                .filter(|record| matches!(record.event, DomainEvent::SectorTransitRequested(_)))
                .count(),
            0,
            "an already materialized destination must only reissue Ack"
        );
    }

    #[test]
    fn decode_returns_none_for_garbage_payload() {
        assert!(TransitOp::decode(&[0xFF, 0xFE, 0xFD]).is_none());
    }
}
