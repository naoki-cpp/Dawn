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
mod commands;
mod navigation;
mod sector_map;
mod serialization;
mod ship_registry;
mod snapshot_io;
mod spawner_logic;
mod tackle;
mod tick;
mod transit_flow;

use sector_map::SectorMap;
use ship_registry::ShipRegistry;

use std::collections::HashMap;
use std::sync::Arc;

use dawn_core::{
    ship_type::{ShipTypeDefinition, ShipTypeId},
    DomainEvent, JumpGateDef, JumpGateId,
    ModuleDefinition, ModuleId, NodeId, Position, SectorBounds, SectorId, ShipId,
    Tick, WarpTarget,
};
use dawn_ecs::{
    components::{PositionComp, ShipStatsComp, WarpComp},
    SimWorld,
};
use dawn_event_store::{store::EventStore, InMemoryEventStore};

#[cfg(test)]
use dawn_ecs::components::{CapacitorComp, FittingComp, HullComp};

use crate::persistence::StateSnapshot;

/// Per-Sector population backstop (ADR-0018 final resort). Set far above the
/// TiDi budget so dynamic split / LoD / local TiDi all engage first; only
/// extreme density ever reaches this admission limit.
pub const POPULATION_CAP: usize = 100_000;

// ── Warp tuning (short-range Fold, ADR-0022 §9) ─────────────────────────────────

/// Warp engages once the ship is moving at this fraction of its max speed
/// toward the gate (EVE-style 75% alignment, ADR-0022). Align time therefore
/// emerges from ship agility (thrust / max_speed) — the tackle window
/// (ADR-0023) is longer for sluggish ships.
const WARP_ALIGN_FRACTION: f32 = 0.75;
/// Warp cruise speed (units/tick), far above any sublight `max_speed`.
const WARP_SPEED: f32 = 5000.0;
/// Deceleration ramp: while approaching the arrival ring the warp speed is
/// capped at `remaining_distance * WARP_DECEL_RATE`, so the ship eases in
/// instead of stopping dead (EVE-like warp deceleration). Decel begins at
/// `WARP_SPEED / WARP_DECEL_RATE` units of remaining distance.
const WARP_DECEL_RATE: f32 = 0.4;
/// Speed (units/tick) at or below which the warp settles and stops.
const WARP_EXIT_SPEED: f32 = 250.0;
/// Minimum distance to a gate for warp to be allowed (units). Closer than this,
/// the `WarpCommand` is rejected and the player should approach instead.
const MIN_WARP_DISTANCE: f32 = 3000.0;
/// Stop this far inside the gate's activation radius on arrival, so the jump
/// prompt is available immediately (mirrors approach, ADR-0015).
const WARP_ARRIVAL_FACTOR: f32 = 0.8;
/// Warp to a celestial body: arrive at this multiple of the body's radius from
/// its centre (ADR-0025). 1.5 = orbit insertion outside the body surface.
const BODY_WARP_ARRIVAL_FACTOR: f32 = 1.5;


// ── TickResult ────────────────────────────────────────────────────────────────

/// Result returned after executing one tick.
#[derive(Debug)]
pub struct TickResult {
    /// The tick that was just completed.
    pub tick          : Tick,
    /// Number of events emitted this tick.
    pub events_emitted: usize,
    /// The actual events produced (used by Actor layer for replication).
    pub events        : Vec<DomainEvent>,
    /// Ships whose active module was force-deactivated by cap shortage this tick.
    pub cap_depletions: Vec<dawn_core::ShipId>,
}

// ── SimulationNode ────────────────────────────────────────────────────────────

