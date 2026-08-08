use super::*;
use dawn_consensus::RaftActorMessage;
use dawn_core::{NodeId, SectorBounds};
use dawn_event_store::{AppendReceipt, JournalBatch, JournalError, JournalIndex, JournalRecord};
use dawn_event_store::{DurabilityMode, InMemoryJournal, JournalStream};

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

#[test]
fn durable_runtime_tick_persists_before_publishing_the_frame() {
    let mut node = mem_node();
    node.spawn_ship(
        dawn_core::ShipTypeId(1),
        dawn_core::Position::new(10.0, 20.0, 30.0),
        dawn_core::Velocity::new(2.0, 0.0, 0.0),
    );
    let _ = node.drain_pending_events();
    let (raft_tx, mut raft_messages) = mpsc::unbounded_channel();
    let raft = RaftActorHandle::new(raft_tx);
    let (_committed_tx, mut committed_rx) = mpsc::unbounded_channel();
    let mut journal = InMemoryJournal::new();
    let mut hook_called = false;

    let output = run_durable_runtime_tick(
        &mut node,
        &mut journal,
        &raft,
        &mut committed_rx,
        &[],
        DurableRuntimeTickContext {
            transition_id: crate::transition::SectorTransitionId(100),
            owner_epoch: 7,
            durability: DurabilityMode::Synced,
        },
        |node, result, events| {
            hook_called = true;
            assert_eq!(node.current_tick(), result.tick);
            assert_eq!(events, result.events.as_slice());
        },
    )
    .expect("durable runtime Tick should succeed");

    assert!(hook_called);
    assert_eq!(output.tick_result.tick, dawn_core::Tick(1));
    assert_eq!(node.current_tick(), dawn_core::Tick(1));
    assert_eq!(journal.records().len(), 1);
    assert_eq!(journal.records()[0].stream, JournalStream::RecoveryDelta);
    assert!(matches!(
        raft_messages.try_recv(),
        Ok(RaftActorMessage::TickElapsed)
    ));
}

#[test]
fn durable_runtime_tick_restores_pending_output_when_append_fails() {
    let mut node = mem_node();
    node.spawn_ship(
        dawn_core::ShipTypeId(1),
        dawn_core::Position::ORIGIN,
        dawn_core::Velocity::ZERO,
    );
    let expected = node.pending_events().to_vec();
    let (raft_tx, _raft_messages) = mpsc::unbounded_channel();
    let raft = RaftActorHandle::new(raft_tx);
    let (_committed_tx, mut committed_rx) = mpsc::unbounded_channel();
    let mut journal = FailingJournal;

    let result = run_durable_runtime_tick(
        &mut node,
        &mut journal,
        &raft,
        &mut committed_rx,
        &[],
        DurableRuntimeTickContext {
            transition_id: crate::transition::SectorTransitionId(101),
            owner_epoch: 7,
            durability: DurabilityMode::Synced,
        },
        |_, _, _| panic!("failed append must not publish a frame"),
    );

    assert!(result.is_err());
    assert_eq!(node.current_tick(), dawn_core::Tick::ZERO);
    assert_eq!(node.pending_events(), expected.as_slice());
}

struct FailingJournal;

impl dawn_event_store::DurableJournal for FailingJournal {
    fn append_batch(&mut self, _batch: JournalBatch) -> Result<AppendReceipt, JournalError> {
        Err(JournalError::Io(std::io::Error::other(
            "injected append failure",
        )))
    }

    fn read_from(
        &self,
        _index: JournalIndex,
    ) -> Result<Box<dyn Iterator<Item = Result<JournalRecord, JournalError>> + '_>, JournalError>
    {
        Ok(Box::new(std::iter::empty()))
    }

    fn next_index(&self) -> Result<JournalIndex, JournalError> {
        Ok(JournalIndex::ZERO)
    }
}
