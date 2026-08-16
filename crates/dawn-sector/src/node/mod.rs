//! `SimulationNode` — the composition root for one Sector's state owners.
//!
//! The authoritative engine is storage-independent. Every normal construction
//! and restore path requires a complete validated [`GameDataCatalog`].
//!
//! # Recovery contract status (ADR-0049 / #284)
//!
//! The node is now storage-independent: it owns no journal and never appends
//! its public output. The runtime prepares a transition, persists it through
//! its journal boundary, and then applies the committed delta to this node.
//! `restore_from()` restores only the state carried by the supplied snapshot;
//! the test-only `restore_from_test()` helper retains the old public-event
//! replay fixture for reducer coverage, not as a production recovery path.
//!
//! The accepted target is a storage-independent engine (#272) whose committed
//! Sector world state is recovered from a compatible versioned checkpoint plus
//! every contiguous committed authoritative `RecoveryDelta`. Public
//! `DomainEvent`s remain durable facts but are not the complete state reducer.
//! #277 separately owns durable pre-materialization admission/identity protocol
//! authority and reconciliation; #275 splits this broad aggregate into explicit
//! state owners.
//!
//! The composition root still wires the current SQLite adapter for compatibility
//! with existing station/admission APIs. That adapter is isolated in
//! `PersistenceBoundary`, outside the domain state owners and outside the durable
//! recovery authority selected by ADR-0049.

mod admission_provisional;
mod apply_event;
mod approach;
mod bot_ai;
mod command_module;
mod commands;
mod coordinates;
mod inventory;
mod jump;
mod movement_commands;
mod navigation;
mod orbit;
mod player_loadout_projection;
mod range_gate;
mod repositories;
mod sector_map;
mod serialization;
mod ship_cargo;
mod ship_command;
mod ship_registry;
mod snapshot_io;
mod spawner_logic;
mod state;
mod station;
mod station_inventory;
mod station_lifecycle;
mod station_materialization;
mod station_operation_execution;
mod tackle;
mod tick;
mod transit;
mod warp;

pub use crate::transit::StopTransitionError;
pub use crate::transit::TickTransitionError;
pub use command_module::ModuleActivationRejection;
pub use commands::{
    collect_runtime_commands, ClientCommandFollowup, ClientRequestAdmissionError,
    RuntimeCommandDispatch,
};
pub use jump::JumpOutcome;
pub use repositories::{
    ProjectionApplyError, ProjectionApplyResult, ProjectionReadError, StationProjectionMutation,
};
pub use serialization::{HandoffPayload, MissingObserverShip};
pub use tick::TickPreparationError;

use coordinates::debug_assert_missing_anchor;

use sector_map::SectorMap;
use ship_registry::ShipRegistry;
use state::{
    FrameOutputs, GameData, PersistenceBoundary, PlayerState, SectorTopology, SimulationState,
    StationState, TransitState,
};

use std::sync::Arc;

use dawn_core::MIN_WARP_DISTANCE;
use dawn_core::{
    DomainEvent, JumpGateDef, JumpGateId, NodeId, Position, SectorBounds, SectorId, ShipId,
    StationDef, StationId, Tick,
};
use dawn_ecs::{
    components::{PositionComp, ShipStatsComp, WarpComp},
    Entity, SimWorld,
};

#[cfg(test)]
use dawn_core::{ship_type::ShipTypeDefinition, ModuleDefinition, ModuleId};
#[cfg(test)]
use dawn_ecs::components::{CapacitorComp, FittingComp, HullComp};

use crate::game_data::GameDataCatalog;
use crate::persistence::StateSnapshot;
use crate::transit::handoff::TransitJournal;
use crate::view::SectorView;

/// Per-Sector population backstop (ADR-0018 final resort). Set far above the
/// TiDi budget so dynamic split / LoD / local TiDi all engage first; only
/// extreme density ever reaches this admission limit.
pub const POPULATION_CAP: usize = 100_000;

// -- Warp tuning (short-range Fold, ADR-0022 section 9) ---------------------

/// Warp engages once the ship is moving at this fraction of its max speed
/// toward the gate (EVE-style 75% alignment, ADR-0022). Align time therefore
/// emerges from ship agility (thrust / max_speed) - the tackle window
/// (ADR-0023) is longer for sluggish ships.
const WARP_ALIGN_FRACTION: f64 = 0.75;
/// Reference warp speed (units/tick), far above any sublight `max_speed`. Used
/// to derive the warp's duration: `total_ticks = max(WARP_MIN_TICKS,
/// ceil(warp_distance / WARP_SPEED))`. Warp then follows a smoothstep ease
/// along the start→arrival segment (ADR-0022 amendment), so this is the rough
/// peak speed, not a constant velocity.
///
/// Scaled by the same factor as `UNITS_PER_AU`'s true-AU reactivation
/// (galaxy::UNITS_PER_AU went from 200,000 to 1.495978707e11, a ×747,989.35
/// jump), so every warp's tick count — and therefore its felt duration —
/// is unchanged from the compressed scale (ADR-0029 true-AU reactivation).
const WARP_SPEED: f64 = 7_479_893_535.0;
/// Floor on warp duration (ticks) so even a short warp reads as a warp rather
/// than a blink. At 10 tick/s this is ~2 s.
const WARP_MIN_TICKS: u32 = 20;
/// Stop this far inside the gate's activation radius on arrival, so the jump
/// prompt is available immediately (mirrors approach, ADR-0015).
const WARP_ARRIVAL_FACTOR: f64 = 0.8;
/// Warp to a celestial body: arrive at this multiple of the body's radius from
/// its centre (ADR-0025). 1.5 = orbit insertion outside the body surface.
const BODY_WARP_ARRIVAL_FACTOR: f64 = 1.5;