/// A single-Sector simulation node, generic over its event store.
///
/// The default store is `InMemoryEventStore`; use `FileEventStore` for
/// persistent operation (Phase 3).
pub struct SimulationNode<S = InMemoryEventStore>
where
    S: EventStore,
{
    node_id     : NodeId,
    sector_id   : SectorId,
    bounds      : SectorBounds,
    world       : SimWorld,
    event_store : S,
    current_tick: Tick,
    id_counter  : u64,
    /// Ship identity and ownership maps (entity index, type ids, player ownership).
    ships: ShipRegistry,
    /// Module definition registry.
    module_registry   : HashMap<ModuleId, ModuleDefinition>,
    /// Ship type definition registry.
    ship_type_registry: HashMap<ShipTypeId, ShipTypeDefinition>,
    /// Bare ShipStats without fitting. Used as the base for fitting aggregation.
    base_stats         : HashMap<ShipId, ShipStatsComp>,
    /// PlayerId allocation counter.
    player_id_counter  : u64,
    /// Lock-on commands queued by the bot AI during `process_bots()`.
    ///
    /// Bot AI runs after the LockSystem each tick.  These commands are held
    /// here and injected into the LockSystem at the start of the NEXT tick,
    /// ensuring they are processed exactly like human-issued lock commands.
    pending_bot_lock_commands: Vec<dawn_core::LockOnCommand>,
    /// Static navigation topology for this Sector (gates, bodies, star map).
    sector_map: SectorMap,
    /// Per-Sector population backstop (ADR-0018). Defaults to [`POPULATION_CAP`];
    /// tunable via [`Self::set_population_cap`].
    population_cap: usize,
    /// Auto-jump triggers accumulated during `process_warp()` for ships that
    /// completed a warp with `WarpComp::auto_jump = true`. Drained by the
    /// caller after each tick so the jump can be proposed to the Raft Log
    /// (or handled however the server path requires).
    pending_auto_jumps: Vec<(ShipId, JumpGateId)>,
}

