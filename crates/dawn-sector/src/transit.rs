//! Sector runtime adapters (ADR-0014 / ADR-0049).
//!
//! The authoritative handoff state machine lives in the internal `handoff`
//! module. The pipeline consumes the node's observed transit journal and
//! translates explicit effects to Raft proposals. The low-level ECS lifecycle
//! operations remain crate-private behind that policy. This module also owns
//! the runtime-side durable Stop/Tick transition functions so the
//! storage-independent `SimulationNode` only prepares and applies state.

pub(crate) mod handoff;
pub(crate) mod pipeline;

use crate::node::SimulationNode;
use dawn_core::{
    AbsolutePosition, DomainEvent, JumpGateId, NodeId, SectorId, ShipId, Tick, TransitAttemptId,
    TransitHandoffState,
};
use dawn_distributed::RaftActorHandle;
use dawn_storage::{
    AppendReceipt, DurabilityEvidence, DurabilityEvidenceSource, DurabilityMode, DurableJournal,
    JournalError,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use thiserror::Error;
use tokio::sync::mpsc;

/// Failure returned by the Stop prepare -> durable -> live-apply boundary.
#[derive(Debug, Error)]
pub enum StopTransitionError {
    #[error(transparent)]
    Preparation(#[from] crate::transition::TransitionError),
    #[error("durable transition append failed: {0}")]
    Durable(#[from] JournalError),
    #[error("prepared Stop cannot be applied to the current state: {0}")]
    Validation(#[from] crate::transition::TransitionApplyError),
}

/// Failure returned by the Tick prepare -> durable -> live-apply boundary.
#[derive(Debug, Error)]
pub enum TickTransitionError {
    #[error(transparent)]
    Preparation(#[from] crate::node::TickPreparationError),
    #[error("durable transition append failed: {0}")]
    Durable(#[from] JournalError),
    #[error("prepared Tick cannot be applied to the current state: {0}")]
    Validation(#[from] crate::transition::TransitionApplyError),
    #[error(transparent)]
    Policy(#[from] RuntimeDurabilityPolicyError),
    #[error(transparent)]
    Reconciliation(#[from] RuntimeReconciliationError),
}

/// Failure while bringing required local repositories/projections to the
/// committed transition before presentation or acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RuntimeReconciliationError {
    #[error("required projection reconciliation failed: {reason}")]
    Projection { reason: String },
    #[error("required repository reconciliation failed: {reason}")]
    Repository { reason: String },
    #[error("runtime was fenced during reconciliation: {reason}")]
    Fenced { reason: String },
}

/// Run the repository reconciliation owned by the shared runtime frame.
///
/// Station projection mutation remains an injected follow-up port, while the
/// existing admission/identity allocator watermark reconciliation is common to
/// every deployment adapter.
pub fn reconcile_runtime_repositories(
    node: &mut SimulationNode,
    _result: &crate::node::TickResult,
    _events: &[DomainEvent],
) -> Result<(), RuntimeReconciliationError> {
    node.reconcile_runtime_repositories()
        .map_err(|reason| RuntimeReconciliationError::Repository { reason })
}

/// Health gate for one long-lived runtime adapter.
///
/// A durable append followed by an apply or reconciliation failure leaves a
/// committed transition that must be recovered before another transition can
/// be acknowledged. The adapter owns this value and calls
/// [`Self::mark_recovered`] only after replay/reconciliation has completed.
#[derive(Debug, Default)]
pub struct RuntimeHealth {
    fenced_reason: Option<String>,
}

impl RuntimeHealth {
    /// Create a healthy runtime gate.
    pub const fn new() -> Self {
        Self {
            fenced_reason: None,
        }
    }

    /// Return whether the adapter must stop processing transitions.
    pub fn is_fenced(&self) -> bool {
        self.fenced_reason.is_some()
    }

    /// Return the reason recorded when this runtime was fenced.
    pub fn fenced_reason(&self) -> Option<&str> {
        self.fenced_reason.as_deref()
    }

    /// Mark the runtime healthy after the caller has completed recovery.
    ///
    /// This does not perform recovery itself. Callers must replay the durable
    /// journal and reconcile required projections before invoking this method.
    pub fn mark_recovered(&mut self) {
        self.fenced_reason = None;
    }

    fn ensure_healthy(&self) -> Result<(), RuntimeReconciliationError> {
        match &self.fenced_reason {
            Some(reason) => Err(RuntimeReconciliationError::Fenced {
                reason: reason.clone(),
            }),
            None => Ok(()),
        }
    }

    fn fence(&mut self, reason: impl Into<String>) {
        self.fenced_reason = Some(reason.into());
    }
}

/// Commit a prepared Stop transition through the runtime-owned journal.
pub fn commit_stop_transition<J: DurableJournal>(
    node: &mut SimulationNode,
    journal: &mut J,
    ship_id: ShipId,
    transition_id: crate::transition::SectorTransitionId,
    owner_epoch: u64,
    durability: DurabilityMode,
) -> Result<AppendReceipt, StopTransitionError> {
    let prepared = node.prepare_stop_transition(ship_id, transition_id, owner_epoch)?;
    let delta = match &prepared.recovery_delta {
        crate::transition::SectorRecoveryDelta::Stop(delta) => *delta,
        crate::transition::SectorRecoveryDelta::Tick(_) => {
            unreachable!("prepare_stop_transition always produces a Stop delta")
        }
    };
    // Resolve the live entity before the durable append. Once the journal
    // accepts the transition, applying this already-validated entity is
    // infallible; a post-commit lookup failure must not masquerade as a
    // normal command rejection.
    let entity = node.stop_entity(delta.ship_id)?;
    let receipt =
        crate::transition_journal::append_prepared_transition(journal, &prepared, durability)?;
    node.apply_stop_delta(entity, delta);
    Ok(receipt)
}

/// Commit the complete bounded ECS Tick write set through the runtime journal.
pub fn commit_tick_state_transition<J: DurableJournal>(
    node: &mut SimulationNode,
    journal: &mut J,
    lock_commands: &[dawn_core::LockOnCommand],
    transition_id: crate::transition::SectorTransitionId,
    owner_epoch: u64,
    durability: DurabilityMode,
) -> Result<AppendReceipt, TickTransitionError> {
    let (prepared, _) =
        node.prepare_tick_state_transition_with_result(lock_commands, transition_id, owner_epoch)?;
    let crate::transition::SectorRecoveryDelta::Tick(ref delta) = prepared.recovery_delta else {
        unreachable!("prepare_tick_state_transition always produces a Tick delta")
    };
    node.validate_tick_transition(delta, prepared.context)?;
    let receipt =
        crate::transition_journal::append_prepared_transition(journal, &prepared, durability)?;
    node.apply_validated_full_tick(delta.as_ref().clone())?;
    Ok(receipt)
}

/// Commit the logical Tick counter through the runtime journal.
pub fn commit_tick_transition<J: DurableJournal>(
    node: &mut SimulationNode,
    journal: &mut J,
    transition_id: crate::transition::SectorTransitionId,
    owner_epoch: u64,
    durability: DurabilityMode,
) -> Result<AppendReceipt, TickTransitionError> {
    let prepared = node
        .prepare_tick_transition(transition_id, owner_epoch)
        .map_err(crate::node::TickPreparationError::from)?;
    let crate::transition::SectorRecoveryDelta::Tick(ref delta) = prepared.recovery_delta else {
        unreachable!("prepare_tick_transition always produces a Tick delta")
    };
    node.validate_tick_transition(delta, prepared.context)?;
    let receipt =
        crate::transition_journal::append_prepared_transition(journal, &prepared, durability)?;
    node.apply_validated_logical_tick(delta.as_ref().clone());
    Ok(receipt)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransitOp {
    Request {
        ship_id: ShipId,
        to: SectorId,
        gate_id: Option<JumpGateId>,
    },
    Commit {
        attempt_id: TransitAttemptId,
        handoff: Box<TransitHandoffState>,
        from: SectorId,
        to: SectorId,
        entry_pos: AbsolutePosition,
        gate_id: Option<JumpGateId>,
        request_tick: Tick,
    },
    Ack {
        attempt_id: TransitAttemptId,
        ship_id: ShipId,
        from: SectorId,
        to: SectorId,
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

fn propose_commit<C: RuntimeConsensus>(consensus: &mut C, proposal: pipeline::CommitProposal) {
    consensus.propose(TransitOp::Commit {
        attempt_id: proposal.attempt_id,
        handoff: Box::new(proposal.handoff),
        from: proposal.from,
        to: proposal.to,
        entry_pos: proposal.entry_pos,
        gate_id: proposal.gate_id,
        request_tick: proposal.request_tick,
    });
}

fn propose_ack<C: RuntimeConsensus>(consensus: &mut C, proposal: pipeline::AckProposal) {
    consensus.propose(TransitOp::Ack {
        attempt_id: proposal.attempt_id,
        ship_id: proposal.ship_id,
        from: proposal.from,
        to: proposal.to,
        request_tick: proposal.request_tick,
    });
}

/// Consensus and transport capabilities required by the shared Sector frame.
///
/// Production supplies a Raft-backed implementation while local simulation
/// supplies [`LocalRuntimeConsensus`]. Keeping this port at the runtime
/// boundary prevents each deployment from implementing its own Tick ordering.
pub trait RuntimeConsensus {
    /// Drain entries that became committed since the previous frame.
    fn drain_committed(&mut self) -> Vec<Vec<u8>>;

    /// Submit a transit operation to the authoritative consensus log.
    fn propose(&mut self, operation: TransitOp);

    /// Advance consensus time after the Sector transition is complete.
    fn tick(&mut self);
}

/// Local adapter used by single-sector simulation runs.
///
/// It deliberately has no transit authority: the single-sector server cannot
/// hand ownership to another Sector, so transit proposals are ignored while
/// ordinary Tick processing still uses the shared durable frame.
#[derive(Debug, Default)]
pub struct LocalRuntimeConsensus;

impl RuntimeConsensus for LocalRuntimeConsensus {
    fn drain_committed(&mut self) -> Vec<Vec<u8>> {
        Vec::new()
    }

    fn propose(&mut self, _operation: TransitOp) {}

    fn tick(&mut self) {}
}

/// Adapter that gives the shared runtime access to one Raft actor and its
/// committed-entry stream.
#[derive(Debug)]
pub struct RaftRuntimeConsensus<'a> {
    raft: &'a RaftActorHandle,
    committed_rx: &'a mut mpsc::UnboundedReceiver<Vec<u8>>,
}

impl<'a> RaftRuntimeConsensus<'a> {
    /// Connect the shared runtime to a Raft actor for one frame loop.
    pub fn new(
        raft: &'a RaftActorHandle,
        committed_rx: &'a mut mpsc::UnboundedReceiver<Vec<u8>>,
    ) -> Self {
        Self { raft, committed_rx }
    }
}

impl RuntimeConsensus for RaftRuntimeConsensus<'_> {
    fn drain_committed(&mut self) -> Vec<Vec<u8>> {
        let mut entries = Vec::new();
        while let Ok(payload) = self.committed_rx.try_recv() {
            entries.push(payload);
        }
        entries
    }

    fn propose(&mut self, operation: TransitOp) {
        self.raft.propose(operation.encode());
    }

    fn tick(&mut self) {
        self.raft.tick();
    }
}

fn apply_committed_entries<C: RuntimeConsensus>(node: &mut SimulationNode, consensus: &mut C) {
    for payload in consensus.drain_committed() {
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
                    propose_commit(consensus, proposal);
                }
            }
            TransitOp::Commit {
                attempt_id,
                handoff,
                from,
                to,
                entry_pos,
                gate_id,
                request_tick,
            } => {
                if let Some(proposal) = pipeline::apply_commit(
                    node,
                    &handoff,
                    from,
                    to,
                    entry_pos,
                    gate_id,
                    request_tick,
                    attempt_id,
                ) {
                    propose_ack(consensus, proposal);
                }
            }
            TransitOp::Ack {
                attempt_id,
                ship_id,
                from,
                to,
                request_tick,
            } => {
                pipeline::apply_ack(node, ship_id, from, to, request_tick, attempt_id);
            }
        }
    }

    for proposal in pipeline::due_retries(node) {
        propose_commit(consensus, proposal);
    }
}

/// Apply committed Raft entries through the shared consensus port.
pub fn apply_committed_raft_entries(
    node: &mut SimulationNode,
    raft: &RaftActorHandle,
    committed_rx: &mut mpsc::UnboundedReceiver<Vec<u8>>,
) {
    let mut consensus = RaftRuntimeConsensus::new(raft, committed_rx);
    apply_committed_entries(node, &mut consensus);
}

#[derive(Debug)]
pub struct RuntimeTickOutput {
    pub tick_result: crate::node::TickResult,
    pub events: Vec<DomainEvent>,
    /// Auto-jump triggers that passed final validation and were proposed to Raft.
    /// Rejected one-shot triggers are drained but deliberately omitted.
    pub pending_auto_jumps: Vec<(ShipId, JumpGateId)>,
    pub completed_warps: Vec<ShipId>,
}

#[derive(Debug, Clone, Copy)]
pub struct DurableRuntimeTickContext {
    pub transition_id: crate::transition::SectorTransitionId,
    pub owner_epoch: u64,
    pub durability: DurabilityMode,
    pub profile: RuntimeDurabilityProfile,
}

/// Derive the stable transition identity for the next logical frame of one
/// Sector owner. The tick occupies the high half so node identity cannot
/// collide with another frame in the same Sector timeline.
pub fn runtime_transition_id(node: &SimulationNode) -> crate::transition::SectorTransitionId {
    crate::transition::SectorTransitionId(
        (u128::from(node.current_tick().value()) << 64) | u128::from(node.node_id().0),
    )
}

/// Durability contract selected by the runtime for one transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeDurabilityProfile {
    /// Local `Synced` journal evidence protects against process/OS/power loss
    /// while the authoritative local storage remains available.
    LocalDurable,
    /// Requires a configured quorum/fencing policy and #280 remote transport
    /// before a production adapter may enable it.
    ReplicatedDurable,
}

/// Runtime durability policy failures detected before state mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RuntimeDurabilityPolicyError {
    #[error("LocalDurable requires Synced journal durability")]
    LocalDurableRequiresSync,
    #[error(
        "ReplicatedDurable is not available until quorum transport and fencing are configured"
    )]
    ReplicatedDurableUnavailable,
    #[error("replicated durability quorum {quorum} exceeds replica set size {replicas}")]
    InvalidQuorum { quorum: usize, replicas: usize },
    #[error("local replica is not a member of the configured replica set")]
    LocalReplicaMissing,
    #[error(
        "matching durability evidence reached {matched} replicas, but quorum requires {required}"
    )]
    QuorumNotReached { matched: usize, required: usize },
    #[error("durability evidence from replica {replica_id} does not match the local receipt")]
    ReceiptMismatch { replica_id: NodeId },
    #[error("replica {replica_id} supplied duplicate durability evidence")]
    DuplicateReplicaEvidence { replica_id: NodeId },
}