// -- Orbit / Keep at Range tuning (ADR-0031) --------------------------------

/// Fallback orbit radius / keep-at-range distance (units) used when the ship
/// has no fitted weapon (`ShipStatsComp.weapon_range == 0.0`) and the command
/// did not specify one explicitly. Otherwise the default is the ship's own
/// weapon range, so the default distance is already a useful one to fight at.
const DEFAULT_MANEUVER_RADIUS: f64 = 5000.0;
/// Tangential lead distance for orbit steering, as a fraction of the orbit
/// radius (ADR-0031): the steering target is offset along the tangent by
/// `radius * ORBIT_LEAD_FACTOR` so the ship sweeps around the target instead
/// of just closing the radial gap. Larger = wider sweep, slower radius
/// convergence; smaller = tighter radial correction, less visible orbiting.
const ORBIT_LEAD_FACTOR: f64 = 0.5;
/// Keep at Range deadband, as a fraction of the chosen `range` (ADR-0031):
/// thrust briefly toggling between "too close" and "too far" every tick once
/// the ship settles near `range` would look like jitter, so brake instead
/// while within this band of the target distance.
const KEEP_AT_RANGE_DEADBAND_FRACTION: f64 = 0.05;

// -- TickResult --------------------------------------------------------------

/// Result returned after executing one tick.
#[derive(Debug)]
pub struct TickResult {
    /// The tick that was just completed.
    pub tick: Tick,
    /// Number of events emitted this tick.
    pub events_emitted: usize,
    /// The actual events produced (used by Actor layer for replication).
    pub events: Vec<DomainEvent>,
    /// Ships whose active module was force-deactivated by cap shortage this tick.
    pub cap_depletions: Vec<dawn_core::ShipId>,
    /// Outcome of each `FrameInput::market_settlements` entry admitted this
    /// tick, in input order (issue #315).
    pub market_settlement_outcomes: Vec<crate::transition::MarketSettlementOutcome>,
}

/// Test-facing snapshot of one fitted module's identity and activation state.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FittedModuleStatus {
    pub module_id: ModuleId,
    pub is_active: bool,
}

// -- SimulationNode ----------------------------------------------------------

/// Single-Sector composition root for the authoritative simulation engine.
///
/// The recovery journal is owned by the runtime boundary. The engine only
/// prepares and exposes public outputs; it never appends them to a journal
/// itself. The compatibility SQLite adapter is isolated in `PersistenceBoundary`
/// until the runtime-owned repository ports replace the legacy API.
pub struct SimulationNode {
    node_id: NodeId,
    sector_id: SectorId,
    bounds: SectorBounds,
    /// ECS, ship identity, tick, and bot cross-tick state.
    simulation: SimulationState,
    /// Player ownership and admission state.
    players: PlayerState,
    /// Station projection and docking state.
    stations: StationState,
    /// Composition-wired persistence adapter, kept outside all domain state owners.
    persistence: PersistenceBoundary,
    /// Transit attempts and receipts, restored from the #276 Saga snapshot.
    transit: TransitState,
    /// Static topology and anchor calculations.
    topology: SectorTopology,
    /// Validated immutable game data.
    game_data: GameData,
    /// Runtime-drained public and presentation outputs.
    frame_outputs: FrameOutputs,
}

impl std::fmt::Debug for SimulationNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimulationNode")
            .field("node_id", &self.node_id)
            .field("sector_id", &self.sector_id)
            .field("current_tick", &self.simulation.current_tick)
            .field("ship_count", &self.ship_count())
            .field("total_event_count", &self.total_event_count())
            .field("population_cap", &self.players.population_cap)
            .field("pending_auto_jumps", &self.frame_outputs.pending_auto_jumps)
            .field("completed_warps", &self.frame_outputs.completed_warps)
            .finish_non_exhaustive()
    }
}

impl SectorView for SimulationNode {
    fn ship_absolute_positions(&self) -> Vec<(ShipId, dawn_core::AbsolutePosition)> {
        SimulationNode::ship_absolute_positions(self)
    }

    fn ship_absolute_pos(&self, ship_id: ShipId) -> Option<dawn_core::AbsolutePosition> {
        SimulationNode::ship_absolute_pos(self, ship_id)
    }

