//! Shared one-Sector runtime frame host.
//!
//! This module owns the mutable Sector state and the prepare -> durable ->
//! live-apply boundary. Server entry points only collect deployment-specific
//! inputs and consume the typed output returned by [`RuntimeFrameHost`].
use dawn_core::{
    DomainEvent, JumpGateId, PlayerId, Position, SectorId, ShipId, ShipTypeId, Velocity,
};
use dawn_distributed::RaftActorHandle;
use dawn_sector::client_admission::{
    ClientAdmissionAttempt, ClientAdmissionIntent, ClientAdmissionRefusal,
};
use dawn_sector::client_admission_resolution::{
    resolve_client_admission, ClientAdmissionResolution,
};
use dawn_sector::node::{ProjectionReadError, SimulationNode};
use dawn_sector::persistence::checkpoint::CheckpointJournal;
use dawn_sector::persistence::{CheckpointScheduler, StateSnapshot};
use dawn_sector::transit::{
    reconcile_runtime_repositories, run_durable_runtime_frame, DurableRuntimeTickContext,
    LocalRuntimeDurabilityPolicy, RuntimeConsensus, RuntimeDurabilityPolicy,
    RuntimeDurabilityProfile, RuntimeHealth, RuntimeTickOutput, TransitOp,
};
use dawn_storage::{DurabilityMode, DurableJournal};
use std::fmt;
use tokio::sync::mpsc;

/// Owned Raft adapter used by a long-lived one-Sector Host.
pub struct OwnedRaftRuntimeConsensus {
    raft: RaftActorHandle,
    committed_rx: mpsc::UnboundedReceiver<Vec<u8>>,
}

impl OwnedRaftRuntimeConsensus {
    /// Bind a Raft actor to the committed-entry receiver owned by this host.
    pub fn new(raft: RaftActorHandle, committed_rx: mpsc::UnboundedReceiver<Vec<u8>>) -> Self {
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

impl fmt::Debug for OwnedRaftRuntimeConsensus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnedRaftRuntimeConsensus")
            .finish_non_exhaustive()
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
pub struct RuntimeFramePolicy<P = LocalRuntimeDurabilityPolicy> {
    owner_epoch: u64,
    durability: DurabilityMode,
    profile: RuntimeDurabilityProfile,
    durability_policy: P,
}

impl RuntimeFramePolicy<LocalRuntimeDurabilityPolicy> {
    /// Select synchronized local durability for one owner epoch.
    pub const fn local_durable(owner_epoch: u64) -> Self {
        Self::new(
            owner_epoch,
            DurabilityMode::Synced,
            RuntimeDurabilityProfile::LocalDurable,
            LocalRuntimeDurabilityPolicy,
        )
    }
}

impl<P> RuntimeFramePolicy<P> {
    const fn new(
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
enum RuntimeFramePhase {
    Bootstrapping,
    Running,
    Fenced,
}

/// Errors that cross the runtime-host boundary.
#[derive(Debug)]
pub enum RuntimeFrameHostError {
    /// A fixture or genesis mutation was attempted after frame processing began.
    BootstrapClosed,
    /// The host rejected mutation after an authoritative frame failure.
    Fenced,
    /// The durable runtime transition failed.
    Tick(dawn_sector::transit::TickTransitionError),
    /// A required Station projection read failed; the host is fenced.
    StationProjectionRead(ProjectionReadError),
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
            Self::Fenced => write!(
                formatter,
                "runtime host is fenced and requires recovery before further mutation"
            ),
            Self::Tick(error) => write!(formatter, "runtime frame failed: {error}"),
            Self::StationProjectionRead(error) => {
                write!(formatter, "Station projection read failed: {error}")
            }
        }
    }
}

impl std::error::Error for RuntimeFrameHostError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::BootstrapClosed | Self::Fenced => None,
            Self::Tick(error) => Some(error),
            Self::StationProjectionRead(error) => Some(error),
        }
    }
}