/// One remote receipt supplied by a peer-transport adapter.
///
/// The receipt itself is owned by `dawn-storage`; this wrapper adds the
/// replica identity that quorum membership and duplicate suppression require.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeReplicaReceipt {
    pub replica_id: NodeId,
    pub evidence: DurabilityEvidence,
}

/// Policy port used by the shared runtime to validate the selected durability
/// profile before mutation and to validate the receipt before live apply.
pub trait RuntimeDurabilityPolicy {
    /// Reject an unsupported profile before the engine mutates state.
    fn validate(
        &self,
        profile: RuntimeDurabilityProfile,
        durability: DurabilityMode,
    ) -> Result<(), RuntimeDurabilityPolicyError>;

    /// Validate local and, when configured, remote evidence before live apply.
    fn validate_receipt(
        &self,
        profile: RuntimeDurabilityProfile,
        local_receipt: &AppendReceipt,
    ) -> Result<(), RuntimeDurabilityPolicyError>;
}

/// Local policy used by one process or a local simulation adapter.
#[derive(Debug, Default, Clone, Copy)]
pub struct LocalRuntimeDurabilityPolicy;

impl RuntimeDurabilityPolicy for LocalRuntimeDurabilityPolicy {
    fn validate(
        &self,
        profile: RuntimeDurabilityProfile,
        durability: DurabilityMode,
    ) -> Result<(), RuntimeDurabilityPolicyError> {
        match (profile, durability) {
            (RuntimeDurabilityProfile::LocalDurable, DurabilityMode::Synced) => Ok(()),
            (RuntimeDurabilityProfile::LocalDurable, _) => {
                Err(RuntimeDurabilityPolicyError::LocalDurableRequiresSync)
            }
            (RuntimeDurabilityProfile::ReplicatedDurable, _) => {
                Err(RuntimeDurabilityPolicyError::ReplicatedDurableUnavailable)
            }
        }
    }