    fn ship_state(&self, ship_id: ShipId) -> Option<dawn_protocol::ShipStateWire> {
        self.ship_state_json(ship_id)
    }

    fn ship_is_warping(&self, ship_id: ShipId) -> bool {
        SimulationNode::ship_is_warping(self, ship_id)
    }
}

// -- Constructors ------------------------------------------------------------

impl SimulationNode {
    /// Create an authoritative engine with no persistence side effects.
    pub fn new(
        node_id: NodeId,
        sector_id: SectorId,
        bounds: SectorBounds,
        galaxy: Arc<crate::galaxy::Galaxy>,
        catalog: Arc<GameDataCatalog>,
    ) -> Self {
        Self::with_catalog(node_id, sector_id, bounds, galaxy, catalog)
    }

    /// Test fixture constructor using the complete validated repository catalog.
    #[cfg(test)]
    pub(crate) fn new_test(
        node_id: NodeId,
        sector_id: SectorId,
        bounds: SectorBounds,
        galaxy: Arc<crate::galaxy::Galaxy>,
    ) -> Self {
        Self::new(
            node_id,
            sector_id,
            bounds,
            galaxy,
            crate::game_data::test_catalog_arc(),
        )
    }

    #[cfg(test)]
    pub(crate) fn with_test_store<T>(
        node_id: NodeId,
        sector_id: SectorId,
        bounds: SectorBounds,
        galaxy: Arc<crate::galaxy::Galaxy>,
        _store: T,
    ) -> Self {
        Self::new_test(node_id, sector_id, bounds, galaxy)
    }

    #[cfg(test)]
    pub(crate) fn restore_from_test<S: dawn_storage::EventStore>(
        store: S,
        snapshot: &StateSnapshot,
        galaxy: Arc<crate::galaxy::Galaxy>,
        modules: &[ModuleDefinition],
        ship_types: &[ShipTypeDefinition],
    ) -> Self {
        let catalog = crate::game_data::test_catalog_with_overrides(modules, ship_types);
        let mut node = Self::restore_from(snapshot, galaxy, catalog);
        let events: Vec<_> = store
            .iter_from(snapshot.covered_recovery_index)
            .map(|record| record.event.clone())
            .collect();
        for event in &events {
            node.apply_event(event);
        }
        node.frame_outputs.pending_events = store
            .iter_from(0)
            .map(|record| record.event.clone())
            .collect();
        node
    }
}

impl SimulationNode {
    fn with_catalog(
        node_id: NodeId,
        sector_id: SectorId,
        bounds: SectorBounds,
        galaxy: Arc<crate::galaxy::Galaxy>,
        catalog: Arc<GameDataCatalog>,
    ) -> Self {
        let sector_map = SectorMap::from_galaxy(sector_id, Arc::clone(&galaxy));
        let anchor_table = crate::anchor::AnchorTable::from_galaxy(&galaxy);

        Self {
            node_id,
            sector_id,
            bounds,
            simulation: SimulationState {
                world: SimWorld::new(sector_id),
                current_tick: Tick::ZERO,
                id_counter: 0,
                ships: ShipRegistry::new(),
                base_stats: std::collections::HashMap::new(),
                pending_bot_lock_commands: Vec::new(),
                applied_market_settlements: std::collections::HashSet::new(),
            },
            players: PlayerState {
                player_id_counter: 0,
                active_ship: std::collections::HashMap::new(),
                owners: std::collections::HashMap::new(),
                pending_fresh_admissions: std::collections::HashSet::new(),
                pending_resume_admissions: std::collections::HashMap::new(),
                population_cap: POPULATION_CAP,
            },
            stations: StationState::empty(),
            persistence: PersistenceBoundary::in_memory(),
            transit: TransitState {
                transit_attempt_counter: 0,
                transit_journal: TransitJournal::new(sector_id),
            },
            topology: SectorTopology {
                sector_map,
                anchor_table,
            },
            game_data: GameData {
                module_registry: catalog.module_index(),
                ship_type_registry: catalog.ship_type_index(),
                catalog_fingerprint: catalog.fingerprint(),
            },
            frame_outputs: FrameOutputs {
                pending_events: Vec::new(),
                pending_auto_jumps: Vec::new(),
                completed_warps: Vec::new(),
            },
        }
    }

    /// Restore the engine from a snapshot using the exact validated catalog
    /// selected by the runtime.
    pub fn restore_from(
        snapshot: &StateSnapshot,
        galaxy: Arc<crate::galaxy::Galaxy>,
        catalog: Arc<GameDataCatalog>,
    ) -> Self {
        Self::restore_from_checked(snapshot, galaxy, catalog)
            .expect("checkpoint catalog is incompatible with the runtime catalog")
    }