/// Failure to begin admission through a runtime-owned Sector.
#[derive(Debug)]
pub enum RuntimeClientAdmissionError {
    /// The runtime host could not safely accept admission work.
    Host(RuntimeFrameHostError),
    /// Sector admission policy refused the requested identity or ship.
    Refused(ClientAdmissionRefusal),
}

impl fmt::Display for RuntimeClientAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Host(error) => write!(formatter, "client admission unavailable: {error}"),
            Self::Refused(refusal) => write!(formatter, "client admission refused: {refusal}"),
        }
    }
}

impl std::error::Error for RuntimeClientAdmissionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Host(error) => Some(error),
            Self::Refused(refusal) => Some(refusal),
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
/// than retaining a mutable borrow across frame boundaries. Admission and
/// checkpoint access are narrow typed operations guarded by the host phase.
pub struct RuntimeFrameHost<J, C, P = LocalRuntimeDurabilityPolicy> {
    node: SimulationNode,
    journal: J,
    consensus: C,
    health: RuntimeHealth,
    policy: RuntimeFramePolicy<P>,
    phase: RuntimeFramePhase,
}

impl<J, C, P> fmt::Debug for RuntimeFrameHost<J, C, P> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeFrameHost")
            .field("node_id", &self.node.node_id())
            .field("sector_id", &self.node.sector_id())
            .field("phase", &self.phase)
            .finish_non_exhaustive()
    }
}

/// Read-only access required by server presentation and routing adapters.
pub trait RuntimeNodeView {
    /// Return the authoritative Sector node without exposing mutable access.
    fn runtime_node(&self) -> &SimulationNode;
}