    fn validate_receipt(
        &self,
        profile: RuntimeDurabilityProfile,
        local_receipt: &AppendReceipt,
    ) -> Result<(), RuntimeDurabilityPolicyError> {
        if profile == RuntimeDurabilityProfile::LocalDurable
            && local_receipt.durability == DurabilityMode::Synced
        {
            Ok(())
        } else {
            Err(RuntimeDurabilityPolicyError::ReplicatedDurableUnavailable)
        }
    }
}

/// Deterministic quorum/fencing policy supplied by a peer transport adapter.
///
/// #280 owns how `remote_receipts` arrive. This type owns the #278 decision:
/// only distinct members of the configured replica set whose immutable
/// receipt exactly matches the local transition can satisfy the quorum.
#[derive(Debug, Clone)]
pub struct ReplicatedRuntimeDurabilityPolicy {
    local_replica: NodeId,
    replica_set: HashSet<NodeId>,
    quorum: usize,
    remote_receipts: Vec<RuntimeReplicaReceipt>,
}

impl ReplicatedRuntimeDurabilityPolicy {
    /// Create a policy for one owner and its configured replica set.
    pub fn new(
        local_replica: NodeId,
        replica_set: Vec<NodeId>,
        quorum: usize,
        remote_receipts: Vec<RuntimeReplicaReceipt>,
    ) -> Result<Self, RuntimeDurabilityPolicyError> {
        let replica_set: HashSet<_> = replica_set.into_iter().collect();
        if !replica_set.contains(&local_replica) {
            return Err(RuntimeDurabilityPolicyError::LocalReplicaMissing);
        }
        if quorum == 0 || quorum > replica_set.len() {
            return Err(RuntimeDurabilityPolicyError::InvalidQuorum {
                quorum,
                replicas: replica_set.len(),
            });
        }
        Ok(Self {
            local_replica,
            replica_set,
            quorum,
            remote_receipts,
        })
    }
}

