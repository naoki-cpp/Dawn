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
//! SimulationNode::restore_from(store, &snapshot)
//!     -> reconstruct ECS from snapshot, replay events from log_index N onward
//! ```

use std::collections::HashMap;

use dawn_core::{
    events::{ShipFitted, ShipSpawned},
    ship_type::{ShipTypeDefinition, ShipTypeId},
    DomainEvent, FitModuleCommand, ModuleDefinition, ModuleId, NodeId, PlayerId, Position,
    SectorBounds, SectorId, ShipId, Tick, Velocity,
};
use dawn_ecs::{
    components::{CapacitorComp, FittingComp, HullComp, IsBotComp, IsNpcComp, LockComp, PositionComp, ShipStatsComp, ThrustComp, VelocityComp},
    systems::{CapacitorSystem, CombatSystem, LockSystem, MovementSystem, apply_fitting},
    Entity, SimWorld,
};
use dawn_event_store::{store::EventStore, InMemoryEventStore};

use crate::snapshot::{ShipSnapshot, StateSnapshot};

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
    /// 装備なし時の素の ShipStats。Fitting 集計の base として使う。
    base_stats         : HashMap<ShipId, ShipStatsComp>,
    /// PlayerId → ShipId（プレイヤー船の所有権管理）
    player_ships       : HashMap<PlayerId, ShipId>,
    /// ShipId → PlayerId（逆引き）
    ship_owners        : HashMap<ShipId, PlayerId>,
    /// ShipId → ShipTypeId（船種の逆引き・InitialState に ship_type_name を含めるために使用）
    ship_type_ids      : HashMap<ShipId, ShipTypeId>,
    /// PlayerId 採番カウンタ
    player_id_counter  : u64,
    /// Lock-on commands queued by the bot AI during `process_bots()`.
    ///
    /// Bot AI runs after the LockSystem each tick.  These commands are held
    /// here and injected into the LockSystem at the start of the NEXT tick,
    /// ensuring they are processed exactly like human-issued lock commands.
    pending_bot_lock_commands: Vec<dawn_core::LockOnCommand>,
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
        }
    }

    pub fn restore_from(store: S, snapshot: &StateSnapshot) -> Self {
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
        };

        // Restore ECS state from snapshot.
        for ship in &snapshot.ships {
            node.insert_to_world(ship.ship_id, ship.position, ship.velocity);
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

        // 3. Movement System
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

        // 5. Lock System — merge human commands with queued bot commands
        let merged_locks: Vec<dawn_core::LockOnCommand> = bot_locks
            .into_iter()
            .chain(lock_commands.iter().cloned())
            .collect();
        let lock = LockSystem(&mut self.world, tick, &merged_locks);

        // 6. Combat System — fire only when the capacitor weapon cycle started this tick
        let combat = CombatSystem(&mut self.world, tick, &cap.weapon_cycles_started);

        // 破壊された Ship を ECS と ship_index から削除
        // CLAUDE.md §6: Combat の後に Bot System を実行する
        for ship_id in &combat.destroyed {
            if let Some(entity) = self.ship_index.remove(ship_id) {
                self.world.despawn_ship(entity);
            }
            self.ship_type_ids.remove(ship_id);
        }

        // 6. Bot System — bots issue the same commands as human players
        self.process_bots();

        // 7. EventStore に Append
        let all_events: Vec<DomainEvent> = move_events.iter()
            .chain(cap.events.iter())
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
        let ships: Vec<ShipSnapshot> = self
            .ship_index
            .iter()
            .filter_map(|(&ship_id, &entity)| {
                let pos = self.world.inner().get::<&PositionComp>(entity).ok()?.0;
                let vel = self.world.inner().get::<&VelocityComp>(entity).ok()?.0;
                Some(ShipSnapshot { ship_id, position: pos, velocity: vel })
            })
            .collect();

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

    /// Look up the current position of a Ship by its ID.
    pub fn get_ship_position(&self, ship_id: ShipId) -> Option<Position> {
        let entity = self.ship_index.get(&ship_id)?;
        self.world.inner().get::<&PositionComp>(*entity).ok().map(|c| c.0)
    }

    /// Look up the current `ShipStatsComp` of a Ship by its ID.
    #[allow(dead_code)]
    pub fn get_ship_stats(&self, ship_id: ShipId) -> Option<ShipStatsComp> {
        let entity = self.ship_index.get(&ship_id)?;
        self.world.inner().get::<&ShipStatsComp>(*entity).ok().map(|c| *c)
    }

    /// Look up the current HP of a Ship by its ID.
    #[allow(dead_code)]
    pub fn get_ship_hp(&self, ship_id: ShipId) -> Option<f32> {
        let entity = self.ship_index.get(&ship_id)?;
        self.world.inner().get::<&HullComp>(*entity).ok()
            .map(|c| c.current_shield + c.current_armor + c.current_hull)
    }

    /// `apply_event` のテスト用公開ラッパー。
    #[cfg(test)]
    pub fn apply_event_pub(&mut self, event: DomainEvent) {
        self.apply_event(&event);
    }

    pub fn apply_move_command(&mut self, ship_id: ShipId, target: Position) {
        let entity = match self.ship_index.get(&ship_id) {
            Some(&e) => e,
            None     => return,
        };
        let pos = match self.world.inner().get::<&PositionComp>(entity).ok() {
            Some(c) => c.0,
            None    => return,
        };

        let dx   = target.x - pos.x;
        let dy   = target.y - pos.y;
        let dz   = target.z - pos.z;
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

    /// Begin decelerating the ship toward zero velocity using its thrust.
    ///
    /// The movement system applies thrust opposite to velocity each tick until
    /// the ship stops. Cancels any active thrust direction.
    pub fn apply_stop_command(&mut self, ship_id: ShipId) {
        let entity = match self.ship_index.get(&ship_id) {
            Some(&e) => e,
            None     => return,
        };
        if let Ok(mut t) = self.world.inner_mut().get::<&mut ThrustComp>(entity) {
            t.direction  = Velocity::ZERO;
            t.is_braking = true;
        }
    }

    /// `apply_stop_command` wrapped with ownership check.
    pub fn apply_stop_command_owned(&mut self, player_id: PlayerId, ship_id: ShipId) -> bool {
        if self.ship_owners.get(&ship_id) != Some(&player_id) {
            return false;
        }
        self.apply_stop_command(ship_id);
        true
    }

    /// プレイヤー船として指定し、PLAYER 性能値を設定する。
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

    // ── Phase 5: PlayerId / 所有権管理 ────────────────────────────────────────

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

        self.event_store.append(DomainEvent::ShipSpawned(ShipSpawned {
            ship_id,
            sector_id        : self.sector_id,
            initial_position : pos,
            ship_type_id     : SHIP_TYPE_MAGPIE,
            tick             : self.current_tick,
        }));

        ship_id
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
            modules       : Vec<(dawn_core::ModuleId, dawn_core::fitting::SlotKind)>,
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
            let modules = self.world.inner()
                .get::<&FittingComp>(entity)
                .map(|f| {
                    f.high.iter().chain(f.mid.iter()).chain(f.low.iter()).chain(f.rig.iter())
                        .map(|s| (s.def.id, s.def.slot))
                        .collect()
                })
                .unwrap_or_default();
            bots.push(BotState {
                player_id, ship_id,
                position    : pos.0,
                weapon_range: stats.weapon_range,
                locked_targets: locked,
                modules,
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
                for (module_id, slot) in &bot.modules {
                    self.activate_module_owned(bot.player_id, dawn_core::ActivateModuleCommand {
                        ship_id  : bot.ship_id,
                        module_id: *module_id,
                        slot     : *slot,
                    });
                }
            }
        }
    }

    #[allow(dead_code)]
    pub fn get_player_ship(&self, player_id: PlayerId) -> Option<ShipId> {
        self.player_ships.get(&player_id).copied()
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

    /// プレイヤー船の Fitting 状態を PlayerFitting JSON として返す。    ///
    /// 接続時に Welcome + InitialState の後に送信する、E    /// フォーマッチE
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
                        "max_speed_add"       : d.max_speed_add,
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

    pub fn build_initial_state_json(&self) -> String {
        let ships: Vec<serde_json::Value> = self.ship_index.keys().filter_map(|&ship_id| {
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
        }).collect();

        serde_json::json!({
            "type"  : "InitialState",
            "ships" : ships,
        }).to_string()
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

        // ShipFitted イベントを Append
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

            // TargetLocked: LockComp エントリを Locked 状態に更新
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

            // LockLost: LockComp からエントリを削除
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
            }
        }
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
        let mut node2 = SimulationNode::restore_from(store2, &snap);

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
        use dawn_core::events::DamageTaken;

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
}