/// Narrow admission port used by the socket adapter. Admission owns a durable
/// protocol repository, while this port owns the Sector-specific validation
/// and materialisation calls without exposing arbitrary node mutation.
pub trait RuntimeClientAdmissionHost {
    /// Return the topology-derived spawn point for a fresh player.
    fn default_player_spawn_position(&self) -> Position;
    /// Validate and reserve one client admission attempt.
    fn begin_client_admission(
        &mut self,
        intent: ClientAdmissionIntent,
        aoi_cell_size: f64,
    ) -> Result<ClientAdmissionAttempt, RuntimeClientAdmissionError>;
    /// Commit or abort an admission after its asynchronous handoff completes.
    fn resolve_client_admission<T>(
        &mut self,
        attempt: ClientAdmissionAttempt,
        result: Result<T, String>,
    ) -> Result<ClientAdmissionResolution<T, String>, RuntimeFrameHostError>;
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

impl<J, C, P> RuntimeClientAdmissionHost for RuntimeFrameHost<J, C, P>
where
    J: DurableJournal,
    C: RuntimeConsensus,
    P: RuntimeDurabilityPolicy,
{
    fn default_player_spawn_position(&self) -> Position {
        self.node.default_player_spawn_position()
    }

    fn begin_client_admission(
        &mut self,
        intent: ClientAdmissionIntent,
        aoi_cell_size: f64,
    ) -> Result<ClientAdmissionAttempt, RuntimeClientAdmissionError> {
        RuntimeFrameHost::begin_client_admission(self, intent, aoi_cell_size)
    }

    fn resolve_client_admission<T>(
        &mut self,
        attempt: ClientAdmissionAttempt,
        result: Result<T, String>,
    ) -> Result<ClientAdmissionResolution<T, String>, RuntimeFrameHostError> {
        RuntimeFrameHost::resolve_client_admission(self, attempt, result)
    }
}

impl<J, C, P> RuntimeFrameHost<J, C, P>
where
    J: DurableJournal,
    C: RuntimeConsensus,
    P: RuntimeDurabilityPolicy,
{
    /// Assemble one Sector from its authoritative node and deployment adapters.
    pub fn new(
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

    /// Return the owned Sector node for read-only routing and presentation.
    pub fn node(&self) -> &SimulationNode {
        &self.node
    }

    #[cfg(test)]
    fn phase(&self) -> RuntimeFramePhase {
        self.phase
    }

    #[cfg(test)]
    fn health(&self) -> &RuntimeHealth {
        &self.health
    }

    /// Whether this host has fenced itself after a failed authoritative frame.
    pub fn is_fenced(&self) -> bool {
        self.phase == RuntimeFramePhase::Fenced
    }

    fn ensure_mutation_available(&self) -> Result<(), RuntimeFrameHostError> {
        if self.phase == RuntimeFramePhase::Fenced {
            Err(RuntimeFrameHostError::Fenced)
        } else {
            Ok(())
        }
    }

    fn fence(&mut self, reason: impl Into<String>) {
        self.health.fence(reason);
        self.phase = RuntimeFramePhase::Fenced;
    }

    fn ensure_bootstrapping(&self) -> Result<(), RuntimeFrameHostError> {
        self.ensure_mutation_available()?;
        if self.phase == RuntimeFramePhase::Bootstrapping {
            Ok(())
        } else {
            Err(RuntimeFrameHostError::BootstrapClosed)
        }
    }

    /// Create initial world state before this host starts processing frames.
    #[cfg(test)]
    fn bootstrap_ship(
        &mut self,
        ship_type_id: ShipTypeId,
        position: Position,
        velocity: Velocity,
    ) -> Result<ShipId, RuntimeFrameHostError> {
        self.ensure_bootstrapping()?;
        Ok(self.node.spawn_ship(ship_type_id, position, velocity))
    }

    /// Drain bootstrap events while respecting the host fencing gate.
    pub fn drain_pending_events(&mut self) -> Result<Vec<DomainEvent>, RuntimeFrameHostError> {
        self.ensure_mutation_available()?;
        Ok(self.node.drain_pending_events())
    }

    /// Seed initial NPC frigates into the Sector.
    ///
    /// Fixture/composition-root use only (initial server population), not a
    /// per-tick operation.
    pub fn spawn_npc_frigates(&mut self, count: usize) -> Result<(), RuntimeFrameHostError> {
        self.ensure_bootstrapping()?;
        self.node.spawn_npc_frigates(count);
        Ok(())
    }

    /// Spawn a duel-mode Bot ship and its owning player identity.
    pub fn spawn_bot_ship(
        &mut self,
        position: Position,
    ) -> Result<(PlayerId, ShipId), RuntimeFrameHostError> {
        self.ensure_bootstrapping()?;
        Ok(self.node.spawn_bot_ship(position))
    }

    /// The Sector's topology-derived default spawn point for a fresh player.
    pub fn default_player_spawn_position(&self) -> Position {
        self.node.default_player_spawn_position()
    }

    /// Begin a client admission attempt against the owned node.
    ///
    /// Admission identity reservation is durable through its own
    /// repository-backed transaction (see
    /// `dawn_sector::node::admission_provisional`), independent of the tick
    /// pipeline, so this narrow bridge is admission's typed entry point
    /// rather than a correctness gap.
    pub fn begin_client_admission(
        &mut self,
        intent: ClientAdmissionIntent,
        aoi_cell_size: f64,
    ) -> Result<ClientAdmissionAttempt, RuntimeClientAdmissionError> {
        self.ensure_mutation_available()
            .map_err(RuntimeClientAdmissionError::Host)?;
        match self.node.begin_client_admission(intent, aoi_cell_size) {
            Ok(attempt) => Ok(attempt),
            Err(ClientAdmissionRefusal::StationProjectionRead(error)) => {
                self.fence(format!(
                    "Station projection read failed during client admission: {error}"
                ));
                Err(RuntimeClientAdmissionError::Host(
                    RuntimeFrameHostError::StationProjectionRead(error),
                ))
            }
            Err(refusal) => Err(RuntimeClientAdmissionError::Refused(refusal)),
        }
    }

    /// Commit or abort an admission after its asynchronous handoff completes.
    pub fn resolve_client_admission<T>(
        &mut self,
        attempt: ClientAdmissionAttempt,
        result: Result<T, String>,
    ) -> Result<ClientAdmissionResolution<T, String>, RuntimeFrameHostError> {
        self.ensure_mutation_available()?;
        Ok(resolve_client_admission(&mut self.node, attempt, result))
    }

    /// Adopt a ship that just jumped into this Sector under `player_id`'s
    /// ownership. Returns `false` if the ship is not (yet) present in this
    /// Sector's ECS.
    pub fn adopt_player_ship(
        &mut self,
        ship_id: ShipId,
        player_id: PlayerId,
    ) -> Result<bool, RuntimeFrameHostError> {
        self.ensure_mutation_available()?;
        Ok(self.node.adopt_player_ship(ship_id, player_id))
    }

    /// Spawn a ship outside the tick pipeline, for deterministic fixture
    /// setup (test/demo actor callers of `SectorRuntimeDriver`). Production
    /// runtime mutations must enter through `run_frame` once a frame has
    /// started; this is not that path.
    pub fn spawn_fixture_ship(
        &mut self,
        ship_type_id: ShipTypeId,
        position: Position,
        velocity: Velocity,
    ) -> Result<ShipId, RuntimeFrameHostError> {
        self.ensure_bootstrapping()?;
        Ok(self.node.spawn_ship(ship_type_id, position, velocity))
    }

    /// Propose a validated cross-Sector request through the owned consensus
    /// adapter. Callers do the admission check; the Host owns the proposal.
    pub fn propose_transit_request(
        &mut self,
        ship_id: ShipId,
        to: SectorId,
        gate_id: Option<JumpGateId>,
    ) -> Result<(), RuntimeFrameHostError> {
        self.ensure_mutation_available()?;
        self.consensus.propose(TransitOp::Request {
            ship_id,
            to,
            gate_id,
        });
        Ok(())
    }

    /// Take and publish a checkpoint through the explicit node/journal
    /// boundary. The scheduler never receives a closure that can mutate an
    /// arbitrary runtime state.
    pub fn checkpoint(
        &mut self,
        scheduler: &mut CheckpointScheduler,
        public_event_next_index: dawn_storage::PublicEventIndex,
    ) -> Result<Option<StateSnapshot>, std::io::Error>
    where
        J: CheckpointJournal,
    {
        self.ensure_mutation_available()
            .map_err(std::io::Error::other)?;
        scheduler.maybe_checkpoint(&mut self.node, &mut self.journal, public_event_next_index)
    }

    /// Run exactly one authoritative frame for this Sector.
    pub fn run_frame(
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
    pub fn run_frame_with_output<F>(
        &mut self,
        input: dawn_sector::transition::FrameInput<'_>,
        after_commit: F,
    ) -> Result<RuntimeTickOutput, RuntimeFrameHostError>
    where
        F: FnOnce(&SimulationNode, &dawn_sector::node::TickResult, &[DomainEvent]),
    {
        self.run_frame_with_reconcile(input, reconcile_runtime_repositories, after_commit)
    }

    #[cfg(test)]
    fn run_frame_with_reconciliation<R, F>(
        &mut self,
        input: dawn_sector::transition::FrameInput<'_>,
        reconcile: R,
        after_commit: F,
    ) -> Result<RuntimeTickOutput, RuntimeFrameHostError>
    where
        R: FnOnce(
            &mut SimulationNode,
            &dawn_sector::node::TickResult,
            &[DomainEvent],
        ) -> Result<(), dawn_sector::transit::RuntimeReconciliationError>,
        F: FnOnce(&SimulationNode, &dawn_sector::node::TickResult, &[DomainEvent]),
    {
        self.run_frame_with_reconcile(input, reconcile, after_commit)
    }

    fn run_frame_with_reconcile<R, F>(
        &mut self,
        input: dawn_sector::transition::FrameInput<'_>,
        reconcile: R,
        after_commit: F,
    ) -> Result<RuntimeTickOutput, RuntimeFrameHostError>
    where
        R: FnOnce(
            &mut SimulationNode,
            &dawn_sector::node::TickResult,
            &[DomainEvent],
        ) -> Result<(), dawn_sector::transit::RuntimeReconciliationError>,
        F: FnOnce(&SimulationNode, &dawn_sector::node::TickResult, &[DomainEvent]),
    {
        self.ensure_mutation_available()?;
        self.phase = RuntimeFramePhase::Running;

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
            reconcile,
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
    /// Query the owned Raft actor without exposing its mailbox.
    pub fn raft_role(
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
    use dawn_core::{ClientRequest, NodeId, SectorBounds, SectorId, StationId};
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

    fn temporary_station_repository(
        host: &mut RuntimeFrameHost<InMemoryJournal, dawn_sector::transit::LocalRuntimeConsensus>,
    ) -> (tempfile::TempDir, std::path::PathBuf) {
        let directory = tempfile::tempdir().expect("temporary projection directory");
        let path = directory.path().join("station.sqlite");
        host.node
            .open_repositories(path.to_str().expect("temporary path is UTF-8"))
            .expect("temporary Station repository should open");
        (directory, path)
    }

    fn remove_station_inventory_table(path: &std::path::Path) {
        rusqlite::Connection::open(path)
            .expect("temporary Station repository should reopen")
            .execute("DROP TABLE station_inventory", [])
            .expect("Station projection table should be droppable in the test");
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
    fn fixture_spawns_are_closed_after_the_first_frame() {
        let mut host = host();
        host.spawn_fixture_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO)
            .expect("fixture spawn should be available during bootstrap");

        host.run_frame(dawn_sector::transition::FrameInput::lock_only(&[]))
            .expect("local frame should commit");
        let ship_count = host.node().ship_count();

        assert!(matches!(
            host.spawn_fixture_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO),
            Err(RuntimeFrameHostError::BootstrapClosed)
        ));
        assert!(matches!(
            host.spawn_npc_frigates(1),
            Err(RuntimeFrameHostError::BootstrapClosed)
        ));
        assert!(matches!(
            host.spawn_bot_ship(Position::ORIGIN),
            Err(RuntimeFrameHostError::BootstrapClosed)
        ));
        assert_eq!(host.node().ship_count(), ship_count);
    }

    #[test]
    fn client_admission_remains_available_while_running() {
        let mut host = host();
        host.run_frame(dawn_sector::transition::FrameInput::lock_only(&[]))
            .expect("local frame should commit");

        let attempt = host
            .begin_client_admission(
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::ORIGIN,
                },
                1_000.0,
            )
            .expect("healthy running hosts should accept admission attempts");
        assert_eq!(host.node().ship_count(), 0);

        let resolution = host
            .resolve_client_admission(attempt, Ok::<(), String>(()))
            .expect("healthy running hosts should resolve admission attempts");

        assert_eq!(host.phase(), RuntimeFramePhase::Running);
        assert_eq!(host.node().ship_count(), 1);
        assert!(matches!(
            resolution,
            dawn_sector::client_admission_resolution::ClientAdmissionResolution::Committed { .. }
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
    fn station_projection_read_failure_fences_host_and_rejects_next_frame() {
        let mut host = host();
        let (_directory, path) = temporary_station_repository(&mut host);
        let spawn_position = host.default_player_spawn_position();
        let (player_id, ship_id) = host
            .spawn_bot_ship(spawn_position)
            .expect("bot fixture should be available during bootstrap");
        remove_station_inventory_table(&path);
        let requests = [
            dawn_sector::transition::AuthenticatedClientRequest {
                session_index: 0,
                player_id,
                request: ClientRequest::Dock {
                    station: StationId(0),
                },
            },
            dawn_sector::transition::AuthenticatedClientRequest {
                session_index: 0,
                player_id,
                request: ClientRequest::BuildPackagedShip {
                    ship: ship_id,
                    station: StationId(0),
                    ship_type: ShipTypeId(1),
                },
            },
        ];

        let result = host.run_frame(dawn_sector::transition::FrameInput {
            lock_commands: &[],
            authenticated_requests: &requests,
            market_settlements: &[],
            acknowledged_settlements: &[],
        });

        assert!(matches!(
            result,
            Err(RuntimeFrameHostError::Tick(
                dawn_sector::transit::TickTransitionError::Preparation(
                    dawn_sector::node::TickPreparationError::StationProjectionRead(_)
                )
            ))
        ));
        assert_eq!(host.phase(), RuntimeFramePhase::Fenced);
        assert!(host.health().is_fenced());
        assert_eq!(host.node().docked_station(ship_id), None);
        assert_eq!(host.node().player_docked_station(player_id), None);
        assert!(matches!(
            host.run_frame(dawn_sector::transition::FrameInput::lock_only(&[])),
            Err(RuntimeFrameHostError::Fenced)
        ));
        assert!(matches!(
            host.begin_client_admission(
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::ORIGIN,
                },
                1_000.0,
            ),
            Err(RuntimeClientAdmissionError::Host(
                RuntimeFrameHostError::Fenced
            ))
        ));
    }

    #[test]
    fn station_projection_read_failure_during_resume_fences_host_and_keeps_ship() {
        let mut host = host();
        let (_directory, path) = temporary_station_repository(&mut host);
        let spawn_position = host.default_player_spawn_position();
        let fresh = host
            .begin_client_admission(ClientAdmissionIntent::Fresh { spawn_position }, 1_000.0)
            .expect("fresh admission should begin");
        let resume_ticket = fresh.resume_ticket();
        let committed = host
            .resolve_client_admission(fresh, Ok::<(), String>(()))
            .expect("fresh admission should resolve");
        assert!(matches!(
            &committed,
            dawn_sector::client_admission_resolution::ClientAdmissionResolution::Committed { .. }
        ));
        let committed_admission = match committed {
            dawn_sector::client_admission_resolution::ClientAdmissionResolution::Committed {
                admission,
                ..
            } => admission,
            _ => unreachable!("fresh admission was asserted committed above"),
        };
        host.run_frame(dawn_sector::transition::FrameInput {
            lock_commands: &[],
            authenticated_requests: &[dawn_sector::transition::AuthenticatedClientRequest {
                session_index: 0,
                player_id: committed_admission.player_id,
                request: ClientRequest::Dock {
                    station: StationId(0),
                },
            }],
            market_settlements: &[],
            acknowledged_settlements: &[],
        })
        .expect("fresh admission projection should commit");

        let ship_count = host.node().ship_count();
        remove_station_inventory_table(&path);
        let result =
            host.begin_client_admission(ClientAdmissionIntent::Resume { resume_ticket }, 1_000.0);

        assert!(matches!(
            result,
            Err(RuntimeClientAdmissionError::Host(
                RuntimeFrameHostError::StationProjectionRead(_)
            ))
        ));
        assert_eq!(host.node().ship_count(), ship_count);
        assert!(host.node().hosts_client_resume_ticket(resume_ticket));
        assert_eq!(host.phase(), RuntimeFramePhase::Fenced);
        assert!(matches!(
            host.begin_client_admission(ClientAdmissionIntent::Resume { resume_ticket }, 1_000.0,),
            Err(RuntimeClientAdmissionError::Host(
                RuntimeFrameHostError::Fenced
            ))
        ));
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
        let ship_id = host
            .bootstrap_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO)
            .expect("bootstrap should be available");
        let diagnostic_spawn_position = host.default_player_spawn_position();
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

        assert!(matches!(
            host.begin_client_admission(
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::ORIGIN,
                },
                1_000.0,
            ),
            Err(RuntimeClientAdmissionError::Host(
                RuntimeFrameHostError::Fenced
            ))
        ));
        assert!(matches!(
            host.adopt_player_ship(ship_id, PlayerId(9)),
            Err(RuntimeFrameHostError::Fenced)
        ));
        assert!(matches!(
            host.bootstrap_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO),
            Err(RuntimeFrameHostError::Fenced)
        ));
        assert!(matches!(
            host.spawn_npc_frigates(1),
            Err(RuntimeFrameHostError::Fenced)
        ));
        assert!(matches!(
            host.spawn_bot_ship(Position::ORIGIN),
            Err(RuntimeFrameHostError::Fenced)
        ));
        assert!(matches!(
            host.spawn_fixture_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO),
            Err(RuntimeFrameHostError::Fenced)
        ));
        assert!(matches!(
            host.drain_pending_events(),
            Err(RuntimeFrameHostError::Fenced)
        ));

        assert!(matches!(
            host.propose_transit_request(ship_id, SectorId(1), None),
            Err(RuntimeFrameHostError::Fenced)
        ));

        assert_eq!(host.node().pending_events(), expected_events.as_slice());
        assert_eq!(host.node().ship_count(), 1);
        assert_eq!(
            host.default_player_spawn_position(),
            diagnostic_spawn_position
        );
    }

    #[test]
    fn fenced_host_rejects_generic_admission_port_without_mutation() {
        let mut host = host_with_journal(FailingJournal, RuntimeFramePolicy::local_durable(0));
        let attempt = RuntimeClientAdmissionHost::begin_client_admission(
            &mut host,
            ClientAdmissionIntent::Fresh {
                spawn_position: Position::ORIGIN,
            },
            1_000.0,
        )
        .expect("admission attempt should be reservable before fencing");
        let result = host.run_frame(dawn_sector::transition::FrameInput::lock_only(&[]));
        assert!(matches!(
            result,
            Err(RuntimeFrameHostError::Tick(
                dawn_sector::transit::TickTransitionError::Durable(_)
            ))
        ));

        let ship_count = host.node().ship_count();
        let pending_events = host.node().pending_events().to_vec();
        assert!(matches!(
            RuntimeClientAdmissionHost::begin_client_admission(
                &mut host,
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::ORIGIN,
                },
                1_000.0,
            ),
            Err(RuntimeClientAdmissionError::Host(
                RuntimeFrameHostError::Fenced
            ))
        ));
        assert!(matches!(
            RuntimeClientAdmissionHost::resolve_client_admission::<()>(
                &mut host,
                attempt,
                Err("client disconnected".to_owned()),
            ),
            Err(RuntimeFrameHostError::Fenced)
        ));
        assert_eq!(host.node().ship_count(), ship_count);
        assert_eq!(host.node().pending_events(), pending_events.as_slice());
    }

