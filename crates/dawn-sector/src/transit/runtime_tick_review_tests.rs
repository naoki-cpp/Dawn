use super::*;
use dawn_consensus::RaftActorMessage;
use dawn_core::{NodeId, SectorBounds};
use dawn_event_store::{
    AppendReceipt, DurabilityContext, DurabilityEvidence, DurabilityEvidenceSource, DurabilityMode,
    InMemoryJournal, JournalBatch, JournalError, JournalIndex, JournalRange, JournalRecord,
    JournalStream, TransitionId,
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
            profile: RuntimeDurabilityProfile::LocalDurable,
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
fn local_runtime_uses_the_same_durable_frame_boundary() {
    let mut node = mem_node();
    let _ = node.drain_pending_events();
    let mut journal = InMemoryJournal::new();
    let mut consensus = LocalRuntimeConsensus;
    let mut hook_called = false;

    let output = run_durable_runtime_tick_with_consensus(
        &mut node,
        &mut journal,
        &mut consensus,
        &[],
        DurableRuntimeTickContext {
            transition_id: crate::transition::SectorTransitionId(102),
            owner_epoch: 0,
            durability: DurabilityMode::Synced,
            profile: RuntimeDurabilityProfile::LocalDurable,
        },
        |_, _, _| hook_called = true,
    )
    .expect("local runtime Tick should use the shared durable boundary");

    assert!(hook_called);
    assert_eq!(output.tick_result.tick, dawn_core::Tick(1));
    assert_eq!(journal.records().len(), 1);
    assert_eq!(journal.records()[0].stream, JournalStream::RecoveryDelta);
}

#[test]
fn runtime_rejects_unavailable_durability_profiles_before_mutation() {
    let mut node = mem_node();
    let mut journal = InMemoryJournal::new();
    let mut consensus = LocalRuntimeConsensus;

    let buffered = run_durable_runtime_tick_with_consensus(
        &mut node,
        &mut journal,
        &mut consensus,
        &[],
        DurableRuntimeTickContext {
            transition_id: crate::transition::SectorTransitionId(103),
            owner_epoch: 0,
            durability: DurabilityMode::Buffered,
            profile: RuntimeDurabilityProfile::LocalDurable,
        },
        |_, _, _| panic!("invalid local durability must not publish"),
    );
    assert!(matches!(
        buffered,
        Err(TickTransitionError::Policy(
            RuntimeDurabilityPolicyError::LocalDurableRequiresSync
        ))
    ));

    let replicated = run_durable_runtime_tick_with_consensus(
        &mut node,
        &mut journal,
        &mut consensus,
        &[],
        DurableRuntimeTickContext {
            transition_id: crate::transition::SectorTransitionId(104),
            owner_epoch: 0,
            durability: DurabilityMode::Synced,
            profile: RuntimeDurabilityProfile::ReplicatedDurable,
        },
        |_, _, _| panic!("unavailable replication must not publish"),
    );
    assert!(matches!(
        replicated,
        Err(TickTransitionError::Policy(
            RuntimeDurabilityPolicyError::ReplicatedDurableUnavailable
        ))
    ));
    assert_eq!(node.current_tick(), dawn_core::Tick::ZERO);
    assert!(journal.records().is_empty());
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

    let result = run_durable_runtime_tick_with_policy(
        &mut node,
        &mut journal,
        &mut consensus,
        &RejectingPolicy,
        &[],
        DurableRuntimeTickContext {
            transition_id: crate::transition::SectorTransitionId(105),
            owner_epoch: 4,
            durability: DurabilityMode::Synced,
            profile: RuntimeDurabilityProfile::ReplicatedDurable,
        },
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

    let result = run_durable_runtime_tick_with_policy_and_reconciliation(
        &mut node,
        &mut journal,
        &mut consensus,
        &LocalRuntimeDurabilityPolicy,
        &[],
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
fn post_append_failure_fences_a_long_lived_runtime_until_recovery() {
    let mut node = mem_node();
    let mut journal = InMemoryJournal::new();
    let mut consensus = LocalRuntimeConsensus;
    let mut health = RuntimeHealth::new();

    let first = run_durable_runtime_tick_with_policy_and_reconciliation_and_health(
        &mut node,
        &mut journal,
        &mut consensus,
        &LocalRuntimeDurabilityPolicy,
        &mut health,
        &[],
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

    let blocked = run_durable_runtime_tick_with_policy_and_reconciliation_and_health(
        &mut node,
        &mut journal,
        &mut consensus,
        &LocalRuntimeDurabilityPolicy,
        &mut health,
        &[],
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
    let recovered = run_durable_runtime_tick_with_policy_and_reconciliation_and_health(
        &mut node,
        &mut journal,
        &mut consensus,
        &LocalRuntimeDurabilityPolicy,
        &mut health,
        &[],
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
            profile: RuntimeDurabilityProfile::LocalDurable,
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