// ── Constructors ──────────────────────────────────────────────────────────────

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
        node_id  : NodeId,
        sector_id: SectorId,
        bounds   : SectorBounds,
        store    : S,
    ) -> Self {
        Self {
            node_id,
            sector_id,
            bounds,
            world             : SimWorld::new(sector_id),
            event_store       : store,
            current_tick      : Tick::ZERO,
            id_counter        : 0,
            ships             : ShipRegistry::new(),
            module_registry   : HashMap::new(),
            ship_type_registry: HashMap::new(),
            base_stats        : HashMap::new(),
            player_id_counter : 0,
            pending_bot_lock_commands: Vec::new(),
            sector_map        : {
                let sm = Arc::new(crate::star_map::StarMap::builtin());
                SectorMap {
                    gates  : sm.gates_in_sector(sector_id).into_iter().map(|g| (g.id, g)).collect(),
                    bodies : sm.bodies_in_sector(sector_id).into_iter().map(|b| (b.id, b)).collect(),
                    star_map: sm,
                }
            },
            population_cap    : POPULATION_CAP,
            pending_auto_jumps: Vec::new(),
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
        store     : S,
        snapshot  : &StateSnapshot,
        modules   : &[ModuleDefinition],
        ship_types: &[ShipTypeDefinition],
    ) -> Self {
        let mut node = Self {
            node_id            : snapshot.node_id,
            sector_id          : snapshot.sector_id,
            bounds             : snapshot.bounds,
            world              : SimWorld::new(snapshot.sector_id),
            event_store        : store,
            current_tick       : snapshot.tick,
            id_counter         : snapshot.id_counter,
            ships              : ShipRegistry::new(),
            module_registry    : HashMap::new(),
            ship_type_registry : HashMap::new(),
            base_stats         : HashMap::new(),
            player_id_counter  : 0,
            pending_bot_lock_commands: Vec::new(),
            sector_map         : {
                let sm = Arc::new(crate::star_map::StarMap::builtin());
                SectorMap {
                    gates  : sm.gates_in_sector(snapshot.sector_id).into_iter().map(|g| (g.id, g)).collect(),
                    bodies : sm.bodies_in_sector(snapshot.sector_id).into_iter().map(|b| (b.id, b)).collect(),
                    star_map: sm,
                }
            },
            population_cap    : POPULATION_CAP,
            pending_auto_jumps: Vec::new(),
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

    // ── Population backstop (ADR-0018) ──────────────────────────────────────────

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
    pub fn set_star_map(&mut self, map: Arc<crate::star_map::StarMap>) {
        let sid = self.sector_id;
        self.sector_map.gates = map.gates_in_sector(sid).into_iter().map(|g| (g.id, g)).collect();
        self.sector_map.bodies = map.bodies_in_sector(sid).into_iter().map(|b| (b.id, b)).collect();
        self.sector_map.star_map = map;
    }

    /// Read access to the navigation topology.
    pub fn star_map(&self) -> &crate::star_map::StarMap { &self.sector_map.star_map }

    // ── Identity ──────────────────────────────────────────────────────────────

    pub fn node_id(&self)    -> NodeId   { self.node_id }
    pub fn sector_id(&self)  -> SectorId { self.sector_id }

    // ── Jump Gate Navigation (ADR-0009) ──────────────────────────────────────

    /// Look up a Jump Gate originating in this Sector by `gate_id`.
    pub fn jump_gate(&self, gate_id: JumpGateId) -> Option<&JumpGateDef> {
        self.sector_map.gates.get(&gate_id)
    }

    /// Whether a `JumpCommand` for `ship_id` via `gate_id` would currently be
    /// accepted: the Ship exists, is not already in transit, the gate
    /// originates in this Sector, and the Ship is within its
    /// `activation_radius`. Used to reject commands up front, before
    /// proposing to the Raft Log (INV-006).
    pub fn can_propose_jump(&self, ship_id: ShipId, gate_id: JumpGateId) -> bool {
        let Some(&entity) = self.ships.index.get(&ship_id) else { return false };
        if self.world.transit_state(entity).is_in_transit() {
            return false;
        }
        if self.world.is_tackled(entity) { return false; }
        let Some(gate) = self.sector_map.gates.get(&gate_id) else { return false };
        let Ok(pos) = self.world.inner().get::<&PositionComp>(entity) else { return false };
        gate.is_in_range(pos.0)
    }

    // ── Intra-Sector Warp (short-range Fold, ADR-0022) ───────────────────────

    /// Whether a `WarpCommand` for `ship_id` toward `target` would currently be
    /// accepted (INV-006 Validation, before attaching `WarpComp`):
    /// the Ship exists, is not in transit, is not already warping, not tackled,
    /// the target belongs to this Sector, and is at least
    /// `MIN_WARP_DISTANCE` away (closer → use approach instead).
    pub fn can_propose_warp(&self, ship_id: ShipId, target: WarpTarget) -> bool {
        let Some(&entity) = self.ships.index.get(&ship_id) else { return false };
        if self.world.transit_state(entity).is_in_transit() {
            return false;
        }
        if self.world.inner().get::<&WarpComp>(entity).is_ok() {
            return false; // already aligning or warping
        }
        if self.world.is_tackled(entity) { return false; }
        let Ok(pos) = self.world.inner().get::<&PositionComp>(entity) else { return false };
        match target {
            WarpTarget::Gate(gate_id) => {
                let Some(gate) = self.sector_map.gates.get(&gate_id) else { return false };
                pos.0.distance(gate.position) >= MIN_WARP_DISTANCE
            }
            WarpTarget::Body(body_id) => {
                let Some(body) = self.sector_map.bodies.get(&body_id) else { return false };
                pos.0.distance(body.position) >= MIN_WARP_DISTANCE
            }
        }
    }

    // ── Observation ───────────────────────────────────────────────────────────

    pub fn current_tick(&self)      -> Tick  { self.current_tick }
    pub fn ship_count(&self)        -> usize { self.world.ship_count() }
    pub fn total_event_count(&self) -> usize { self.event_store.len() }
    pub fn event_store(&self)       -> &S    { &self.event_store }

    /// The Ship's current approach target, if any (ADR-0015).
    #[cfg(test)]
    pub fn approach_target(&self, ship_id: ShipId) -> Option<dawn_core::ApproachTarget> {
        let entity = self.ships.index.get(&ship_id)?;
        self.world.inner().get::<&dawn_ecs::components::ApproachComp>(*entity).ok().map(|a| a.target)
    }

    /// The Ship's current warp phase, if it is warping (ADR-0022).
    #[cfg(test)]
    pub fn warp_phase(&self, ship_id: ShipId) -> Option<dawn_ecs::components::WarpPhase> {
        let entity = self.ships.index.get(&ship_id)?;
        self.world.inner().get::<&WarpComp>(*entity).ok().map(|w| w.phase)
    }

    /// Look up the current position of a Ship by its ID.
    pub fn get_ship_position(&self, ship_id: ShipId) -> Option<Position> {
        let entity = self.ships.index.get(&ship_id)?;
        self.world.inner().get::<&PositionComp>(*entity).ok().map(|c| c.0)
    }

    /// Look up the current `ShipStatsComp` of a Ship by its ID. Test-only.
    #[cfg(test)]
    pub fn get_ship_stats(&self, ship_id: ShipId) -> Option<ShipStatsComp> {
        let entity = self.ships.index.get(&ship_id)?;
        self.world.inner().get::<&ShipStatsComp>(*entity).ok().map(|c| *c)
    }

    /// Look up the current HP of a Ship by its ID. Test-only.
    #[cfg(test)]
    pub fn get_ship_hp(&self, ship_id: ShipId) -> Option<f32> {
        let entity = self.ships.index.get(&ship_id)?;
        self.world.inner().get::<&HullComp>(*entity).ok()
            .map(|c| c.current_shield + c.current_armor + c.current_hull)
    }

    /// Look up the current `CapacitorComp.current` of a Ship by its ID.
    #[cfg(test)]
    pub fn get_ship_capacitor(&self, ship_id: ShipId) -> Option<f32> {
        let entity = self.ships.index.get(&ship_id)?;
        self.world.inner().get::<&CapacitorComp>(*entity).ok().map(|c| c.current)
    }

    /// `(ModuleId, is_active)` for every fitted module on a Ship, across all slots.
    #[cfg(test)]
    pub fn get_fitted_module_ids(&self, ship_id: ShipId) -> Vec<(ModuleId, bool)> {
        let entity = match self.ships.index.get(&ship_id) {
            Some(&e) => e,
            None => return Vec::new(),
        };
        self.world.inner().get::<&FittingComp>(entity)
            .map(|f| {
                f.high.iter().chain(f.mid.iter()).chain(f.low.iter()).chain(f.rig.iter())
                    .map(|s| (s.def.id, s.is_active))
                    .collect()
            })
            .unwrap_or_default()
    }

}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{PlayerId, Velocity};
    use dawn_ecs::components::ThrustComp;

    fn mem_node() -> SimulationNode {
        SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        )
    }

    // ── Existing behaviour (unchanged) ───────────────────────────────────────

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
        node.world.set_transit_state(entity, dawn_ecs::TransitState::InTransit { to: SectorId(1) });

        node.apply_move_command(ship_id, Position::new(10000.0, 0.0, 0.0));
        let thrust = node.world.inner().get::<&ThrustComp>(entity).unwrap();
        assert_eq!(thrust.direction, Velocity::ZERO, "move command must be rejected while in transit");
    }

    #[test]
    fn stop_command_is_ignored_while_ship_is_in_transit() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::new(100.0, 100.0, 100.0), Velocity::ZERO);
        node.set_player_ship(ship_id);
        node.apply_move_command(ship_id, Position::new(10000.0, 0.0, 0.0));

        let entity = *node.ships.index.get(&ship_id).unwrap();
        let direction_before = node.world.inner().get::<&ThrustComp>(entity).unwrap().direction;

        node.world.set_transit_state(entity, dawn_ecs::TransitState::InTransit { to: SectorId(1) });
        node.apply_stop_command(ship_id);

        let thrust = node.world.inner().get::<&ThrustComp>(entity).unwrap();
        assert_eq!(thrust.direction, direction_before, "stop command must be rejected while in transit");
        assert!(!thrust.is_braking, "is_braking must not be set while in transit");
    }

    #[test]
    fn total_event_count_grows_monotonically_across_ticks() {
        let mut node = mem_node();
        node.spawn_ship(dawn_core::ShipTypeId(1), Position::new(100.0, 100.0, 100.0), Velocity::new(1.0, 1.0, 1.0));
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
            node.spawn_ship(dawn_core::ShipTypeId(1), Position::new(i as f32 * 100.0, 0.0, 0.0), Velocity::new(1.0, 0.0, 0.0));
        }
        node.tick();
        let spawned = node.event_store().iter_from(0)
            .filter(|r| matches!(r.event, DomainEvent::ShipSpawned(_)))
            .count();
        assert_eq!(spawned, 5);
    }

    // ── Area of Interest (ADR-0019) ────────────────────────────────────────────

    #[test]
    fn ships_visible_to_an_observer_are_only_those_in_the_27_cell_neighborhood() {
        let mut node = mem_node();
        let cell = 1_000.0;
        // Observer at origin (cell 0,0,0).
        let observer = node.spawn_ship(crate::ship_types::SHIP_TYPE_NPC_FRIGATE, Position::ORIGIN, Velocity::ZERO);
        // Adjacent cell (1,0,0) — within the 3×3×3 neighborhood.
        let near = node.spawn_ship(crate::ship_types::SHIP_TYPE_NPC_FRIGATE, Position::new(1_500.0, 0.0, 0.0), Velocity::ZERO);
        // Two cells away (2,0,0) — outside the neighborhood.
        let far = node.spawn_ship(crate::ship_types::SHIP_TYPE_NPC_FRIGATE, Position::new(2_500.0, 0.0, 0.0), Velocity::ZERO);

        let visible = node.ships_visible_to(Position::ORIGIN, cell);
        assert!(visible.contains(&observer), "observer's own cell is visible");
        assert!(visible.contains(&near), "adjacent-cell ship is visible");
        assert!(!visible.contains(&far), "two-cells-away ship is not visible");
    }

    #[test]
    fn scoped_initial_state_excludes_ships_outside_the_observer_neighborhood() {
        let mut node = mem_node();
        let cell = 1_000.0;
        let _observer = node.spawn_ship(crate::ship_types::SHIP_TYPE_NPC_FRIGATE, Position::ORIGIN, Velocity::ZERO);
        let far = node.spawn_ship(crate::ship_types::SHIP_TYPE_NPC_FRIGATE, Position::new(9_000.0, 0.0, 0.0), Velocity::ZERO);

        let json = node.build_initial_state_json_for(Position::ORIGIN, cell);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let ids: Vec<u64> = v["ships"].as_array().unwrap().iter()
            .map(|s| s["ship_id"].as_u64().unwrap())
            .collect();
        assert!(ids.contains(&_observer.raw()), "observer is in its own scoped state");
        assert!(!ids.contains(&far.raw()), "distant ship is excluded from scoped InitialState");
        // The full-world InitialState still includes the distant ship.
        let full: serde_json::Value =
            serde_json::from_str(&node.build_initial_state_json()).unwrap();
        assert_eq!(full["ships"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn aoi_enter_json_wraps_the_ship_state_for_a_known_ship() {
        let mut node = mem_node();
        let sid = node.spawn_ship(
            crate::ship_types::SHIP_TYPE_NPC_FRIGATE,
            Position::new(1.0, 2.0, 3.0),
            Velocity::ZERO,
        );
        let json = node.aoi_enter_json(sid).expect("known ship yields a message");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "AoiEnter");
        assert_eq!(v["ship"]["ship_id"].as_u64().unwrap(), sid.raw());
        assert_eq!(v["ship"]["position"]["x"].as_f64().unwrap() as f32, 1.0);
    }

    #[test]
    fn aoi_enter_json_is_none_for_an_unknown_ship() {
        let node = mem_node();
        let unknown = ShipId::new(NodeId(9), 999);
        assert!(node.aoi_enter_json(unknown).is_none());
    }

    // ── Population backstop (ADR-0018 / 8B-1) ────────────────────────────────

    #[test]
    fn at_population_cap_is_true_only_when_ship_count_reaches_the_cap() {
        let mut node = mem_node();
        node.set_population_cap(2);
        assert!(!node.at_population_cap());
        node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        assert!(!node.at_population_cap());
        node.spawn_ship(dawn_core::ShipTypeId(1), Position::new(100.0, 0.0, 0.0), Velocity::ZERO);
        assert_eq!(node.ship_count(), 2);
        assert!(node.at_population_cap());
    }

    #[test]
    fn a_constant_velocity_ship_still_counts_against_the_population_cap() {
        // It emits no events (INV-MOVE) yet is present and bandwidth-bearing,
        // so the raw-count backstop must keep counting it across ticks.
        let mut node = mem_node();
        node.set_population_cap(1);
        node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::new(50.0, 0.0, 0.0));
        assert!(node.at_population_cap());
        for _ in 0..500 { node.tick(); }
        assert!(node.at_population_cap());
    }

    #[test]
    fn destroying_a_ship_frees_capacity_against_the_population_cap() {
        let mut node = mem_node();
        node.set_population_cap(1);
        let sid = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        assert!(node.at_population_cap());
        // Despawn drops the ship from the world, lowering the count.
        node.apply_event_pub(DomainEvent::ShipDespawned(dawn_core::events::ShipDespawned {
            ship_id: sid,
            tick   : Tick::ZERO,
        }));
        assert_eq!(node.ship_count(), 0);
        assert!(!node.at_population_cap());
    }

}
