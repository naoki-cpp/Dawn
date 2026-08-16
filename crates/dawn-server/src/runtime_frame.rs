//! Shared one-Sector runtime frame host.
//!
//! This module owns the mutable Sector state and the prepare -> durable ->
//! live-apply boundary. Server entry points only collect deployment-specific
//! inputs and consume the typed output returned by [`RuntimeFrameHost`].
// The source is compiled into both executable roots; each root uses a
// different subset of the shared host helpers.
#![allow(dead_code)]

use dawn_core::{
    DomainEvent, JumpGateId, LockOnCommand, PlayerId, Position, SectorId, ShipId, ShipTypeId,
    Velocity,
};
use dawn_distributed::RaftActorHandle;
use dawn_sector::client_admission::{
    ClientAdmissionAttempt, ClientAdmissionIntent, ClientAdmissionRefusal,
};
use dawn_sector::node::{JumpOutcome, RuntimeCommandDispatch, SimulationNode};
use dawn_sector::transit::{
    reconcile_runtime_repositories, run_durable_runtime_frame, DurableRuntimeTickContext,
    LocalRuntimeDurabilityPolicy, RuntimeConsensus, RuntimeDurabilityPolicy,
    RuntimeDurabilityProfile, RuntimeHealth, RuntimeTickOutput, TransitOp,
};
use dawn_storage::{DurabilityMode, DurableJournal};
use std::fmt;
use tokio::sync::mpsc;

/// Owned Raft adapter used by a long-lived one-Sector Host.
pub(crate) struct OwnedRaftRuntimeConsensus {
    raft: RaftActorHandle,
    committed_rx: mpsc::UnboundedReceiver<Vec<u8>>,
}

impl OwnedRaftRuntimeConsensus {
    pub(crate) fn new(
        raft: RaftActorHandle,
        committed_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    ) -> Self {
        Self { raft, committed_rx }
    }

    fn role_request(
        &self,
    ) -> impl std::future::Future<Output = (dawn_distributed::Role, dawn_distributed::Term)>
           + Send
           + 'static {
        let raft = self.raft.clone();
        async move { raft.role().await }
    }
}

impl RuntimeConsensus for OwnedRaftRuntimeConsensus {
    fn drain_committed(&mut self) -> Vec<Vec<u8>> {
        let mut entries = Vec::new();
        while let Ok(payload) = self.committed_rx.try_recv() {
            entries.push(payload);
        }
        entries
    }

    fn propose(&mut self, operation: dawn_sector::transit::TransitOp) {
        self.raft.propose(operation.encode());
    }

    fn tick(&mut self) {
        self.raft.tick();
    }
}

/// Immutable policy selected by the server composition root for one Sector.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RuntimeFramePolicy<P = LocalRuntimeDurabilityPolicy> {
    pub(crate) owner_epoch: u64,
    pub(crate) durability: DurabilityMode,
    pub(crate) profile: RuntimeDurabilityProfile,
    durability_policy: P,
}

impl RuntimeFramePolicy<LocalRuntimeDurabilityPolicy> {
    pub(crate) const fn local_durable(owner_epoch: u64) -> Self {
        Self::new(
            owner_epoch,
            DurabilityMode::Synced,
            RuntimeDurabilityProfile::LocalDurable,
            LocalRuntimeDurabilityPolicy,
        )
    }
}

impl<P> RuntimeFramePolicy<P> {
    pub(crate) const fn new(
        owner_epoch: u64,
        durability: DurabilityMode,
        profile: RuntimeDurabilityProfile,
        durability_policy: P,
    ) -> Self {
        Self {
            owner_epoch,
            durability,
            profile,
            durability_policy,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeFramePhase {
    Bootstrapping,
    Running,
    Fenced,
}

/// Errors that cross the runtime-host boundary.
#[derive(Debug)]
pub(crate) enum RuntimeFrameHostError {
    BootstrapClosed,
    Tick(dawn_sector::transit::TickTransitionError),
}

impl fmt::Display for RuntimeFrameHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BootstrapClosed => {
                write!(
                    formatter,
                    "bootstrap mutation is only available before the first runtime frame"
                )
            }
            Self::Tick(error) => write!(formatter, "runtime frame failed: {error}"),
        }
    }
}