    /// Restore a checkpoint only when its catalog fingerprint matches the
    /// definitions supplied by the runtime.
    pub fn restore_from_checked(
        snapshot: &StateSnapshot,
        galaxy: Arc<crate::galaxy::Galaxy>,
        catalog: Arc<GameDataCatalog>,
    ) -> Result<Self, String> {
        let expected_fingerprint = catalog.fingerprint();
        if snapshot.catalog_fingerprint != expected_fingerprint {
            return Err(format!(
                "checkpoint catalog fingerprint {} does not match runtime catalog {}",
                snapshot.catalog_fingerprint, expected_fingerprint
            ));
        }
        let node = Self::with_catalog(
            snapshot.node_id,
            snapshot.sector_id,
            snapshot.bounds,
            galaxy,
            catalog,
        );
        let node = Self::finish_restore(node, snapshot);
        node.observe_materialized_identities()
            .map_err(|error| format!("cannot rebuild repository identity watermarks: {error}"))?;
        Ok(node)
    }

    /// Apply one committed authoritative RecoveryDelta during restart or
    /// replica promotion. Public events are intentionally not involved here:
    /// the delta is the exact state transition selected by ADR-0049.
    pub(crate) fn apply_recovery_delta(
        &mut self,
        delta: crate::transition::SectorRecoveryDelta,
        context: crate::transition::TransitionContext,
    ) -> Result<(), crate::transition::TransitionApplyError> {
        match delta {
            crate::transition::SectorRecoveryDelta::Stop(delta) => {
                self.apply_stop_transition(delta)
            }
            crate::transition::SectorRecoveryDelta::Tick(delta) => {
                self.apply_tick_transition(*delta, context)
            }
        }
    }

    fn finish_restore(mut node: Self, snapshot: &StateSnapshot) -> Self {
        node.apply_snapshot(snapshot);

        for ship in &snapshot.ships {
            node.restore_ship_from_snapshot(&ship.snapshot);
            node.apply_tick_ship_state(ship)
                .expect("checkpoint ship state must be internally consistent");
        }

        node.restore_transit_saga(snapshot.transit_saga.clone())
            .expect("checkpoint Transit Saga must match the restored Sector");

        node
    }

    // -- Population backstop (ADR-0018) --------------------------------------

    /// Whether the Sector is at its population backstop and should refuse new
    /// entrants. Last resort in the degradation hierarchy (ADR-0018): dynamic
    /// split, LoD, and local TiDi all engage before this admission limit.
    ///
    /// Keyed off the raw ship count, the same unit the TiDi budget uses
    /// (`DilationController`). An earlier "effective population" that excluded
    /// idle ships was dropped: by INV-MOVE a constant-velocity ship emits no
    /// events yet is fully present and bandwidth-bearing, so "no recent events"
    /// is not a sound idle signal. Reducing the cost of idle ships belongs in
    /// LoD (8B-3) as lowered fidelity, not in a count that pretends they are
    /// absent.
    pub(crate) fn at_population_cap(&self) -> bool {
        self.ship_count()
            .saturating_add(self.players.pending_fresh_admissions.len())
            >= self.players.population_cap
    }

    /// Override the per-Sector population backstop (default [`POPULATION_CAP`]).
    pub fn set_population_cap(&mut self, cap: usize) {
        self.players.population_cap = cap;
    }

    /// Point the current repository port at a real on-disk adapter instead of
    /// the private in-memory implementation used by `new`/`restore_from`.
    ///
    /// This is current implementation wiring. Under ADR-0049/#277 the final
    /// runtime owns separate repository APIs: Station rows are an idempotent
    /// projection of journal authority, while admission/identity rows may be
    /// durable protocol authority with explicit reconciliation. #272 removes the
    /// repository object from the pure engine.
    pub fn open_repositories(&mut self, path: &str) -> Result<(), String> {
        self.persistence = PersistenceBoundary::open(path)?;
        self.reconcile_runtime_repositories()?;
        Ok(())
    }

    /// Reconcile repository-owned admission and identity watermarks after a
    /// committed runtime transition and before its outputs are published.
    ///
    /// The runtime owns when this boundary runs; the repository owns the
    /// transaction and allocator invariants. Station projection mutation
    /// remains a separate idempotent projection port.
    pub(crate) fn reconcile_runtime_repositories(&mut self) -> Result<(), String> {
        self.observe_materialized_identities()?;
        self.reconcile_client_admission_identities()
    }

    fn observe_materialized_identities(&self) -> Result<(), String> {
        self.persistence
            .observe_materialized_identities(
                self.simulation.ships.index.keys().copied(),
                self.players
                    .owners
                    .values()
                    .copied()
                    .chain(self.players.active_ship.keys().copied())
                    .chain(self.stations.docked_player_ids()),
            )
            .map_err(|error| error.to_string())
    }

    /// Read access to the navigation topology.
    pub(crate) fn galaxy(&self) -> &crate::galaxy::Galaxy {
        &self.topology.sector_map.galaxy
    }

    // -- Identity ------------------------------------------------------------

    pub fn node_id(&self) -> NodeId {
        self.node_id
    }
    pub fn sector_id(&self) -> SectorId {
        self.sector_id
    }

    // -- Jump Gate Navigation (ADR-0009) -------------------------------------

