//! `SimulationNode` — the self-contained simulation unit for one Sector.
//!
//! # Generic over `S: EventStore`
//!
//! `SimulationNode<S>` defaults to `SimulationNode<InMemoryEventStore>` so all
//! existing call sites continue to compile unchanged.  Pass a `FileEventStore`
//! to persist events to disk (Phase 3).
//!
//! # Snapshot / Restore (INV-002)
//!
//! ```text
//! node.take_snapshot()           -> StateSnapshot (ECS state at log_index N)
//! SimulationNode::restore_from(store, &snapshot, &modules, &ship_types)
//!     -> reconstruct ECS from snapshot, replay events from log_index N onward
//! ```

mod apply_event;
mod approach;
mod commands;
mod inventory;
mod navigation;
mod orbit;
mod sector_map;
mod serialization;
mod ship_registry;
mod snapshot_io;
mod spawner_logic;
mod tackle;
mod tick;
mod transit_flow;
mod warp;

use sector_map::SectorMap;
use ship_registry::ShipRegistry;

use std::collections::HashMap;
use std::sync::Arc;

use dawn_core::{
    ship_type::{ShipTypeDefinition, ShipTypeId},
    DomainEvent, JumpGateDef, JumpGateId, ModuleDefinition, ModuleId, NodeId, Position,
    SectorBounds, SectorId, ShipId, Tick,
};
use dawn_ecs::{
    components::{PositionComp, ShipStatsComp},
    Entity, SimWorld,
};
use dawn_event_store::{store::EventStore, InMemoryEventStore};

#[cfg(test)]
use dawn_ecs::components::{CapacitorComp, FittingComp, HullComp, WarpComp};

use crate::persistence::StateSnapshot;

/// Per-Sector population backstop (ADR-0018 final resort). Set far above the
/// TiDi budget so dynamic split / LoD / local TiDi all engage first; only
/// extreme density ever reaches this admission limit.
pub const POPULATION_CAP: usize = 100_000;

// -- Warp tuning (short-range Fold, ADR-0022 section 9) ---------------------

/// Warp engages once the ship is moving at this fraction of its max speed
/// toward the gate (EVE-style 75% alignment, ADR-0022). Align time therefore
/// emerges from ship agility (thrust / max_speed) - the tackle window
/// (ADR-0023) is longer for sluggish ships.
const WARP_ALIGN_FRACTION: f32 = 0.75;
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
const WARP_SPEED: f32 = 7_479_893_535.0;
/// Floor on warp duration (ticks) so even a short warp reads as a warp rather
/// than a blink. At 10 tick/s this is ~2 s.
const WARP_MIN_TICKS: u32 = 20;
/// Minimum distance to a gate for warp to be allowed (units). Closer than this,
/// the `WarpCommand` is rejected and the player should approach instead.
const MIN_WARP_DISTANCE: f32 = 3000.0;
/// Stop this far inside the gate's activation radius on arrival, so the jump
/// prompt is available immediately (mirrors approach, ADR-0015).
const WARP_ARRIVAL_FACTOR: f32 = 0.8;
/// Warp to a celestial body: arrive at this multiple of the body's radius from
/// its centre (ADR-0025). 1.5 = orbit insertion outside the body surface.
const BODY_WARP_ARRIVAL_FACTOR: f32 = 1.5;

// -- Orbit / Keep at Range tuning (ADR-0031) --------------------------------

/// Fallback orbit radius / keep-at-range distance (units) used when the ship
/// has no fitted weapon (`ShipStatsComp.weapon_range == 0.0`) and the command
/// did not specify one explicitly. Otherwise the default is the ship's own
/// weapon range, so the default distance is already a useful one to fight at.
const DEFAULT_MANEUVER_RADIUS: f32 = 5000.0;
/// Tangential lead distance for orbit steering, as a fraction of the orbit
/// radius (ADR-0031): the steering target is offset along the tangent by
/// `radius * ORBIT_LEAD_FACTOR` so the ship sweeps around the target instead
/// of just closing the radial gap. Larger = wider sweep, slower radius
/// convergence; smaller = tighter radial correction, less visible orbiting.
const ORBIT_LEAD_FACTOR: f32 = 0.5;
/// Keep at Range deadband, as a fraction of the chosen `range` (ADR-0031):
/// thrust briefly toggling between "too close" and "too far" every tick once
/// the ship settles near `range` would look like jitter, so brake instead
/// while within this band of the target distance.
const KEEP_AT_RANGE_DEADBAND_FRACTION: f32 = 0.05;

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

// -- SimulationNode ----------------------------------------------------------