impl std::error::Error for RuntimeFrameHostError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::BootstrapClosed => None,
            Self::Tick(error) => Some(error),
        }
    }
}

impl From<dawn_sector::transit::TickTransitionError> for RuntimeFrameHostError {
    fn from(error: dawn_sector::transit::TickTransitionError) -> Self {
        Self::Tick(error)
    }
}

/// Owns one Sector's authoritative state and its durable runtime dependencies.
///
/// The host is intentionally one-Sector wide. A cluster coordinator may own
/// several hosts, but it must use typed outputs and handoff operations rather
/// than retaining a mutable borrow across frame boundaries. Transitional
/// mutation bridges are closure-scoped and do not expose the owned state.
pub(crate) struct RuntimeFrameHost<J, C, P = LocalRuntimeDurabilityPolicy> {
    node: SimulationNode,
    journal: J,
    consensus: C,
    health: RuntimeHealth,
    policy: RuntimeFramePolicy<P>,
    phase: RuntimeFramePhase,
}

pub(crate) trait RuntimeNodeView {
    fn runtime_node(&self) -> &SimulationNode;
}

pub(crate) trait RuntimeNodeMutation: RuntimeNodeView {
    fn with_runtime_node_mut<R>(&mut self, operation: impl FnOnce(&mut SimulationNode) -> R) -> R;
}

impl RuntimeNodeMutation for SimulationNode {
    fn with_runtime_node_mut<R>(&mut self, operation: impl FnOnce(&mut SimulationNode) -> R) -> R {
        operation(self)
    }
}

impl RuntimeNodeView for SimulationNode {
    fn runtime_node(&self) -> &SimulationNode {
        self
    }
}

impl<J, C, P> RuntimeNodeView for RuntimeFrameHost<J, C, P>
where
    J: DurableJournal,
    C: RuntimeConsensus,
    P: RuntimeDurabilityPolicy,
{
    fn runtime_node(&self) -> &SimulationNode {
        self.node()
    }
}

impl<J, C, P> RuntimeNodeMutation for RuntimeFrameHost<J, C, P>
where
    J: DurableJournal,
    C: RuntimeConsensus,
    P: RuntimeDurabilityPolicy,
{
    fn with_runtime_node_mut<R>(&mut self, operation: impl FnOnce(&mut SimulationNode) -> R) -> R {
        self.with_node_mut(operation)
    }
}

