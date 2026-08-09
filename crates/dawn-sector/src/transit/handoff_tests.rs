use super::*;
use dawn_core::fitting::FittingSnapshot;
use dawn_core::{NodeId, Position, SectorBounds, ShipTypeId, Velocity};

fn node(node_id: u8, sector_id: u8) -> SimulationNode {
    SimulationNode::new_test(
        NodeId(node_id),
        SectorId(sector_id),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
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

fn commit_with(handoff: TransitHandoffState) -> TransitOp {
    TransitOp::Commit {
        attempt_id: TransitAttemptId::new(SectorId(0), handoff.ship_id, 12),
        handoff: Box::new(handoff),
        from: SectorId(0),
        to: SectorId(1),
        entry_pos: AbsolutePosition::ORIGIN,
        gate_id: None,
        request_tick: Tick(12),
    }
}

#[test]
fn retry_commit_recreates_the_complete_canonical_handoff() {
    let mut source = node(0, 0);
    let player_id = source.next_player_id();
    let ship_id = source.spawn_player_ship_at_pub(player_id, Position::ORIGIN);
    source.apply_move_command(ship_id, Position::new(1_000.0, 250.0, -100.0));
    source.tick();

    let data = source
        .prepare_transit_commit(ship_id, SectorId(1), None)
        .expect("request must be durable");
    let expected = (*data.handoff).clone();

    let fitted_count = expected.fitting.high.len()
        + expected.fitting.mid.len()
        + expected.fitting.low.len()
        + expected.fitting.rig.len();
    assert!(
        fitted_count > 0,
        "fixture must carry non-empty fitting state"
    );
    assert!(
        !expected.inventory.is_empty(),
        "fixture must carry non-empty inventory state"
    );
    assert!(
        expected.velocity != Velocity::ZERO,
        "fixture must carry non-default velocity state"
    );
    assert!(
        expected.current_shield > 0.0
            && expected.current_armor > 0.0
            && expected.current_hull > 0.0,
        "fixture must carry non-default HP state"
    );
    assert!(
        expected.capacitor.is_some_and(|value| value > 0.0),
        "fixture must carry non-default capacitor state"
    );

    let (raft, mut proposals) = raft_handle();
    let (_tx, mut committed_rx) = mpsc::unbounded_channel();
    apply_committed_raft_entries(&mut source, &raft, &mut committed_rx);

    match decode_proposed_transit(&mut proposals) {
        TransitOp::Commit {
            handoff,
            request_tick,
            ..
        } => {
            assert_eq!(*handoff, expected);
            assert_eq!(request_tick, data.request_tick);
        }
        other => panic!("expected Commit, got {other:?}"),
    }
}

#[test]
fn transit_decode_rejects_destroyed_state_that_disagrees_with_hull_hp() {
    let consistent = commit_with(sample_handoff()).encode();
    assert!(TransitOp::decode(&consistent).is_some());

    let mut inconsistent = sample_handoff();
    inconsistent.is_destroyed = true;
    let inconsistent = commit_with(inconsistent).encode();
    assert!(
        TransitOp::decode(&inconsistent).is_none(),
        "a positive hull with is_destroyed=true must be rejected"
    );

    let mut destroyed = sample_handoff();
    destroyed.current_hull = 0.0;
    destroyed.is_destroyed = true;
    let destroyed = commit_with(destroyed).encode();
    assert!(TransitOp::decode(&destroyed).is_some());
}
