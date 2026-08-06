use super::*;
use dawn_consensus::RaftActorMessage;
use dawn_core::{NodeId, SectorBounds};

fn mem_node() -> SimulationNode {
    SimulationNode::new_test(
        NodeId(0),
        SectorId(0),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
    )
}

#[test]
fn rejected_auto_jump_is_drained_without_being_reported_as_proposed() {
    let mut node = mem_node();
    let missing_ship = ShipId::new(NodeId(0), 999);
    node.queue_runtime_transients_for_test((missing_ship, JumpGateId(0)), missing_ship);

    let (raft_tx, mut raft_messages) = mpsc::unbounded_channel();
    let raft = RaftActorHandle::new(raft_tx);
    let (_committed_tx, mut committed_rx) = mpsc::unbounded_channel();

    let output = run_runtime_tick(&mut node, &raft, &mut committed_rx, &[], |_, _, _| {});

    assert!(
        output.pending_auto_jumps.is_empty(),
        "a rejected one-shot trigger must not be reported as a Raft proposal"
    );
    assert_eq!(output.completed_warps, vec![missing_ship]);
    assert!(node.drain_pending_auto_jumps().is_empty());
    assert!(node.drain_completed_warps().is_empty());

    assert!(matches!(
        raft_messages.try_recv(),
        Ok(RaftActorMessage::TickElapsed)
    ));
    assert!(
        raft_messages.try_recv().is_err(),
        "rejected auto-jump must not emit a Propose message"
    );
}