impl RuntimeDurabilityPolicy for ReplicatedRuntimeDurabilityPolicy {
    fn validate(
        &self,
        profile: RuntimeDurabilityProfile,
        durability: DurabilityMode,
    ) -> Result<(), RuntimeDurabilityPolicyError> {
        match (profile, durability) {
            (RuntimeDurabilityProfile::ReplicatedDurable, DurabilityMode::Synced) => Ok(()),
            (RuntimeDurabilityProfile::ReplicatedDurable, _) => {
                Err(RuntimeDurabilityPolicyError::LocalDurableRequiresSync)
            }
            (RuntimeDurabilityProfile::LocalDurable, _) => {
                Err(RuntimeDurabilityPolicyError::ReplicatedDurableUnavailable)
            }
        }
    }

    fn validate_receipt(
        &self,
        profile: RuntimeDurabilityProfile,
        local_receipt: &AppendReceipt,
    ) -> Result<(), RuntimeDurabilityPolicyError> {
        if profile != RuntimeDurabilityProfile::ReplicatedDurable {
            return Err(RuntimeDurabilityPolicyError::ReplicatedDurableUnavailable);
        }

        let mut matched = HashSet::from([self.local_replica]);
        for remote in &self.remote_receipts {
            if !self.replica_set.contains(&remote.replica_id) {
                continue;
            }
            if !matched.insert(remote.replica_id) {
                return Err(RuntimeDurabilityPolicyError::DuplicateReplicaEvidence {
                    replica_id: remote.replica_id,
                });
            }
            if remote.evidence.source != DurabilityEvidenceSource::Remote
                || remote.evidence.receipt != *local_receipt
            {
                return Err(RuntimeDurabilityPolicyError::ReceiptMismatch {
                    replica_id: remote.replica_id,
                });
            }
        }

        if matched.len() < self.quorum {
            return Err(RuntimeDurabilityPolicyError::QuorumNotReached {
                matched: matched.len(),
                required: self.quorum,
            });
        }
        Ok(())
    }
}