    #[test]
    fn authenticated_dispatches_are_not_exposed_when_durable_append_fails() {
        let mut host = host_with_journal(FailingJournal, RuntimeFramePolicy::local_durable(0));
        let (player_id, _ship_id) = host
            .spawn_bot_ship(Position::ORIGIN)
            .expect("bot fixture should be available during bootstrap");
        let requests = [dawn_sector::transition::AuthenticatedClientRequest {
            session_index: 4,
            player_id,
            request: ClientRequest::Jump {
                gate: JumpGateId(0),
            },
        }];
        let output_seen = Arc::new(AtomicBool::new(false));

        let result = host.run_frame_with_output(
            dawn_sector::transition::FrameInput {
                lock_commands: &[],
                authenticated_requests: &requests,
                market_settlements: &[],
                acknowledged_settlements: &[],
            },
            {
                let output_seen = Arc::clone(&output_seen);
                move |_, _, _| {
                    output_seen.store(true, Ordering::SeqCst);
                }
            },
        );

        assert!(matches!(
            result,
            Err(RuntimeFrameHostError::Tick(
                dawn_sector::transit::TickTransitionError::Durable(_)
            ))
        ));
        assert!(!output_seen.load(Ordering::SeqCst));
        assert_eq!(host.node().current_tick().value(), 0);
        assert_eq!(host.node().ship_count(), 1);
        assert_eq!(host.phase(), RuntimeFramePhase::Fenced);
        assert!(host.health().is_fenced());
    }

