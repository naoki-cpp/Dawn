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
    components::{CapacitorComp, FittingComp, HullComp, PositionComp, ShipStatsComp, VelocityComp, WarpComp},
    SimWorld,
};
use dawn_event_store::{store::EventStore, InMemoryEventStore};

use crate::persistence::{ShipSnapshot, StateSnapshot};

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

    // ── Sector Transit (ADR-0014) ────────────────────────────────────────────

    /// Validate and begin a Sector Transit (CLAUDE.md §4 Step 2).
    ///
    /// On success, marks the Ship `TransitState::InTransit` and appends a
    /// `SectorTransitRequested` event (ownership stays with this Sector).
    /// On failure, no event is appended (CommandRejected per INV-006).
    ///
    /// In the Raft pipeline (ADR-0014) this is invoked when a committed
    /// `TransitOp::Request` is applied at Step 7.5 — never directly from a
    /// client command.
    pub fn propose_transit(&mut self, cmd: dawn_core::commands::TransitCommand) -> Result<(), dawn_core::DawnError> {
        let &entity = self.ships.index.get(&cmd.ship_id)
            .ok_or(dawn_core::DawnError::ShipNotFound(cmd.ship_id))?;

        if self.world.transit_state(entity).is_in_transit() {
            return Err(dawn_core::DawnError::ShipInTransit(cmd.ship_id));
        }

        self.world.set_transit_state(entity, dawn_ecs::TransitState::InTransit { to: cmd.to });

        self.event_store.append(DomainEvent::SectorTransitRequested(dawn_core::events::SectorTransitRequested {
            ship_id: cmd.ship_id,
            from   : self.sector_id,
            to     : cmd.to,
            tick   : self.current_tick,
        }));

        Ok(())
    }

    /// Whether a `TransitCommand` for `ship_id` would currently be accepted
    /// (Ship exists and is not already in transit). Used to reject commands
    /// up front, before proposing to the Raft Log (INV-006).
    pub fn can_propose_transit(&self, ship_id: ShipId) -> bool {
        self.ships.index
            .get(&ship_id)
            .is_some_and(|&entity| !self.world.transit_state(entity).is_in_transit())
    }

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

    /// Append `JumpGateUsed` (and `StarSystemChanged` if the destination
    /// Sector belongs to a different Star System) for a Ship that just
    /// completed a Jump-Gate Transit (ADR-0009).
    ///
    /// Called from Step 7.5 on the destination node, after
    /// [`import_transit`](Self::import_transit) appends
    /// `SectorTransitCompleted` — `JumpGateUsed` records *how* the Ship
    /// moved, in addition to (not instead of) `SectorTransitCompleted`.
    pub fn append_jump_events(&mut self, ship_id: ShipId, gate_id: JumpGateId, from: SectorId, to: SectorId, entry_pos: Position) {
        self.event_store.append(DomainEvent::JumpGateUsed(dawn_core::events::JumpGateUsed {
            ship_id,
            gate_id,
            from_sector: from,
            to_sector  : to,
            entry_pos,
            tick       : self.current_tick,
        }));

        let from_system = self.sector_map.star_map.system_for_sector(from);
        let to_system   = self.sector_map.star_map.system_for_sector(to);
        if from_system != to_system {
            self.event_store.append(DomainEvent::StarSystemChanged(dawn_core::events::StarSystemChanged {
                ship_id,
                from_system,
                to_system,
                tick: self.current_tick,
            }));
        }
    }

    /// Complete an outgoing Sector Transit: remove the Ship from this node's
    /// ECS and return a snapshot for the destination node to import.
    ///
    /// Appends `SectorTransitCompleted` from this (the `from`) Sector's
    /// perspective. Returns `None` if `ship_id` is unknown or not currently
    /// `InTransit`.
    pub fn export_transit(&mut self, ship_id: ShipId, entry_pos: Position) -> Option<ShipSnapshot> {
        let &entity = self.ships.index.get(&ship_id)?;
        let to = match self.world.transit_state(entity) {
            dawn_ecs::TransitState::InTransit { to } => to,
            dawn_ecs::TransitState::None => return None,
        };

        let pos  = self.world.inner().get::<&PositionComp>(entity).ok()?.0;
        let vel  = self.world.inner().get::<&VelocityComp>(entity).ok()?.0;
        let (current_shield, current_armor, current_hull, is_destroyed) = {
            let hull = self.world.inner().get::<&HullComp>(entity).ok()?;
            (hull.current_shield, hull.current_armor, hull.current_hull, hull.is_destroyed)
        };
        let capacitor = self.world.inner().get::<&CapacitorComp>(entity).ok().map(|c| c.current);
        let fitting = self.world.inner().get::<&FittingComp>(entity)
            .map(|f| f.to_snapshot())
            .unwrap_or_else(|_| dawn_core::fitting::FittingSnapshot::empty());
        let ship_type_id = self.ships.type_ids.get(&ship_id).copied().unwrap_or(ShipTypeId(0));

        // Tackle state is not transferred on sector transit (tacklers are in this
        // sector; they lose the tackle as the ship leaves).
        let snapshot = ShipSnapshot {
            ship_id,
            ship_type_id,
            position: pos,
            velocity: vel,
            current_shield,
            current_armor,
            current_hull,
            is_destroyed,
            capacitor,
            fitting,
            tackled_by: Vec::new(),
        };

        self.ships.index.remove(&ship_id);
        self.world.despawn_ship(entity);
        self.ships.type_ids.remove(&ship_id);
        self.base_stats.remove(&ship_id);

        self.event_store.append(DomainEvent::SectorTransitCompleted(dawn_core::events::SectorTransitCompleted {
            ship_id,
            from     : self.sector_id,
            to,
            entry_pos: entry_pos,
            velocity : vel,
            tick     : self.current_tick,
        }));

        Some(snapshot)
    }

    /// Complete an incoming Sector Transit: restore `ship` (exported from the
    /// `from` Sector via [`export_transit`]) into this node's ECS at
    /// `entry_pos`, preserving its `ShipId` (INV-004 — no ID reuse, the same
    /// Ship simply changes Sector ownership).
    ///
    /// Appends `SectorTransitCompleted` from this (the `to`) Sector's
    /// perspective.
    pub fn import_transit(&mut self, ship: &ShipSnapshot, from: SectorId, entry_pos: Position) {
        let mut ship = ship.clone();
        ship.position = entry_pos;
        self.restore_ship_from_snapshot(&ship);

        self.event_store.append(DomainEvent::SectorTransitCompleted(dawn_core::events::SectorTransitCompleted {
            ship_id  : ship.ship_id,
            from,
            to       : self.sector_id,
            entry_pos,
            velocity : ship.velocity,
            tick     : self.current_tick,
        }));
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
    use dawn_ecs::components::{ThrustComp, WarpPhase};
    use dawn_event_store::FileEventStore;

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
    fn tick_advances_the_logical_tick_counter_by_one() {
        let mut node = mem_node();
        assert_eq!(node.current_tick(), Tick::ZERO);
        node.tick();
        assert_eq!(node.current_tick(), Tick(1));
        node.tick();
        assert_eq!(node.current_tick(), Tick(2));
    }

    #[test]
    fn npc_ships_at_constant_velocity_produce_no_velocity_changed_events() {
        let mut node = mem_node();
        node.spawn_ship(dawn_core::ShipTypeId(1), Position::new(100.0, 100.0, 100.0), Velocity::new(1.0, 0.0, 0.0));
        node.spawn_ship(dawn_core::ShipTypeId(1), Position::new(200.0, 100.0, 100.0), Velocity::new(0.0, 1.0, 0.0));
        assert_eq!(node.tick().events_emitted, 0,
            "NPC ships at constant velocity do not emit VelocityChanged");
    }

    #[test]
    fn stationary_ships_produce_no_events() {
        let mut node = mem_node();
        node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        assert_eq!(node.tick().events_emitted, 0);
    }

    #[test]
    fn velocity_changed_events_carry_the_current_tick_value() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::new(100.0, 100.0, 100.0), Velocity::ZERO);
        node.set_player_ship(ship_id);
        node.apply_move_command(ship_id, Position::new(10000.0, 0.0, 0.0));
        node.tick();
        node.tick();
        let last = node.event_store().all_records().last().unwrap();
        assert_eq!(last.event.tick(), Tick(2));
    }

    #[test]
    fn propose_transit_marks_ship_in_transit_and_appends_requested_event() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);

        node.propose_transit(dawn_core::commands::TransitCommand { ship_id, to: SectorId(1) }).unwrap();

        let entity = *node.ships.index.get(&ship_id).unwrap();
        assert_eq!(node.world.transit_state(entity), dawn_ecs::TransitState::InTransit { to: SectorId(1) });

        let last = node.event_store().all_records().last().unwrap();
        match &last.event {
            DomainEvent::SectorTransitRequested(e) => {
                assert_eq!(e.ship_id, ship_id);
                assert_eq!(e.from, node.sector_id());
                assert_eq!(e.to, SectorId(1));
            }
            other => panic!("expected SectorTransitRequested, got {other:?}"),
        }
    }

    #[test]
    fn propose_transit_is_rejected_when_ship_is_already_in_transit() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        node.propose_transit(dawn_core::commands::TransitCommand { ship_id, to: SectorId(1) }).unwrap();

        let err = node.propose_transit(dawn_core::commands::TransitCommand { ship_id, to: SectorId(2) }).unwrap_err();
        assert!(matches!(err, dawn_core::DawnError::ShipInTransit(id) if id == ship_id));
    }

    #[test]
    fn propose_transit_is_rejected_for_unknown_ship() {
        let mut node = mem_node();
        let unknown = dawn_core::ShipId::new(NodeId(99), 0);
        let err = node.propose_transit(dawn_core::commands::TransitCommand { ship_id: unknown, to: SectorId(1) }).unwrap_err();
        assert!(matches!(err, dawn_core::DawnError::ShipNotFound(id) if id == unknown));
    }

    #[test]
    fn export_transit_removes_ship_and_appends_completed_event() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::new(1.0, 0.0, 0.0));
        node.propose_transit(dawn_core::commands::TransitCommand { ship_id, to: SectorId(1) }).unwrap();

        let entry_pos = Position::new(500.0, 0.0, 0.0);
        let snapshot = node.export_transit(ship_id, entry_pos).expect("ship should export");
        assert_eq!(snapshot.ship_id, ship_id);

        assert!(node.ships.index.get(&ship_id).is_none(), "ship must leave the from-sector ECS");
        assert_eq!(node.ship_count(), 0);

        let last = node.event_store().all_records().last().unwrap();
        match &last.event {
            DomainEvent::SectorTransitCompleted(e) => {
                assert_eq!(e.ship_id, ship_id);
                assert_eq!(e.from, node.sector_id());
                assert_eq!(e.to, SectorId(1));
                assert_eq!(e.entry_pos, entry_pos);
            }
            other => panic!("expected SectorTransitCompleted, got {other:?}"),
        }
    }

    #[test]
    fn export_transit_returns_none_for_ship_not_in_transit() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        assert!(node.export_transit(ship_id, Position::ORIGIN).is_none());
        assert_eq!(node.ship_count(), 1, "ship must remain when not in transit");
    }

    #[test]
    fn import_transit_restores_ship_with_same_id_at_entry_position_and_appends_completed_event() {
        let mut from_node = SimulationNode::new(NodeId(0), SectorId(0), SectorBounds::centered(SectorBounds::DEFAULT_HALF));
        let mut to_node   = SimulationNode::new(NodeId(1), SectorId(1), SectorBounds::centered(SectorBounds::DEFAULT_HALF));

        let ship_id = from_node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::new(1.0, 0.0, 0.0));
        from_node.propose_transit(dawn_core::commands::TransitCommand { ship_id, to: SectorId(1) }).unwrap();

        let entry_pos = Position::new(500.0, 0.0, 0.0);
        let snapshot = from_node.export_transit(ship_id, entry_pos).unwrap();

        to_node.import_transit(&snapshot, SectorId(0), entry_pos);

        assert_eq!(to_node.ship_count(), 1);
        assert_eq!(to_node.get_ship_position(ship_id), Some(entry_pos));

        let last = to_node.event_store().all_records().last().unwrap();
        match &last.event {
            DomainEvent::SectorTransitCompleted(e) => {
                assert_eq!(e.ship_id, ship_id);
                assert_eq!(e.from, SectorId(0));
                assert_eq!(e.to, SectorId(1));
                assert_eq!(e.entry_pos, entry_pos);
            }
            other => panic!("expected SectorTransitCompleted, got {other:?}"),
        }
    }

    #[test]
    fn adopted_player_ship_accepts_owned_commands_on_the_destination_node() {
        let mut from_node = SimulationNode::new(NodeId(0), SectorId(0), SectorBounds::centered(SectorBounds::DEFAULT_HALF));
        let mut to_node   = SimulationNode::new(NodeId(1), SectorId(1), SectorBounds::centered(SectorBounds::DEFAULT_HALF));

        let player_id = from_node.next_player_id();
        let ship_id   = from_node.spawn_player_ship(player_id);
        from_node.propose_transit(dawn_core::commands::TransitCommand { ship_id, to: SectorId(1) }).unwrap();
        let snapshot = from_node.export_transit(ship_id, Position::ORIGIN).unwrap();
        to_node.import_transit(&snapshot, SectorId(0), Position::ORIGIN);

        // Before the handoff, the destination node rejects owned commands.
        assert!(!to_node.apply_stop_command_owned(player_id, ship_id));

        assert!(to_node.adopt_player_ship(ship_id, player_id));
        assert!(to_node.apply_stop_command_owned(player_id, ship_id));
    }

    #[test]
    fn adopt_player_ship_returns_false_for_ship_not_in_this_node() {
        let mut node = mem_node();
        let unknown = dawn_core::ShipId::new(NodeId(99), 0);
        assert!(!node.adopt_player_ship(unknown, dawn_core::PlayerId(0)));
        assert!(!node.apply_stop_command_owned(dawn_core::PlayerId(0), unknown));
    }

    // ── Approach (ADR-0015) ──────────────────────────────────────────────────

    /// Spawn a player-owned ship at `pos` and return (player_id, ship_id).
    fn spawn_owned_player_at(node: &mut SimulationNode, pos: Position) -> (PlayerId, ShipId) {
        let player_id = node.next_player_id();
        let ship_id   = node.spawn_player_ship_at_pub(player_id, pos);
        (player_id, ship_id)
    }

    #[test]
    fn approach_command_attaches_an_approach_target_to_the_owned_ship() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target = node.spawn_ship(dawn_core::ShipTypeId(1), Position::new(10_000.0, 0.0, 0.0), Velocity::ZERO);

        assert!(node.apply_approach_command_owned(player, dawn_core::ApproachCommand { ship_id: chaser, target: dawn_core::ApproachTarget::Ship(target) }));
        assert_eq!(node.approach_target(chaser), Some(dawn_core::ApproachTarget::Ship(target)));
    }

    #[test]
    fn approach_command_is_rejected_for_a_ship_the_player_does_not_own() {
        let mut node = mem_node();
        let (_owner, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target = node.spawn_ship(dawn_core::ShipTypeId(1), Position::new(10_000.0, 0.0, 0.0), Velocity::ZERO);

        let stranger = node.next_player_id();
        assert!(!node.apply_approach_command_owned(stranger, dawn_core::ApproachCommand { ship_id: chaser, target: dawn_core::ApproachTarget::Ship(target) }));
        assert_eq!(node.approach_target(chaser), None);
    }

    #[test]
    fn approaching_ship_steers_thrust_toward_its_target_each_tick() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target = node.spawn_ship(dawn_core::ShipTypeId(1), Position::new(10_000.0, 0.0, 0.0), Velocity::ZERO);
        node.apply_approach_command_owned(player, dawn_core::ApproachCommand { ship_id: chaser, target: dawn_core::ApproachTarget::Ship(target) });

        let entity = *node.ships.index.get(&chaser).unwrap();
        node.process_approach();
        let thrust = node.world.inner().get::<&ThrustComp>(entity).unwrap();
        assert!(thrust.direction.dx > 0.9, "thrust should point toward +X target, got {:?}", thrust.direction);
        assert!(!thrust.is_braking);
    }

    #[test]
    fn approaching_ship_closes_distance_to_its_target_over_several_ticks() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target = node.spawn_ship(dawn_core::ShipTypeId(1), Position::new(10_000.0, 0.0, 0.0), Velocity::ZERO);
        node.apply_approach_command_owned(player, dawn_core::ApproachCommand { ship_id: chaser, target: dawn_core::ApproachTarget::Ship(target) });

        let start = node.get_ship_position(chaser).unwrap().distance(Position::new(10_000.0, 0.0, 0.0));
        for _ in 0..30 { node.tick(); }
        let end = node.get_ship_position(chaser).unwrap().distance(Position::new(10_000.0, 0.0, 0.0));
        assert!(end < start, "approaching ship should reduce distance: {start} -> {end}");
    }

    #[test]
    fn move_command_cancels_an_active_approach() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target = node.spawn_ship(dawn_core::ShipTypeId(1), Position::new(10_000.0, 0.0, 0.0), Velocity::ZERO);
        node.apply_approach_command_owned(player, dawn_core::ApproachCommand { ship_id: chaser, target: dawn_core::ApproachTarget::Ship(target) });
        assert_eq!(node.approach_target(chaser), Some(dawn_core::ApproachTarget::Ship(target)));

        node.apply_move_command(chaser, Position::new(-10_000.0, 0.0, 0.0));
        assert_eq!(node.approach_target(chaser), None, "manual move must cancel approach");
    }

    #[test]
    fn stop_command_cancels_an_active_approach() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target = node.spawn_ship(dawn_core::ShipTypeId(1), Position::new(10_000.0, 0.0, 0.0), Velocity::ZERO);
        node.apply_approach_command_owned(player, dawn_core::ApproachCommand { ship_id: chaser, target: dawn_core::ApproachTarget::Ship(target) });

        node.apply_stop_command(chaser);
        assert_eq!(node.approach_target(chaser), None, "stop must cancel approach");
    }

    #[test]
    fn approach_is_dropped_and_ship_brakes_when_the_target_disappears() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target = node.spawn_ship(dawn_core::ShipTypeId(1), Position::new(10_000.0, 0.0, 0.0), Velocity::ZERO);
        node.apply_approach_command_owned(player, dawn_core::ApproachCommand { ship_id: chaser, target: dawn_core::ApproachTarget::Ship(target) });

        // Remove the target from the ECS, then run the approach step.
        let target_entity = node.ships.index.remove(&target).unwrap();
        node.world.despawn_ship(target_entity);

        node.process_approach();
        assert_eq!(node.approach_target(chaser), None, "approach must drop when target is gone");
        let entity = *node.ships.index.get(&chaser).unwrap();
        assert!(node.world.inner().get::<&ThrustComp>(entity).unwrap().is_braking, "ship should brake when target vanishes");
    }

    #[test]
    fn approach_command_is_rejected_when_target_is_self() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        assert!(!node.apply_approach_command_owned(player, dawn_core::ApproachCommand { ship_id: chaser, target: dawn_core::ApproachTarget::Ship(chaser) }));
        assert_eq!(node.approach_target(chaser), None);
    }

    #[test]
    fn approaching_a_jump_gate_steers_the_ship_toward_the_gate_and_into_range() {
        // Sector 0 owns Gate 0 (position near +X edge). Start at the origin.
        let mut node = mem_node();
        let gate = node.jump_gate(dawn_core::JumpGateId(0)).expect("Sector 0 has Gate 0").clone();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);

        assert!(node.apply_approach_command_owned(player, dawn_core::ApproachCommand {
            ship_id: chaser,
            target : dawn_core::ApproachTarget::Gate(dawn_core::JumpGateId(0)),
        }));
        assert_eq!(node.approach_target(chaser), Some(dawn_core::ApproachTarget::Gate(dawn_core::JumpGateId(0))));

        let start = node.get_ship_position(chaser).unwrap().distance(gate.position);
        for _ in 0..400 { node.tick(); }
        let end = node.get_ship_position(chaser).unwrap().distance(gate.position);
        assert!(end < start, "ship should close on the gate: {start} -> {end}");
        assert!(node.can_propose_jump(chaser, dawn_core::JumpGateId(0)),
            "after approaching, the ship should be within the gate's activation radius");
    }

    #[test]
    fn approach_command_is_rejected_for_a_gate_not_in_this_sector() {
        // Gate 1 originates in Sector 1, so a Sector-0 node does not know it.
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        assert!(!node.apply_approach_command_owned(player, dawn_core::ApproachCommand {
            ship_id: chaser,
            target : dawn_core::ApproachTarget::Gate(dawn_core::JumpGateId(1)),
        }));
        assert_eq!(node.approach_target(chaser), None);
    }

    // ── Warp (short-range Fold, ADR-0022) ────────────────────────────────────

    #[test]
    fn warp_is_rejected_when_the_gate_is_closer_than_the_minimum_warp_distance() {
        // Spawn right on top of Gate 0: too close to warp, must approach instead.
        let mut node = mem_node();
        let gate = node.jump_gate(dawn_core::JumpGateId(0)).unwrap().clone();
        let (player, ship) = spawn_owned_player_at(&mut node, gate.position);
        assert!(!node.can_propose_warp(ship, dawn_core::WarpTarget::Gate(dawn_core::JumpGateId(0))));
        assert!(!node.apply_warp_command_owned(player, dawn_core::WarpCommand {
            ship_id: ship, target: dawn_core::WarpTarget::Gate(dawn_core::JumpGateId(0)),
        }));
        assert_eq!(node.warp_phase(ship), None);
    }

    #[test]
    fn warp_is_rejected_for_a_gate_not_in_this_sector() {
        // Gate 1 originates in Sector 1; a Sector-0 node does not know it.
        let mut node = mem_node();
        let (player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        assert!(!node.apply_warp_command_owned(player, dawn_core::WarpCommand {
            ship_id: ship, target: dawn_core::WarpTarget::Gate(dawn_core::JumpGateId(1)),
        }));
        assert_eq!(node.warp_phase(ship), None);
    }

    #[test]
    fn warp_aligns_by_accelerating_then_flies_into_gate_range_and_completes() {
        let mut node = mem_node();
        let (player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        assert!(node.apply_warp_command_owned(player, dawn_core::WarpCommand {
            ship_id: ship, target: dawn_core::WarpTarget::Gate(dawn_core::JumpGateId(0)),
        }));

        // During alignment the ship accelerates toward the gate under thrust
        // (sublight), not yet at warp speed: it moves only a little.
        node.tick();
        assert_eq!(node.warp_phase(ship), Some(WarpPhase::Aligning));
        assert!(node.get_ship_position(ship).unwrap().x < 100.0,
            "an aligning ship accelerates sublight, far short of warp speed");

        // Run well past alignment + warp travel; the ship arrives and stops.
        for _ in 0..80 { node.tick(); }
        assert_eq!(node.warp_phase(ship), None, "warp completes and the component is removed");
        assert!(node.can_propose_jump(ship, dawn_core::JumpGateId(0)),
            "warp drops the ship inside the gate's activation radius");
    }

    #[test]
    fn warp_align_time_emerges_from_ship_agility() {
        // A sluggish ship (high mass) takes longer to reach 75% max speed and
        // thus to engage warp than an agile ship — EVE-style align (ADR-0023).
        fn ticks_to_engage(mass: f32) -> u32 {
            let mut node = mem_node();
            let (player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
            let entity = *node.ships.index.get(&ship).unwrap();
            let mut stats = *node.world.inner().get::<&ShipStatsComp>(entity).unwrap();
            stats.mass = mass;
            node.world.set_ship_stats(entity, stats);
            node.apply_warp_command_owned(player, dawn_core::WarpCommand { ship_id: ship, target: dawn_core::WarpTarget::Gate(dawn_core::JumpGateId(0)) });
            for t in 1..=500u32 {
                node.tick();
                if node.warp_phase(ship) == Some(WarpPhase::Warping) { return t; }
            }
            u32::MAX
        }
        assert!(ticks_to_engage(50_000_000.0) > ticks_to_engage(1_000_000.0),
            "a heavier ship spends longer aligning (a longer tackle window)");
    }

    #[test]
    fn warp_decelerates_smoothly_near_the_gate_instead_of_stopping_dead() {
        let mut node = mem_node();
        let (player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        node.apply_warp_command_owned(player, dawn_core::WarpCommand { ship_id: ship, target: dawn_core::WarpTarget::Gate(dawn_core::JumpGateId(0)) });

        // Track warp speed each tick. A cliff stop would jump straight from
        // cruise (~WARP_SPEED) to zero; a smooth ramp produces intermediate
        // steps well below cruise before stopping.
        let entity = *node.ships.index.get(&ship).unwrap();
        let mut saw_decel_step = false;
        for _ in 0..100 {
            node.tick();
            // Only count steps in the committed warping phase (align is sublight).
            let warping = node.warp_phase(ship) == Some(WarpPhase::Warping);
            let v = node.world.inner().get::<&VelocityComp>(entity).unwrap().0;
            let speed = (v.dx * v.dx + v.dy * v.dy + v.dz * v.dz).sqrt();
            if warping && speed > f32::EPSILON && speed < WARP_SPEED * 0.9 {
                saw_decel_step = true;
            }
            if node.warp_phase(ship).is_none() { break; }
        }
        assert!(saw_decel_step, "warp must ramp down through intermediate speeds, not stop dead");
        assert_eq!(node.warp_phase(ship), None, "warp should have completed");
    }

    #[test]
    fn a_move_command_cancels_an_aligning_warp() {
        let mut node = mem_node();
        let (player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        node.apply_warp_command_owned(player, dawn_core::WarpCommand { ship_id: ship, target: dawn_core::WarpTarget::Gate(dawn_core::JumpGateId(0)) });
        node.tick(); // still aligning (one tick is far from 75% max speed)
        assert_eq!(node.warp_phase(ship), Some(WarpPhase::Aligning));

        node.apply_move_command(ship, Position::new(0.0, 1000.0, 0.0));
        assert_eq!(node.warp_phase(ship), None, "a move during alignment cancels the warp");
    }

    #[test]
    fn a_move_command_is_ignored_during_the_committed_warping_phase() {
        let mut node = mem_node();
        let (player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        node.apply_warp_command_owned(player, dawn_core::WarpCommand { ship_id: ship, target: dawn_core::WarpTarget::Gate(dawn_core::JumpGateId(0)) });
        // Tick until the warp engages (just past alignment); it is then far from
        // arrival, so it stays committed. Player ship: mass=10M, inertia=0.3
        // → τ=30 ticks → align ≈ 42 ticks, so allow up to 100.
        for _ in 0..100 {
            node.tick();
            if node.warp_phase(ship) == Some(WarpPhase::Warping) { break; }
        }
        assert_eq!(node.warp_phase(ship), Some(WarpPhase::Warping), "warp should be committed");

        node.apply_move_command(ship, Position::new(0.0, 1000.0, 0.0));
        assert_eq!(node.warp_phase(ship), Some(WarpPhase::Warping),
            "a committed warp cannot be interrupted by a move command");
    }

    #[test]
    fn auto_jump_is_queued_in_pending_list_when_warp_completes_with_auto_jump_true() {
        let mut node = mem_node();
        let (_player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        assert!(node.apply_warp_command(ship, dawn_core::WarpTarget::Gate(dawn_core::JumpGateId(0)), true));

        // Run until warp completes.
        for _ in 0..100 { node.tick(); }
        assert_eq!(node.warp_phase(ship), None, "warp must complete");
        assert!(node.can_propose_jump(ship, dawn_core::JumpGateId(0)),
            "ship must be within gate range after warp");

        let pending = node.drain_pending_auto_jumps();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0], (ship, dawn_core::JumpGateId(0)));

        // Draining twice returns nothing.
        assert!(node.drain_pending_auto_jumps().is_empty());
    }

    #[test]
    fn normal_warp_without_auto_jump_does_not_queue_pending_jump() {
        let mut node = mem_node();
        let (_player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        assert!(node.apply_warp_command(ship, dawn_core::WarpTarget::Gate(dawn_core::JumpGateId(0)), false));

        for _ in 0..100 { node.tick(); }
        assert_eq!(node.warp_phase(ship), None);
        assert!(node.drain_pending_auto_jumps().is_empty());
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

    #[test]
    fn fitting_same_module_twice_does_not_double_count_stats() {
        use dawn_core::{FitModuleCommand, ModuleId, SlotKind};
        use dawn_core::fitting::{ModuleDefinition, ModuleKind, StatDelta};

        let mut node = mem_node();
        let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);

        let railgun = ModuleDefinition {
            id                : ModuleId(1),
            name              : "Test Railgun".to_string(),
            kind              : ModuleKind::Weapon,
            slot              : SlotKind::High,
            activation_mode   : dawn_core::ActivationMode::Active,
            cap_cost_per_cycle: 60.0,
            cycle_time_ticks  : 10,
            stat_delta        : StatDelta { weapon_damage_add: 25.0, weapon_range_add: 1000.0, ..StatDelta::ZERO },
        };
        node.register_module(railgun);

        // 1回目の装備
        node.fit_module(FitModuleCommand { ship_id, slot: SlotKind::High, module_id: ModuleId(1) });
        let stats_after_first = node.get_ship_stats(ship_id).unwrap();
        assert_eq!(stats_after_first.weapon_damage, 25.0, "1回装備後は base(0) + delta(25) = 25");

        // 2回目の装備
        node.fit_module(FitModuleCommand { ship_id, slot: SlotKind::High, module_id: ModuleId(1) });
        let stats_after_second = node.get_ship_stats(ship_id).unwrap();
        assert_eq!(stats_after_second.weapon_damage, 50.0,
            "2個装備後は base(0) + 2×delta(25) = 50（二重加算なら75になる）");
    }

    // ── Full pipeline: player fires at bot ───────────────────────────────────

    /// Helper: build a SimulationNode with modules and Magpie ship type registered.
    fn node_with_modules() -> SimulationNode {
        use crate::{modules, ship_types};
        let mut node = mem_node();
        for def in modules::all_modules() {
            node.register_module(def);
        }
        for def in ship_types::all_ship_types() {
            node.register_ship_type(def);
        }
        node
    }

    #[test]
    fn player_weapon_deals_damage_to_bot_after_lock_and_activation() {
        use dawn_core::{LockOnCommand, ActivateModuleCommand, SlotKind, ModuleId};

        let mut node = node_with_modules();

        // Spawn bot within weapon range (1500 u optimal, bot at 500 u).
        let bot_pos = Position::new(500.0, 0.0, 0.0);
        let (_, bot_ship_id) = node.spawn_bot_ship(bot_pos);

        // Spawn player at origin.
        let player_id = node.next_player_id();
        let player_ship_id = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);

        // Player locks on bot.
        let lock_cmd = LockOnCommand { ship_id: player_ship_id, target_id: bot_ship_id };

        // Player activates weapon (F1 equivalent).
        assert!(node.activate_module_owned(player_id, ActivateModuleCommand {
            ship_id  : player_ship_id,
            module_id: ModuleId(1),  // Small Railgun I
            slot     : SlotKind::High,
        }), "activate_module_owned should return true for player's own ship");

        // Run 25 ticks — enough for lock (2 ticks) + first weapon cycle (10 ticks)
        // + a few more cycles to guarantee a hit even with RNG variance.
        let mut damage_events = 0;
        for _ in 0..25 {
            let result = node.tick_with_lock_commands(&[lock_cmd.clone()]);
            damage_events += result.events.iter()
                .filter(|e| matches!(e, DomainEvent::DamageTaken(d) if d.ship_id == bot_ship_id))
                .count();
        }

        assert!(damage_events > 0,
            "player should have dealt at least 1 DamageTaken to bot within 25 ticks \
             (lock_time=2, cycle_time=10, bot within optimal range → hit_chance=1.0)");
    }

    // ── ADR-0014 Task 8: Sector Transit scenario tests ───────────────────────

    /// Normal-path Sector Transit: ownership ends up in exactly one Sector,
    /// and at no point do both Sectors hold the Ship at once (INV-003).
    #[test]
    fn transit_moves_ship_ownership_to_destination_sector_exactly_once() {
        let mut from_node = SimulationNode::new(NodeId(0), SectorId(0), SectorBounds::centered(SectorBounds::DEFAULT_HALF));
        let mut to_node   = SimulationNode::new(NodeId(1), SectorId(1), SectorBounds::centered(SectorBounds::DEFAULT_HALF));

        let ship_id = from_node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::new(1.0, 0.0, 0.0));
        assert_eq!(from_node.ship_count() + to_node.ship_count(), 1, "ship starts owned by exactly one sector");

        from_node.propose_transit(dawn_core::commands::TransitCommand { ship_id, to: SectorId(1) }).unwrap();
        // Proposal alone does not move ownership yet.
        assert_eq!(from_node.ship_count() + to_node.ship_count(), 1);

        let entry_pos = Position::new(500.0, 0.0, 0.0);
        let snapshot = from_node.export_transit(ship_id, entry_pos).unwrap();

        // In flight: neither sector owns the ship (no split-brain double-ownership).
        assert_eq!(from_node.ship_count(), 0);
        assert_eq!(to_node.ship_count(), 0);

        to_node.import_transit(&snapshot, SectorId(0), entry_pos);

        // Final state: destination sector owns the ship, exactly once overall.
        assert_eq!(from_node.ship_count(), 0);
        assert_eq!(to_node.ship_count(), 1);
        assert_eq!(to_node.get_ship_position(ship_id), Some(entry_pos));
    }

    /// INV-002: after a Sector Transit, the destination Sector's state can be
    /// fully reproduced from a snapshot + Event Log replay (node restart).
    #[test]
    fn destination_sector_state_after_transit_is_fully_restored_from_snapshot_and_replay() {
        let dir        = tempfile::tempdir().unwrap();
        let event_path = dir.path().join("events.log");
        let snap_path  = dir.path().join("snapshot.bin");

        let mut from_node = SimulationNode::new(NodeId(0), SectorId(0), SectorBounds::centered(SectorBounds::DEFAULT_HALF));
        let ship_id = from_node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::new(1.0, 0.0, 0.0));
        from_node.propose_transit(dawn_core::commands::TransitCommand { ship_id, to: SectorId(1) }).unwrap();
        let entry_pos = Position::new(500.0, 0.0, 0.0);
        let snapshot = from_node.export_transit(ship_id, entry_pos).unwrap();

        {
            let store = FileEventStore::open(&event_path).unwrap();
            let mut to_node = SimulationNode::with_store(
                NodeId(1), SectorId(1),
                SectorBounds::centered(SectorBounds::DEFAULT_HALF),
                store,
            );
            to_node.import_transit(&snapshot, SectorId(0), entry_pos);

            let snap = to_node.take_snapshot();
            snap.save(&snap_path).unwrap();
        } // node drops; FileEventStore flushes via BufWriter

        let snap   = StateSnapshot::load(&snap_path).unwrap();
        let store2 = FileEventStore::open(&event_path).unwrap();
        let restored = SimulationNode::restore_from(store2, &snap, &[], &[]);

        assert_eq!(restored.ship_count(), 1);
        assert_eq!(restored.get_ship_position(ship_id), Some(entry_pos));
    }

    /// ADR-0014 Task 9: measures the cost of a single Sector Transit
    /// (propose + export + import), excluding Raft commit latency.
    ///
    /// Ignored by default (it's a benchmark, not a correctness check).
    /// Run with: `cargo test -p dawn-simulation --release transit_latency_benchmark -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn transit_latency_benchmark() {
        use std::time::Instant;

        const ITERATIONS: u32 = 1_000;
        let mut total = std::time::Duration::ZERO;

        for i in 0..ITERATIONS {
            let mut from_node = SimulationNode::new(NodeId(0), SectorId(0), SectorBounds::centered(SectorBounds::DEFAULT_HALF));
            let mut to_node   = SimulationNode::new(NodeId(1), SectorId(1), SectorBounds::centered(SectorBounds::DEFAULT_HALF));
            let ship_id = from_node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::new(1.0, 0.0, 0.0));
            let entry_pos = Position::new(500.0, 0.0, 0.0);

            let start = Instant::now();
            from_node.propose_transit(dawn_core::commands::TransitCommand { ship_id, to: SectorId(1) }).unwrap();
            let snapshot = from_node.export_transit(ship_id, entry_pos).unwrap();
            to_node.import_transit(&snapshot, SectorId(0), entry_pos);
            total += start.elapsed();

            let _ = i;
        }

        let avg = total / ITERATIONS;
        println!("transit (propose+export+import) avg over {ITERATIONS} iterations: {avg:?}");
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

    // ── Tackle System (ADR-0024) ──────────────────────────────────────────────

    #[test]
    fn bot_starts_aligning_when_hp_drops_below_50_percent() {
        use dawn_core::events::DamageTaken;

        let mut node = node_with_modules();

        // Bot at (1200, 0, 0) — far from gate at ~49000 so MIN_WARP_DISTANCE passes.
        let bot_pos = Position::new(1200.0, 0.0, 0.0);
        let (_, bot_ship_id) = node.spawn_bot_ship(bot_pos);

        // Player at origin — required so process_bots has a target and doesn't early-return.
        let player_id = node.next_player_id();
        let _ = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);

        // Magpie max HP: shield=200, armor=120, hull=100, total=420.
        // Deal 215 damage → shield=0, armor=105, hull=100 → 205/420 ≈ 48.8% < 50%.
        node.apply_event_pub(DomainEvent::DamageTaken(DamageTaken {
            ship_id        : bot_ship_id,
            damage         : 215.0,
            current_shield : 0.0,
            current_armor  : 105.0,
            current_hull   : 100.0,
            tick           : Tick(1),
        }));

        // One tick: process_bots detects hp_fraction < 0.50, calls apply_warp_command.
        node.tick();

        assert!(
            matches!(node.warp_phase(bot_ship_id), Some(WarpPhase::Aligning)),
            "bot should be in WarpPhase::Aligning after hp drops below 50%"
        );
    }

    #[test]
    fn tackled_bot_cannot_warp_but_keeps_fighting() {
        use dawn_core::{FitModuleCommand, LockOnCommand, ActivateModuleCommand, SlotKind, events::DamageTaken};
        use crate::modules::MODULE_FOLD_DISRUPTOR;

        let mut node = node_with_modules();

        // Bot close to player so tackle range is satisfied.
        let bot_pos = Position::new(1000.0, 0.0, 0.0);
        let (_, bot_ship_id) = node.spawn_bot_ship(bot_pos);

        let player_id = node.next_player_id();
        let player_ship_id = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);

        node.fit_module(FitModuleCommand { ship_id: player_ship_id, slot: SlotKind::Mid, module_id: MODULE_FOLD_DISRUPTOR });

        // Activate disruptor and lock bot.
        node.activate_module_owned(player_id, ActivateModuleCommand {
            ship_id  : player_ship_id,
            module_id: MODULE_FOLD_DISRUPTOR,
            slot     : SlotKind::Mid,
        });
        let lock_cmd = LockOnCommand { ship_id: player_ship_id, target_id: bot_ship_id };
        // Run enough ticks for lock to resolve (lock_time=2) and tackle to apply.
        for _ in 0..5 {
            node.tick_with_lock_commands(&[lock_cmd.clone()]);
        }

        // Confirm bot is tackled.
        let gate_id = node.sector_map.gates.keys().next().copied().unwrap();
        assert!(!node.can_propose_warp(bot_ship_id, dawn_core::WarpTarget::Gate(gate_id)), "bot should be tackled");

        // Damage bot below 50% HP.
        node.apply_event_pub(DomainEvent::DamageTaken(DamageTaken {
            ship_id        : bot_ship_id,
            damage         : 215.0,
            current_shield : 0.0,
            current_armor  : 105.0,
            current_hull   : 100.0,
            tick           : Tick(10),
        }));

        node.tick_with_lock_commands(&[lock_cmd.clone()]);

        // Tackle blocks warp — bot must NOT have WarpComp.
        assert!(
            node.warp_phase(bot_ship_id).is_none(),
            "tackled bot should not enter warp"
        );
    }

    // -- Celestial body warp (ADR-0025) -----------------------------------------

    #[test]
    fn warp_to_body_reaches_arrival_distance_of_radius_times_1_5() {
        let mut node = mem_node();
        let (player, ship_id) = spawn_owned_player_at(&mut node, Position::new(0.0, 0.0, 0.0));

        // Sector 0 (Alpha) body_id=1 is "Forge" at x=22_000, radius=3_500.
        let body_id = dawn_core::CelestialBodyId(1);
        let ok = node.apply_warp_command_owned(player, dawn_core::WarpCommand {
            ship_id: ship_id, target: WarpTarget::Body(body_id),
        });
        assert!(ok, "warp to body should be accepted");
        assert!(node.warp_phase(ship_id).is_some(), "ship should have WarpComp");

        // Run ticks until warp completes.
        for _ in 0..5_000 {
            node.tick();
            if node.warp_phase(ship_id).is_none() {
                break;
            }
        }
        assert!(node.warp_phase(ship_id).is_none(), "warp should have completed");

        // Ship should be within radius * 1.5 of the body centre.
        let body = crate::star_map::StarMap::builtin()
            .bodies_in_sector(SectorId(0))
            .into_iter()
            .find(|b| b.id == body_id)
            .unwrap();
        let ship_pos = node.ship_positions()
            .into_iter()
            .find(|(id, _)| *id == ship_id)
            .map(|(_, p)| p)
            .expect("ship exists");
        let dx = ship_pos.x - body.position.x;
        let dy = ship_pos.y - body.position.y;
        let dz = ship_pos.z - body.position.z;
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        let arrival_max = body.radius * 1.5 * 1.05; // 5% tolerance
        assert!(
            dist <= arrival_max,
            "ship distance {:.0} should be within {:.0} of body centre",
            dist, arrival_max,
        );
    }

    #[test]
    fn warp_to_body_is_rejected_for_body_not_in_this_sector() {
        let mut node = mem_node();
        let ship_id  = node.spawn_ship(dawn_core::ShipTypeId(1), Position::new(0.0, 0.0, 0.0), Velocity::ZERO);
        // body_id=2 is in Sector 1 (Beta), not Sector 0.
        let ok = node.apply_warp_command(ship_id, WarpTarget::Body(dawn_core::CelestialBodyId(2)), false);
        assert!(!ok, "warp to body in another sector should be rejected");
    }
}