    /// Look up a Jump Gate originating in this Sector by `gate_id`.
    pub fn jump_gate(&self, gate_id: JumpGateId) -> Option<&JumpGateDef> {
        self.topology.sector_map.gates.get(&gate_id)
    }

    /// Look up an NPC station in this Sector by `station_id`.
    pub fn station(&self, station_id: StationId) -> Option<&StationDef> {
        self.topology.sector_map.stations.get(&station_id)
    }

    // -- Observation ---------------------------------------------------------

    pub fn current_tick(&self) -> Tick {
        self.simulation.current_tick
    }
    pub fn ship_count(&self) -> usize {
        self.simulation.world.ship_count()
    }
    pub fn total_event_count(&self) -> usize {
        self.frame_outputs.pending_events.len()
    }

    /// Drain public events produced by the authoritative engine since the
    /// previous runtime boundary. This is the transition-output seam used by
    /// application adapters; it does not expose the legacy event log.
    pub fn drain_pending_events(&mut self) -> Vec<DomainEvent> {
        std::mem::take(&mut self.frame_outputs.pending_events)
    }

    pub fn pending_events(&self) -> &[DomainEvent] {
        &self.frame_outputs.pending_events
    }

    /// Put an output batch back at the front when an external durable append
    /// fails. This is a runtime rollback of the output buffer only; the
    /// authoritative ECS rollback is owned by the prepared transition.
    pub(crate) fn restore_pending_events(&mut self, mut events: Vec<DomainEvent>) {
        events.append(&mut self.frame_outputs.pending_events);
        self.frame_outputs.pending_events = events;
    }

    #[cfg(test)]
    pub(crate) fn pending_event_count(&self) -> usize {
        self.frame_outputs.pending_events.len()
    }

    pub(crate) fn transit_journal(&self) -> &TransitJournal {
        &self.transit.transit_journal
    }

    fn emit_event(&mut self, event: DomainEvent) {
        self.frame_outputs.pending_events.push(event);
    }

    fn emit_events<I>(&mut self, events: I)
    where
        I: IntoIterator<Item = DomainEvent>,
    {
        for event in events {
            self.emit_event(event);
        }
    }

    /// The Ship's current approach target, if any (ADR-0015).
    #[cfg(test)]
    pub(crate) fn approach_target(&self, ship_id: ShipId) -> Option<dawn_core::ApproachTarget> {
        let entity = self.simulation.ships.index.get(&ship_id)?;
        self.simulation
            .world
            .get::<dawn_ecs::components::ApproachComp>(*entity)
            .map(|a| a.target)
    }

    /// The Ship's current warp phase, if it is warping (ADR-0022).
    #[cfg(test)]
    pub(crate) fn warp_phase(&self, ship_id: ShipId) -> Option<dawn_ecs::components::WarpPhase> {
        let entity = self.simulation.ships.index.get(&ship_id)?;
        self.simulation
            .world
            .get::<WarpComp>(*entity)
            .map(|w| w.phase)
    }

    /// Look up the current position of a Ship by its ID.
    pub fn get_ship_position(&self, ship_id: ShipId) -> Option<Position> {
        let entity = self.simulation.ships.index.get(&ship_id)?;
        self.simulation
            .world
            .get::<PositionComp>(*entity)
            .map(|c| c.0)
    }

    /// The coordinate anchor a Ship's position is relative to (ADR-0029).
    #[cfg(test)]
    pub(crate) fn get_ship_anchor(&self, ship_id: ShipId) -> Option<dawn_core::AnchorId> {
        let entity = self.simulation.ships.index.get(&ship_id)?;
        self.simulation.world.ship_anchor(*entity)
    }

    /// Read access to this node's per-body anchor table (ADR-0029).
    #[cfg(test)]
    pub(crate) fn anchor_table(&self) -> &crate::anchor::AnchorTable {
        &self.topology.anchor_table
    }

    /// A Ship's absolute position in the Sector-local frame (metres, f64),
    /// composing its anchor's absolute position with its f64 offset (ADR-0029).
    /// Falls back to treating the raw offset as absolute if the anchor is
    /// unknown (pre-anchor data / tests).
    pub(crate) fn ship_absolute(&self, ship_id: ShipId) -> Option<dawn_core::AbsolutePosition> {
        let entity = *self.simulation.ships.index.get(&ship_id)?;
        let offset = self.simulation.world.get::<PositionComp>(entity)?.0;
        Some(self.entity_absolute_f64(entity, offset))
    }

    /// Whether a ship is in committed warp. AoI delivery uses this to keep
    /// normal-flight prediction corrections separate from warp authority.
    pub(crate) fn ship_is_warping(&self, ship_id: ShipId) -> bool {
        let Some(entity) = self.simulation.ships.index.get(&ship_id) else {
            return false;
        };
        self.simulation
            .world
            .get::<WarpComp>(*entity)
            .is_some_and(|warp| warp.is_warping())
    }