    #[test]
    fn committed_frame_exposes_jump_refresh_and_rejection_dispatches() {
        let mut host = host();
        let (player_id, ship_id) = host
            .spawn_bot_ship(Position::ORIGIN)
            .expect("bot fixture should be available during bootstrap");
        let requests = [
            dawn_sector::transition::AuthenticatedClientRequest {
                session_index: 2,
                player_id,
                request: ClientRequest::SelectActiveShip { ship: ship_id },
            },
            dawn_sector::transition::AuthenticatedClientRequest {
                session_index: 2,
                player_id,
                request: ClientRequest::Jump {
                    gate: JumpGateId(0),
                },
            },
            dawn_sector::transition::AuthenticatedClientRequest {
                session_index: 2,
                player_id,
                request: ClientRequest::Attack {
                    target: dawn_core::ShipId::new(NodeId(0), 999),
                },
            },
        ];

        let output = host
            .run_frame(dawn_sector::transition::FrameInput {
                lock_commands: &[],
                authenticated_requests: &requests,
                market_settlements: &[],
                acknowledged_settlements: &[],
            })
            .expect("authenticated requests should be returned after commit");

        assert_eq!(output.tick_result.runtime_command_dispatches.len(), 3);
        assert!(matches!(
            output.tick_result.runtime_command_dispatches[0],
            dawn_sector::node::RuntimeCommandDispatch::RefreshPlayerLoadout {
                session_index: 2,
                player_id: id,
            } if id == player_id
        ));
        assert!(matches!(
            output.tick_result.runtime_command_dispatches[1],
            dawn_sector::node::RuntimeCommandDispatch::Jump {
                session_index: 2,
                ship_id: id,
                ..
            } if id == ship_id
        ));
        assert!(matches!(
            output.tick_result.runtime_command_dispatches[2],
            dawn_sector::node::RuntimeCommandDispatch::Rejected {
                session_index: 2,
                error: dawn_sector::node::ClientRequestAdmissionError::UnsupportedRequest {
                    request: "Attack"
                },
            }
        ));
    }