/// Execute the authoritative server frame pipeline.
///
/// Ordering is deliberately centralized here for every runtime adapter:
/// committed Raft entries -> simulation Tick -> Event collection ->
/// replication hook -> Raft clock advancement -> auto-jump proposal ->
/// transient warp-output drain.
pub fn run_runtime_tick<F>(
    node: &mut SimulationNode,
    raft: &RaftActorHandle,
    committed_rx: &mut mpsc::UnboundedReceiver<Vec<u8>>,
    lock_commands: &[dawn_core::LockOnCommand],
    after_events_collected: F,
) -> RuntimeTickOutput
where
    F: FnOnce(&SimulationNode, &crate::node::TickResult, &[DomainEvent]),
{
    let mut consensus = RaftRuntimeConsensus::new(raft, committed_rx);
    run_runtime_tick_with_consensus(node, &mut consensus, lock_commands, after_events_collected)
}

/// Execute the non-durable legacy frame through an injected consensus port.
///
/// This remains useful for focused engine tests. Runtime adapters should use
/// [`run_durable_runtime_tick_with_consensus`] so local and production paths
/// share the ADR-0049 commit boundary.
pub fn run_runtime_tick_with_consensus<C, F>(
    node: &mut SimulationNode,
    consensus: &mut C,
    lock_commands: &[dawn_core::LockOnCommand],
    after_events_collected: F,
) -> RuntimeTickOutput
where
    C: RuntimeConsensus,
    F: FnOnce(&SimulationNode, &crate::node::TickResult, &[DomainEvent]),
{
    // Consume the engine's explicit transition output instead of deriving the
    // frame from a mutable public-event cursor. The legacy log remains a
    // mirror during migration, but runtime publication is driven by output.
    let mut events = node.drain_pending_events();
    apply_committed_entries(node, consensus);
    let result = node.tick_with_lock_commands(lock_commands);
    events.extend(node.drain_pending_events());

    // Replication must observe the newly produced transition output before the
    // consensus clock advances. The immutable node reference keeps this hook
    // publication-only so the collected output cannot diverge from the node.
    after_events_collected(node, &result, &events);
    consensus.tick();

    // Auto-jump is a simulation transient, not adapter-owned Tick ordering.
    // Drain and propose it here so actor, clustered serve, and production Node
    // paths all complete the same-frame handoff. Only successful proposals are
    // surfaced to adapters; rejected one-shot attempts retain their historical
    // silent-drop behavior.
    let pending_auto_jumps = node
        .drain_pending_auto_jumps()
        .into_iter()
        .filter_map(|(ship_id, gate_id)| {
            propose_auto_jump_with_consensus(node, consensus, ship_id, gate_id)
                .map(|_| (ship_id, gate_id))
        })
        .collect();
    let completed_warps = node.drain_completed_warps();

    RuntimeTickOutput {
        tick_result: result,
        events,
        pending_auto_jumps,
        completed_warps,
    }
}

