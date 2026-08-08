//! `SimulationNode` — the current broad simulation/composition unit for one Sector.
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
//! Until those migrations land, comments below distinguish current in-memory or
//! SQLite adapter mechanics from the authority selected by ADR-0049.

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
mod sector_map;
mod serialization;
mod ship_cargo;
mod ship_command;
mod ship_registry;
mod snapshot_io;
mod spawner_logic;
mod station;
mod station_inventory;
mod station_inventory_db;
mod station_lifecycle;
mod station_materialization;
mod station_operation_execution;
mod tackle;
mod tick;
mod transit;
mod warp;

pub use command_module::ModuleActivationRejection;
pub use commands::{ClientCommandFollowup, ClientRequestAdmissionError};
pub use jump::JumpOutcome;
pub use movement_commands::StopTransitionError;
pub use serialization::{HandoffPayload, MissingObserverShip};
pub use tick::TickTransitionError;

use coordinates::debug_assert_missing_anchor;

use sector_map::SectorMap;
use ship_registry::ShipRegistry;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use dawn_core::MIN_WARP_DISTANCE;
use dawn_core::{
    ship_type::{ShipTypeDefinition, ShipTypeId},
    DomainEvent, JumpGateDef, JumpGateId, ModuleDefinition, ModuleId, NodeId, PlayerId, Position,
    SectorBounds, SectorId, ShipId, StationDef, StationId, Tick,
};
use dawn_ecs::{
    components::{PositionComp, ShipStatsComp, WarpComp},
    Entity, SimWorld,
};

#[cfg(test)]
use dawn_ecs::components::{CapacitorComp, FittingComp, HullComp};

use crate::game_data::GameDataCatalog;
use crate::persistence::{CompletedIncomingTransit, StateSnapshot};
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
}

/// Test-facing snapshot of one fitted module's identity and activation state.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FittedModuleStatus {
    pub module_id: ModuleId,
    pub is_active: bool,
}

// -- SimulationNode ----------------------------------------------------------