/// A single-Sector simulation node, generic over its event store.
///
/// The default store is `InMemoryEventStore`; use `FileEventStore` for
/// persistent operation (Phase 3).
pub struct SimulationNode<S = InMemoryEventStore>
where
    S: EventStore,
{
    node_id: NodeId,
    sector_id: SectorId,
    bounds: SectorBounds,
    world: SimWorld,
    event_store: S,
    current_tick: Tick,
    id_counter: u64,
    /// Ship identity and ownership maps (entity index, type ids, player ownership).
    ships: ShipRegistry,
    /// Module definition registry.
    module_registry: HashMap<ModuleId, ModuleDefinition>,
    /// Ship type definition registry.
    ship_type_registry: HashMap<ShipTypeId, ShipTypeDefinition>,
    /// Bare ShipStats without fitting. Used as the base for fitting aggregation.
    base_stats: HashMap<ShipId, ShipStatsComp>,
    /// PlayerId allocation counter.
    player_id_counter: u64,
    /// Lock-on commands queued by the bot AI during `process_bots()`.
    ///
    /// Bot AI runs after the LockSystem each tick.  These commands are held
    /// here and injected into the LockSystem at the start of the NEXT tick,
    /// ensuring they are processed exactly like human-issued lock commands.
    pending_bot_lock_commands: Vec<dawn_core::LockOnCommand>,
    /// Static navigation topology for this Sector (gates, bodies, star map).
    sector_map: SectorMap,
    /// Per-body coordinate anchors (ADR-0029): absolute Sector-local positions
    /// in f64, derived from `sector_map.galaxy`. Rebuilt on `set_galaxy`.
    anchor_table: crate::anchor::AnchorTable,
    /// Per-Sector population backstop (ADR-0018). Defaults to [`POPULATION_CAP`];
    /// tunable via [`Self::set_population_cap`].
    population_cap: usize,
    /// Auto-jump triggers accumulated during `process_warp()` for ships that
    /// completed a warp with `WarpComp::auto_jump = true`. Drained by the
    /// caller after each tick so the jump can be proposed to the Raft Log
    /// (or handled however the server path requires).
    pending_auto_jumps: Vec<(ShipId, JumpGateId)>,
    /// Ships that finished a warp this tick (ADR-0029 warp-arrival authority).
    /// Transient and non-persisted (like `pending_auto_jumps`): the serve loop
    /// drains it each tick and sends the owner an authoritative `PositionSnap`,
    /// correcting the client's capped warp-visual dead-reckoning. Independent of
    /// whether the arrival changed the ship's anchor, so it covers every warp
    /// (gate / body / same-anchor) with one mechanism.
    completed_warps: Vec<ShipId>,
}

// -- Constructors ------------------------------------------------------------

impl SimulationNode<InMemoryEventStore> {
    /// Create a node backed by an in-memory event store (Phase 0 default).
    pub fn new(node_id: NodeId, sector_id: SectorId, bounds: SectorBounds) -> Self {
        Self::with_store(node_id, sector_id, bounds, InMemoryEventStore::new())
    }
}

impl<S: EventStore> SimulationNode<S> {
    /// Create a node with a caller-supplied event store.
    ///
    /// Use this with `FileEventStore` for persistent operation.
    pub fn with_store(
        node_id: NodeId,
        sector_id: SectorId,
        bounds: SectorBounds,
        store: S,
    ) -> Self {
        Self {
            node_id,
            sector_id,
            bounds,
            world: SimWorld::new(sector_id),
            event_store: store,
            current_tick: Tick::ZERO,
            id_counter: 0,
            ships: ShipRegistry::new(),
            module_registry: HashMap::new(),
            ship_type_registry: HashMap::new(),
            base_stats: HashMap::new(),
            player_id_counter: 0,
            pending_bot_lock_commands: Vec::new(),
            sector_map: {
                let sm = Arc::new(crate::galaxy::Galaxy::demo());
                SectorMap {
                    gates: sm
                        .gates_in_sector(sector_id)
                        .into_iter()
                        .map(|g| (g.id, g))
                        .collect(),
                    bodies: sm
                        .bodies_in_sector(sector_id)
                        .into_iter()
                        .map(|b| (b.id, b))
                        .collect(),
                    galaxy: sm,
                }
            },
            anchor_table: crate::anchor::AnchorTable::from_galaxy(&crate::galaxy::Galaxy::demo()),
            population_cap: POPULATION_CAP,
            pending_auto_jumps: Vec::new(),
            completed_warps: Vec::new(),
        }
    }