impl<J, C, P> RuntimeFrameHost<J, C, P>
where
    J: DurableJournal,
    C: RuntimeConsensus,
    P: RuntimeDurabilityPolicy,
{
    pub(crate) fn new(
        node: SimulationNode,
        journal: J,
        consensus: C,
        policy: RuntimeFramePolicy<P>,
    ) -> Self {
        Self {
            node,
            journal,
            consensus,
            health: RuntimeHealth::new(),
            policy,
            phase: RuntimeFramePhase::Bootstrapping,
        }
    }

    pub(crate) fn node(&self) -> &SimulationNode {
        &self.node
    }

    /// Run one bounded composition operation against the owned node.
    ///
    /// This is intentionally a closure rather than a returned mutable
    /// reference, so callers cannot retain the authoritative state while a
    /// frame is running. Runtime mutations are being migrated into FrameInput
    /// in follow-up slices; current admission and Market adapters use this
    /// narrow bridge until then.
    pub(crate) fn with_node_mut<R>(
        &mut self,
        operation: impl FnOnce(&mut SimulationNode) -> R,
    ) -> R {
        operation(&mut self.node)
    }

    pub(crate) fn with_state_mut<R>(
        &mut self,
        operation: impl FnOnce(&mut SimulationNode, &mut J) -> R,
    ) -> R {
        operation(&mut self.node, &mut self.journal)
    }

    pub(crate) fn phase(&self) -> RuntimeFramePhase {
        self.phase
    }

    pub(crate) fn health(&self) -> &RuntimeHealth {
        &self.health
    }

    pub(crate) fn mark_recovered(&mut self) {
        self.health.mark_recovered();
        self.phase = RuntimeFramePhase::Running;
    }

    /// Create initial world state before this host starts processing frames.
    pub(crate) fn bootstrap_ship(
        &mut self,
        ship_type_id: ShipTypeId,
        position: Position,
        velocity: Velocity,
    ) -> Result<ShipId, RuntimeFrameHostError> {
        if self.phase != RuntimeFramePhase::Bootstrapping {
            return Err(RuntimeFrameHostError::BootstrapClosed);
        }
        Ok(self.node.spawn_ship(ship_type_id, position, velocity))
    }

    pub(crate) fn drain_pending_events(&mut self) -> Vec<DomainEvent> {
        self.node.drain_pending_events()
    }

    /// Seed initial NPC frigates into the Sector.
    ///
    /// Fixture/composition-root use only (initial server population), not a
    /// per-tick operation.
    pub(crate) fn spawn_npc_frigates(&mut self, count: usize) {
        self.with_node_mut(|node| node.spawn_npc_frigates(count));
    }

    /// Spawn a duel-mode Bot ship and its owning player identity.
    pub(crate) fn spawn_bot_ship(&mut self, position: Position) -> (PlayerId, ShipId) {
        self.with_node_mut(|node| node.spawn_bot_ship(position))
    }

    /// The Sector's topology-derived default spawn point for a fresh player.
    pub(crate) fn default_player_spawn_position(&self) -> Position {
        self.node.default_player_spawn_position()
    }

    /// Begin a client admission attempt against the owned node.
    ///
    /// Admission identity reservation is durable through its own
    /// repository-backed transaction (see
    /// `dawn_sector::node::admission_provisional`), independent of the tick
    /// pipeline, so this narrow bridge is admission's typed entry point
    /// rather than a correctness gap.
    pub(crate) fn begin_client_admission(
        &mut self,
        intent: ClientAdmissionIntent,
        aoi_cell_size: f64,
    ) -> Result<ClientAdmissionAttempt, ClientAdmissionRefusal> {
        self.with_node_mut(|node| node.begin_client_admission(intent, aoi_cell_size))
    }

    /// Adopt a ship that just jumped into this Sector under `player_id`'s
    /// ownership. Returns `false` if the ship is not (yet) present in this
    /// Sector's ECS.
    pub(crate) fn adopt_player_ship(&mut self, ship_id: ShipId, player_id: PlayerId) -> bool {
        self.with_node_mut(|node| node.adopt_player_ship(ship_id, player_id))
    }

    /// Spawn a ship outside the tick pipeline, for deterministic fixture
    /// setup (test/demo actor callers of `SectorRuntimeDriver`). Production
    /// runtime mutations must enter through `run_frame` once a frame has
    /// started; this is not that path.
    pub(crate) fn spawn_fixture_ship(
        &mut self,
        ship_type_id: ShipTypeId,
        position: Position,
        velocity: Velocity,
    ) -> ShipId {
        self.with_node_mut(|node| node.spawn_ship(ship_type_id, position, velocity))
    }

    /// Admit requests through the Sector's single typed request seam.
    pub(crate) fn collect_runtime_commands<S, Player, Request>(
        &mut self,
        sessions: &mut [S],
        lock_commands: &mut Vec<LockOnCommand>,
        player: Player,
        request: Request,
    ) -> Vec<RuntimeCommandDispatch>
    where
        Player: Fn(&S) -> dawn_core::PlayerId,
        Request: FnMut(&mut S) -> Option<dawn_core::ClientRequest>,
    {
        dawn_sector::node::collect_runtime_commands(
            &mut self.node,
            sessions,
            lock_commands,
            player,
            request,
        )
    }

    /// Propose a validated cross-Sector request through the owned consensus
    /// adapter. Callers do the admission check; the Host owns the proposal.
    pub(crate) fn propose_transit_request(
        &mut self,
        ship_id: ShipId,
        to: SectorId,
        gate_id: Option<JumpGateId>,
    ) {
        self.consensus.propose(TransitOp::Request {
            ship_id,
            to,
            gate_id,
        });
    }

    /// Apply jump fallback rules and submit any resulting transit proposal
    /// through the owned consensus adapter.
    pub(crate) fn propose_jump(&mut self, ship_id: ShipId, gate_id: JumpGateId) -> JumpOutcome {
        dawn_sector::transit::propose_jump_with_consensus(
            &mut self.node,
            &mut self.consensus,
            ship_id,
            gate_id,
        )
    }

    /// Run exactly one authoritative frame for this Sector.
    pub(crate) fn run_frame(
        &mut self,
        input: dawn_sector::transition::FrameInput<'_>,
    ) -> Result<RuntimeTickOutput, RuntimeFrameHostError> {
        self.run_frame_with_output(input, |_, _, _| {})
    }

    /// Run one frame and invoke `after_commit` after reconciliation succeeds.
    ///
    /// The callback is still before the consensus adapter advances its logical
    /// clock, so publication cannot be mistaken for an acknowledged frame.
    /// `after_commit`'s `TickResult` carries `market_settlement_outcomes` for
    /// any settlement admitted through `input` (issue #315).
    pub(crate) fn run_frame_with_output<F>(
        &mut self,
        input: dawn_sector::transition::FrameInput<'_>,
        after_commit: F,
    ) -> Result<RuntimeTickOutput, RuntimeFrameHostError>
    where
        F: FnOnce(&SimulationNode, &dawn_sector::node::TickResult, &[DomainEvent]),
    {
        if self.phase == RuntimeFramePhase::Fenced {
            // The shared runtime health gate returns the durable recovery error
            // and keeps the failure visible to the caller.
        } else {
            self.phase = RuntimeFramePhase::Running;
        }

        let transition_id = dawn_sector::transit::runtime_transition_id(&self.node);
        let output = run_durable_runtime_frame(
            &mut self.node,
            &mut self.journal,
            &mut self.consensus,
            &self.policy.durability_policy,
            &mut self.health,
            input,
            DurableRuntimeTickContext {
                transition_id,
                owner_epoch: self.policy.owner_epoch,
                durability: self.policy.durability,
                profile: self.policy.profile,
            },
            reconcile_runtime_repositories,
            after_commit,
        )
        .map_err(RuntimeFrameHostError::Tick);

        if self.health.is_fenced() {
            self.phase = RuntimeFramePhase::Fenced;
        }
        output
    }
}