    #[test]
    fn reconciliation_failure_fences_after_durable_apply_without_output_or_ack() {
        let mut host = host();
        let (player_id, _) = host
            .spawn_bot_ship(Position::ORIGIN)
            .expect("bot fixture should be available during bootstrap");
        let journal_records_before = host.journal.records().len();
        let requests = [dawn_sector::transition::AuthenticatedClientRequest {
            session_index: 7,
            player_id,
            request: ClientRequest::Attack {
                target: dawn_core::ShipId::new(NodeId(0), 999),
            },
        }];
        let output_seen = Arc::new(AtomicBool::new(false));

        let result = host.run_frame_with_reconciliation(
            dawn_sector::transition::FrameInput {
                lock_commands: &[],
                authenticated_requests: &requests,
                market_settlements: &[],
                acknowledged_settlements: &[],
            },
            |_, _, _| {
                Err(
                    dawn_sector::transit::RuntimeReconciliationError::Repository {
                        reason: "injected reconciliation failure".to_owned(),
                    },
                )
            },
            {
                let output_seen = Arc::clone(&output_seen);
                move |_, _, _| {
                    output_seen.store(true, Ordering::SeqCst);
                }
            },
        );

        assert!(matches!(
            result,
            Err(RuntimeFrameHostError::Tick(
                dawn_sector::transit::TickTransitionError::Reconciliation(
                    dawn_sector::transit::RuntimeReconciliationError::Repository { .. }
                )
            ))
        ));
        assert!(!output_seen.load(Ordering::SeqCst));
        assert!(
            host.journal.records().len() > journal_records_before,
            "the committed transition must be durable even when reconciliation fails"
        );
        assert_eq!(host.node().current_tick().value(), 1);
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