    /// Restore a node from a `StateSnapshot` plus the events appended since.
    ///
    /// `modules` and `ship_types` are the same data-driven definitions the
    /// node was originally configured with (e.g. `modules::all_modules()` /
    /// `ship_types::all_ship_types()`). They are needed to resolve
    /// `FittingSnapshot` entries back into `FittedSlot`s and to recompute
    /// `base_stats` for `apply_fitting` (INV-002: snapshot + registries +
    /// log replay must fully reproduce the pre-shutdown ECS state).
    pub fn restore_from(
        store: S,
        snapshot: &StateSnapshot,
        modules: &[ModuleDefinition],
        ship_types: &[ShipTypeDefinition],
    ) -> Self {
        let mut node = Self {
            node_id: snapshot.node_id,
            sector_id: snapshot.sector_id,
            bounds: snapshot.bounds,
            world: SimWorld::new(snapshot.sector_id),
            event_store: store,
            current_tick: snapshot.tick,
            id_counter: snapshot.id_counter,
            ships: ShipRegistry::new(),
            module_registry: HashMap::new(),
            ship_type_registry: HashMap::new(),
            base_stats: HashMap::new(),
            player_id_counter: 0,
            pending_bot_lock_commands: Vec::new(),
            sector_map: {
                let sm = Arc::new(crate::galaxy::Galaxy::demo());
                SectorMap {
                    gates: sm
                        .gates_in_sector(snapshot.sector_id)
                        .into_iter()
                        .map(|g| (g.id, g))
                        .collect(),
                    bodies: sm
                        .bodies_in_sector(snapshot.sector_id)
                        .into_iter()
                        .map(|b| (b.id, b))
                        .collect(),
                    galaxy: sm,
                }
            },
            anchor_table: crate::anchor::AnchorTable::from_galaxy(&crate::galaxy::Galaxy::demo()),
            population_cap: POPULATION_CAP,
            pending_auto_jumps: Vec::new(),
            completed_warps: Vec::new(),
        };

        for def in modules {
            node.register_module(def.clone());
        }
        for def in ship_types {
            node.register_ship_type(def.clone());
        }

        // Restore ECS state from snapshot.
        for ship in &snapshot.ships {
            node.restore_ship_from_snapshot(ship);
        }

        // Replay events that occurred after the snapshot was taken.
        // Collect first to avoid a simultaneous borrow of `node`.
        let post_events: Vec<DomainEvent> = node
            .event_store
            .iter_from(snapshot.log_index)
            .map(|r| r.event.clone())
            .collect();

        for event in &post_events {
            node.apply_event(event);
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
        self.ship_count() >= self.population_cap
    }

    /// Override the per-Sector population backstop (default [`POPULATION_CAP`]).
    pub fn set_population_cap(&mut self, cap: usize) {
        self.population_cap = cap;
    }

    /// Replace the navigation topology.  Updates `jump_gates` and
    /// `celestial_bodies` for this node's Sector immediately.
    pub fn set_galaxy(&mut self, map: Arc<crate::galaxy::Galaxy>) {
        let sid = self.sector_id;
        self.sector_map.gates = map
            .gates_in_sector(sid)
            .into_iter()
            .map(|g| (g.id, g))
            .collect();
        self.sector_map.bodies = map
            .bodies_in_sector(sid)
            .into_iter()
            .map(|b| (b.id, b))
            .collect();
        self.anchor_table = crate::anchor::AnchorTable::from_galaxy(&map);
        self.sector_map.galaxy = map;
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

    // -- Observation ---------------------------------------------------------

    pub fn current_tick(&self) -> Tick {
        self.current_tick
    }
    pub fn ship_count(&self) -> usize {
        self.world.ship_count()
    }
    pub fn total_event_count(&self) -> usize {
        self.event_store.len()
    }
    pub fn event_store(&self) -> &S {
        &self.event_store
    }

    /// The Ship's current approach target, if any (ADR-0015).
    #[cfg(test)]
    pub fn approach_target(&self, ship_id: ShipId) -> Option<dawn_core::ApproachTarget> {
        let entity = self.ships.index.get(&ship_id)?;
        self.world
            .inner()
            .get::<&dawn_ecs::components::ApproachComp>(*entity)
            .ok()
            .map(|a| a.target)
    }

    /// The Ship's current warp phase, if it is warping (ADR-0022).
    #[cfg(test)]
    pub fn warp_phase(&self, ship_id: ShipId) -> Option<dawn_ecs::components::WarpPhase> {
        let entity = self.ships.index.get(&ship_id)?;
        self.world
            .inner()
            .get::<&WarpComp>(*entity)
            .ok()
            .map(|w| w.phase)
    }

    /// Look up the current position of a Ship by its ID.
    pub fn get_ship_position(&self, ship_id: ShipId) -> Option<Position> {
        let entity = self.ships.index.get(&ship_id)?;
        self.world
            .inner()
            .get::<&PositionComp>(*entity)
            .ok()
            .map(|c| c.0)
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
    /// composing its anchor's absolute position with its f32 offset (ADR-0029).
    /// Falls back to treating the raw offset as absolute if the anchor is
    /// unknown (pre-anchor data / tests).
    pub fn ship_absolute(&self, ship_id: ShipId) -> Option<[f64; 3]> {
        let entity = *self.ships.index.get(&ship_id)?;
        let offset = self.world.inner().get::<&PositionComp>(entity).ok()?.0;
        match self.world.ship_anchor(entity) {
            Some(anchor) => self.anchor_table.absolute(anchor, offset).or_else(|| {
                debug_assert_missing_anchor(anchor, "ship_absolute");
                Some([offset.x as f64, offset.y as f64, offset.z as f64])
            }),
            None => Some([offset.x as f64, offset.y as f64, offset.z as f64]),
        }
    }

    /// Absolute position (Sector-frame) of a ship entity given its raw offset,
    /// composing its anchor (ADR-0029). f32 result (compressed-scale safe).
    /// Used by steering/AI code so positions across anchors are comparable.
    pub(super) fn entity_absolute(&self, entity: Entity, offset: Position) -> Position {
        let Some(anchor) = self.world.ship_anchor(entity) else {
            return offset;
        };
        let Some(a) = self.anchor_table.abs(anchor) else {
            debug_assert_missing_anchor(anchor, "entity_absolute");
            return offset;
        };
        Position::new(
            (a[0] + offset.x as f64) as f32,
            (a[1] + offset.y as f64) as f32,
            (a[2] + offset.z as f64) as f32,
        )
    }

    /// Absolute (Sector-frame) position of a ship entity, composing its anchor
    /// with its current `PositionComp` offset (ADR-0029). The single accessor any
    /// gameplay code should use to compare a ship against Sector-frame data
    /// (gates, bodies, other ships) — never read the raw anchor-relative offset
    /// for cross-anchor geometry.
    pub(super) fn entity_abs_pos(&self, entity: Entity) -> Position {
        let off = self
            .world
            .inner()
            .get::<&PositionComp>(entity)
            .ok()
            .map(|p| p.0)
            .unwrap_or(Position::ORIGIN);
        self.entity_absolute(entity, off)
    }

    /// Absolute (Sector-frame, metres, f64) position of a ship entity, composing
    /// its anchor with its current `PositionComp` offset (ADR-0029). The f64
    /// counterpart of [`Self::entity_abs_pos`] — the AoI grid input that stays
    /// precise at true-AU distances (R2).
    pub(super) fn entity_abs_pos_f64(&self, entity: Entity) -> [f64; 3] {
        let off = self
            .world
            .inner()
            .get::<&PositionComp>(entity)
            .ok()
            .map(|p| p.0)
            .unwrap_or(Position::ORIGIN);
        self.entity_absolute_f64(entity, off)
    }

    /// Absolute position (Sector-frame, metres, f64) of a ship entity given its
    /// raw offset, composing its anchor (ADR-0029). Used by warp arrival math
    /// that must stay precise at true-AU distances.
    pub(super) fn entity_absolute_f64(&self, entity: Entity, offset: Position) -> [f64; 3] {
        let Some(anchor) = self.world.ship_anchor(entity) else {
            return [offset.x as f64, offset.y as f64, offset.z as f64];
        };
        let Some(a) = self.anchor_table.abs(anchor) else {
            debug_assert_missing_anchor(anchor, "entity_absolute_f64");
            return [offset.x as f64, offset.y as f64, offset.z as f64];
        };
        [
            a[0] + offset.x as f64,
            a[1] + offset.y as f64,
            a[2] + offset.z as f64,
        ]
    }

    /// Distance from a Ship to a Sector-frame point (a gate/body position),
    /// composing the ship's anchor (ADR-0029). The accessor gameplay/tests use
    /// instead of comparing a raw offset to absolute data.
    pub fn ship_distance_to_point(&self, ship_id: ShipId, point: Position) -> Option<f32> {
        let entity = *self.ships.index.get(&ship_id)?;
        Some(self.entity_abs_pos(entity).distance(point))
    }

    /// True distance (metres) between two Ships, composing each ship's anchor
    /// and offset in f64 so the result is correct even if the two ships are
    /// anchored on different bodies (ADR-0029 step 3 / spike B-3).
    pub fn ship_distance(&self, a: ShipId, b: ShipId) -> Option<f64> {
        let pa = self.ship_absolute(a)?;
        let pb = self.ship_absolute(b)?;
        let d = [pa[0] - pb[0], pa[1] - pb[1], pa[2] - pb[2]];
        Some((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt())
    }

    /// Look up the current `ShipStatsComp` of a Ship by its ID. Test-only.
    #[cfg(test)]
    pub fn get_ship_stats(&self, ship_id: ShipId) -> Option<ShipStatsComp> {
        let entity = self.ships.index.get(&ship_id)?;
        self.world
            .inner()
            .get::<&ShipStatsComp>(*entity)
            .ok()
            .map(|c| *c)
    }

    /// Look up the current HP of a Ship by its ID. Test-only.
    #[cfg(test)]
    pub fn get_ship_hp(&self, ship_id: ShipId) -> Option<f32> {
        let entity = self.ships.index.get(&ship_id)?;
        self.world
            .inner()
            .get::<&HullComp>(*entity)
            .ok()
            .map(|c| c.current_shield + c.current_armor + c.current_hull)
    }

    /// Look up the current `CapacitorComp.current` of a Ship by its ID.
    #[cfg(test)]
    pub fn get_ship_capacitor(&self, ship_id: ShipId) -> Option<f32> {
        let entity = self.ships.index.get(&ship_id)?;
        self.world
            .inner()
            .get::<&CapacitorComp>(*entity)
            .ok()
            .map(|c| c.current)
    }

    /// `(ModuleId, is_active)` for every fitted module on a Ship, across all slots.
    #[cfg(test)]
    pub fn get_fitted_module_ids(&self, ship_id: ShipId) -> Vec<(ModuleId, bool)> {
        let entity = match self.ships.index.get(&ship_id) {
            Some(&e) => e,
            None => return Vec::new(),
        };
        self.world
            .inner()
            .get::<&FittingComp>(entity)
            .map(|f| {
                f.high
                    .iter()
                    .chain(f.mid.iter())
                    .chain(f.low.iter())
                    .chain(f.rig.iter())
                    .map(|s| (s.def.id, s.is_active))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Fire in debug builds when a ship's `AnchorComp` points at an `AnchorId` that
/// the `AnchorTable` doesn't know (ADR-0029 R3). The absolute-position accessors
/// fall back to treating the raw offset as absolute, which is only correct at the
/// Sector origin — at true-AU scale a missing anchor silently misplaces the ship
/// by the body's absolute position (~10^11 m). This can't happen for node-spawned
/// ships (every spawn anchors on a real table entry), so a miss means a data /
/// galaxy-table integrity bug; surface it loudly instead of returning a wrong
/// frame. Release builds keep the silent fallback as a safety net.
#[inline]
#[allow(clippy::assertions_on_constants)]
fn debug_assert_missing_anchor(anchor: dawn_core::AnchorId, site: &str) {
    debug_assert!(
        false,
        "{site}: ship anchored on {anchor:?} which is absent from the AnchorTable \
         — absolute position fell back to the raw offset (wrong frame at true AU)"
    );
}

// -- Tests -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::Velocity;
    use dawn_ecs::components::ThrustComp;

    fn mem_node() -> SimulationNode {
        SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        )
    }

    // -- Existing behaviour (unchanged) --------------------------------------

    #[test]
    fn spawning_a_ship_appends_a_ship_spawned_event() {
        let mut node = mem_node();
        node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);

        assert_eq!(node.total_event_count(), 1);
        assert!(matches!(
            node.event_store().all_records()[0].event,
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
        let thrust = node.world.inner().get::<&ThrustComp>(entity).unwrap();
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
        let direction_before = node
            .world
            .inner()
            .get::<&ThrustComp>(entity)
            .unwrap()
            .direction;

        node.world.set_transit_state(
            entity,
            dawn_ecs::TransitState::InTransit { to: SectorId(1) },
        );
        node.apply_stop_command(ship_id);

        let thrust = node.world.inner().get::<&ThrustComp>(entity).unwrap();
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
                Position::new(i as f32 * 100.0, 0.0, 0.0),
                Velocity::new(1.0, 0.0, 0.0),
            );
        }
        node.tick();
        let spawned = node
            .event_store()
            .iter_from(0)
            .filter(|r| matches!(r.event, DomainEvent::ShipSpawned(_)))
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