impl<J> RuntimeFrameHost<J, OwnedRaftRuntimeConsensus>
where
    J: DurableJournal,
{
    pub(crate) fn raft_role(
        &self,
    ) -> impl std::future::Future<Output = (dawn_distributed::Role, dawn_distributed::Term)>
           + Send
           + 'static {
        self.consensus.role_request()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{NodeId, SectorBounds, SectorId};
    use dawn_sector::{
        galaxy::Galaxy,
        game_data::{GameDataCatalog, PRODUCTION_MODULES_PATH, PRODUCTION_SHIP_TYPES_PATH},
    };
    use dawn_storage::{
        AppendReceipt, DurableJournal, InMemoryJournal, JournalBatch, JournalError, JournalIndex,
        JournalRecord,
    };
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::{path::Path, sync::Arc};

    #[derive(Debug, Clone, Copy)]
    struct RejectingPolicy;

    impl RuntimeDurabilityPolicy for RejectingPolicy {
        fn validate(
            &self,
            _profile: RuntimeDurabilityProfile,
            _durability: DurabilityMode,
        ) -> Result<(), dawn_sector::transit::RuntimeDurabilityPolicyError> {
            Err(dawn_sector::transit::RuntimeDurabilityPolicyError::ReplicatedDurableUnavailable)
        }

        fn validate_receipt(
            &self,
            _profile: RuntimeDurabilityProfile,
            _local_receipt: &dawn_storage::AppendReceipt,
        ) -> Result<(), dawn_sector::transit::RuntimeDurabilityPolicyError> {
            Err(dawn_sector::transit::RuntimeDurabilityPolicyError::ReplicatedDurableUnavailable)
        }
    }

    fn catalog() -> Arc<GameDataCatalog> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        Arc::new(
            GameDataCatalog::load_from_paths(
                root.join(PRODUCTION_MODULES_PATH),
                root.join(PRODUCTION_SHIP_TYPES_PATH),
            )
            .expect("repository game-data catalog"),
        )
    }

    fn host_with_journal<J: DurableJournal, P: RuntimeDurabilityPolicy>(
        journal: J,
        policy: RuntimeFramePolicy<P>,
    ) -> RuntimeFrameHost<J, dawn_sector::transit::LocalRuntimeConsensus, P> {
        RuntimeFrameHost::new(
            SimulationNode::new(
                NodeId(0),
                SectorId(0),
                SectorBounds::centered(SectorBounds::DEFAULT_HALF),
                Arc::new(Galaxy::demo()),
                catalog(),
            ),
            journal,
            dawn_sector::transit::LocalRuntimeConsensus,
            policy,
        )
    }

    fn host_with_policy<P: RuntimeDurabilityPolicy>(
        policy: RuntimeFramePolicy<P>,
    ) -> RuntimeFrameHost<InMemoryJournal, dawn_sector::transit::LocalRuntimeConsensus, P> {
        host_with_journal(InMemoryJournal::new(), policy)
    }

    fn host() -> RuntimeFrameHost<InMemoryJournal, dawn_sector::transit::LocalRuntimeConsensus> {
        host_with_policy(RuntimeFramePolicy::local_durable(0))
    }

    #[test]
    fn bootstrap_is_closed_after_the_first_frame() {
        let mut host = host();
        host.bootstrap_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO)
            .expect("bootstrap should be available");

        host.run_frame(dawn_sector::transition::FrameInput::lock_only(&[]))
            .expect("local frame should commit");

        assert_eq!(host.phase(), RuntimeFramePhase::Running);
        assert!(matches!(
            host.bootstrap_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO),
            Err(RuntimeFrameHostError::BootstrapClosed)
        ));
    }

    #[test]
    fn host_returns_the_committed_tick_output() {
        let mut host = host();
        let output = host
            .run_frame(dawn_sector::transition::FrameInput::lock_only(&[]))
            .expect("local frame should commit");

        assert_eq!(output.tick_result.tick.value(), 1);
        assert_eq!(host.phase(), RuntimeFramePhase::Running);
        assert!(!host.health().is_fenced());
    }

    #[test]
    fn output_hook_observes_live_state_at_the_commit_boundary() {
        let mut host = host();
        let mut observed = None;

        let output = host
            .run_frame_with_output(
                dawn_sector::transition::FrameInput::lock_only(&[]),
                |node, tick_result, events| {
                    observed = Some((node.current_tick(), tick_result.tick, events.len()));
                },
            )
            .expect("local frame should commit");

        let (current_tick, observed_tick, event_count) = observed.expect("output hook should run");
        assert_eq!(current_tick, output.tick_result.tick);
        assert_eq!(observed_tick, output.tick_result.tick);
        assert_eq!(event_count, output.events.len());
    }

    #[test]
    fn durable_append_is_complete_before_the_output_hook_runs() {
        let appended = std::sync::Arc::new(AtomicBool::new(false));
        let mut host = host_with_journal(
            RecordingJournal::new(std::sync::Arc::clone(&appended)),
            RuntimeFramePolicy::local_durable(0),
        );

        host.run_frame_with_output(
            dawn_sector::transition::FrameInput::lock_only(&[]),
            |_, _, _| {
                assert!(
                    appended.load(Ordering::SeqCst),
                    "the output hook must run after the journal append"
                );
            },
        )
        .expect("local frame should commit");
        assert_eq!(host.journal.records().len(), 1);
    }

    #[test]
    fn composition_policy_is_checked_before_the_frame_mutates_state() {
        let mut host = host_with_policy(RuntimeFramePolicy::new(
            0,
            DurabilityMode::Synced,
            RuntimeDurabilityProfile::LocalDurable,
            RejectingPolicy,
        ));

        let result = host.run_frame(dawn_sector::transition::FrameInput::lock_only(&[]));

        assert!(matches!(
            result,
            Err(RuntimeFrameHostError::Tick(
                dawn_sector::transit::TickTransitionError::Policy(
                    dawn_sector::transit::RuntimeDurabilityPolicyError::
                        ReplicatedDurableUnavailable
                )
            ))
        ));
        assert_eq!(host.node().current_tick().value(), 0);
        assert!(!host.health().is_fenced());
    }

    #[test]
    fn invalid_durability_profile_is_rejected_before_state_change() {
        let mut local_host = host_with_policy(RuntimeFramePolicy::new(
            0,
            DurabilityMode::Buffered,
            RuntimeDurabilityProfile::LocalDurable,
            LocalRuntimeDurabilityPolicy,
        ));
        let local_result =
            local_host.run_frame(dawn_sector::transition::FrameInput::lock_only(&[]));
        assert!(matches!(
            local_result,
            Err(RuntimeFrameHostError::Tick(
                dawn_sector::transit::TickTransitionError::Policy(
                    dawn_sector::transit::RuntimeDurabilityPolicyError::LocalDurableRequiresSync
                )
            ))
        ));
        assert_eq!(local_host.node().current_tick().value(), 0);
        assert!(local_host.journal.records().is_empty());

        let mut replicated_host = host_with_policy(RuntimeFramePolicy::new(
            0,
            DurabilityMode::Synced,
            RuntimeDurabilityProfile::ReplicatedDurable,
            LocalRuntimeDurabilityPolicy,
        ));
        let replicated_result =
            replicated_host.run_frame(dawn_sector::transition::FrameInput::lock_only(&[]));
        assert!(matches!(
            replicated_result,
            Err(RuntimeFrameHostError::Tick(
                dawn_sector::transit::TickTransitionError::Policy(
                    dawn_sector::transit::RuntimeDurabilityPolicyError::ReplicatedDurableUnavailable
                )
            ))
        ));
        assert_eq!(replicated_host.node().current_tick().value(), 0);
        assert!(replicated_host.journal.records().is_empty());
    }

    #[test]
    fn append_failure_fences_the_host_and_preserves_pending_output() {
        let mut host = host_with_journal(FailingJournal, RuntimeFramePolicy::local_durable(0));
        host.bootstrap_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO)
            .expect("bootstrap should be available");
        let expected_events = host.node().pending_events().to_vec();

        let result = host.run_frame(dawn_sector::transition::FrameInput::lock_only(&[]));

        assert!(matches!(
            result,
            Err(RuntimeFrameHostError::Tick(
                dawn_sector::transit::TickTransitionError::Durable(_)
            ))
        ));
        assert_eq!(host.node().current_tick().value(), 0);
        assert_eq!(host.node().pending_events(), expected_events.as_slice());
        assert_eq!(host.phase(), RuntimeFramePhase::Fenced);
        assert!(host.health().is_fenced());
    }

    struct RecordingJournal {
        inner: InMemoryJournal,
        appended: std::sync::Arc<AtomicBool>,
    }

    impl RecordingJournal {
        fn new(appended: std::sync::Arc<AtomicBool>) -> Self {
            Self {
                inner: InMemoryJournal::new(),
                appended,
            }
        }

        fn records(&self) -> &[JournalRecord] {
            self.inner.records()
        }
    }

    impl DurableJournal for RecordingJournal {
        fn append_batch(&mut self, batch: JournalBatch) -> Result<AppendReceipt, JournalError> {
            let receipt = self.inner.append_batch(batch)?;
            self.appended.store(true, Ordering::SeqCst);
            Ok(receipt)
        }

        fn read_from(
            &self,
            index: JournalIndex,
        ) -> Result<Box<dyn Iterator<Item = Result<JournalRecord, JournalError>> + '_>, JournalError>
        {
            self.inner.read_from(index)
        }

        fn next_index(&self) -> Result<JournalIndex, JournalError> {
            self.inner.next_index()
        }
    }

    struct FailingJournal;

    impl DurableJournal for FailingJournal {
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
}
