use super::*;
use dawn_core::{NodeId, SectorBounds};
use dawn_distributed::RaftActorMessage;
use dawn_storage::{
    AppendReceipt, DurabilityContext, DurabilityEvidence, DurabilityEvidenceSource, DurabilityMode,
    InMemoryJournal, JournalIndex, JournalRange, TransitionId,
};

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
    let mut journal = InMemoryJournal::new();
    let mut consensus = RaftRuntimeConsensus::new(&raft, &mut committed_rx);
    let mut health = RuntimeHealth::new();

    let output = run_durable_runtime_frame(
        &mut node,
        &mut journal,
        &mut consensus,
        &LocalRuntimeDurabilityPolicy,
        &mut health,
        crate::transition::FrameInput::lock_only(&[]),
        DurableRuntimeTickContext {
            transition_id: crate::transition::SectorTransitionId(110),
            owner_epoch: 0,
            durability: DurabilityMode::Synced,
            profile: RuntimeDurabilityProfile::LocalDurable,
        },
        reconcile_runtime_repositories,
        |_, _, _| {},
    )
    .expect("durable runtime frame should succeed");

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
fn replicated_policy_accepts_only_matching_distinct_replica_receipts() {
    let local_receipt = AppendReceipt {
        transition_id: TransitionId(22),
        context: DurabilityContext {
            sector_id: SectorId(0),
            owner_epoch: 9,
        },
        range: JournalRange {
            first: JournalIndex(12),
            len: 1,
        },
        content_hash: 0xfeed,
        durability: DurabilityMode::Synced,
    };
    let matching_remote = RuntimeReplicaReceipt {
        replica_id: NodeId(1),
        evidence: DurabilityEvidence {
            receipt: local_receipt,
            source: DurabilityEvidenceSource::Remote,
        },
    };
    let policy = ReplicatedRuntimeDurabilityPolicy::new(
        NodeId(0),
        vec![NodeId(0), NodeId(1), NodeId(2)],
        2,
        vec![matching_remote],
    )
    .expect("valid quorum policy");

    policy
        .validate(
            RuntimeDurabilityProfile::ReplicatedDurable,
            DurabilityMode::Synced,
        )
        .expect("replicated profile should validate");
    policy
        .validate_receipt(RuntimeDurabilityProfile::ReplicatedDurable, &local_receipt)
        .expect("local plus one matching remote should satisfy quorum");

    let stale = RuntimeReplicaReceipt {
        replica_id: NodeId(2),
        evidence: DurabilityEvidence {
            receipt: AppendReceipt {
                context: DurabilityContext {
                    sector_id: SectorId(0),
                    owner_epoch: 8,
                },
                ..local_receipt
            },
            source: DurabilityEvidenceSource::Remote,
        },
    };
    let stale_policy = ReplicatedRuntimeDurabilityPolicy::new(
        NodeId(0),
        vec![NodeId(0), NodeId(1), NodeId(2)],
        2,
        vec![stale],
    )
    .expect("valid quorum policy");
    assert!(matches!(
        stale_policy.validate_receipt(RuntimeDurabilityProfile::ReplicatedDurable, &local_receipt),
        Err(RuntimeDurabilityPolicyError::ReceiptMismatch {
            replica_id: NodeId(2)
        })
    ));

    let missing_policy = ReplicatedRuntimeDurabilityPolicy::new(
        NodeId(0),
        vec![NodeId(0), NodeId(1)],
        2,
        Vec::new(),
    )
    .expect("valid quorum policy");
    assert!(matches!(
        missing_policy
            .validate_receipt(RuntimeDurabilityProfile::ReplicatedDurable, &local_receipt),
        Err(RuntimeDurabilityPolicyError::QuorumNotReached {
            matched: 1,
            required: 2
        })
    ));
}

#[test]
fn failed_replicated_policy_does_not_apply_or_publish_after_local_append() {
    struct RejectingPolicy;

    impl RuntimeDurabilityPolicy for RejectingPolicy {
        fn validate(
            &self,
            _profile: RuntimeDurabilityProfile,
            _durability: DurabilityMode,
        ) -> Result<(), RuntimeDurabilityPolicyError> {
            Ok(())
        }

        fn validate_receipt(
            &self,
            _profile: RuntimeDurabilityProfile,
            _local_receipt: &AppendReceipt,
        ) -> Result<(), RuntimeDurabilityPolicyError> {
            Err(RuntimeDurabilityPolicyError::QuorumNotReached {
                matched: 1,
                required: 2,
            })
        }
    }

    let mut node = mem_node();
    let mut journal = InMemoryJournal::new();
    let mut consensus = LocalRuntimeConsensus;
    let mut hook_called = false;
    let mut health = RuntimeHealth::new();

    let result = run_durable_runtime_frame(
        &mut node,
        &mut journal,
        &mut consensus,
        &RejectingPolicy,
        &mut health,
        crate::transition::FrameInput::lock_only(&[]),
        DurableRuntimeTickContext {
            transition_id: crate::transition::SectorTransitionId(105),
            owner_epoch: 4,
            durability: DurabilityMode::Synced,
            profile: RuntimeDurabilityProfile::ReplicatedDurable,
        },
        reconcile_runtime_repositories,
        |_, _, _| hook_called = true,
    );

    assert!(matches!(
        result,
        Err(TickTransitionError::Policy(
            RuntimeDurabilityPolicyError::QuorumNotReached {
                matched: 1,
                required: 2
            }
        ))
    ));
    assert!(!hook_called);
    assert_eq!(node.current_tick(), dawn_core::Tick::ZERO);
    assert_eq!(
        journal.records().len(),
        1,
        "local evidence remains recoverable"
    );
}