/// Current single-Sector authoritative simulation engine.
///
/// Persistence is owned by the runtime boundary. The engine only prepares and
/// exposes public outputs; it never appends them to a journal itself.
pub struct SimulationNode {
    node_id: NodeId,
    sector_id: SectorId,
    bounds: SectorBounds,
    world: SimWorld,
    /// Public events produced by state-changing operations since the last
    /// runtime drain.
    pending_events: Vec<DomainEvent>,
    current_tick: Tick,
    id_counter: u64,
    /// Ship identity and ownership maps (entity index, type ids, player ownership).
    ships: ShipRegistry,
    /// Immutable module definitions shared from the validated catalog.
    module_registry: Arc<BTreeMap<ModuleId, ModuleDefinition>>,
    /// Immutable ship-type definitions shared from the validated catalog.
    ship_type_registry: Arc<BTreeMap<ShipTypeId, ShipTypeDefinition>>,
    /// Bare ShipStats without fitting. Used as the base for fitting aggregation.
    base_stats: HashMap<ShipId, ShipStatsComp>,
    /// Current in-memory PlayerId allocation counter. ADR-0049/#277 requires
    /// recovery to advance allocation beyond every materialized or durably
    /// reserved identity; this field alone is not the final allocator authority.
    player_id_counter: u64,
    /// In-memory claim set for fresh admissions currently being processed.
    ///
    /// The **claim set itself** is non-durable and intentionally omitted from
    /// snapshots. The reserved `PlayerId` / `ShipId` is different: once exposed
    /// by a durable #277 reservation it is permanently consumed and may not be
    /// reused after crash, abort, or expiry.
    pending_fresh_admissions: HashSet<ShipId>,
    /// Ship-level lock held by an in-flight resume handshake.
    /// Non-durable concurrency guard: losing this lock on crash does not change
    /// durable ownership/ticket authority, which #277 must recover/reconcile.
    pending_resume_admissions: HashMap<ShipId, PlayerId>,
    /// Lock-on commands queued by the bot AI during `process_bots()`.
    ///
    /// Bot AI runs after the LockSystem each tick. These commands are held here
    /// and injected into the LockSystem at the start of the NEXT tick. Because
    /// they affect a later authoritative Tick, ADR-0049 classifies this queue as
    /// recovery authority until same-Tick consumption or another redesign removes
    /// the cross-Tick state. Current persistence is migration debt.
    pending_bot_lock_commands: Vec<dawn_core::LockOnCommand>,
    /// Static navigation topology for this Sector (gates, bodies, star map).
    sector_map: SectorMap,
    /// Per-body coordinate anchors (ADR-0029): absolute Sector-local positions
    /// in f64, derived from the topology supplied at construction.
    anchor_table: crate::anchor::AnchorTable,
    /// Per-Sector population backstop (ADR-0018). Defaults to [`POPULATION_CAP`];
    /// tunable via [`Self::set_population_cap`].
    population_cap: usize,
    /// Current catch-all SQLite adapter inherited from ADR-0038.
    ///
    /// Under ADR-0049/#277, Station inventory rows become an idempotent
    /// projection/read model of journal-owned Station world state, while
    /// pre-materialization admission/identity rows may remain repository-owned
    /// protocol authority with explicit reconciliation. The current object mixes
    /// both domains and is scheduled to be split; SQLite as a whole is **not**
    /// the Sector world-state authority.
    station_inventory_db: station_inventory_db::StationInventoryDb,
    /// Bounded derived cache over current Station repository access. It is not
    /// independent recovery authority.
    station_inventory_cache: std::cell::RefCell<station_inventory::StationInventoryCache>,
    /// Current docked station per ship. Docking is authoritative state, so
    /// station operations must consult this rather than raw spatial proximity.
    docked_ships: BTreeMap<ShipId, StationId>,
    /// Current docked station context per player. This is separate from the
    /// active ship map so station access can survive ship-specific actions.
    docked_players: BTreeMap<PlayerId, StationId>,
    /// Current in-memory auto-jump work queue populated by `process_warp()` and
    /// drained by the runtime to propose a Raft handoff.
    ///
    /// ADR-0049 explicitly forbids this queue from being the sole durable source
    /// after an auto-jump Warp arrival commits. The committed transition must
    /// also create durable replayable/idempotent continuation state; #276 may
    /// represent that as a Transit Saga attempt.
    pending_auto_jumps: Vec<(ShipId, JumpGateId)>,
    /// Ships that finished a warp this tick (ADR-0029 warp-arrival authority).
    /// This is deliberately lossy presentation output: the serve loop drains it
    /// and sends an authoritative `PositionSnap`; reconnect/current-state sync can
    /// repair a missed presentation correction. It is not the durable auto-jump
    /// obligation described above.
    completed_warps: Vec<ShipId>,
    /// Current destination-side Transit receipt representation used for Commit
    /// deduplication. #276 replaces this legacy snapshot-era shape with the final
    /// durable Transit Saga/receipt authority under ADR-0049.
    completed_incoming_transits: Vec<CompletedIncomingTransit>,
    /// In-memory transit policy projection maintained from the engine's own
    /// event output. Runtime persistence is deliberately outside this type.
    transit_journal: TransitJournal,
}

