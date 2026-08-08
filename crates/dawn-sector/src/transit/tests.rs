use super::*;
use crate::client_admission::{ClientAdmissionIntent, ClientAdmissionRefusal};
use dawn_core::fitting::FittingSnapshot;
use dawn_core::{NodeId, Position, SectorBounds, ShipTypeId, Velocity};
use dawn_event_store::{EventStore, InMemoryEventStore};

fn node(node_id: u8, sector_id: u8) -> SimulationNode {
    SimulationNode::new_test(
        NodeId(node_id),
        SectorId(sector_id),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
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

fn sample_handoff() -> TransitHandoffState {
    TransitHandoffState {
        ship_id: ShipId::new(NodeId(0), 7),
        owner_player_id: None,
        resume_ticket: None,
        pending_resume_ticket: None,
        ship_type_id: ShipTypeId(1),
        velocity: Velocity::new(4.0, 5.0, 6.0),
        current_shield: 10.0,
        current_armor: 20.0,
        current_hull: 30.0,
        is_destroyed: false,
        capacitor: Some(50.0),
        fitting: FittingSnapshot::empty(),
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
        handoff: Box::new(sample_handoff()),
        from: SectorId(0),
        to: SectorId(1),
        entry_pos: AbsolutePosition::new(500.0, 0.0, 0.0),
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
        ship_id: sample_handoff().ship_id,
        from: SectorId(0),
        to: SectorId(1),
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
        handoff: data.handoff,
        from: SectorId(0),
        to: SectorId(1),
        entry_pos: data.entry_pos,
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
fn transit_carries_owner_binding_to_destination_and_snapshot_restore() {
    let mut source = node(0, 0);
    let mut destination = node(1, 1);
    let player_id = source.next_player_id();
    let ship_id = source.spawn_player_ship(player_id);
    let data = source
        .prepare_transit_commit(ship_id, SectorId(1), None)
        .expect("owned Ship transit");
    assert_eq!(data.handoff.owner_player_id, Some(player_id));
    let resume_ticket = data.handoff.resume_ticket.expect("owned Ship ticket");

    let (raft, mut proposals) = raft_handle();
    let (commit_tx, mut commit_rx) = mpsc::unbounded_channel();
    commit_tx
        .send(
            (TransitOp::Commit {
                handoff: data.handoff,
                from: SectorId(0),
                to: SectorId(1),
                entry_pos: data.entry_pos,
                gate_id: None,
                request_tick: data.request_tick,
            })
            .encode(),
        )
        .unwrap();
    apply_committed_raft_entries(&mut destination, &raft, &mut commit_rx);
    assert!(matches!(
        decode_proposed_transit(&mut proposals),
        TransitOp::Ack { .. }
    ));

    assert!(matches!(
        destination.begin_client_admission(
            ClientAdmissionIntent::Resume {
                resume_ticket: dawn_core::ResumeTicket::from_bytes([99; 32]),
            },
            1_000.0,
        ),
        Err(ClientAdmissionRefusal::ResumeTicketInvalid)
    ));

    let reconnect = destination
        .begin_client_admission(ClientAdmissionIntent::Resume { resume_ticket }, 1_000.0)
        .expect("transit ticket should identify the owner");
    reconnect.abort(&mut destination);

    let snapshot = destination.take_snapshot();
    let mut store = InMemoryEventStore::new();
    for event in destination.pending_events() {
        store.append(event.clone());
    }
    let mut restored = SimulationNode::restore_from_test(
        store,
        &snapshot,
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        &[],
        &[],
    );
    assert!(matches!(
        restored.begin_client_admission(
            ClientAdmissionIntent::Resume {
                resume_ticket: dawn_core::ResumeTicket::from_bytes([99; 32]),
            },
            1_000.0,
        ),
        Err(ClientAdmissionRefusal::ResumeTicketInvalid)
    ));
}

#[test]
fn transit_preserves_a_pending_resume_ticket_for_the_destination() {
    let mut source = node(0, 0);
    let mut destination = node(1, 1);
    let player_id = source.next_player_id();
    let ship_id = source.spawn_player_ship(player_id);
    let current_ticket = source
        .client_resume_ticket(ship_id)
        .expect("player Ship has a current ticket");
    let pending_attempt = source
        .begin_client_admission(
            ClientAdmissionIntent::Resume {
                resume_ticket: current_ticket,
            },
            1_000.0,
        )
        .expect("resume attempt should stage a next ticket");
    let pending_ticket = pending_attempt.resume_ticket();

    let data = source
        .prepare_transit_commit(ship_id, SectorId(1), None)
        .expect("owned Ship transit");
    assert_eq!(data.handoff.resume_ticket, Some(current_ticket));
    assert_eq!(data.handoff.pending_resume_ticket, Some(pending_ticket));

    let (raft, mut proposals) = raft_handle();
    let (commit_tx, mut commit_rx) = mpsc::unbounded_channel();
    commit_tx
        .send(
            (TransitOp::Commit {
                handoff: data.handoff,
                from: SectorId(0),
                to: SectorId(1),
                entry_pos: data.entry_pos,
                gate_id: None,
                request_tick: data.request_tick,
            })
            .encode(),
        )
        .unwrap();
    apply_committed_raft_entries(&mut destination, &raft, &mut commit_rx);
    assert!(matches!(
        decode_proposed_transit(&mut proposals),
        TransitOp::Ack { .. }
    ));

    pending_attempt.abort(&mut source);
    let reconnect = destination
        .begin_client_admission(
            ClientAdmissionIntent::Resume {
                resume_ticket: pending_ticket,
            },
            1_000.0,
        )
        .expect("the advertised ticket must survive Transit");
    reconnect.abort(&mut destination);
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
        handoff: data.handoff,
        from: SectorId(0),
        to: SectorId(1),
        entry_pos: data.entry_pos,
        gate_id: None,
        request_tick: source.current_tick(),
    };
    let completed_before = destination
        .pending_events()
        .iter()
        .filter(|event| matches!(event, DomainEvent::SectorTransitCompleted(_)))
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
        .pending_events()
        .iter()
        .filter(|event| matches!(event, DomainEvent::SectorTransitCompleted(_)))
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
    for event in source.pending_events() {
        store.append(event.clone());
    }
    let mut restored = SimulationNode::restore_from_test(
        store,
        &snapshot_before,
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        &[],
        &[],
    );
    let (raft, mut proposals) = raft_handle();
    let (_tx, mut committed_rx) = mpsc::unbounded_channel();
    apply_committed_raft_entries(&mut restored, &raft, &mut committed_rx);

    match decode_proposed_transit(&mut proposals) {
        TransitOp::Commit {
            handoff,
            gate_id,
            entry_pos,
            request_tick,
            ..
        } => {
            assert_eq!(handoff.ship_id, ship_id);
            assert_eq!(gate_id, None);
            assert_eq!(entry_pos, AbsolutePosition::ORIGIN);
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
            handoff: Box::new(sample_handoff()),
            from: SectorId(0),
            to: SectorId(1),
            entry_pos: AbsolutePosition::ORIGIN,
            gate_id: None,
            request_tick: Tick(99),
        }
        .encode(),
    )
    .unwrap();
    apply_committed_raft_entries(&mut destination, &raft, &mut committed_rx);

    let marker = destination
        .pending_events()
        .iter()
        .find_map(|event| match event {
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
        TransitOp::Commit { handoff, .. } => assert!(
            handoff.ship_id == ship_id,
            "retry handoff must preserve the canonical Ship identity"
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
        handoff: data.handoff,
        from: SectorId(0),
        to: SectorId(1),
        entry_pos: data.entry_pos,
        gate_id: None,
        request_tick: data.request_tick,
    };

    let (raft, _proposals) = raft_handle();
    let (tx, mut rx) = mpsc::unbounded_channel();
    tx.send(commit.encode()).unwrap();
    apply_committed_raft_entries(&mut destination, &raft, &mut rx);

    let checkpoint = destination.take_snapshot();
    let mut restored = SimulationNode::restore_from_test(
        InMemoryEventStore::new(),
        &checkpoint,
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        &[],
        &[],
    );
    let (dup_tx, mut dup_rx) = mpsc::unbounded_channel();
    dup_tx.send(commit.encode()).unwrap();
    apply_committed_raft_entries(&mut restored, &raft, &mut dup_rx);

    assert!(restored.can_propose_transit(ship_id));
    assert_eq!(
        restored
            .pending_events()
            .iter()
            .filter(|event| matches!(event, DomainEvent::SectorTransitRequested(_)))
            .count(),
        0,
        "an already materialized destination must only reissue Ack"
    );
}

#[test]
fn duplicate_commit_after_checkpoint_does_not_resurrect_removed_ship() {
    let mut source = node(0, 0);
    let mut destination = node(1, 1);
    let ship_id = source.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
    let data = source
        .prepare_transit_commit(ship_id, SectorId(1), None)
        .unwrap();
    let commit = TransitOp::Commit {
        handoff: data.handoff,
        from: SectorId(0),
        to: SectorId(1),
        entry_pos: data.entry_pos,
        gate_id: None,
        request_tick: data.request_tick,
    };

    let (raft, mut proposals) = raft_handle();
    let (tx, mut rx) = mpsc::unbounded_channel();
    tx.send(commit.encode()).unwrap();
    apply_committed_raft_entries(&mut destination, &raft, &mut rx);
    assert!(matches!(
        decode_proposed_transit(&mut proposals),
        TransitOp::Ack { .. }
    ));

    let mut checkpoint = destination.take_snapshot();
    checkpoint.ships.retain(|ship| ship.ship_id != ship_id);
    let mut restored = SimulationNode::restore_from_test(
        InMemoryEventStore::new(),
        &checkpoint,
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        &[],
        &[],
    );
    assert!(restored.get_ship_position(ship_id).is_none());

    let (dup_tx, mut dup_rx) = mpsc::unbounded_channel();
    dup_tx.send(commit.encode()).unwrap();
    apply_committed_raft_entries(&mut restored, &raft, &mut dup_rx);

    assert!(matches!(
        decode_proposed_transit(&mut proposals),
        TransitOp::Ack { .. }
    ));
    assert!(restored.get_ship_position(ship_id).is_none());
    assert!(restored.pending_events().is_empty());
}

#[test]
fn decode_returns_none_for_garbage_payload() {
    assert!(TransitOp::decode(&[0xFF, 0xFE, 0xFD]).is_none());
}

#[test]
fn runtime_tick_owns_replication_raft_and_transient_order() {
    let mut node = mem_node();
    let gate = *node.jump_gate(JumpGateId(0)).expect("Sector 0 has Gate 0");
    let near_gate_abs = [
        gate.abs_m[0] - (gate.activation_radius * 0.5),
        gate.abs_m[1],
        gate.abs_m[2],
    ];
    let player_id = node.next_player_id();
    let ship = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);
    node.set_spawn_anchor_abs(ship, near_gate_abs);
    node.queue_runtime_transients_for_test((ship, JumpGateId(0)), ship);
    // The spawn belongs to setup, not to this runtime frame. Clear its
    // explicit output just as the production runtime does between frames.
    let _ = node.drain_pending_events();

    let (raft, mut raft_messages) = raft_handle();
    let (_committed_tx, mut committed_rx) = mpsc::unbounded_channel();
    let mut replication_hook_called = false;

    let output = run_runtime_tick(
        &mut node,
        &raft,
        &mut committed_rx,
        &[],
        |_, tick_result, events| {
            replication_hook_called = true;
            assert!(
                raft_messages.try_recv().is_err(),
                "replication hook must run before raft.tick()"
            );
            assert_eq!(
                events,
                tick_result.events.as_slice(),
                "hook receives the explicit transition output"
            );
        },
    );

    assert!(replication_hook_called);
    assert_eq!(output.pending_auto_jumps, vec![(ship, JumpGateId(0))]);
    assert_eq!(output.completed_warps, vec![ship]);
    assert!(node.drain_pending_auto_jumps().is_empty());
    assert!(node.drain_completed_warps().is_empty());

    assert!(matches!(
        raft_messages.try_recv(),
        Ok(dawn_consensus::RaftActorMessage::TickElapsed)
    ));
    assert!(matches!(
        decode_proposed_transit(&mut raft_messages),
        TransitOp::Request {
            ship_id,
            gate_id: Some(JumpGateId(0)),
            ..
        } if ship_id == ship
    ));
}