#[test]
fn failed_reconciliation_stops_publication_after_live_apply() {
    let mut node = mem_node();
    let mut journal = InMemoryJournal::new();
    let mut consensus = LocalRuntimeConsensus;
    let mut hook_called = false;
    let mut health = RuntimeHealth::new();

    let result = run_durable_runtime_frame(
        &mut node,
        &mut journal,
        &mut consensus,
        &LocalRuntimeDurabilityPolicy,
        &mut health,
        crate::transition::FrameInput::lock_only(&[]),
        DurableRuntimeTickContext {
            transition_id: crate::transition::SectorTransitionId(106),
            owner_epoch: 4,
            durability: DurabilityMode::Synced,
            profile: RuntimeDurabilityProfile::LocalDurable,
        },
        |_, _, _| {
            Err(RuntimeReconciliationError::Projection {
                reason: "injected projection failure".to_string(),
            })
        },
        |_, _, _| hook_called = true,
    );

    assert!(matches!(
        result,
        Err(TickTransitionError::Reconciliation(
            RuntimeReconciliationError::Projection { .. }
        ))
    ));
    assert!(!hook_called);
    assert_eq!(node.current_tick(), dawn_core::Tick(1));
    assert_eq!(journal.records().len(), 1);
}

#[test]
fn station_projection_gap_fences_before_publication() {
    let mut node = mem_node();
    node.apply_station_projection(
        "projection-ahead-of-journal",
        JournalRange {
            first: JournalIndex::ZERO,
            len: 1,
        },
        &[],
    )
    .expect("test setup should advance the projection cursor");
    let mut journal = InMemoryJournal::new();
    let mut consensus = LocalRuntimeConsensus;
    let mut hook_called = false;
    let mut health = RuntimeHealth::new();

    let result = run_durable_runtime_frame(
        &mut node,
        &mut journal,
        &mut consensus,
        &LocalRuntimeDurabilityPolicy,
        &mut health,
        crate::transition::FrameInput::lock_only(&[]),
        DurableRuntimeTickContext {
            transition_id: crate::transition::SectorTransitionId(206),
            owner_epoch: 4,
            durability: DurabilityMode::Synced,
            profile: RuntimeDurabilityProfile::LocalDurable,
        },
        reconcile_runtime_repositories,
        |_, _, _| hook_called = true,
    );

    assert!(matches!(
        result,
        Err(TickTransitionError::Reconciliation(
            RuntimeReconciliationError::Projection { .. }
        ))
    ));
    assert!(health.is_fenced());
    assert!(!hook_called);
    assert_eq!(node.current_tick(), dawn_core::Tick(1));
    assert_eq!(journal.records().len(), 1);
}

#[test]
fn post_append_failure_fences_a_long_lived_runtime_until_recovery() {
    let mut node = mem_node();
    let mut journal = InMemoryJournal::new();
    let mut consensus = LocalRuntimeConsensus;
    let mut health = RuntimeHealth::new();

    let first = run_durable_runtime_frame(
        &mut node,
        &mut journal,
        &mut consensus,
        &LocalRuntimeDurabilityPolicy,
        &mut health,
        crate::transition::FrameInput::lock_only(&[]),
        DurableRuntimeTickContext {
            transition_id: crate::transition::SectorTransitionId(107),
            owner_epoch: 4,
            durability: DurabilityMode::Synced,
            profile: RuntimeDurabilityProfile::LocalDurable,
        },
        |_, _, _| {
            Err(RuntimeReconciliationError::Projection {
                reason: "injected projection failure".to_string(),
            })
        },
        |_, _, _| {},
    );
    assert!(matches!(
        first,
        Err(TickTransitionError::Reconciliation(
            RuntimeReconciliationError::Projection { .. }
        ))
    ));
    assert!(health.is_fenced());

    let blocked = run_durable_runtime_frame(
        &mut node,
        &mut journal,
        &mut consensus,
        &LocalRuntimeDurabilityPolicy,
        &mut health,
        crate::transition::FrameInput::lock_only(&[]),
        DurableRuntimeTickContext {
            transition_id: crate::transition::SectorTransitionId(108),
            owner_epoch: 4,
            durability: DurabilityMode::Synced,
            profile: RuntimeDurabilityProfile::LocalDurable,
        },
        |_, _, _| panic!("a fenced runtime must not reconcile"),
        |_, _, _| panic!("a fenced runtime must not publish"),
    );
    assert!(matches!(
        blocked,
        Err(TickTransitionError::Reconciliation(
            RuntimeReconciliationError::Fenced { .. }
        ))
    ));
    assert_eq!(journal.records().len(), 1);

    health.mark_recovered();
    let recovered = run_durable_runtime_frame(
        &mut node,
        &mut journal,
        &mut consensus,
        &LocalRuntimeDurabilityPolicy,
        &mut health,
        crate::transition::FrameInput::lock_only(&[]),
        DurableRuntimeTickContext {
            transition_id: crate::transition::SectorTransitionId(109),
            owner_epoch: 4,
            durability: DurabilityMode::Synced,
            profile: RuntimeDurabilityProfile::LocalDurable,
        },
        |_, _, _| Ok(()),
        |_, _, _| {},
    )
    .expect("explicit recovery should reopen the runtime gate");
    assert_eq!(recovered.tick_result.tick, dawn_core::Tick(2));
    assert!(!health.is_fenced());
}