impl std::fmt::Debug for SimulationNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimulationNode")
            .field("node_id", &self.node_id)
            .field("sector_id", &self.sector_id)
            .field("current_tick", &self.current_tick)
            .field("ship_count", &self.ship_count())
            .field("total_event_count", &self.total_event_count())
            .field("population_cap", &self.population_cap)
            .field("pending_auto_jumps", &self.pending_auto_jumps)
            .field("completed_warps", &self.completed_warps)
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

    fn ship_state(&self, ship_id: ShipId) -> Option<dawn_wire::ShipStateWire> {
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
    pub(crate) fn restore_from_test<S: dawn_event_store::EventStore>(
        store: S,
        snapshot: &StateSnapshot,
        galaxy: Arc<crate::galaxy::Galaxy>,
        modules: &[ModuleDefinition],
        ship_types: &[ShipTypeDefinition],
    ) -> Self {
        let catalog = crate::game_data::test_catalog_with_overrides(modules, ship_types);
        let mut node = Self::restore_from(snapshot, galaxy, catalog);
        let events: Vec<_> = store
            .iter_from(snapshot.log_index)
            .map(|record| record.event.clone())
            .collect();
        for event in &events {
            node.apply_event(event);
        }
        node.pending_events = store
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
            world: SimWorld::new(sector_id),
            pending_events: Vec::new(),
            current_tick: Tick::ZERO,
            id_counter: 0,
            ships: ShipRegistry::new(),
            module_registry: catalog.module_index(),
            ship_type_registry: catalog.ship_type_index(),
            base_stats: HashMap::new(),
            player_id_counter: 0,
            pending_fresh_admissions: HashSet::new(),
            pending_resume_admissions: HashMap::new(),
            pending_bot_lock_commands: Vec::new(),
            sector_map,
            anchor_table,
            population_cap: POPULATION_CAP,
            station_inventory_db: station_inventory_db::StationInventoryDb::open_in_memory()
                .expect("in-memory sqlite connection never fails to open"),
            station_inventory_cache: std::cell::RefCell::new(
                station_inventory::StationInventoryCache::new(),
            ),
            docked_ships: BTreeMap::new(),
            docked_players: BTreeMap::new(),
            pending_auto_jumps: Vec::new(),
            completed_warps: Vec::new(),
            completed_incoming_transits: Vec::new(),
            transit_journal: TransitJournal::new(sector_id),
        }
    }

    /// Restore the engine from a snapshot using the exact validated catalog
    /// selected by the runtime.
    pub fn restore_from(
        snapshot: &StateSnapshot,
        galaxy: Arc<crate::galaxy::Galaxy>,
        catalog: Arc<GameDataCatalog>,
    ) -> Self {
        let node = Self::with_catalog(
            snapshot.node_id,
            snapshot.sector_id,
            snapshot.bounds,
            galaxy,
            catalog,
        );
        Self::finish_restore(node, snapshot)
    }

    fn finish_restore(mut node: Self, snapshot: &StateSnapshot) -> Self {
        node.apply_snapshot(snapshot);

        for ship in &snapshot.ships {
            node.restore_ship_from_snapshot(ship);
        }

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
    pub fn at_population_cap(&self) -> bool {
        self.ship_count()
            .saturating_add(self.pending_fresh_admissions.len())
            >= self.population_cap
    }

    /// Override the per-Sector population backstop (default [`POPULATION_CAP`]).
    pub fn set_population_cap(&mut self, cap: usize) {
        self.population_cap = cap;
    }

    /// Point the current catch-all Station/admission/identity SQLite adapter at
    /// a real on-disk file instead of the private in-memory database used by
    /// `new`/`restore_from`.
    ///
    /// This is current implementation wiring. Under ADR-0049/#277 the final
    /// runtime owns separate repository APIs: Station rows are an idempotent
    /// projection of journal authority, while admission/identity rows may be
    /// durable protocol authority with explicit reconciliation. #272 removes the
    /// repository object from the pure engine.
    pub fn open_station_inventory_db(&mut self, path: &str) -> rusqlite::Result<()> {
        self.station_inventory_db = station_inventory_db::StationInventoryDb::open(path)?;
        self.station_inventory_cache
            .replace(station_inventory::StationInventoryCache::new());
        self.reconcile_client_admission_grants()?;
        Ok(())
    }

    /// Read access to the navigation topology.
    pub fn galaxy(&self) -> &crate::galaxy::Galaxy {
        &self.sector_map.galaxy
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
        self.sector_map.gates.get(&gate_id)
    }

    /// Look up an NPC station in this Sector by `station_id`.
    pub fn station(&self, station_id: StationId) -> Option<&StationDef> {
        self.sector_map.stations.get(&station_id)
    }

    // -- Observation ---------------------------------------------------------

    pub fn current_tick(&self) -> Tick {
        self.current_tick
    }
    pub fn ship_count(&self) -> usize {
        self.world.ship_count()
    }
    pub fn total_event_count(&self) -> usize {
        self.pending_events.len()
    }

    /// Drain public events produced by the authoritative engine since the
    /// previous runtime boundary. This is the transition-output seam used by
    /// application adapters; it does not expose the legacy event log.
    pub fn drain_pending_events(&mut self) -> Vec<DomainEvent> {
        std::mem::take(&mut self.pending_events)
    }

    pub fn pending_events(&self) -> &[DomainEvent] {
        &self.pending_events
    }

    /// Put an output batch back at the front when an external durable append
    /// fails. This is a runtime rollback of the output buffer only; the
    /// authoritative ECS rollback is owned by the prepared transition.
    pub(crate) fn restore_pending_events(&mut self, mut events: Vec<DomainEvent>) {
        events.append(&mut self.pending_events);
        self.pending_events = events;
    }

    pub fn pending_event_count(&self) -> usize {
        self.pending_events.len()
    }

    pub(crate) fn transit_journal(&self) -> &TransitJournal {
        &self.transit_journal
    }

    fn emit_event(&mut self, event: DomainEvent) {
        self.transit_journal.observe(&event);
        self.pending_events.push(event);
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
    pub fn approach_target(&self, ship_id: ShipId) -> Option<dawn_core::ApproachTarget> {
        let entity = self.ships.index.get(&ship_id)?;
        self.world
            .get::<dawn_ecs::components::ApproachComp>(*entity)
            .map(|a| a.target)
    }

    /// The Ship's current warp phase, if it is warping (ADR-0022).
    #[cfg(test)]
    pub fn warp_phase(&self, ship_id: ShipId) -> Option<dawn_ecs::components::WarpPhase> {
        let entity = self.ships.index.get(&ship_id)?;
        self.world.get::<WarpComp>(*entity).map(|w| w.phase)
    }

    /// Look up the current position of a Ship by its ID.
    pub fn get_ship_position(&self, ship_id: ShipId) -> Option<Position> {
        let entity = self.ships.index.get(&ship_id)?;
        self.world.get::<PositionComp>(*entity).map(|c| c.0)
    }

    /// The coordinate anchor a Ship's position is relative to (ADR-0029).
    pub fn get_ship_anchor(&self, ship_id: ShipId) -> Option<dawn_core::AnchorId> {
        let entity = self.ships.index.get(&ship_id)?;
        self.world.ship_anchor(*entity)
    }

    /// Read access to this node's per-body anchor table (ADR-0029).
    pub fn anchor_table(&self) -> &crate::anchor::AnchorTable {
        &self.anchor_table
    }

    /// A Ship's absolute position in the Sector-local frame (metres, f64),
    /// composing its anchor's absolute position with its f64 offset (ADR-0029).
    /// Falls back to treating the raw offset as absolute if the anchor is
    /// unknown (pre-anchor data / tests).
    pub fn ship_absolute(&self, ship_id: ShipId) -> Option<dawn_core::AbsolutePosition> {
        let entity = *self.ships.index.get(&ship_id)?;
        let offset = self.world.get::<PositionComp>(entity)?.0;
        Some(self.entity_absolute_f64(entity, offset))
    }

    /// Whether a ship is in committed warp. AoI delivery uses this to keep
    /// normal-flight prediction corrections separate from warp authority.
    pub(crate) fn ship_is_warping(&self, ship_id: ShipId) -> bool {
        let Some(entity) = self.ships.index.get(&ship_id) else {
            return false;
        };
        self.world
            .get::<WarpComp>(*entity)
            .is_some_and(|warp| warp.is_warping())
    }

    /// Removes a ship entirely: despawns its ECS entity, clears it from every
    /// `ShipRegistry` map (`ShipRegistry::remove`), and drops its `base_stats`
    /// entry. The single removal path for combat death, `ShipDespawned`
    /// replay, and Sector Transit departure — each used to hand-roll this
    /// sequence, and one (Transit) forgot the ownership maps entirely.
    pub(super) fn remove_ship(&mut self, ship_id: ShipId) {
        self.ships.remove(ship_id, &mut self.world);
        self.base_stats.remove(&ship_id);
        self.docked_ships.remove(&ship_id);
    }

    /// Recomputes `ShipStatsComp` from `ship_id`'s current `FittingComp`
    /// against its stored `base_stats` (falling back to `ShipStatsComp::NPC`
    /// if the ship has none, e.g. a bot). Callers still decide separately
    /// whether the fitting change also warrants a `ShipFitted` event (see
    /// `emit_ship_fitted`) — force-off paths (capacitor, Range Gate) must
    /// not emit one, so that stays their own call.
    pub(super) fn reapply_fitting(&mut self, ship_id: ShipId) {
        let base = self
            .base_stats
            .get(&ship_id)
            .copied()
            .unwrap_or(ShipStatsComp::NPC);
        dawn_ecs::systems::apply_fitting(&mut self.world, ship_id, base);
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
        let fitting = self
            .world
            .get::<dawn_ecs::components::FittingComp>(entity)
            .map(|f| f.to_snapshot())
            .unwrap_or_else(dawn_core::FittingSnapshot::empty);
        let inventory = self
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
            tick: self.current_tick,
        }));
    }

    /// Look up the current `ShipStatsComp` of a Ship by its ID. Test-only.
    #[cfg(test)]
    pub fn get_ship_stats(&self, ship_id: ShipId) -> Option<ShipStatsComp> {
        let entity = self.ships.index.get(&ship_id)?;
        self.world.get::<ShipStatsComp>(*entity).map(|c| *c)
    }

    /// Look up the current HP of a Ship by its ID. Test-only.
    #[cfg(test)]
    pub fn get_ship_hp(&self, ship_id: ShipId) -> Option<f32> {
        let entity = self.ships.index.get(&ship_id)?;
        self.world.get::<HullComp>(*entity).map(|c| c.total_hp())
    }

    /// Look up the current `CapacitorComp.current` of a Ship by its ID.
    #[cfg(test)]
    pub fn get_ship_capacitor(&self, ship_id: ShipId) -> Option<f32> {
        let entity = self.ships.index.get(&ship_id)?;
        self.world.get::<CapacitorComp>(*entity).map(|c| c.current)
    }

    /// Module identity and activation state for every fitted module on a Ship.
    #[cfg(test)]
    pub fn get_fitted_module_ids(&self, ship_id: ShipId) -> Vec<FittedModuleStatus> {
        let entity = match self.ships.index.get(&ship_id) {
            Some(&e) => e,
            None => return Vec::new(),
        };
        self.world
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

        assert!(Arc::ptr_eq(&node.sector_map.galaxy, &galaxy));
        assert_eq!(
            node.sector_map.gates,
            galaxy
                .gates_in_sector(sector_id)
                .into_iter()
                .map(|gate| (gate.id, gate))
                .collect()
        );
        assert_eq!(
            node.sector_map.bodies,
            galaxy
                .bodies_in_sector(sector_id)
                .into_iter()
                .map(|body| (body.id, body))
                .collect()
        );
        assert_eq!(
            node.sector_map.stations,
            galaxy
                .stations_in_sector(sector_id)
                .into_iter()
                .map(|station| (station.id, station))
                .collect()
        );

        for body in &galaxy.bodies {
            assert_eq!(
                node.anchor_table.abs(dawn_core::AnchorId::from(body.id)),
                Some(body.abs_m)
            );
        }
        assert!(node.anchor_table.abs(dawn_core::AnchorId(0)).is_none());

        let snapshot = node.take_snapshot();
        let restored = SimulationNode::restore_from_test(
            dawn_event_store::InMemoryEventStore::new(),
            &snapshot,
            Arc::clone(&galaxy),
            &[],
            &[],
        );
        assert!(Arc::ptr_eq(&restored.sector_map.galaxy, &galaxy));
        assert_eq!(restored.sector_map.gates, node.sector_map.gates);
        assert_eq!(restored.sector_map.bodies, node.sector_map.bodies);
        assert_eq!(restored.sector_map.stations, node.sector_map.stations);
        for body in &galaxy.bodies {
            assert_eq!(
                restored
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

        let entity = *node.ships.index.get(&ship_id).unwrap();
        node.world.set_transit_state(
            entity,
            dawn_ecs::TransitState::InTransit { to: SectorId(1) },
        );

        node.apply_move_command(ship_id, Position::new(10000.0, 0.0, 0.0));
        let thrust = node.world.get::<ThrustComp>(entity).unwrap();
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

        let entity = *node.ships.index.get(&ship_id).unwrap();
        let direction_before = node.world.get::<ThrustComp>(entity).unwrap().direction;

        node.world.set_transit_state(
            entity,
            dawn_ecs::TransitState::InTransit { to: SectorId(1) },
        );
        node.apply_stop_command(ship_id);

        let thrust = node.world.get::<ThrustComp>(entity).unwrap();
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