    /// Removes a ship entirely: despawns its ECS entity, clears identity and
    /// PlayerState ownership maps, and drops its `base_stats` entry. The single
    /// removal path is shared by combat death, `ShipDespawned` replay, and
    /// Sector Transit departure.
    pub(super) fn remove_ship(&mut self, ship_id: ShipId) {
        if self
            .simulation
            .ships
            .remove(ship_id, &mut self.simulation.world)
            .is_some()
        {
            if let Some(player_id) = self.players.owners.remove(&ship_id) {
                if self.players.active_ship.get(&player_id) == Some(&ship_id) {
                    self.players.active_ship.remove(&player_id);
                }
            }
        }
        self.simulation.base_stats.remove(&ship_id);
        self.stations.undock_ship(ship_id);
    }

    /// Recomputes `ShipStatsComp` from `ship_id`'s current `FittingComp`
    /// against its stored `base_stats` (falling back to `ShipStatsComp::NPC`
    /// if the ship has none, e.g. a bot). Callers still decide separately
    /// whether the fitting change also warrants a `ShipFitted` event (see
    /// `emit_ship_fitted`) — force-off paths (capacitor, Range Gate) must
    /// not emit one, so that stays their own call.
    pub(super) fn reapply_fitting(&mut self, ship_id: ShipId) {
        let base = self
            .simulation
            .base_stats
            .get(&ship_id)
            .copied()
            .unwrap_or(ShipStatsComp::NPC);
        dawn_ecs::systems::apply_fitting(&mut self.simulation.world, ship_id, base);
    }

    /// Snapshots `entity`'s current `FittingComp`/`InventoryComp` and appends
    /// a `ShipFitted` event — the "and tell the world" half of a fitting
    /// change, called after `reapply_fitting` (its "recompute stats" half)
    /// once the caller has decided the change warrants an event. Both Fit
    /// paths (the privileged/NPC path in `commands.rs::fit_module` and the
    /// owned path in `inventory.rs::fit_module_owned`/`unfit_module_owned`)
    /// used to duplicate this exact four-step tail (ADR-0032 §5: one event
    /// covers both sides of the move). Their *validation* still differs
    /// (ownership/inventory checks vs. none, M-8) — only the tail is shared.
    pub(super) fn emit_ship_fitted(&mut self, ship_id: ShipId, entity: Entity) {
        self.emit_ship_fitted_with_settlement(ship_id, entity, None);
    }

    pub(super) fn emit_ship_fitted_with_settlement(
        &mut self,
        ship_id: ShipId,
        entity: Entity,
        market_settlement_id: Option<u64>,
    ) {
        let fitting = self
            .simulation
            .world
            .get::<dawn_ecs::components::FittingComp>(entity)
            .map(|f| f.to_snapshot())
            .unwrap_or_else(dawn_core::FittingSnapshot::empty);
        let inventory = self
            .simulation
            .world
            .get::<dawn_ecs::components::InventoryComp>(entity)
            .map(|inv| inv.items.clone())
            .map(|items| {
                items
                    .into_iter()
                    .flat_map(|(item_id, count)| std::iter::repeat_n(item_id, count as usize))
                    .collect()
            })
            .unwrap_or_default();
        self.emit_event(DomainEvent::ShipFitted(dawn_core::events::ShipFitted {
            ship_id,
            fitting,
            inventory,
            market_settlement_id,
            tick: self.simulation.current_tick,
        }));
    }

    /// Look up the current `ShipStatsComp` of a Ship by its ID. Test-only.
    #[cfg(test)]
    pub(crate) fn get_ship_stats(&self, ship_id: ShipId) -> Option<ShipStatsComp> {
        let entity = self.simulation.ships.index.get(&ship_id)?;
        self.simulation
            .world
            .get::<ShipStatsComp>(*entity)
            .map(|c| *c)
    }

    /// Look up the current HP of a Ship by its ID. Test-only.
    #[cfg(test)]
    pub(crate) fn get_ship_hp(&self, ship_id: ShipId) -> Option<f32> {
        let entity = self.simulation.ships.index.get(&ship_id)?;
        self.simulation
            .world
            .get::<HullComp>(*entity)
            .map(|c| c.total_hp())
    }

    /// Look up the current `CapacitorComp.current` of a Ship by its ID.
    #[cfg(test)]
    pub(crate) fn get_ship_capacitor(&self, ship_id: ShipId) -> Option<f32> {
        let entity = self.simulation.ships.index.get(&ship_id)?;
        self.simulation
            .world
            .get::<CapacitorComp>(*entity)
            .map(|c| c.current)
    }