/// Execute one frame with the ADR-0049 durable Tick boundary.
///
/// Unlike [`run_runtime_tick`], this path does not let the ECS Tick mutate the
/// live state before persistence. It prepares the bounded recovery write set,
/// appends the recovery/public batch to the supplied journal, applies the same
/// delta, and only then publishes the public events and transient effects.
/// The legacy frame remains available while callers migrate their journal
/// wiring to this explicit runtime-owned path.
pub fn run_durable_runtime_tick<J, F>(
    node: &mut SimulationNode,
    journal: &mut J,
    raft: &RaftActorHandle,
    committed_rx: &mut mpsc::UnboundedReceiver<Vec<u8>>,
    lock_commands: &[dawn_core::LockOnCommand],
    context: DurableRuntimeTickContext,
    after_events_collected: F,
) -> Result<RuntimeTickOutput, TickTransitionError>
where
    J: DurableJournal,
    F: FnOnce(&SimulationNode, &crate::node::TickResult, &[DomainEvent]),
{
    let mut consensus = RaftRuntimeConsensus::new(raft, committed_rx);
    run_durable_runtime_tick_with_consensus(
        node,
        journal,
        &mut consensus,
        lock_commands,
        context,
        after_events_collected,
    )
}

/// Execute one durable frame through an injected consensus port.
///
/// This is the authoritative orchestration seam for production and local
/// simulation. It prepares the full recovery/public transition, persists it,
/// applies the same recovery delta, and only then exposes events and effects.
pub fn run_durable_runtime_tick_with_consensus<J, C, F>(
    node: &mut SimulationNode,
    journal: &mut J,
    consensus: &mut C,
    lock_commands: &[dawn_core::LockOnCommand],
    context: DurableRuntimeTickContext,
    after_events_collected: F,
) -> Result<RuntimeTickOutput, TickTransitionError>
where
    J: DurableJournal,
    C: RuntimeConsensus,
    F: FnOnce(&SimulationNode, &crate::node::TickResult, &[DomainEvent]),
{
    let mut health = RuntimeHealth::new();
    run_durable_runtime_tick_with_consensus_and_health(
        node,
        journal,
        consensus,
        &mut health,
        lock_commands,
        context,
        after_events_collected,
    )
}

/// Execute one local-durability frame with an adapter-owned health gate.
///
/// Long-lived production and simulation adapters should use this entry point
/// so a post-append failure fences the adapter across subsequent calls.
pub fn run_durable_runtime_tick_with_consensus_and_health<J, C, F>(
    node: &mut SimulationNode,
    journal: &mut J,
    consensus: &mut C,
    health: &mut RuntimeHealth,
    lock_commands: &[dawn_core::LockOnCommand],
    context: DurableRuntimeTickContext,
    after_events_collected: F,
) -> Result<RuntimeTickOutput, TickTransitionError>
where
    J: DurableJournal,
    C: RuntimeConsensus,
    F: FnOnce(&SimulationNode, &crate::node::TickResult, &[DomainEvent]),
{
    let policy = LocalRuntimeDurabilityPolicy;
    run_durable_runtime_tick_with_policy_and_reconciliation_and_health(
        node,
        journal,
        consensus,
        &policy,
        health,
        lock_commands,
        context,
        reconcile_runtime_repositories,
        after_events_collected,
    )
}

