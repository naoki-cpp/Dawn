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

use std::collections::HashMap;

use dawn_core::{
    events::{ShipFitted, ShipSpawned},
    ship_type::{ShipTypeDefinition, ShipTypeId},
    DomainEvent, FitModuleCommand, JumpGateDef, JumpGateId, ModuleDefinition, ModuleId, NodeId, PlayerId, Position,
    SectorBounds, SectorId, ShipId, Tick, Velocity,
};
use dawn_ecs::{
    components::{ApproachComp, CapacitorComp, FittingComp, HullComp, IsBotComp, IsNpcComp, LockComp, PositionComp, ShipIdComp, ShipStatsComp, TackledComp, ThrustComp, VelocityComp, WarpComp, WarpPhase},
    systems::{CapacitorSystem, CombatSystem, LockSystem, MovementSystem, apply_fitting},
    Entity, SimWorld,
};
use dawn_event_store::{store::EventStore, InMemoryEventStore};

use crate::snapshot::{ShipSnapshot, StateSnapshot};

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

/// Component of `vel` directed from `pos` toward `target` (units/tick). Negative
/// when moving away. Zero when `pos == target`. Used for EVE-style 75% warp
/// alignment (ADR-0022).
fn speed_toward(vel: Velocity, pos: Position, target: Position) -> f32 {
    let dx = target.x - pos.x;
    let dy = target.y - pos.y;
    let dz = target.z - pos.z;
    let dist = (dx * dx + dy * dy + dz * dz).sqrt();
    if dist < f32::EPSILON {
        return 0.0;
    }
    (vel.dx * dx + vel.dy * dy + vel.dz * dz) / dist
}

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
    /// Maps `ShipId` to hecs `Entity` for O(1) position updates during replay.
    ship_index      : HashMap<ShipId, Entity>,
    /// Module definition registry.
    module_registry   : HashMap<ModuleId, ModuleDefinition>,
    /// Ship type definition registry.
    ship_type_registry: HashMap<ShipTypeId, ShipTypeDefinition>,
    /// Bare ShipStats without fitting. Used as the base for fitting aggregation.
    base_stats         : HashMap<ShipId, ShipStatsComp>,
    /// PlayerId -> ShipId (player ship ownership).
    player_ships       : HashMap<PlayerId, ShipId>,
    /// ShipId -> PlayerId (reverse lookup).
    ship_owners        : HashMap<ShipId, PlayerId>,
    /// ShipId -> ShipTypeId (reverse lookup; used to include ship_type_name in InitialState).
    ship_type_ids      : HashMap<ShipId, ShipTypeId>,
    /// PlayerId allocation counter.
    player_id_counter  : u64,
    /// Lock-on commands queued by the bot AI during `process_bots()`.
    ///
    /// Bot AI runs after the LockSystem each tick.  These commands are held
    /// here and injected into the LockSystem at the start of the NEXT tick,
    /// ensuring they are processed exactly like human-issued lock commands.
    pending_bot_lock_commands: Vec<dawn_core::LockOnCommand>,
    /// Jump Gates whose `from_sector` is this node's Sector (ADR-0009).
    jump_gates: HashMap<JumpGateId, JumpGateDef>,
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
            ship_index         : HashMap::new(),
            module_registry    : HashMap::new(),
            ship_type_registry : HashMap::new(),
            base_stats         : HashMap::new(),
            player_ships       : HashMap::new(),
            ship_owners        : HashMap::new(),
            ship_type_ids             : HashMap::new(),
            player_id_counter         : 0,
            pending_bot_lock_commands : Vec::new(),
            jump_gates                : crate::star_map::gates_in_sector(sector_id)
                .into_iter()
                .map(|g| (g.id, g))
                .collect(),
            population_cap            : POPULATION_CAP,
            pending_auto_jumps        : Vec::new(),
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
            ship_index         : HashMap::new(),
            module_registry    : HashMap::new(),
            ship_type_registry : HashMap::new(),
            base_stats         : HashMap::new(),
            player_ships       : HashMap::new(),
            ship_owners        : HashMap::new(),
            ship_type_ids             : HashMap::new(),
            player_id_counter         : 0,
            pending_bot_lock_commands : Vec::new(),
            jump_gates                : crate::star_map::gates_in_sector(snapshot.sector_id)
                .into_iter()
                .map(|g| (g.id, g))
                .collect(),
            population_cap            : POPULATION_CAP,
            pending_auto_jumps        : Vec::new(),
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

    // ── Identity ──────────────────────────────────────────────────────────────

    pub fn node_id(&self)    -> NodeId   { self.node_id }
    pub fn sector_id(&self)  -> SectorId { self.sector_id }

    // ── Spawn ─────────────────────────────────────────────────────────────────

    /// Spawn a Ship, record it in the ECS, append a `ShipSpawned` event.
    ///
    /// INV-004: the ID is generated from a monotonically increasing counter
    /// combined with `NodeId`.  IDs are never reused.
    pub fn spawn_ship(&mut self, ship_type_id: ShipTypeId, position: Position, velocity: Velocity) -> ShipId {
        let ship_id = ShipId::new(self.node_id, self.id_counter);
        self.id_counter += 1;

        let base = self.ship_type_registry
            .get(&ship_type_id)
            .map(|def| ShipStatsComp::from_base(&def.base_stats))
            .unwrap_or(ShipStatsComp::NPC);

        self.insert_to_world(ship_id, position, velocity);
        self.base_stats.insert(ship_id, base);
        self.ship_type_ids.insert(ship_id, ship_type_id);

        // Update ShipStatsComp, HullComp, and CapacitorComp to match base stats.
        if let Some(&entity) = self.ship_index.get(&ship_id) {
            self.world.set_ship_stats(entity, base);
            if let Ok(mut hull) = self.world.inner_mut().get::<&mut HullComp>(entity) {
                *hull = HullComp::new(base.max_shield, base.max_armor, base.max_hull);
            }
            // Initialize capacitor to full.
            let _ = self.world.inner_mut().insert_one(entity, CapacitorComp { current: base.cap_max });
        }

        self.event_store.append(DomainEvent::ShipSpawned(ShipSpawned {
            ship_id,
            sector_id        : self.sector_id,
            initial_position : position,
            ship_type_id,
            tick             : self.current_tick,
        }));

        ship_id
    }

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
        let &entity = self.ship_index.get(&cmd.ship_id)
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
        self.ship_index
            .get(&ship_id)
            .is_some_and(|&entity| !self.world.transit_state(entity).is_in_transit())
    }

    // ── Jump Gate Navigation (ADR-0009) ──────────────────────────────────────

    /// Look up a Jump Gate originating in this Sector by `gate_id`.
    pub fn jump_gate(&self, gate_id: JumpGateId) -> Option<&JumpGateDef> {
        self.jump_gates.get(&gate_id)
    }

    /// Whether a `JumpCommand` for `ship_id` via `gate_id` would currently be
    /// accepted: the Ship exists, is not already in transit, the gate
    /// originates in this Sector, and the Ship is within its
    /// `activation_radius`. Used to reject commands up front, before
    /// proposing to the Raft Log (INV-006).
    pub fn can_propose_jump(&self, ship_id: ShipId, gate_id: JumpGateId) -> bool {
        let Some(&entity) = self.ship_index.get(&ship_id) else { return false };
        if self.world.transit_state(entity).is_in_transit() {
            return false;
        }
        // Tackled ships cannot jump (ADR-0024).
        if self.world.inner().get::<&TackledComp>(entity).is_ok() {
            return false;
        }
        let Some(gate) = self.jump_gates.get(&gate_id) else { return false };
        let Ok(pos) = self.world.inner().get::<&PositionComp>(entity) else { return false };
        gate.is_in_range(pos.0)
    }

    // ── Intra-Sector Warp (short-range Fold, ADR-0022) ───────────────────────

    /// Whether a `WarpCommand` for `ship_id` toward `gate_id` would currently be
    /// accepted (INV-006 Validation, before attaching `WarpComp`):
    /// the Ship exists, is not in transit, is not already warping, not tackled,
    /// the gate originates in this Sector, and the gate is at least
    /// `MIN_WARP_DISTANCE` away (closer than that → use approach instead).
    pub fn can_propose_warp(&self, ship_id: ShipId, gate_id: JumpGateId) -> bool {
        let Some(&entity) = self.ship_index.get(&ship_id) else { return false };
        if self.world.transit_state(entity).is_in_transit() {
            return false;
        }
        if self.world.inner().get::<&WarpComp>(entity).is_ok() {
            return false; // already aligning or warping
        }
        // Tackled ships cannot warp (ADR-0024).
        if self.world.inner().get::<&TackledComp>(entity).is_ok() {
            return false;
        }
        let Some(gate) = self.jump_gates.get(&gate_id) else { return false };
        let Ok(pos) = self.world.inner().get::<&PositionComp>(entity) else { return false };
        pos.0.distance(gate.position) >= MIN_WARP_DISTANCE
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

        let from_system = crate::star_map::system_for_sector(from);
        let to_system   = crate::star_map::system_for_sector(to);
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
        let &entity = self.ship_index.get(&ship_id)?;
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
        let ship_type_id = self.ship_type_ids.get(&ship_id).copied().unwrap_or(ShipTypeId(0));

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

        self.ship_index.remove(&ship_id);
        self.world.despawn_ship(entity);
        self.ship_type_ids.remove(&ship_id);
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

    // ── Tick ──────────────────────────────────────────────────────────────────

    /// Execute one simulation tick.
    ///
    /// 1. Advance the logical tick counter.
    /// 2. Run the Movement System (ECS batch).
    /// 3. Append produced events to the Event Store.
    /// 4. Return a `TickResult` (events included for Actor-layer replication).
    pub fn tick(&mut self) -> TickResult {
        self.tick_with_lock_commands(&[])
    }

    pub fn tick_with_lock_commands(
        &mut self,
        lock_commands: &[dawn_core::LockOnCommand],
    ) -> TickResult {
        self.current_tick = self.current_tick.next();
        let tick = self.current_tick;

        // 2.5 Approach System — re-aim thrust at approach targets (ADR-0015)
        self.process_approach();

        // 2.6 Warp System — advance intra-Sector warps (short-range Fold, ADR-0022)
        let warp_events = self.process_warp(tick);

        // 3. Movement System (skips ships in the committed warping phase)
        let move_events = MovementSystem::run(&mut self.world, tick);

        // 4. Capacitor System — recharge and drain for active modules
        let cap = CapacitorSystem(&mut self.world, tick);

        // Drain pending bot lock commands and merge with human-issued ones.
        let bot_locks: Vec<dawn_core::LockOnCommand> =
            std::mem::take(&mut self.pending_bot_lock_commands);
        // Re-apply fitting for any ship whose module was force-deactivated.
        for ship_id in &cap.refitted {
            if let Some(&base) = self.base_stats.get(ship_id) {
                apply_fitting(&mut self.world, *ship_id, base);
            }
        }

        // 4.5 Tackle System — update TackledComp for active Tackle modules (ADR-0024)
        let tackle_events = self.process_tackle(tick);

        // 5. Lock System — merge human commands with queued bot commands
        let merged_locks: Vec<dawn_core::LockOnCommand> = bot_locks
            .into_iter()
            .chain(lock_commands.iter().cloned())
            .collect();
        let lock = LockSystem(&mut self.world, tick, &merged_locks);

        // 6. Combat System — fire only when the capacitor weapon cycle started this tick
        let combat = CombatSystem(&mut self.world, tick, &cap.weapon_cycles_started);

        // Remove destroyed ships from the ECS and all lookup maps.
        // CLAUDE.md §6: run the Bot System after Combat.
        for ship_id in &combat.destroyed {
            if let Some(entity) = self.ship_index.remove(ship_id) {
                self.world.despawn_ship(entity);
            }
            self.ship_type_ids.remove(ship_id);
            self.base_stats.remove(ship_id);
            if let Some(player_id) = self.ship_owners.remove(ship_id) {
                self.player_ships.remove(&player_id);
            }
        }

        // 7. Bot System — bots issue the same commands as human players
        self.process_bots();

        // 8. Append to the EventStore
        let all_events: Vec<DomainEvent> = warp_events.iter()
            .chain(move_events.iter())
            .chain(cap.events.iter())
            .chain(tackle_events.iter())
            .chain(lock.events.iter())
            .chain(combat.events.iter())
            .cloned()
            .collect();

        let count = all_events.len();
        self.event_store.append_batch(all_events.iter().cloned());

        TickResult {
            tick,
            events_emitted: count,
            events: all_events,
            cap_depletions: cap.refitted.clone(),
        }
    }

    // ── Snapshot ──────────────────────────────────────────────────────────────

    /// Capture the current ECS state as a `StateSnapshot`.
    ///
    /// The snapshot covers all events with `log_index < event_store.len()`.
    /// Pair with the event log to reconstruct this exact state on restart.
    pub fn take_snapshot(&self) -> StateSnapshot {
        let mut ships: Vec<ShipSnapshot> = self
            .ship_index
            .iter()
            .filter_map(|(&ship_id, &entity)| {
                let pos  = self.world.inner().get::<&PositionComp>(entity).ok()?.0;
                let vel  = self.world.inner().get::<&VelocityComp>(entity).ok()?.0;
                let hull = self.world.inner().get::<&HullComp>(entity).ok()?;
                let capacitor = self.world.inner().get::<&CapacitorComp>(entity).ok().map(|c| c.current);
                let fitting = self.world.inner().get::<&FittingComp>(entity)
                    .map(|f| f.to_snapshot())
                    .unwrap_or_else(|_| dawn_core::fitting::FittingSnapshot::empty());
                let tackled_by = self.world.inner().get::<&TackledComp>(entity)
                    .map(|t| t.tacklers.clone())
                    .unwrap_or_default();
                let ship_type_id = self.ship_type_ids.get(&ship_id).copied().unwrap_or(ShipTypeId(0));
                Some(ShipSnapshot {
                    ship_id,
                    ship_type_id,
                    position      : pos,
                    velocity      : vel,
                    current_shield: hull.current_shield,
                    current_armor : hull.current_armor,
                    current_hull  : hull.current_hull,
                    is_destroyed  : hull.is_destroyed,
                    capacitor,
                    fitting,
                    tackled_by,
                })
            })
            .collect();

        // Canonical ordering: a snapshot of a given state must serialise
        // identically regardless of HashMap iteration order, so it can be
        // byte-compared (INV-002: verifiable snapshot / round-trip).
        // `ShipId: Ord` is canonical id order (node_id then counter).
        ships.sort_by_key(|s| s.ship_id);

        StateSnapshot {
            node_id   : self.node_id,
            sector_id : self.sector_id,
            bounds    : self.bounds,
            log_index : self.event_store.len() as u64,
            tick      : self.current_tick,
            id_counter: self.id_counter,
            ships,
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
        let entity = self.ship_index.get(&ship_id)?;
        self.world.inner().get::<&ApproachComp>(*entity).ok().map(|a| a.target)
    }

    /// The Ship's current warp phase, if it is warping (ADR-0022).
    #[cfg(test)]
    pub fn warp_phase(&self, ship_id: ShipId) -> Option<WarpPhase> {
        let entity = self.ship_index.get(&ship_id)?;
        self.world.inner().get::<&WarpComp>(*entity).ok().map(|w| w.phase)
    }

    /// Look up the current position of a Ship by its ID.
    pub fn get_ship_position(&self, ship_id: ShipId) -> Option<Position> {
        let entity = self.ship_index.get(&ship_id)?;
        self.world.inner().get::<&PositionComp>(*entity).ok().map(|c| c.0)
    }

    /// Look up the current `ShipStatsComp` of a Ship by its ID. Test-only.
    #[cfg(test)]
    pub fn get_ship_stats(&self, ship_id: ShipId) -> Option<ShipStatsComp> {
        let entity = self.ship_index.get(&ship_id)?;
        self.world.inner().get::<&ShipStatsComp>(*entity).ok().map(|c| *c)
    }

    /// Look up the current HP of a Ship by its ID. Test-only.
    #[cfg(test)]
    pub fn get_ship_hp(&self, ship_id: ShipId) -> Option<f32> {
        let entity = self.ship_index.get(&ship_id)?;
        self.world.inner().get::<&HullComp>(*entity).ok()
            .map(|c| c.current_shield + c.current_armor + c.current_hull)
    }

    /// Public test wrapper for `apply_event`.
    #[cfg(test)]
    pub fn apply_event_pub(&mut self, event: DomainEvent) {
        self.apply_event(&event);
    }

    /// Look up the current `CapacitorComp.current` of a Ship by its ID.
    #[cfg(test)]
    pub fn get_ship_capacitor(&self, ship_id: ShipId) -> Option<f32> {
        let entity = self.ship_index.get(&ship_id)?;
        self.world.inner().get::<&CapacitorComp>(*entity).ok().map(|c| c.current)
    }

    /// `(ModuleId, is_active)` for every fitted module on a Ship, across all slots.
    #[cfg(test)]
    pub fn get_fitted_module_ids(&self, ship_id: ShipId) -> Vec<(ModuleId, bool)> {
        let entity = match self.ship_index.get(&ship_id) {
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

    pub fn apply_move_command(&mut self, ship_id: ShipId, target: Position) {
        let entity = match self.ship_index.get(&ship_id) {
            Some(&e) => e,
            None     => return,
        };
        if self.world.transit_state(entity).is_in_transit() {
            return;
        }
        // A committed warp cannot be interrupted; an aligning warp is cancelled
        // (ADR-0022 §7).
        if self.is_warping(entity) {
            return;
        }
        let _ = self.world.inner_mut().remove_one::<WarpComp>(entity);
        // Manual thrust overrides any active approach (ADR-0015 §4).
        let _ = self.world.inner_mut().remove_one::<ApproachComp>(entity);
        let pos = match self.world.inner().get::<&PositionComp>(entity).ok() {
            Some(c) => c.0,
            None    => return,
        };
        self.steer_thrust_toward(entity, pos, target);
    }

    /// True if the ship is in the committed warping phase (ADR-0022): its warp
    /// cannot be interrupted by Move/Stop. Aligning or absent warp → false.
    fn is_warping(&self, entity: Entity) -> bool {
        self.world.inner().get::<&WarpComp>(entity).map(|w| w.is_warping()).unwrap_or(false)
    }

    /// Point `entity`'s thrust at `to` from `from` (unit direction, not braking).
    /// Zero thrust if already at the target. Shared by `apply_move_command` and
    /// the Approach System (ADR-0015) so the steering math lives in one place.
    fn steer_thrust_toward(&mut self, entity: Entity, from: Position, to: Position) {
        let dx   = to.x - from.x;
        let dy   = to.y - from.y;
        let dz   = to.z - from.z;
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        let dir = if dist > f32::EPSILON {
            Velocity { dx: dx / dist, dy: dy / dist, dz: dz / dist }
        } else {
            Velocity::ZERO
        };
        if let Ok(mut t) = self.world.inner_mut().get::<&mut ThrustComp>(entity) {
            t.direction  = dir;
            t.is_braking = false;
        }
    }

    /// Set `entity`'s thrust to braking (decelerate toward zero velocity).
    /// Shared by `apply_stop_command` and the Approach System (ADR-0015).
    fn brake_thrust(&mut self, entity: Entity) {
        if let Ok(mut t) = self.world.inner_mut().get::<&mut ThrustComp>(entity) {
            t.direction  = Velocity::ZERO;
            t.is_braking = true;
        }
    }

    /// Begin decelerating the ship toward zero velocity using its thrust.
    ///
    /// The movement system applies thrust opposite to velocity each tick until
    /// the ship stops. Cancels any active thrust direction.
    pub fn apply_stop_command(&mut self, ship_id: ShipId) {
        let entity = match self.ship_index.get(&ship_id) {
            Some(&e) => e,
            None     => return,
        };
        if self.world.transit_state(entity).is_in_transit() {
            return;
        }
        // A committed warp cannot be interrupted; an aligning warp is cancelled
        // (ADR-0022 §7).
        if self.is_warping(entity) {
            return;
        }
        let _ = self.world.inner_mut().remove_one::<WarpComp>(entity);
        // Stopping cancels any active approach (ADR-0015 §4).
        let _ = self.world.inner_mut().remove_one::<ApproachComp>(entity);
        self.brake_thrust(entity);
    }

    /// `apply_stop_command` wrapped with ownership check.
    pub fn apply_stop_command_owned(&mut self, player_id: PlayerId, ship_id: ShipId) -> bool {
        if self.ship_owners.get(&ship_id) != Some(&player_id) {
            return false;
        }
        self.apply_stop_command(ship_id);
        true
    }

    /// Mark a ship as a player ship and apply the PLAYER stat profile.
    ///
    /// Test-only setup helper; the serve path adopts player ships via
    /// `adopt_player_ship`.
    #[cfg(test)]
    pub fn set_player_ship(&mut self, ship_id: ShipId) {
        if let Some(&entity) = self.ship_index.get(&ship_id) {
            self.base_stats.insert(ship_id, ShipStatsComp::PLAYER);
            self.world.set_ship_stats(entity, ShipStatsComp::PLAYER);
            if let Ok(mut hull) = self.world.inner_mut().get::<&mut HullComp>(entity) {
                *hull = HullComp::new(
                    ShipStatsComp::PLAYER.max_shield,
                    ShipStatsComp::PLAYER.max_armor,
                    ShipStatsComp::PLAYER.max_hull,
                );
            }
            let base = ShipStatsComp::PLAYER;
            apply_fitting(&mut self.world, ship_id, base);
        }
    }

    // ── ShipType ──────────────────────────────────────────────────────────────

    pub fn register_ship_type(&mut self, def: ShipTypeDefinition) {
        self.ship_type_registry.insert(def.id, def);
    }

    // ── Phase 5: PlayerId / ownership management ─────────────────────────────

    pub fn next_player_id(&mut self) -> PlayerId {
        let id = PlayerId(self.player_id_counter);
        self.player_id_counter += 1;
        id
    }

    /// Spawn a player ship at the default starting position.
    pub fn spawn_player_ship(&mut self, player_id: PlayerId) -> ShipId {
        self.spawn_player_ship_at(player_id, Position::new(0.0, 0.0, 0.0))
    }

    /// Spawn a player ship at a specific position.
    pub fn spawn_player_ship_at_pub(&mut self, player_id: PlayerId, pos: Position) -> ShipId {
        self.spawn_player_ship_at(player_id, pos)
    }

    fn spawn_player_ship_at(&mut self, player_id: PlayerId, pos: Position) -> ShipId {
        use crate::ship_types::SHIP_TYPE_MAGPIE;
        let ship_id = ShipId::new(self.node_id, self.id_counter);
        self.id_counter += 1;

        let base = self.ship_type_registry
            .get(&SHIP_TYPE_MAGPIE)
            .map(|def| ShipStatsComp::from_base(&def.base_stats))
            .unwrap_or(ShipStatsComp::PLAYER);

        self.insert_to_world(ship_id, pos, Velocity::ZERO);
        self.base_stats.insert(ship_id, base);
        self.ship_type_ids.insert(ship_id, SHIP_TYPE_MAGPIE);

        if let Some(&entity) = self.ship_index.get(&ship_id) {
            self.world.set_ship_stats(entity, base);
            if let Ok(mut hull) = self.world.inner_mut().get::<&mut HullComp>(entity) {
                *hull = HullComp::new(base.max_shield, base.max_armor, base.max_hull);
            }
            let _ = self.world.inner_mut().insert_one(entity, CapacitorComp { current: base.cap_max });
            let _ = self.world.inner_mut().remove_one::<IsNpcComp>(entity);
        }

        // Record ownership before fit_module (needed for is_npc check).
        self.player_ships.insert(player_id, ship_id);
        self.ship_owners.insert(ship_id, player_id);

        use dawn_core::SlotKind;
        self.fit_module(FitModuleCommand {
            ship_id,
            slot      : SlotKind::High,
            module_id : crate::modules::MODULE_RAILGUN_SMALL,
        });
        self.fit_module(FitModuleCommand {
            ship_id,
            slot      : SlotKind::Mid,
            module_id : crate::modules::MODULE_AFTERBURNER,
        });

        self.event_store.append(DomainEvent::ShipSpawned(ShipSpawned {
            ship_id,
            sector_id        : self.sector_id,
            initial_position : pos,
            ship_type_id     : SHIP_TYPE_MAGPIE,
            tick             : self.current_tick,
        }));

        ship_id
    }

    /// Register `player_id` as the owner of a ship already present in this
    /// node's ECS — the ownership handoff after a Sector Transit moved a
    /// player's ship into this Sector (ADR-0009 / ADR-0014).
    ///
    /// `import_transit` restores the ship's state but ownership
    /// (`ship_owners`) is node-local and is NOT part of `ShipSnapshot`;
    /// the serving layer calls this on the destination node so the player's
    /// `*_owned` commands keep working after the jump.
    ///
    /// Returns `false` (and registers nothing) if the ship is not in this
    /// node's ECS.
    pub fn adopt_player_ship(&mut self, ship_id: ShipId, player_id: PlayerId) -> bool {
        if !self.ship_index.contains_key(&ship_id) {
            return false;
        }
        self.player_ships.insert(player_id, ship_id);
        self.ship_owners.insert(ship_id, player_id);
        true
    }

    /// Spawn a Bot ship at the given position.
    ///
    /// The Bot receives a real `PlayerId` and goes through the same player
    /// command pipeline. `IsBotComp` marks it for `process_bots()` each tick.
    pub fn spawn_bot_ship(&mut self, spawn_pos: Position) -> (PlayerId, ShipId) {
        let player_id = self.next_player_id();
        let ship_id   = self.spawn_player_ship_at(player_id, spawn_pos);
        if let Some(&entity) = self.ship_index.get(&ship_id) {
            let _ = self.world.inner_mut().insert_one(entity, IsBotComp);
        }
        (player_id, ship_id)
    }

    /// Run the Bot AI for all `IsBotComp` ships.
    ///
    /// Bots issue the exact same commands as a human player:
    /// `MoveCommand`, `LockOnCommand`, `ActivateModuleCommand`.
    /// Called each tick after Combat System.
    pub fn process_bots(&mut self) {
        // ── 1. Snapshot bot state (read-only pass) ────────────────────────────
        struct BotState {
            player_id     : PlayerId,
            ship_id       : ShipId,
            position      : Position,
            weapon_range  : f32,
            locked_targets: Vec<ShipId>,
            weapon_modules: Vec<(dawn_core::ModuleId, dawn_core::fitting::SlotKind)>,
        }

        let mut bots: Vec<BotState> = Vec::new();
        for (&ship_id, &entity) in &self.ship_index {
            if self.world.inner().get::<&IsBotComp>(entity).is_err() { continue }
            let Some(&player_id) = self.ship_owners.get(&ship_id) else { continue };
            let Ok(pos)   = self.world.inner().get::<&PositionComp>(entity) else { continue };
            let Ok(stats) = self.world.inner().get::<&ShipStatsComp>(entity) else { continue };
            let Ok(lock)  = self.world.inner().get::<&LockComp>(entity) else { continue };
            let locked: Vec<ShipId> = lock.entries.iter()
                .filter(|e| matches!(e.state, dawn_ecs::components::LockState::Locked))
                .map(|e| e.target_id)
                .collect();
            // Bots only auto-activate Weapon modules. Other Active modules
            // (e.g. Afterburner) would drain the capacitor pointlessly while
            // the bot is braking to hold its firing position.
            let weapon_modules = self.world.inner()
                .get::<&FittingComp>(entity)
                .map(|f| {
                    f.high.iter().chain(f.mid.iter()).chain(f.low.iter()).chain(f.rig.iter())
                        .filter(|s| s.def.kind == dawn_core::fitting::ModuleKind::Weapon)
                        .map(|s| (s.def.id, s.def.slot))
                        .collect()
                })
                .unwrap_or_default();
            bots.push(BotState {
                player_id, ship_id,
                position    : pos.0,
                weapon_range: stats.weapon_range,
                locked_targets: locked,
                weapon_modules,
            });
        }

        // ── 2. Snapshot human player target positions ─────────────────────────
        struct TargetInfo { ship_id: ShipId, position: Position }
        let mut targets: Vec<TargetInfo> = Vec::new();
        for (&ship_id, &entity) in &self.ship_index {
            if self.world.inner().get::<&IsBotComp>(entity).is_ok()  { continue }
            if self.world.inner().get::<&IsNpcComp>(entity).is_ok()  { continue }
            if !self.ship_owners.contains_key(&ship_id)              { continue }
            let Ok(pos) = self.world.inner().get::<&PositionComp>(entity) else { continue };
            targets.push(TargetInfo { ship_id, position: pos.0 });
        }

        if targets.is_empty() { return; }

        // ── 3. Issue commands (same pipeline as human player) ─────────────────
        for bot in bots {
            // Find closest human target.
            let Some(target) = targets.iter().min_by(|a, b| {
                bot.position.distance_squared(a.position)
                    .partial_cmp(&bot.position.distance_squared(b.position))
                    .unwrap_or(std::cmp::Ordering::Equal)
            }) else { continue };

            let dist = bot.position.distance(target.position);

            // Lock on if not already locked or locking.
            let already_targeting = bot.locked_targets.contains(&target.ship_id);
            if !already_targeting {
                // Queue lock command for the NEXT tick's LockSystem.
                // (LockSystem already ran this tick before process_bots.)
                self.pending_bot_lock_commands.push(dawn_core::LockOnCommand {
                    ship_id  : bot.ship_id,
                    target_id: target.ship_id,
                });
            }

            // Move: approach until within 75% of weapon range, then brake to stop.
            // Stopping within engage range keeps transversal velocity near zero so
            // that the player's turrets can easily track back — and the bot's shots
            // also benefit from a stable firing platform.
            let engage_range = (bot.weapon_range * 0.75).max(500.0);
            if dist > engage_range {
                let dx = target.position.x - bot.position.x;
                let dy = target.position.y - bot.position.y;
                let dz = target.position.z - bot.position.z;
                let len = (dx*dx + dy*dy + dz*dz).sqrt().max(1.0);
                let thrust_target = Position::new(
                    bot.position.x + dx / len * 1_000_000.0,
                    bot.position.y + dy / len * 1_000_000.0,
                    bot.position.z + dz / len * 1_000_000.0,
                );
                self.apply_move_command_owned(bot.player_id, bot.ship_id, thrust_target);
            } else {
                // Within engage range: brake to stop so turrets have a clean shot.
                self.apply_stop_command_owned(bot.player_id, bot.ship_id);
            }

            // Activate weapons once target is locked.
            if already_targeting {
                for (module_id, slot) in &bot.weapon_modules {
                    self.activate_module_owned(bot.player_id, dawn_core::ActivateModuleCommand {
                        ship_id  : bot.ship_id,
                        module_id: *module_id,
                        slot     : *slot,
                    });
                }
            }
        }
    }


    pub fn apply_move_command_owned(
        &mut self,
        player_id : PlayerId,
        ship_id   : ShipId,
        target    : Position,
    ) -> bool {
        if self.ship_owners.get(&ship_id) != Some(&player_id) {
            return false;
        }
        self.apply_move_command(ship_id, target);
        true
    }

    pub fn apply_lock_on_owned(
        &mut self,
        player_id : PlayerId,
        cmd       : dawn_core::LockOnCommand,
    ) -> bool {
        if self.ship_owners.get(&cmd.ship_id) != Some(&player_id) {
            return false;
        }
        true
    }

    /// Begin approaching a Ship or a Jump Gate (semi-automatic piloting, ADR-0015).
    ///
    /// Attaches an `ApproachComp` so `process_approach()` re-aims thrust at the
    /// target each tick. Rejected (returns `false`, no component attached) if
    /// the ship is unknown or in transit, a `Ship` target is unknown or the
    /// ship itself, or a `Gate` target does not originate in this Sector.
    pub fn apply_approach_command(&mut self, ship_id: ShipId, target: dawn_core::ApproachTarget) -> bool {
        use dawn_core::ApproachTarget;
        let &entity = match self.ship_index.get(&ship_id) {
            Some(e) => e,
            None    => return false,
        };
        if self.world.transit_state(entity).is_in_transit() {
            return false;
        }
        match target {
            ApproachTarget::Ship(target_id) => {
                if ship_id == target_id || !self.ship_index.contains_key(&target_id) {
                    return false;
                }
            }
            ApproachTarget::Gate(gate_id) => {
                if self.jump_gate(gate_id).is_none() {
                    return false;
                }
            }
        }
        let _ = self.world.inner_mut().insert_one(entity, ApproachComp { target });
        true
    }

    /// `apply_approach_command` wrapped with an ownership check.
    pub fn apply_approach_command_owned(
        &mut self,
        player_id : PlayerId,
        cmd       : dawn_core::ApproachCommand,
    ) -> bool {
        if self.ship_owners.get(&cmd.ship_id) != Some(&player_id) {
            return false;
        }
        self.apply_approach_command(cmd.ship_id, cmd.target)
    }

    /// Begin an intra-Sector warp toward a Jump Gate (short-range Fold, ADR-0022).
    ///
    /// Attaches a `WarpComp` in the `Aligning` phase if `can_propose_warp`
    /// accepts the request; `process_warp()` then advances the alignment and,
    /// once aligned, flies the ship to the gate at warp speed. Returns `false`
    /// (no component attached) on rejection.
    pub fn apply_warp_command(&mut self, ship_id: ShipId, gate_id: JumpGateId, auto_jump: bool) -> bool {
        if !self.can_propose_warp(ship_id, gate_id) {
            return false;
        }
        let &entity = match self.ship_index.get(&ship_id) {
            Some(e) => e,
            None    => return false,
        };
        let _ = self.world.inner_mut().insert_one(entity, WarpComp {
            gate_id,
            phase: WarpPhase::Aligning,
            auto_jump,
        });
        true
    }

    /// `apply_warp_command` wrapped with an ownership check.
    pub fn apply_warp_command_owned(
        &mut self,
        player_id : PlayerId,
        cmd       : dawn_core::WarpCommand,
    ) -> bool {
        if self.ship_owners.get(&cmd.ship_id) != Some(&player_id) {
            return false;
        }
        self.apply_warp_command(cmd.ship_id, cmd.gate_id, false)
    }

    /// Drain auto-jump triggers accumulated during `process_warp()`.
    ///
    /// The caller (server loop) is responsible for proposing each returned
    /// `(ship_id, gate_id)` pair to the Raft Log (cluster mode) or ignoring it
    /// (single-node mode where Jump is not supported).
    pub fn drain_pending_auto_jumps(&mut self) -> Vec<(ShipId, JumpGateId)> {
        std::mem::take(&mut self.pending_auto_jumps)
    }

    /// Warp System (ADR-0022 §6): advance every ship carrying a `WarpComp`.
    ///
    /// Runs each tick as Step 2.6 (after Approach, before Movement). The
    /// `Aligning` phase steers the ship at the gate and accelerates it; warp
    /// engages once it is moving at ≥ `WARP_ALIGN_FRACTION` of max speed toward
    /// the gate (EVE-style alignment — interruptible by Move/Stop and, later,
    /// tackle). Once `Warping`, the ship flies straight to the gate at warp
    /// speed and decelerates into its activation radius. Warping ships are
    /// skipped by the Movement System, so this method owns their position,
    /// velocity, and `VelocityChanged` events. Returns those events.
    pub fn process_warp(&mut self, tick: Tick) -> Vec<DomainEvent> {
        // Collect warpers up front so the ECS query borrow is released before
        // the mutable write pass below.
        let warpers: Vec<(Entity, ShipId, WarpComp, Position, Velocity, f32)> = self.world.inner()
            .query::<(&ShipIdComp, &WarpComp, &PositionComp, &VelocityComp, &ShipStatsComp)>()
            .iter()
            .map(|(e, (id, w, p, v, s))| (e, id.0, *w, p.0, v.0, s.max_speed))
            .collect();

        let mut events = Vec::new();

        for (entity, ship_id, warp, pos, vel, max_speed) in warpers {
            // Resolve the destination gate; if it vanished, cancel and brake.
            let Some((gate_pos, arrival)) = self.jump_gate(warp.gate_id)
                .map(|g| (g.position, g.activation_radius * WARP_ARRIVAL_FACTOR))
            else {
                let _ = self.world.inner_mut().remove_one::<WarpComp>(entity);
                self.brake_thrust(entity);
                continue;
            };

            // Tackle interrupts the aligning phase (ADR-0024): a tackled ship
            // cannot enter warp. Cancel and brake; the Warping phase is committed.
            if warp.phase == WarpPhase::Aligning
                && self.world.inner().get::<&TackledComp>(entity).is_ok()
            {
                let _ = self.world.inner_mut().remove_one::<WarpComp>(entity);
                self.brake_thrust(entity);
                continue;
            }

            // Engage warp once aligned: moving at ≥ 75% of max speed toward the
            // gate. While not yet aligned, keep steering/accelerating at it.
            let aligned = max_speed > f32::EPSILON
                && speed_toward(vel, pos, gate_pos) >= WARP_ALIGN_FRACTION * max_speed;

            match warp.phase {
                WarpPhase::Aligning if !aligned => {
                    self.steer_thrust_toward(entity, pos, gate_pos);
                }
                // Aligned (engage warp) or already warping: fly toward the gate.
                WarpPhase::Aligning | WarpPhase::Warping => {
                    self.set_warp_phase(entity, WarpPhase::Warping);
                    if let Some(ev) = self.warp_step(entity, ship_id, pos, vel, gate_pos, arrival, warp.auto_jump, tick) {
                        events.push(ev);
                    }
                }
            }
        }

        events
    }

    /// One warping-phase step: move `entity` toward `gate_pos` at `WARP_SPEED`,
    /// stopping inside `arrival`. Returns a `VelocityChanged` if velocity moved.
    fn warp_step(
        &mut self,
        entity   : Entity,
        ship_id  : ShipId,
        pos      : Position,
        old_vel  : Velocity,
        gate_pos : Position,
        arrival  : f32,
        auto_jump: bool,
        tick     : Tick,
    ) -> Option<DomainEvent> {
        let dist      = pos.distance(gate_pos);
        let remaining = dist - arrival;
        let (new_pos, new_vel, arrived) = if remaining <= WARP_EXIT_SPEED {
            // Close enough (inside the arrival ring or one slow step away):
            // settle and stop. Still well within the gate's activation radius.
            (pos, Velocity::ZERO, true)
        } else {
            // Ease in: cap speed by remaining distance so the ship decelerates
            // smoothly toward the gate instead of stopping dead (ADR-0022 §9).
            let speed = (remaining * WARP_DECEL_RATE).clamp(WARP_EXIT_SPEED, WARP_SPEED);
            let step  = speed.min(remaining);
            let inv   = step / dist;
            let v = Velocity {
                dx: (gate_pos.x - pos.x) * inv,
                dy: (gate_pos.y - pos.y) * inv,
                dz: (gate_pos.z - pos.z) * inv,
            };
            let p = Position { x: pos.x + v.dx, y: pos.y + v.dy, z: pos.z + v.dz };
            (p, v, false)
        };

        if let Ok(mut p) = self.world.inner_mut().get::<&mut PositionComp>(entity) { p.0 = new_pos; }
        if let Ok(mut v) = self.world.inner_mut().get::<&mut VelocityComp>(entity) { v.0 = new_vel; }

        if arrived {
            // Warp complete: drop the component and clear thrust so the ship
            // sits at the gate (ready to jump). Movement resumes next tick.
            let gate_id = self.world.inner().get::<&WarpComp>(entity).ok().map(|w| w.gate_id);
            let _ = self.world.inner_mut().remove_one::<WarpComp>(entity);
            if let Ok(mut t) = self.world.inner_mut().get::<&mut ThrustComp>(entity) {
                t.direction  = Velocity::ZERO;
                t.is_braking = false;
            }
            // Queue auto-jump for the caller to propose after the tick.
            if auto_jump {
                if let Some(gid) = gate_id {
                    self.pending_auto_jumps.push((ship_id, gid));
                }
            }
        }

        // Emit VelocityChanged only when velocity actually changed (INV-MOVE).
        let changed = (new_vel.dx - old_vel.dx).abs() > f32::EPSILON
            || (new_vel.dy - old_vel.dy).abs() > f32::EPSILON
            || (new_vel.dz - old_vel.dz).abs() > f32::EPSILON;
        changed.then(|| DomainEvent::VelocityChanged(dawn_core::events::VelocityChanged {
            ship_id,
            velocity: new_vel,
            tick,
        }))
    }

    /// Overwrite the phase of a ship's `WarpComp` (no-op if absent).
    fn set_warp_phase(&mut self, entity: Entity, phase: WarpPhase) {
        if let Ok(mut w) = self.world.inner_mut().get::<&mut WarpComp>(entity) {
            w.phase = phase;
        }
    }

    /// Approach System (ADR-0015 §3): for every ship carrying an `ApproachComp`,
    /// re-aim thrust at the target's latest position, or brake on arrival.
    ///
    /// Runs each tick just before the Movement System so the refreshed thrust
    /// takes effect the same tick. Mirrors the Bot AI steering (`process_bots`)
    /// but is driven by a player-issued `ApproachCommand`.
    ///
    /// Removes the component (and brakes) if the target no longer exists.
    pub fn process_approach(&mut self) {
        use dawn_core::ApproachTarget;
        /// Stop and hold once within this distance of a Ship target (units).
        const SHIP_ARRIVAL_RADIUS: f32 = 500.0;

        // Collect approachers up front (entity, target, current position) so the
        // ECS query borrow is released before the mutable write pass below.
        let approachers: Vec<(Entity, ApproachTarget, Position)> = self.world.inner()
            .query::<(&ApproachComp, &PositionComp)>()
            .iter()
            .map(|(entity, (approach, pos))| (entity, approach.target, pos.0))
            .collect();

        for (entity, target, ship_pos) in approachers {
            // Resolve the target's current position and the arrival distance.
            // `None` means the target no longer exists.
            let resolved: Option<(Position, f32)> = match target {
                ApproachTarget::Ship(target_id) => self.ship_index.get(&target_id)
                    .and_then(|&te| self.world.inner().get::<&PositionComp>(te).ok().map(|p| (p.0, SHIP_ARRIVAL_RADIUS))),
                // Stop comfortably inside the gate's activation radius so the
                // jump prompt becomes available on arrival (ADR-0015).
                ApproachTarget::Gate(gate_id) => self.jump_gate(gate_id)
                    .map(|g| (g.position, g.activation_radius * 0.8)),
            };

            match resolved {
                // Target gone: drop the approach and brake (ADR-0015 §4).
                None => {
                    let _ = self.world.inner_mut().remove_one::<ApproachComp>(entity);
                    self.brake_thrust(entity);
                }
                // Arrived: hold position, keep ApproachComp so the ship resumes
                // if a Ship target later drifts back out of range.
                Some((tp, arrival)) if ship_pos.distance(tp) <= arrival => self.brake_thrust(entity),
                // Still closing: steer toward the target's latest position.
                Some((tp, _)) => self.steer_thrust_toward(entity, ship_pos, tp),
            }
        }
    }

    /// Tackle System — Step 4.5 (after Capacitor, before Lock). ADR-0024.
    ///
    /// For each ship that has at least one active Tackle module with a nonzero
    /// `tackle_range`, check whether its locked targets are within that range.
    /// - Enter into `TackledComp` if the target is in range and a lock exists.
    /// - Remove from `TackledComp` if the tackler no longer has a lock on it,
    ///   the target is out of range, or the tackler itself is destroyed.
    ///
    /// Also cleans up `TackledComp` for ships whose tacklers have been destroyed
    /// (so tackle state always reflects live entities).
    pub fn process_tackle(&mut self, tick: Tick) -> Vec<DomainEvent> {
        use dawn_core::fitting::ModuleKind;
        use dawn_ecs::systems::TackleResult;

        // Collect (tackler_id, entity, tackle_range, locked_targets, position)
        // from all ships that have at least one active Tackle module.
        let tacklers: Vec<(ShipId, Entity, f32, Vec<ShipId>, Position)> = self
            .ship_index
            .iter()
            .filter_map(|(&ship_id, &entity)| {
                let stats = self.world.inner().get::<&ShipStatsComp>(entity).ok()?;
                if stats.tackle_range <= 0.0 { return None; }
                // Only if the module is actually active (cap could have killed it).
                let fitting = self.world.inner().get::<&FittingComp>(entity).ok()?;
                let has_active_tackle = fitting.high.iter()
                    .chain(fitting.mid.iter())
                    .chain(fitting.low.iter())
                    .chain(fitting.rig.iter())
                    .any(|s| s.def.kind == ModuleKind::Tackle && s.is_effective());
                if !has_active_tackle { return None; }
                let lock = self.world.inner().get::<&LockComp>(entity).ok()?;
                let locked: Vec<ShipId> = lock.locked_targets().collect();
                let pos = self.world.inner().get::<&PositionComp>(entity).ok()?.0;
                Some((ship_id, entity, stats.tackle_range, locked, pos))
            })
            .collect();

        let mut result = TackleResult::new();

        // Compute which (target, tackler) pairs are in range this tick.
        let mut active_tackles: Vec<(ShipId, ShipId)> = Vec::new(); // (target, tackler)
        for (tackler_id, _, tackle_range, locked_targets, tackler_pos) in &tacklers {
            for &target_id in locked_targets {
                if let Some(&te) = self.ship_index.get(&target_id) {
                    if let Ok(tp) = self.world.inner().get::<&PositionComp>(te) {
                        if tackler_pos.distance(tp.0) <= *tackle_range {
                            active_tackles.push((target_id, *tackler_id));
                        }
                    }
                }
            }
        }

        // Collect live tacklers set for cleanup.
        let live_tackler_ids: std::collections::HashSet<ShipId> =
            tacklers.iter().map(|(id, _, _, _, _)| *id).collect();

        // Collect all ships that currently have TackledComp.
        let currently_tackled: Vec<(ShipId, Entity, Vec<ShipId>)> = self
            .ship_index
            .iter()
            .filter_map(|(&sid, &entity)| {
                let tackled = self.world.inner().get::<&TackledComp>(entity).ok()?;
                Some((sid, entity, tackled.tacklers.clone()))
            })
            .collect();

        // Pass 1: remove stale tacklers from existing TackledComps.
        for (target_id, entity, old_tacklers) in currently_tackled {
            let entity = entity; // copy (Entity: Copy)
            let stale: Vec<ShipId> = old_tacklers.iter()
                .filter(|&&tid| {
                    !live_tackler_ids.contains(&tid)
                        || !active_tackles.contains(&(target_id, tid))
                })
                .copied()
                .collect();

            for tid in stale {
                result.push_released(target_id, tid, tick);
            }

            // Apply releases to the component.
            let should_remove = {
                if let Ok(mut tackled) = self.world.inner_mut().get::<&mut TackledComp>(entity) {
                    tackled.tacklers.retain(|tid| active_tackles.contains(&(target_id, *tid)));
                    tackled.tacklers.is_empty()
                } else { false }
            };
            if should_remove {
                let _ = self.world.inner_mut().remove_one::<TackledComp>(entity);
            }
        }

        // Pass 2: add new tacklers.
        let already_tackled_now: Vec<(ShipId, Entity, Vec<ShipId>)> = self
            .ship_index
            .iter()
            .filter_map(|(&sid, &entity)| {
                let tackled = self.world.inner().get::<&TackledComp>(entity).ok()?;
                Some((sid, entity, tackled.tacklers.clone()))
            })
            .collect();

        for (target_id, tackler_id) in &active_tackles {
            let already = already_tackled_now.iter()
                .find(|(sid, _, _)| sid == target_id)
                .map(|(_, _, tacklers)| tacklers.contains(tackler_id))
                .unwrap_or(false);

            if !already {
                result.push_applied(*target_id, *tackler_id, tick);

                if let Some(&entity) = self.ship_index.get(target_id) {
                    let has_comp = {
                        if let Ok(mut tackled) = self.world.inner_mut().get::<&mut TackledComp>(entity) {
                            if !tackled.tacklers.contains(tackler_id) {
                                tackled.tacklers.push(*tackler_id);
                            }
                            true
                        } else { false }
                    };
                    if !has_comp {
                        let _ = self.world.inner_mut().insert_one(entity, TackledComp { tacklers: vec![*tackler_id] });
                    }
                }
            }
        }

        result.events
    }

    /// Return the player ship's fitting state as a PlayerFitting JSON message.
    ///
    /// Sent after Welcome + InitialState on connect. Format:
    /// ```json
    /// {"type":"PlayerFitting","modules":[
    ///   {"slot":"High","index":0,"module_id":1,"name":"Small Railgun I","is_active":false}
    /// ]}
    /// ```
    pub fn build_player_fitting_json(&self, ship_id: ShipId) -> Option<String> {
        let entity = self.ship_index.get(&ship_id)?;
        let fitting = self.world.inner().get::<&FittingComp>(*entity).ok()?;

        let mut modules: Vec<serde_json::Value> = Vec::new();
        let slot_names = [("High", &fitting.high), ("Mid", &fitting.mid),
                          ("Low", &fitting.low), ("Rig", &fitting.rig)];
        for (slot_name, slots) in &slot_names {
            for (i, slot) in slots.iter().enumerate() {
                let d = &slot.def.stat_delta;
                modules.push(serde_json::json!({
                    "slot"             : slot_name,
                    "index"            : i,
                    "module_id"        : slot.def.id.0,
                    "name"             : slot.def.name,
                    "kind"             : format!("{:?}", slot.def.kind),
                    "is_active"        : slot.is_active,
                    "is_active_module" : matches!(slot.def.activation_mode, dawn_core::ActivationMode::Active),
                    "cap_cost_per_cycle": slot.def.cap_cost_per_cycle,
                    "cycle_time_ticks" : slot.def.cycle_time_ticks,
                    "stat_delta": {
                        "weapon_damage_add"   : d.weapon_damage_add,
                        "weapon_range_add"    : d.weapon_range_add,
                        "falloff_range_add"   : d.falloff_range_add,
                        "tracking_speed_add"  : d.tracking_speed_add,
                        "speed_multiplier"    : d.speed_multiplier,
                        "mass_add"            : d.mass_add,
                        "max_shield_add"      : d.max_shield_add,
                        "max_armor_add"       : d.max_armor_add,
                        "max_hull_add"        : d.max_hull_add,
                    },
                }));
            }
        }

        Some(serde_json::json!({
            "type"   : "PlayerFitting",
            "modules": modules,
        }).to_string())
    }

    /// Full-world `InitialState` (every ship). Used for non-AoI callers.
    pub fn build_initial_state_json(&self) -> String {
        self.initial_state_json(self.ship_index.keys().copied())
    }

    /// `InitialState` scoped to an observer's Area of Interest: only ships in the
    /// 27-cell neighborhood of `observer_pos` (ADR-0019).
    pub fn build_initial_state_json_for(&self, observer_pos: Position, cell_size: f32) -> String {
        self.initial_state_json(self.ships_visible_to(observer_pos, cell_size).into_iter())
    }

    /// Serialise the given ships into an `InitialState` message.
    fn initial_state_json(&self, ship_ids: impl Iterator<Item = ShipId>) -> String {
        let ships: Vec<serde_json::Value> =
            ship_ids.filter_map(|ship_id| self.ship_state_json(ship_id)).collect();

        serde_json::json!({
            "type"  : "InitialState",
            "ships" : ships,
        }).to_string()
    }

    /// Per-ship state object (position, stats, hull, ownership). Shared by
    /// `InitialState` and `AoiEnter` (ADR-0019). `None` if the ship is gone.
    pub fn ship_state_json(&self, ship_id: ShipId) -> Option<serde_json::Value> {
        let entity  = self.ship_index.get(&ship_id)?;
        let pos     = self.world.inner().get::<&PositionComp>(*entity).ok()?.0;
        let stats   = self.world.inner().get::<&ShipStatsComp>(*entity).ok()?;
        let hull    = self.world.inner().get::<&HullComp>(*entity).ok()?;
        let is_player = self.ship_owners.contains_key(&ship_id);
        let ship_type_name = self.ship_type_ids.get(&ship_id)
            .and_then(|tid| self.ship_type_registry.get(tid))
            .map(|def| def.name.as_str())
            .unwrap_or("Unknown");
        Some(serde_json::json!({
            "ship_id"              : ship_id.raw(),
            "ship_type_name"       : ship_type_name,
            "position"             : { "x": pos.x, "y": pos.y, "z": pos.z },
            "max_shield"           : stats.max_shield,
            "max_armor"            : stats.max_armor,
            "max_hull"             : stats.max_hull,
            "current_shield"       : hull.current_shield,
            "current_armor"        : hull.current_armor,
            "current_hull"         : hull.current_hull,
            "cap_max"              : stats.cap_max,
            "cap_recharge_per_tick": stats.cap_recharge_per_tick,
            "is_player"            : is_player,
        }))
    }

    /// `AoiEnter` control message for a ship that just entered an observer's
    /// neighborhood (ADR-0019). `None` if the ship is gone. The matching
    /// `AoiLeave` is a free function ([`crate::aoi::aoi_leave_json`]) since it
    /// needs no node state.
    pub fn aoi_enter_json(&self, ship_id: ShipId) -> Option<String> {
        let ship = self.ship_state_json(ship_id)?;
        Some(serde_json::json!({ "type": "AoiEnter", "ship": ship }).to_string())
    }

    // ── Area of Interest (ADR-0019) ────────────────────────────────────────────

    /// `(ShipId, Position)` for every ship currently in the world — the input to
    /// the AoI cell grid. Iteration order is unspecified, but `CellGrid` sorts
    /// each bucket, so query results are deterministic regardless.
    pub fn ship_positions(&self) -> Vec<(ShipId, Position)> {
        self.ship_index.iter().filter_map(|(&id, &entity)| {
            let pos = self.world.inner().get::<&PositionComp>(entity).ok()?.0;
            Some((id, pos))
        }).collect()
    }

    /// ShipIds visible to an observer at `observer_pos`: those in the 27-cell
    /// neighborhood of its cell (ADR-0019). Returned in `ShipId` order.
    ///
    /// Builds the grid from a single position pass; the serve loop can instead
    /// build one [`crate::aoi::CellGrid`] per tick and query it per session.
    pub fn ships_visible_to(&self, observer_pos: Position, cell_size: f32) -> Vec<ShipId> {
        crate::aoi::CellGrid::build(cell_size, self.ship_positions())
            .neighbors_of(observer_pos)
    }

    // ── Module Activation ─────────────────────────────────────────────────────

    pub fn activate_module(&mut self, cmd: dawn_core::ActivateModuleCommand) -> bool {
        self.set_module_active(cmd.ship_id, cmd.module_id, cmd.slot, true)
    }

    pub fn deactivate_module(&mut self, cmd: dawn_core::DeactivateModuleCommand) -> bool {
        self.set_module_active(cmd.ship_id, cmd.module_id, cmd.slot, false)
    }

    pub fn activate_module_owned(&mut self, player_id: PlayerId, cmd: dawn_core::ActivateModuleCommand) -> bool {
        if self.ship_owners.get(&cmd.ship_id) != Some(&player_id) { return false; }
        self.activate_module(cmd)
    }

    pub fn deactivate_module_owned(&mut self, player_id: PlayerId, cmd: dawn_core::DeactivateModuleCommand) -> bool {
        if self.ship_owners.get(&cmd.ship_id) != Some(&player_id) { return false; }
        self.deactivate_module(cmd)
    }

    fn set_module_active(
        &mut self,
        ship_id  : ShipId,
        module_id: dawn_core::ModuleId,
        slot     : dawn_core::SlotKind,
        active   : bool,
    ) -> bool {
        use dawn_core::events::{ModuleActivated, ModuleDeactivated};
        let entity = match self.ship_index.get(&ship_id).copied() {
            Some(e) => e,
            None    => return false,
        };

        // Return early if the module is already in the requested state —
        // avoids emitting duplicate ModuleActivated/Deactivated events every tick.
        let already_in_state = self.world.inner()
            .get::<&FittingComp>(entity)
            .ok()
            .and_then(|f| f.high.iter().chain(f.mid.iter()).chain(f.low.iter()).chain(f.rig.iter())
                .find(|s| s.def.id == module_id && s.def.slot == slot)
                .map(|s| s.is_active == active))
            .unwrap_or(false);
        if already_in_state { return true; }

        let found = self.world.inner_mut()
            .get::<&mut FittingComp>(entity)
            .ok()
            .and_then(|mut f| f.find_slot_mut(module_id, slot).map(|s| {
                s.is_active = active;
                true
            }))
            .unwrap_or(false);

        if !found { return false; }

        let base = self.base_stats.get(&ship_id).copied().unwrap_or(ShipStatsComp::NPC);
        apply_fitting(&mut self.world, ship_id, base);

        let event = if active {
            DomainEvent::ModuleActivated(ModuleActivated { ship_id, module_id, slot, tick: self.current_tick })
        } else {
            DomainEvent::ModuleDeactivated(ModuleDeactivated { ship_id, module_id, slot, tick: self.current_tick })
        };
        self.event_store.append(event);
        true
    }

    // ── Fitting ───────────────────────────────────────────────────────────────

    pub fn register_module(&mut self, def: ModuleDefinition) {
        self.module_registry.insert(def.id, def);
    }

    /// Returns `true` if successful, `false` if the ship or module is unknown.
    pub fn fit_module(&mut self, cmd: FitModuleCommand) -> bool {
        let def = match self.module_registry.get(&cmd.module_id).cloned() {
            Some(d) => d,
            None    => return false,
        };
        let entity = match self.ship_index.get(&cmd.ship_id).copied() {
            Some(e) => e,
            None    => return false,
        };

        use dawn_core::fitting::ActivationMode;
        use dawn_ecs::components::FittedSlot;
        let is_npc = self.ship_owners.get(&cmd.ship_id).is_none();
        let is_active = match def.activation_mode {
            ActivationMode::Passive => true,
            ActivationMode::Active  => is_npc,
        };
        if let Ok(mut fitting) = self.world.inner_mut().get::<&mut FittingComp>(entity) {
            fitting.slot_mut(cmd.slot).push(FittedSlot { def, is_active, cycle_remaining: 0 });
        } else {
            return false;
        }

        let base = self.base_stats
            .get(&cmd.ship_id)
            .copied()
            .unwrap_or(ShipStatsComp::NPC);

        apply_fitting(&mut self.world, cmd.ship_id, base);

        // Append the ShipFitted event
        let snapshot = self.world.inner()
            .get::<&FittingComp>(entity)
            .map(|f| f.to_snapshot())
            .unwrap_or_else(|_| dawn_core::FittingSnapshot::empty());

        self.event_store.append(DomainEvent::ShipFitted(ShipFitted {
            ship_id : cmd.ship_id,
            fitting : snapshot,
            tick    : self.current_tick,
        }));

        true
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    /// Add a Ship to the ECS World and record the entity in `ship_index`.
    /// Does NOT append any event — used by `spawn_ship` and replay.
    fn insert_to_world(&mut self, ship_id: ShipId, position: Position, velocity: Velocity) {
        let entity = self.world.spawn_ship(ship_id, position, velocity);
        self.ship_index.insert(ship_id, entity);
    }

    /// Reconstruct a Ship's full ECS state (stats, hull, capacitor, fitting)
    /// from a [`ShipSnapshot`]. Used by `from_snapshot` (node restart) and
    /// `import_transit` (Sector Transit, ADR-0014). Does NOT append any event.
    fn restore_ship_from_snapshot(&mut self, ship: &ShipSnapshot) {
        self.insert_to_world(ship.ship_id, ship.position, ship.velocity);
        self.ship_type_ids.insert(ship.ship_id, ship.ship_type_id);

        let base = self.ship_type_registry.get(&ship.ship_type_id)
            .map(|def| ShipStatsComp::from_base(&def.base_stats))
            .unwrap_or(ShipStatsComp::NPC);
        self.base_stats.insert(ship.ship_id, base);

        if let Some(&entity) = self.ship_index.get(&ship.ship_id) {
            self.world.set_ship_stats(entity, base);

            let fitting = FittingComp::from_snapshot(&ship.fitting, &self.module_registry);
            let _ = self.world.inner_mut().insert_one(entity, fitting);

            // apply_fitting recomputes ShipStatsComp and rescales HullComp;
            // restore the exact HP layers from the snapshot afterwards.
            apply_fitting(&mut self.world, ship.ship_id, base);
            if let Ok(mut hull) = self.world.inner_mut().get::<&mut HullComp>(entity) {
                hull.current_shield = ship.current_shield;
                hull.current_armor  = ship.current_armor;
                hull.current_hull   = ship.current_hull;
                hull.is_destroyed   = ship.is_destroyed;
            }

            if let Some(cap) = ship.capacitor {
                let _ = self.world.inner_mut().insert_one(entity, CapacitorComp { current: cap });
            }

            if !ship.tackled_by.is_empty() {
                let _ = self.world.inner_mut().insert_one(entity, TackledComp { tacklers: ship.tackled_by.clone() });
            }
        }
    }

    /// Apply a single domain event to the ECS World without appending it.
    /// Used during `restore_from` to replay post-snapshot events.
    fn apply_event(&mut self, event: &DomainEvent) {
        match event {
            DomainEvent::ShipSpawned(e) => {
                if !self.ship_index.contains_key(&e.ship_id) {
                    self.insert_to_world(e.ship_id, e.initial_position, Velocity::ZERO);
                    // Restore base_stats from ship type registry
                    let base = self.ship_type_registry
                        .get(&e.ship_type_id)
                        .map(|def| ShipStatsComp::from_base(&def.base_stats))
                        .unwrap_or(ShipStatsComp::NPC);
                    self.base_stats.insert(e.ship_id, base);
                    if let Some(&entity) = self.ship_index.get(&e.ship_id) {
                        self.world.set_ship_stats(entity, base);
                        if let Ok(mut hull) = self.world.inner_mut().get::<&mut HullComp>(entity) {
                            *hull = HullComp::new(base.max_shield, base.max_armor, base.max_hull);
                        }
                    }
                }
                let counter = e.ship_id.0.counter();
                if counter >= self.id_counter {
                    self.id_counter = counter + 1;
                }
            }

            DomainEvent::VelocityChanged(e) => {
                if let Some(&entity) = self.ship_index.get(&e.ship_id) {
                    let gap_ticks = e.tick.value().saturating_sub(self.current_tick.value()).saturating_sub(1);
                    let old_vel = self.world.inner().get::<&VelocityComp>(entity).ok().map(|v| v.0).unwrap_or(Velocity::ZERO);
                    if let Ok(mut pos) = self.world.inner_mut().get::<&mut PositionComp>(entity) {
                        pos.0.x += old_vel.dx * gap_ticks as f32 + e.velocity.dx;
                        pos.0.y += old_vel.dy * gap_ticks as f32 + e.velocity.dy;
                        pos.0.z += old_vel.dz * gap_ticks as f32 + e.velocity.dz;
                    }
                    if let Ok(mut vel) = self.world.inner_mut().get::<&mut VelocityComp>(entity) {
                        vel.0 = e.velocity;
                    }
                }
                if e.tick > self.current_tick {
                    self.current_tick = e.tick;
                }
            }

            DomainEvent::ShipDespawned(e) => {
                if let Some(entity) = self.ship_index.remove(&e.ship_id) {
                    self.world.despawn_ship(entity);
                }
                self.ship_type_ids.remove(&e.ship_id);
                self.base_stats.remove(&e.ship_id);
                if let Some(player_id) = self.ship_owners.remove(&e.ship_id) {
                    self.player_ships.remove(&player_id);
                }
            }

            DomainEvent::ShipFitted(e) => {
                if let Some(&entity) = self.ship_index.get(&e.ship_id) {
                    let fitting = FittingComp::from_snapshot(&e.fitting, &self.module_registry);
                    let base = self.base_stats.get(&e.ship_id).copied().unwrap_or(ShipStatsComp::NPC);
                    let _ = self.world.inner_mut().insert_one(entity, fitting);
                    apply_fitting(&mut self.world, e.ship_id, base);
                }
            }

            DomainEvent::ModuleActivated(e) => {
                if let Some(&entity) = self.ship_index.get(&e.ship_id) {
                    if let Ok(mut fitting) = self.world.inner_mut().get::<&mut FittingComp>(entity) {
                        if let Some(slot) = fitting.find_slot_mut(e.module_id, e.slot) {
                            slot.is_active = true;
                        }
                    }
                    let base = self.base_stats.get(&e.ship_id).copied().unwrap_or(ShipStatsComp::NPC);
                    apply_fitting(&mut self.world, e.ship_id, base);
                }
            }

            DomainEvent::ModuleDeactivated(e) => {
                if let Some(&entity) = self.ship_index.get(&e.ship_id) {
                    if let Ok(mut fitting) = self.world.inner_mut().get::<&mut FittingComp>(entity) {
                        if let Some(slot) = fitting.find_slot_mut(e.module_id, e.slot) {
                            slot.is_active = false;
                        }
                    }
                    let base = self.base_stats.get(&e.ship_id).copied().unwrap_or(ShipStatsComp::NPC);
                    apply_fitting(&mut self.world, e.ship_id, base);
                }
            }

            // TargetLocked: set the LockComp entry to the Locked state
            DomainEvent::TargetLocked(e) => {
                use dawn_ecs::components::{LockEntry, LockState};
                if let Some(&entity) = self.ship_index.get(&e.locker_id) {
                    if let Ok(mut lock) = self.world.inner_mut().get::<&mut LockComp>(entity) {
                        if let Some(entry) = lock.entries.iter_mut()
                            .find(|en| en.target_id == e.target_id)
                        {
                            entry.state = LockState::Locked;
                        } else {
                            lock.entries.push(LockEntry { target_id: e.target_id, state: LockState::Locked });
                        }
                    }
                }
            }

            // LockLost: remove the entry from LockComp
            DomainEvent::LockLost(e) => {
                if let Some(&entity) = self.ship_index.get(&e.locker_id) {
                    if let Ok(mut lock) = self.world.inner_mut().get::<&mut LockComp>(entity) {
                        lock.entries.retain(|en| en.target_id != e.target_id);
                    }
                }
            }

            DomainEvent::WeaponFired(_) => {}

            DomainEvent::DamageTaken(e) => {
                if let Some(&entity) = self.ship_index.get(&e.ship_id) {
                    if let Ok(mut hull) = self.world.inner_mut().get::<&mut HullComp>(entity) {
                        hull.current_shield = e.current_shield;
                        hull.current_armor  = e.current_armor;
                        hull.current_hull   = e.current_hull;
                        if e.current_hull <= 0.0 {
                            hull.is_destroyed = true;
                        }
                    }
                }
            }

            DomainEvent::ShipDestroyed(e) => {
                if let Some(entity) = self.ship_index.remove(&e.ship_id) {
                    self.world.despawn_ship(entity);
                }
                self.ship_type_ids.remove(&e.ship_id);
                self.base_stats.remove(&e.ship_id);
                if let Some(player_id) = self.ship_owners.remove(&e.ship_id) {
                    self.player_ships.remove(&player_id);
                }
            }

            // Sector Transit (ADR-0014): TransitState component and ownership
            // transfer are added in a later Phase 7 task.
            DomainEvent::SectorTransitRequested(_)
            | DomainEvent::SectorTransitCompleted(_)
            | DomainEvent::SectorTransitAborted(_) => {}

            // Jump Gate Navigation (ADR-0009): Sector/StarSystem transfer on
            // Replay is added when the Jump pipeline is wired in dawn-simulation.
            DomainEvent::JumpGateUsed(_)
            | DomainEvent::StarSystemChanged(_) => {}

            // Tackle (ADR-0024): TackledComp is managed live; on replay, apply
            // the same add/remove logic to keep the component consistent.
            DomainEvent::TackleApplied(e) => {
                if let Some(&entity) = self.ship_index.get(&e.ship_id) {
                    let has_comp = {
                        if let Ok(mut tackled) = self.world.inner_mut().get::<&mut TackledComp>(entity) {
                            if !tackled.tacklers.contains(&e.by) {
                                tackled.tacklers.push(e.by);
                            }
                            true
                        } else { false }
                    };
                    if !has_comp {
                        let _ = self.world.inner_mut().insert_one(entity, TackledComp { tacklers: vec![e.by] });
                    }
                }
            }

            DomainEvent::TackleReleased(e) => {
                if let Some(&entity) = self.ship_index.get(&e.ship_id) {
                    let should_remove = {
                        if let Ok(mut tackled) = self.world.inner_mut().get::<&mut TackledComp>(entity) {
                            tackled.tacklers.retain(|&id| id != e.by);
                            tackled.tacklers.is_empty()
                        } else { false }
                    };
                    if should_remove {
                        let _ = self.world.inner_mut().remove_one::<TackledComp>(entity);
                    }
                }
            }
        }
    }
}

// ── Checkpointing (ADR-0017 8A-7) ─────────────────────────────────────────────

impl SimulationNode<dawn_event_store::FileEventStore> {
    /// Take a snapshot, persist it durably, then compact the hot log behind it.
    ///
    /// This is the operational checkpoint of ADR-0017: after it returns, recovery
    /// only needs `snapshot_path` + the post-snapshot tail of the hot log; the
    /// prefix it covers lives in the append-only cold archive at `cold_path`.
    ///
    /// Ordering is load-bearing for crash safety: the snapshot is saved **before**
    /// the hot log is compacted. A crash between the two leaves the snapshot
    /// written and the hot log untouched (a redundant but safe state). Compacting
    /// first could strand a snapshot whose `log_index` is older than the new
    /// `base_index`, which would make `iter_from` silently skip events.
    pub fn checkpoint(
        &mut self,
        snapshot_path: impl AsRef<std::path::Path>,
        cold_path: impl AsRef<std::path::Path>,
    ) -> std::io::Result<StateSnapshot> {
        let snapshot = self.take_snapshot();
        snapshot.save(&snapshot_path)?;
        self.event_store.compact(snapshot.log_index, cold_path)?;
        Ok(snapshot)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::Velocity;
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

        let entity = *node.ship_index.get(&ship_id).unwrap();
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

        assert!(node.ship_index.get(&ship_id).is_none(), "ship must leave the from-sector ECS");
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

        let entity = *node.ship_index.get(&chaser).unwrap();
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
        let target_entity = node.ship_index.remove(&target).unwrap();
        node.world.despawn_ship(target_entity);

        node.process_approach();
        assert_eq!(node.approach_target(chaser), None, "approach must drop when target is gone");
        let entity = *node.ship_index.get(&chaser).unwrap();
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
        assert!(!node.can_propose_warp(ship, dawn_core::JumpGateId(0)));
        assert!(!node.apply_warp_command_owned(player, dawn_core::WarpCommand {
            ship_id: ship, gate_id: dawn_core::JumpGateId(0),
        }));
        assert_eq!(node.warp_phase(ship), None);
    }

    #[test]
    fn warp_is_rejected_for_a_gate_not_in_this_sector() {
        // Gate 1 originates in Sector 1; a Sector-0 node does not know it.
        let mut node = mem_node();
        let (player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        assert!(!node.apply_warp_command_owned(player, dawn_core::WarpCommand {
            ship_id: ship, gate_id: dawn_core::JumpGateId(1),
        }));
        assert_eq!(node.warp_phase(ship), None);
    }

    #[test]
    fn warp_aligns_by_accelerating_then_flies_into_gate_range_and_completes() {
        let mut node = mem_node();
        let (player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        assert!(node.apply_warp_command_owned(player, dawn_core::WarpCommand {
            ship_id: ship, gate_id: dawn_core::JumpGateId(0),
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
            let entity = *node.ship_index.get(&ship).unwrap();
            let mut stats = *node.world.inner().get::<&ShipStatsComp>(entity).unwrap();
            stats.mass = mass;
            node.world.set_ship_stats(entity, stats);
            node.apply_warp_command_owned(player, dawn_core::WarpCommand { ship_id: ship, gate_id: dawn_core::JumpGateId(0) });
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
        node.apply_warp_command_owned(player, dawn_core::WarpCommand { ship_id: ship, gate_id: dawn_core::JumpGateId(0) });

        // Track warp speed each tick. A cliff stop would jump straight from
        // cruise (~WARP_SPEED) to zero; a smooth ramp produces intermediate
        // steps well below cruise before stopping.
        let entity = *node.ship_index.get(&ship).unwrap();
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
        node.apply_warp_command_owned(player, dawn_core::WarpCommand { ship_id: ship, gate_id: dawn_core::JumpGateId(0) });
        node.tick(); // still aligning (one tick is far from 75% max speed)
        assert_eq!(node.warp_phase(ship), Some(WarpPhase::Aligning));

        node.apply_move_command(ship, Position::new(0.0, 1000.0, 0.0));
        assert_eq!(node.warp_phase(ship), None, "a move during alignment cancels the warp");
    }

    #[test]
    fn a_move_command_is_ignored_during_the_committed_warping_phase() {
        let mut node = mem_node();
        let (player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        node.apply_warp_command_owned(player, dawn_core::WarpCommand { ship_id: ship, gate_id: dawn_core::JumpGateId(0) });
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
        assert!(node.apply_warp_command(ship, dawn_core::JumpGateId(0), true));

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
        assert!(node.apply_warp_command(ship, dawn_core::JumpGateId(0), false));

        for _ in 0..100 { node.tick(); }
        assert_eq!(node.warp_phase(ship), None);
        assert!(node.drain_pending_auto_jumps().is_empty());
    }

    #[test]
    fn move_command_is_ignored_while_ship_is_in_transit() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        node.set_player_ship(ship_id);

        let entity = *node.ship_index.get(&ship_id).unwrap();
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

        let entity = *node.ship_index.get(&ship_id).unwrap();
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

    // ── Snapshot ─────────────────────────────────────────────────────────────

    #[test]
    fn snapshot_records_correct_ship_count_and_tick() {
        let mut node = mem_node();
        for i in 0..3 {
            node.spawn_ship(dawn_core::ShipTypeId(1), Position::new(i as f32 * 100.0, 0.0, 0.0), Velocity::new(1.0, 0.0, 0.0));
        }
        for _ in 0..5 { node.tick(); }

        let snap = node.take_snapshot();
        assert_eq!(snap.ships.len(), 3);
        assert_eq!(snap.tick, Tick(5));
        assert_eq!(snap.log_index, node.total_event_count() as u64);
    }

    // ── INV-002: restore ──────────────────────────────────────────────────────

    #[test]
    fn ecs_state_is_fully_restored_from_snapshot_and_event_replay_after_simulated_restart() {
        let dir           = tempfile::tempdir().unwrap();
        let event_path    = dir.path().join("events.log");
        let snapshot_path = dir.path().join("snapshot.bin");

        // ── Session 1: run, snapshot mid-way, continue, shut down ───────────
        let ship_ids: Vec<ShipId>;
        let final_tick: Tick;
        let final_positions: Vec<Position>;
        {
            let store = FileEventStore::open(&event_path).unwrap();
            let mut node = SimulationNode::with_store(
                NodeId(0), SectorId(0),
                SectorBounds::centered(SectorBounds::DEFAULT_HALF),
                store,
            );

            // Spawn 5 ships as players with thrust so they emit VelocityChanged events.
            // (ADR-0008: NPC ships at constant velocity emit no events, so tick cannot
            //  be restored from the event log alone. Using player ships with thrust
            //  ensures VelocityChanged events carry the tick for replay.)
            ship_ids = (0..5u64).map(|i| {
                let id = node.spawn_ship(
                    dawn_core::ShipTypeId(1),
                    Position::new(i as f32 * 100.0, 0.0, 0.0),
                    Velocity::ZERO,
                );
                node.set_player_ship(id);  // enable thrust
                node.apply_move_command(id, Position::new(10_000.0, 0.0, 0.0));
                id
            }).collect();

            // Run 5 ticks, then snapshot.
            for _ in 0..5 { node.tick(); }
            let snap = node.take_snapshot();
            snap.save(&snapshot_path).unwrap();

            // Run 3 more ticks before shutdown.
            for _ in 0..3 { node.tick(); }

            final_tick      = node.current_tick();
            final_positions = ship_ids.iter()
                .map(|id| node.get_ship_position(*id).unwrap())
                .collect();
        } // node drops here; FileEventStore flushes on drop via BufWriter

        // ── Session 2: restart, restore, verify ─────────────────────────────
        //
        // ADR-0008: With VelocityChanged, position is derived from velocity + tick steps.
        // `restore_from` replays events (velocity) from the snapshot. To reach the exact
        // final position, we must run the remaining ticks from the restored state.
        // This is by design: position is derived state, not stored in events.
        let snap   = StateSnapshot::load(&snapshot_path).unwrap();
        let store2 = FileEventStore::open(&event_path).unwrap();
        let mut node2 = SimulationNode::restore_from(store2, &snap, &[], &[]);

        // The snapshot restores up to snap.tick. Run remaining ticks to reach final_tick.
        let remaining = final_tick.value() - node2.current_tick().value();
        for _ in 0..remaining { node2.tick(); }

        assert_eq!(
            node2.current_tick(), final_tick,
            "tick must match after restore + replay ticks"
        );
        assert_eq!(
            node2.ship_count(), ship_ids.len(),
            "ship count must match after restore"
        );
        for (id, expected_pos) in ship_ids.iter().zip(final_positions.iter()) {
            let restored_pos = node2.get_ship_position(*id)
                .expect("ship must exist after restore");
            assert_eq!(
                restored_pos, *expected_pos,
                "position of ship {} must match after restore + replay", id
            );
        }
    }

    /// INV-002 / ADR-0017 (8A-1) — a snapshot is a *verifiable* checkpoint:
    /// restoring it and snapshotting again yields a byte-identical snapshot.
    #[test]
    fn snapshot_round_trips_through_restore_byte_for_byte() {
        use crate::{modules, ship_types};
        use dawn_event_store::InMemoryEventStore;

        let mut node = node_with_modules();

        // Real ship type → carries a capacitor; thrust makes the state non-trivial.
        for i in 0..4u64 {
            let id = node.spawn_ship(
                dawn_core::ShipTypeId(1),
                Position::new(i as f32 * 50.0, 0.0, 0.0),
                Velocity::ZERO,
            );
            node.set_player_ship(id);
            node.apply_move_command(id, Position::new(9_000.0, 1_000.0, 0.0));
        }
        for _ in 0..6 { node.tick(); }

        let snap1 = node.take_snapshot();

        // Rebuild purely from the snapshot: store2 carries the same events, so the
        // snapshot's log_index lines up and the replay tail is empty.
        let mut store2 = InMemoryEventStore::new();
        for rec in node.event_store().all_records() {
            store2.append(rec.event.clone());
        }
        let node2 = SimulationNode::restore_from(
            store2, &snap1, &modules::all_modules(), &ship_types::all_ship_types(),
        );
        let snap2 = node2.take_snapshot();

        assert_eq!(
            postcard::to_stdvec(&snap1).unwrap(),
            postcard::to_stdvec(&snap2).unwrap(),
            "snapshot must round-trip through restore byte-for-byte (INV-002 verifiable snapshot)"
        );
    }

    /// INV-002 / ADR-0017 (8A-1) — snapshot + re-running the tail ticks reproduces
    /// the live state, *including* transient derived state (capacitor) that is not
    /// event-sourced.
    ///
    /// Ships are spawned with a constant velocity and NO move command, so there is
    /// no thrust *intent* (which is transient and not snapshotted). Both the live
    /// and the restored node therefore coast identically, isolating the property
    /// under test: state the snapshot captures + deterministic forward simulation.
    #[test]
    fn snapshot_plus_tail_tick_reexecution_matches_live_including_capacitor() {
        use crate::{modules, ship_types};
        use dawn_event_store::InMemoryEventStore;

        let mut live = node_with_modules();

        for i in 0..3u64 {
            live.spawn_ship(
                dawn_core::ShipTypeId(1),
                Position::new(i as f32 * 100.0, 0.0, 0.0),
                Velocity::new(120.0, -40.0, 0.0),
            );
        }

        // Coast for a while (constant velocity → no thrust intent involved), snapshot.
        for _ in 0..12 { live.tick(); }
        let snap = live.take_snapshot();
        let events_up_to_snapshot: Vec<_> = live
            .event_store()
            .all_records()
            .iter()
            .take(snap.log_index as usize)
            .map(|r| r.event.clone())
            .collect();

        // Continue live for the tail.
        for _ in 0..4 { live.tick(); }
        let live_final = live.take_snapshot();

        // Restore from the snapshot ONLY (store carries just the pre-snapshot
        // events, so restore_from applies no tail), then re-run the tail ticks.
        let mut store2 = InMemoryEventStore::new();
        for e in events_up_to_snapshot { store2.append(e); }
        let mut restored = SimulationNode::restore_from(
            store2, &snap, &modules::all_modules(), &ship_types::all_ship_types(),
        );
        for _ in 0..4 { restored.tick(); }
        let restored_final = restored.take_snapshot();

        assert_eq!(
            postcard::to_stdvec(&live_final).unwrap(),
            postcard::to_stdvec(&restored_final).unwrap(),
            "snapshot + tail tick re-execution must match live, including capacitor"
        );
    }

    /// INV-002 / ADR-0017 (8A-4) — operational recovery does NOT require genesis
    /// replay. After compacting the hot log behind the snapshot (so the genesis
    /// events live only in the cold archive), reopening + restoring from the
    /// snapshot still reproduces the live state.
    #[test]
    fn recovery_does_not_require_genesis_replay_after_compaction() {
        use dawn_event_store::FileEventStore;

        let dir  = tempfile::tempdir().unwrap();
        let hot  = dir.path().join("events.log");
        let cold = dir.path().join("cold.log");

        let snap;
        let live_final;
        {
            let store = FileEventStore::open(&hot).unwrap();
            let mut node = SimulationNode::with_store(
                NodeId(0), SectorId(0),
                SectorBounds::centered(SectorBounds::DEFAULT_HALF),
                store,
            );
            // Constant velocity, no move command → no transient thrust intent.
            for i in 0..3u64 {
                node.spawn_ship(
                    dawn_core::ShipTypeId(1),
                    Position::new(i as f32 * 100.0, 0.0, 0.0),
                    Velocity::new(40.0, -15.0, 0.0),
                );
            }
            for _ in 0..8 { node.tick(); }
            snap = node.take_snapshot();        // tick 8
            for _ in 0..4 { node.tick(); }
            live_final = node.take_snapshot();  // tick 12
        } // node dropped; hot file flushed

        // Out-of-band compaction archives the events behind the snapshot, so they
        // are removed from the hot log (they survive only in the cold archive).
        {
            let mut store = FileEventStore::open(&hot).unwrap();
            store.compact(snap.log_index, &cold).unwrap();
            assert_eq!(store.base_index(), snap.log_index);
            assert_eq!(
                store.records_on_disk(), 0,
                "events behind the snapshot are archived, not in the hot log"
            );
        }

        // Restart: reopen the compacted hot log and recover. iter_from(snap.log_index)
        // touches only the (empty) hot tail — no genesis replay is possible or needed.
        let store2 = FileEventStore::open(&hot).unwrap();
        assert_eq!(store2.base_index(), snap.log_index, "hot log holds no genesis events");
        let mut restored = SimulationNode::restore_from(store2, &snap, &[], &[]);
        assert_eq!(restored.current_tick(), snap.tick);
        for _ in 0..4 { restored.tick(); }
        let restored_final = restored.take_snapshot();

        assert_eq!(
            postcard::to_stdvec(&live_final).unwrap(),
            postcard::to_stdvec(&restored_final).unwrap(),
            "recovery from snapshot + bounded hot tail (no genesis) must match live"
        );
    }

    #[test]
    fn hull_capacitor_and_fitting_state_are_restored_from_snapshot() {
        use crate::{modules, ship_types};
        use dawn_core::events::DamageTaken;
        use dawn_core::fitting::{FittingSnapshot, SlotEntry};
        use dawn_core::events::ShipFitted;

        let dir           = tempfile::tempdir().unwrap();
        let event_path    = dir.path().join("events.log");
        let snapshot_path = dir.path().join("snapshot.bin");

        let ship_id: ShipId;
        {
            let store = FileEventStore::open(&event_path).unwrap();
            let mut node = SimulationNode::with_store(
                NodeId(0), SectorId(0),
                SectorBounds::centered(SectorBounds::DEFAULT_HALF),
                store,
            );
            for def in modules::all_modules() {
                node.register_module(def);
            }
            for def in ship_types::all_ship_types() {
                node.register_ship_type(def);
            }

            ship_id = node.spawn_ship(ship_types::SHIP_TYPE_MAGPIE, Position::ORIGIN, Velocity::ZERO);

            // Take damage: HullComp now has non-default current_* values.
            node.apply_event_pub(DomainEvent::DamageTaken(DamageTaken {
                ship_id,
                damage         : 50.0,
                current_shield : 30.0,
                current_armor  : 90.0,
                current_hull   : 100.0,
                tick           : Tick(1),
            }));

            // Fit the Afterburner and switch it on, so FittingComp restore
            // must reproduce both the fitted module and its active state.
            node.apply_event_pub(DomainEvent::ShipFitted(ShipFitted {
                ship_id,
                fitting: FittingSnapshot {
                    high: vec![],
                    mid : vec![SlotEntry { module_id: modules::MODULE_AFTERBURNER, is_active: true }],
                    low : vec![],
                    rig : vec![],
                },
                tick: Tick(1),
            }));

            let snap = node.take_snapshot();
            snap.save(&snapshot_path).unwrap();
        } // node drops; FileEventStore flushes via BufWriter

        let snap   = StateSnapshot::load(&snapshot_path).unwrap();
        let store2 = FileEventStore::open(&event_path).unwrap();
        let node2  = SimulationNode::restore_from(
            store2, &snap,
            &modules::all_modules(),
            &ship_types::all_ship_types(),
        );

        let hp = node2.get_ship_hp(ship_id).unwrap();
        assert_eq!(hp, 30.0 + 90.0 + 100.0, "Hull HP layers must survive restore");

        let cap = node2.get_ship_capacitor(ship_id)
            .expect("capacitor must be restored");
        assert_eq!(cap, node2.get_ship_stats(ship_id).unwrap().cap_max,
            "capacitor must be restored to its snapshot value");

        let fitted = node2.get_fitted_module_ids(ship_id);
        assert!(fitted.contains(&(modules::MODULE_AFTERBURNER, true)),
            "Afterburner must remain fitted and active after restore, got {:?}", fitted);
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

    #[test]
    fn damage_taken_event_is_replayed_to_restore_current_hp() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);

        node.apply_event_pub(DomainEvent::DamageTaken(dawn_core::events::DamageTaken {
            ship_id,
            damage         : 100.0,
            current_shield : 100.0,
            current_armor  : 150.0,
            current_hull   : 150.0,
            tick           : Tick(1),
        }));

        let hp = node.get_ship_hp(ship_id).unwrap();
        assert_eq!(hp, 400.0, "Replay 後の HP 合計 = 100 + 150 + 150 = 400");
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

    fn fit_fold_disruptor(node: &mut SimulationNode, ship_id: ShipId) {
        use dawn_core::{FitModuleCommand, SlotKind};
        use crate::modules::MODULE_FOLD_DISRUPTOR;
        node.fit_module(FitModuleCommand {
            ship_id,
            slot     : SlotKind::Mid,
            module_id: MODULE_FOLD_DISRUPTOR,
        });
    }

    #[test]
    fn tackled_ship_cannot_warp() {
        use dawn_core::{LockOnCommand, ActivateModuleCommand, SlotKind};
        use crate::modules::MODULE_FOLD_DISRUPTOR;

        let mut node = node_with_modules();

        // Spawn two ships near each other (within 20 km tackle range).
        let ship_a = {
            let id = node.next_player_id();
            node.spawn_player_ship_at_pub(id, Position::new(0.0, 0.0, 0.0))
        };
        let ship_b = {
            let id = node.next_player_id();
            node.spawn_player_ship_at_pub(id, Position::new(1000.0, 0.0, 0.0))
        };

        // Ship A fits and activates the Fold Disruptor.
        fit_fold_disruptor(&mut node, ship_a);
        let owner_a = node.ship_owners.get(&ship_a).copied().unwrap();
        node.activate_module_owned(owner_a, ActivateModuleCommand {
            ship_id  : ship_a,
            module_id: MODULE_FOLD_DISRUPTOR,
            slot     : SlotKind::Mid,
        });

        // Ship A locks Ship B.
        let lock_cmd = LockOnCommand { ship_id: ship_a, target_id: ship_b };

        // Run until lock resolves and tackle applies (lock_time is at least 1 tick).
        for _ in 0..10 {
            node.tick_with_lock_commands(&[lock_cmd.clone()]);
        }

        // Ship B should now be tackled → warp must be denied.
        let gate_id = node.jump_gates.keys().next().copied().unwrap();
        assert!(!node.can_propose_warp(ship_b, gate_id),
            "tackled ship must not be allowed to warp");
        assert!(!node.can_propose_jump(ship_b, gate_id),
            "tackled ship must not be allowed to jump");
    }

    #[test]
    fn tackle_releases_when_tackler_dies() {
        use dawn_core::{LockOnCommand, ActivateModuleCommand, SlotKind};
        use crate::modules::MODULE_FOLD_DISRUPTOR;

        let mut node = node_with_modules();

        let ship_a = {
            let id = node.next_player_id();
            node.spawn_player_ship_at_pub(id, Position::new(0.0, 0.0, 0.0))
        };
        let ship_b = {
            let id = node.next_player_id();
            node.spawn_player_ship_at_pub(id, Position::new(1000.0, 0.0, 0.0))
        };

        fit_fold_disruptor(&mut node, ship_a);
        let owner_a = node.ship_owners.get(&ship_a).copied().unwrap();
        node.activate_module_owned(owner_a, ActivateModuleCommand {
            ship_id  : ship_a,
            module_id: MODULE_FOLD_DISRUPTOR,
            slot     : SlotKind::Mid,
        });

        let lock_cmd = LockOnCommand { ship_id: ship_a, target_id: ship_b };
        for _ in 0..10 {
            node.tick_with_lock_commands(&[lock_cmd.clone()]);
        }

        let gate_id = node.jump_gates.keys().next().copied().unwrap();
        assert!(!node.can_propose_warp(ship_b, gate_id), "should be tackled first");

        // Destroy the tackler: it should no longer appear in live_tackler_ids
        // → tackle releases on the next process_tackle call.
        node.apply_event_pub(DomainEvent::ShipDestroyed(dawn_core::events::ShipDestroyed {
            ship_id  : ship_a,
            killer_id: ship_b,
            tick     : node.current_tick(),
        }));
        node.tick_with_lock_commands(&[]);

        assert!(node.can_propose_warp(ship_b, gate_id),
            "tackle should release when the tackler ship is destroyed");
    }

    #[test]
    fn tackle_snapshot_round_trip_preserves_tackle_state() {
        use dawn_core::{LockOnCommand, ActivateModuleCommand, SlotKind};
        use crate::modules::MODULE_FOLD_DISRUPTOR;

        let mut node = node_with_modules();

        let ship_a = {
            let id = node.next_player_id();
            node.spawn_player_ship_at_pub(id, Position::new(0.0, 0.0, 0.0))
        };
        let ship_b = {
            let id = node.next_player_id();
            node.spawn_player_ship_at_pub(id, Position::new(1000.0, 0.0, 0.0))
        };

        fit_fold_disruptor(&mut node, ship_a);
        let owner_a = node.ship_owners.get(&ship_a).copied().unwrap();
        node.activate_module_owned(owner_a, ActivateModuleCommand {
            ship_id  : ship_a,
            module_id: MODULE_FOLD_DISRUPTOR,
            slot     : SlotKind::Mid,
        });

        let lock_cmd = LockOnCommand { ship_id: ship_a, target_id: ship_b };
        for _ in 0..10 {
            node.tick_with_lock_commands(&[lock_cmd.clone()]);
        }

        let gate_id = node.jump_gates.keys().next().copied().unwrap();
        assert!(!node.can_propose_warp(ship_b, gate_id), "should be tackled before snapshot");

        // Take snapshot and verify the tackled_by field is populated.
        let snapshot = node.take_snapshot();
        let ship_b_snap = snapshot.ships.iter().find(|s| s.ship_id == ship_b).unwrap();
        assert!(ship_b_snap.tackled_by.contains(&ship_a),
            "snapshot must record the tackler");

        // Restore and verify tackle is still in effect.
        let store2 = dawn_event_store::InMemoryEventStore::new();
        let modules: Vec<_> = crate::modules::all_modules();
        let ship_types: Vec<_> = crate::ship_types::all_ship_types();
        let node2 = SimulationNode::restore_from(store2, &snapshot, &modules, &ship_types);
        assert!(!node2.can_propose_warp(ship_b, gate_id),
            "restored node must still prevent tackled ship from warping");
    }
}