    /// Module identity and activation state for every fitted module on a Ship.
    #[cfg(test)]
    pub(crate) fn get_fitted_module_ids(&self, ship_id: ShipId) -> Vec<FittedModuleStatus> {
        let entity = match self.simulation.ships.index.get(&ship_id) {
            Some(&e) => e,
            None => return Vec::new(),
        };
        self.simulation
            .world
            .get::<FittingComp>(entity)
            .map(|f| {
                f.iter_slots()
                    .map(|s| FittedModuleStatus {
                        module_id: s.def.id,
                        is_active: s.is_active,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

// -- Tests -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::Velocity;
    use dawn_ecs::components::ThrustComp;

    fn mem_node() -> SimulationNode {
        SimulationNode::new_test(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        )
    }

    #[test]
    fn construction_builds_all_sector_projections_and_anchors_from_supplied_topology() {
        let sector_id = SectorId(7);
        let other_sector = SectorId(8);
        let local_body = dawn_core::CelestialBodyDef {
            id: dawn_core::CelestialBodyId(70),
            sector: sector_id,
            kind: dawn_core::CelestialBodyKind::Star,
            name: "Local Star".to_string(),
            position: Position::new(10.0, 20.0, 30.0),
            abs_m: dawn_core::AbsolutePosition::new(10.0, 20.0, 30.0),
            radius: 1000.0,
            spectral_type: 0.5,
        };
        let remote_body = dawn_core::CelestialBodyDef {
            id: dawn_core::CelestialBodyId(80),
            sector: other_sector,
            kind: dawn_core::CelestialBodyKind::Planet,
            name: "Remote Planet".to_string(),
            position: Position::new(40.0, 50.0, 60.0),
            abs_m: dawn_core::AbsolutePosition::new(40.0, 50.0, 60.0),
            radius: 500.0,
            spectral_type: 0.0,
        };
        let galaxy = Arc::new(crate::galaxy::Galaxy::new(
            vec![dawn_core::StarSystemDef {
                id: dawn_core::StarSystemId(7),
                name: "Replacement".to_string(),
                sectors: vec![sector_id, other_sector],
            }],
            vec![
                JumpGateDef {
                    id: JumpGateId(70),
                    from_sector: sector_id,
                    position: Position::new(100.0, 0.0, 0.0),
                    abs_m: dawn_core::AbsolutePosition::new(100.0, 0.0, 0.0),
                    to_sector: other_sector,
                    activation_radius: 2000.0,
                },
                JumpGateDef {
                    id: JumpGateId(80),
                    from_sector: other_sector,
                    position: Position::new(200.0, 0.0, 0.0),
                    abs_m: dawn_core::AbsolutePosition::new(200.0, 0.0, 0.0),
                    to_sector: sector_id,
                    activation_radius: 2000.0,
                },
            ],
            vec![local_body.clone(), remote_body.clone()],
            vec![
                StationDef {
                    id: StationId(70),
                    sector: sector_id,
                    name: "Local Station".to_string(),
                    position: Position::new(300.0, 0.0, 0.0),
                    abs_m: dawn_core::AbsolutePosition::new(300.0, 0.0, 0.0),
                    docking_radius: 1000.0,
                },
                StationDef {
                    id: StationId(80),
                    sector: other_sector,
                    name: "Remote Station".to_string(),
                    position: Position::new(400.0, 0.0, 0.0),
                    abs_m: dawn_core::AbsolutePosition::new(400.0, 0.0, 0.0),
                    docking_radius: 1000.0,
                },
            ],
        ));

        let node = SimulationNode::new_test(
            NodeId(7),
            sector_id,
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            Arc::clone(&galaxy),
        );

        assert!(Arc::ptr_eq(&node.topology.sector_map.galaxy, &galaxy));
        assert_eq!(
            node.topology.sector_map.gates,
            galaxy
                .gates_in_sector(sector_id)
                .into_iter()
                .map(|gate| (gate.id, gate))
                .collect()
        );
        assert_eq!(
            node.topology.sector_map.bodies,
            galaxy
                .bodies_in_sector(sector_id)
                .into_iter()
                .map(|body| (body.id, body))
                .collect()
        );
        assert_eq!(
            node.topology.sector_map.stations,
            galaxy
                .stations_in_sector(sector_id)
                .into_iter()
                .map(|station| (station.id, station))
                .collect()
        );

        for body in &galaxy.bodies {
            assert_eq!(
                node.topology
                    .anchor_table
                    .abs(dawn_core::AnchorId::from(body.id)),
                Some(body.abs_m)
            );
        }
        assert!(node
            .topology
            .anchor_table
            .abs(dawn_core::AnchorId(0))
            .is_none());

        let snapshot = node.take_snapshot();
        let restored = SimulationNode::restore_from_test(
            dawn_storage::InMemoryEventStore::new(),
            &snapshot,
            Arc::clone(&galaxy),
            &[],
            &[],
        );
        assert!(Arc::ptr_eq(&restored.topology.sector_map.galaxy, &galaxy));
        assert_eq!(
            restored.topology.sector_map.gates,
            node.topology.sector_map.gates
        );
        assert_eq!(
            restored.topology.sector_map.bodies,
            node.topology.sector_map.bodies
        );
        assert_eq!(
            restored.topology.sector_map.stations,
            node.topology.sector_map.stations
        );
        for body in &galaxy.bodies {
            assert_eq!(
                restored
                    .topology
                    .anchor_table
                    .abs(dawn_core::AnchorId::from(body.id)),
                Some(body.abs_m)
            );
        }
    }

    // -- Existing behaviour (unchanged) --------------------------------------

    #[test]
    fn spawning_a_ship_appends_a_ship_spawned_event() {
        let mut node = mem_node();
        node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);

        assert_eq!(node.total_event_count(), 1);
        assert!(matches!(
            node.pending_events()[0],
            DomainEvent::ShipSpawned(_)
        ));
    }

    #[test]
    fn spawned_ships_receive_unique_ids() {
        let mut node = mem_node();
        let id_a = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let id_b = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        assert_ne!(id_a, id_b);
    }

    #[test]
    fn adopt_player_ship_returns_false_for_ship_not_in_this_node() {
        let mut node = mem_node();
        let unknown = dawn_core::ShipId::new(NodeId(99), 0);
        assert!(!node.adopt_player_ship(unknown, dawn_core::PlayerId(0)));
        assert!(!node.apply_stop_command_owned(dawn_core::PlayerId(0), unknown));
    }

    #[test]
    fn move_command_is_ignored_while_ship_is_in_transit() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        node.set_player_ship(ship_id);

        let entity = *node.simulation.ships.index.get(&ship_id).unwrap();
        node.simulation.world.set_transit_state(
            entity,
            dawn_ecs::TransitState::InTransit { to: SectorId(1) },
        );

        node.apply_move_command(ship_id, Position::new(10000.0, 0.0, 0.0));
        let thrust = node.simulation.world.get::<ThrustComp>(entity).unwrap();
        assert_eq!(
            thrust.direction,
            Velocity::ZERO,
            "move command must be rejected while in transit"
        );
    }

    #[test]
    fn stop_command_is_ignored_while_ship_is_in_transit() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(100.0, 100.0, 100.0),
            Velocity::ZERO,
        );
        node.set_player_ship(ship_id);
        node.apply_move_command(ship_id, Position::new(10000.0, 0.0, 0.0));

        let entity = *node.simulation.ships.index.get(&ship_id).unwrap();
        let direction_before = node
            .simulation
            .world
            .get::<ThrustComp>(entity)
            .unwrap()
            .direction;

        node.simulation.world.set_transit_state(
            entity,
            dawn_ecs::TransitState::InTransit { to: SectorId(1) },
        );
        node.apply_stop_command(ship_id);

        let thrust = node.simulation.world.get::<ThrustComp>(entity).unwrap();
        assert_eq!(
            thrust.direction, direction_before,
            "stop command must be rejected while in transit"
        );
        assert!(
            !thrust.is_braking,
            "is_braking must not be set while in transit"
        );
    }

    #[test]
    fn total_event_count_grows_monotonically_across_ticks() {
        let mut node = mem_node();
        node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(100.0, 100.0, 100.0),
            Velocity::new(1.0, 1.0, 1.0),
        );
        let mut last = node.total_event_count();
        for _ in 0..10 {
            node.tick();
            assert!(node.total_event_count() >= last);
            last = node.total_event_count();
        }
    }