/// Execute one durable frame with an explicit durability/quorum policy.
///
/// The policy is checked before preparation and again immediately after the
/// local append. A failed post-append policy check returns before live apply or
/// publication, leaving the caller to fence and recover the committed bytes.
pub fn run_durable_runtime_tick_with_policy<J, C, P, F>(
    node: &mut SimulationNode,
    journal: &mut J,
    consensus: &mut C,
    policy: &P,
    lock_commands: &[dawn_core::LockOnCommand],
    context: DurableRuntimeTickContext,
    after_events_collected: F,
) -> Result<RuntimeTickOutput, TickTransitionError>
where
    J: DurableJournal,
    C: RuntimeConsensus,
    P: RuntimeDurabilityPolicy,
    F: FnOnce(&SimulationNode, &crate::node::TickResult, &[DomainEvent]),
{
    let mut health = RuntimeHealth::new();
    run_durable_runtime_tick_with_policy_and_reconciliation_and_health(
        node,
        journal,
        consensus,
        policy,
        &mut health,
        lock_commands,
        context,
        reconcile_runtime_repositories,
        after_events_collected,
    )
}

/// Execute one durable frame with explicit durability and reconciliation
/// policies.
///
/// Reconciliation runs after live apply but before the output hook or
/// consensus clock. A failure therefore leaves the committed bytes available
/// for recovery while preventing publication and acknowledgement from making
/// an unhealthy runtime look current.
#[allow(clippy::too_many_arguments)]
pub fn run_durable_runtime_tick_with_policy_and_reconciliation<J, C, P, R, F>(
    node: &mut SimulationNode,
    journal: &mut J,
    consensus: &mut C,
    policy: &P,
    lock_commands: &[dawn_core::LockOnCommand],
    context: DurableRuntimeTickContext,
    reconcile: R,
    after_events_collected: F,
) -> Result<RuntimeTickOutput, TickTransitionError>
where
    J: DurableJournal,
    C: RuntimeConsensus,
    P: RuntimeDurabilityPolicy,
    R: FnOnce(
        &mut SimulationNode,
        &crate::node::TickResult,
        &[DomainEvent],
    ) -> Result<(), RuntimeReconciliationError>,
    F: FnOnce(&SimulationNode, &crate::node::TickResult, &[DomainEvent]),
{
    let mut health = RuntimeHealth::new();
    run_durable_runtime_tick_with_policy_and_reconciliation_and_health(
        node,
        journal,
        consensus,
        policy,
        &mut health,
        lock_commands,
        context,
        reconcile,
        after_events_collected,
    )
}