    #[test]
    fn replaying_events_reproduces_correct_spawn_count() {
        let mut node = mem_node();
        for i in 0..5 {
            node.spawn_ship(
                dawn_core::ShipTypeId(1),
                Position::new(i as f64 * 100.0, 0.0, 0.0),
                Velocity::new(1.0, 0.0, 0.0),
            );
        }
        node.tick();
        let spawned = node
            .pending_events()
            .iter()
            .filter(|event| matches!(event, DomainEvent::ShipSpawned(_)))
            .count();
        assert_eq!(spawned, 5);
    }

    // -- Population backstop (ADR-0018 / 8B-1) -------------------------------

    #[test]
    fn at_population_cap_is_true_only_when_ship_count_reaches_the_cap() {
        let mut node = mem_node();
        node.set_population_cap(2);
        assert!(!node.at_population_cap());
        node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        assert!(!node.at_population_cap());
        node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(100.0, 0.0, 0.0),
            Velocity::ZERO,
        );
        assert_eq!(node.ship_count(), 2);
        assert!(node.at_population_cap());
    }

    #[test]
    fn a_constant_velocity_ship_still_counts_against_the_population_cap() {
        // It emits no events (INV-MOVE) yet is present and bandwidth-bearing,
        // so the raw-count backstop must keep counting it across ticks.
        let mut node = mem_node();
        node.set_population_cap(1);
        node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::ORIGIN,
            Velocity::new(50.0, 0.0, 0.0),
        );
        assert!(node.at_population_cap());
        for _ in 0..500 {
            node.tick();
        }
        assert!(node.at_population_cap());
    }

    #[test]
    fn destroying_a_ship_frees_capacity_against_the_population_cap() {
        let mut node = mem_node();
        node.set_population_cap(1);
        let sid = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        assert!(node.at_population_cap());
        // Despawn drops the ship from the world, lowering the count.
        node.apply_event_pub(DomainEvent::ShipDespawned(
            dawn_core::events::ShipDespawned {
                ship_id: sid,
                tick: Tick::ZERO,
            },
        ));
        assert_eq!(node.ship_count(), 0);
        assert!(!node.at_population_cap());
    }
}