/// Execute one durable frame with an adapter-owned health gate.
///
/// A post-append policy, live-apply, or reconciliation failure fences
/// `health`. No later transition may proceed until the caller has recovered
/// the durable transition and explicitly calls [`RuntimeHealth::mark_recovered`].
#[allow(clippy::too_many_arguments)]
pub fn run_durable_runtime_tick_with_policy_and_reconciliation_and_health<J, C, P, R, F>(
    node: &mut SimulationNode,
    journal: &mut J,
    consensus: &mut C,
    policy: &P,
    health: &mut RuntimeHealth,
    lock_commands: &[dawn_core::LockOnCommand],
    context: DurableRuntimeTickContext,
    reconcile: R,
    after_events_collected: F,
) -> Result<RuntimeTickOutput, TickTransitionError>
where
    J: DurableJournal,
    C: RuntimeConsensus,
    P: RuntimeDurabilityPolicy,
    R: FnOnce(
        &mut SimulationNode,
        &crate::node::TickResult,
        &[DomainEvent],
    ) -> Result<(), RuntimeReconciliationError>,
    F: FnOnce(&SimulationNode, &crate::node::TickResult, &[DomainEvent]),
{
    health.ensure_healthy()?;
    policy.validate(context.profile, context.durability)?;
    apply_committed_entries(node, consensus);
    // Command-side and committed-transit public facts belong to this same
    // runtime output boundary. Keep them outside the prepared Tick until the
    // journal append has succeeded so a failed append can restore the buffer.
    let prior_events = node.drain_pending_events();
    let (prepared, result) = match node.prepare_tick_state_transition_with_result(
        lock_commands,
        context.transition_id,
        context.owner_epoch,
    ) {
        Ok(result) => result,
        Err(error) => {
            node.restore_pending_events(prior_events);
            return Err(TickTransitionError::Preparation(error));
        }
    };
    let delta = match &prepared.recovery_delta {
        crate::transition::SectorRecoveryDelta::Tick(delta) => delta.as_ref().clone(),
        crate::transition::SectorRecoveryDelta::Stop(_) => {
            unreachable!("full runtime Tick preparation produces a Tick delta")
        }
    };
    let receipt = match crate::transition_journal::append_prepared_transition_with_events(
        journal,
        &prepared,
        &prior_events,
        context.durability,
    ) {
        Ok(receipt) => receipt,
        Err(error) => {
            node.restore_pending_events(prior_events);
            health.fence(format!("durable append failed: {error}"));
            return Err(TickTransitionError::Durable(error));
        }
    };
    if let Err(error) = policy.validate_receipt(context.profile, &receipt) {
        node.restore_pending_events(prior_events);
        health.fence(format!(
            "durability policy rejected the committed transition: {error}"
        ));
        return Err(TickTransitionError::Policy(error));
    }
    if let Err(error) = node.apply_tick_transition(delta, prepared.context) {
        health.fence(format!("live apply failed after durable append: {error}"));
        return Err(TickTransitionError::Validation(error));
    }
    let mut events = prior_events;
    events.extend(node.drain_pending_events());
    events.extend(prepared.public_events);

    if let Err(error) = reconcile(node, &result, &events) {
        health.fence(format!("required reconciliation failed: {error}"));
        return Err(TickTransitionError::Reconciliation(error));
    }
    after_events_collected(node, &result, &events);
    consensus.tick();

    let pending_auto_jumps = node
        .drain_pending_auto_jumps()
        .into_iter()
        .filter_map(|(ship_id, gate_id)| {
            propose_auto_jump_with_consensus(node, consensus, ship_id, gate_id)
                .map(|_| (ship_id, gate_id))
        })
        .collect();
    let completed_warps = node.drain_completed_warps();

    Ok(RuntimeTickOutput {
        tick_result: result,
        events,
        pending_auto_jumps,
        completed_warps,
    })
}

pub fn propose_jump(
    node: &mut SimulationNode,
    raft: &RaftActorHandle,
    ship_id: ShipId,
    gate_id: JumpGateId,
) -> crate::node::JumpOutcome {
    let (_tx, mut committed_rx) = mpsc::unbounded_channel();
    let mut consensus = RaftRuntimeConsensus::new(raft, &mut committed_rx);
    propose_jump_with_consensus(node, &mut consensus, ship_id, gate_id)
}

/// Apply a jump request and submit a transit proposal through the shared port.
pub fn propose_jump_with_consensus<C: RuntimeConsensus>(
    node: &mut SimulationNode,
    consensus: &mut C,
    ship_id: ShipId,
    gate_id: JumpGateId,
) -> crate::node::JumpOutcome {
    let outcome = node.apply_jump_with_fallback(ship_id, gate_id);
    if let crate::node::JumpOutcome::NeedsTransitProposal { to } = outcome {
        propose_transit_request(consensus, ship_id, to, gate_id);
    }
    outcome
}

pub fn propose_auto_jump(
    node: &mut SimulationNode,
    raft: &RaftActorHandle,
    ship_id: ShipId,
    gate_id: JumpGateId,
) -> Option<SectorId> {
    let (_tx, mut committed_rx) = mpsc::unbounded_channel();
    let mut consensus = RaftRuntimeConsensus::new(raft, &mut committed_rx);
    propose_auto_jump_with_consensus(node, &mut consensus, ship_id, gate_id)
}

/// Propose an auto-jump through the shared consensus port.
pub fn propose_auto_jump_with_consensus<C: RuntimeConsensus>(
    node: &mut SimulationNode,
    consensus: &mut C,
    ship_id: ShipId,
    gate_id: JumpGateId,
) -> Option<SectorId> {
    let to = node.resolve_auto_jump(ship_id, gate_id)?;
    propose_transit_request(consensus, ship_id, to, gate_id);
    Some(to)
}

fn propose_transit_request<C: RuntimeConsensus>(
    consensus: &mut C,
    ship_id: ShipId,
    to: SectorId,
    gate_id: JumpGateId,
) {
    consensus.propose(TransitOp::Request {
        ship_id,
        to,
        gate_id: Some(gate_id),
    });
}

#[cfg(test)]
mod handoff_tests;
#[cfg(test)]
mod runtime_tick_review_tests;
#[cfg(test)]
mod tests;
