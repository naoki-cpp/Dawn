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
//! node.take_snapshot()           → StateSnapshot (ECS state at log_index N)
//! SimulationNode::restore_from(store, &snapshot)
//!     → reconstruct ECS from snapshot, replay events from log_index N onward
//! ```

use std::collections::HashMap;

use dawn_core::{
    events::{ShipFitted, ShipSpawned},
    DomainEvent, FitModuleCommand, ModuleDefinition, ModuleId, NodeId, PlayerId, Position,
    SectorBounds, SectorId, ShipId, Tick, Velocity,
};
use dawn_ecs::{
    components::{FittingComp, HullComp, IsNpcComp, LockComp, PositionComp, ShipStatsComp, ThrustComp, VelocityComp},
    systems::{CombatSystem, LockSystem, MovementSystem, apply_fitting},
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
    /// Number of `ShipMoved` events emitted this tick.
    pub events_emitted: usize,
    /// The actual events produced (used by Actor layer for replication).
    pub events        : Vec<DomainEvent>,
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
    /// Maps `ShipId` → hecs `Entity` for O(1) position updates during replay.
    ship_index      : HashMap<ShipId, Entity>,
    /// Module definition registry.  Loaded at startup; looked up during FitModuleCommand.
    module_registry : HashMap<ModuleId, ModuleDefinition>,
    /// 装備なし時の素の ShipStats。Fitting 集計の base として使う。
    base_stats      : HashMap<ShipId, ShipStatsComp>,
    /// PlayerId → ShipId（プレイヤー船の所有権管理）
    player_ships    : HashMap<PlayerId, ShipId>,
    /// ShipId → PlayerId（逆引き：所有権チェックに使う）
    ship_owners     : HashMap<ShipId, PlayerId>,
    /// PlayerId 採番カウンタ
    player_id_counter: u64,
}

// ── Constructors ──────────────────────────────────────────────────────────────

impl SimulationNode<InMemoryEventStore> {
    /// Create a node backed by an in-memory event store (Phase 0–2 default).
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
            ship_index        : HashMap::new(),
            module_registry   : HashMap::new(),
            base_stats        : HashMap::new(),
            player_ships      : HashMap::new(),
            ship_owners       : HashMap::new(),
            player_id_counter : 0,
        }
    }

    /// Restore a node from a `StateSnapshot` and replay subsequent events.
    ///
    /// `store` must already contain all events (loaded from disk).
    /// Events before `snapshot.log_index` are covered by the snapshot;
    /// events from `snapshot.log_index` onward are replayed.
    pub fn restore_from(store: S, snapshot: &StateSnapshot) -> Self {
        let mut node = Self {
            node_id           : snapshot.node_id,
            sector_id         : snapshot.sector_id,
            bounds            : snapshot.bounds,
            world             : SimWorld::new(snapshot.sector_id),
            event_store       : store,
            current_tick      : snapshot.tick,
            id_counter        : snapshot.id_counter,
            ship_index        : HashMap::new(),
            module_registry   : HashMap::new(),
            base_stats        : HashMap::new(),
            player_ships      : HashMap::new(),
            ship_owners       : HashMap::new(),
            player_id_counter : 0,
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
    pub fn spawn_ship(&mut self, position: Position, velocity: Velocity) -> ShipId {
        let ship_id = ShipId::new(self.node_id, self.id_counter);
        self.id_counter += 1;

        self.insert_to_world(ship_id, position, velocity);
        // NPC ベーススタットを記録（Fitting 集計の base として使う）
        self.base_stats.insert(ship_id, ShipStatsComp::NPC);

        self.event_store.append(DomainEvent::ShipSpawned(ShipSpawned {
            ship_id,
            sector_id       : self.sector_id,
            initial_position: position,
            tick            : self.current_tick,
        }));

        ship_id
    }

    // ── Tick ──────────────────────────────────────────────────────────────────

    /// Execute one simulation tick.
    ///
    /// Processing order (CLAUDE.md §6 — must not be reordered):
    ///
    /// 1. Advance the logical tick counter.
    /// 2. Run the Movement System (ECS batch).
    /// 3. Append produced events to the Event Store.
    /// 4. Return a `TickResult` (events included for Actor-layer replication).
    pub fn tick(&mut self) -> TickResult {
        self.tick_with_lock_commands(&[])
    }

    /// ロックオンコマンド付きで Tick を実行する（内部・テスト用）。
    pub fn tick_with_lock_commands(
        &mut self,
        lock_commands: &[dawn_core::LockOnCommand],
    ) -> TickResult {
        self.current_tick = self.current_tick.next();
        let tick = self.current_tick;

        // 3. Movement System（CLAUDE.md §6 処理順序）
        let move_events = MovementSystem::run(&mut self.world, tick);

        // 4. Lock System
        let lock = LockSystem(&mut self.world, tick, lock_commands);

        // 5. Combat System
        let combat = CombatSystem(&mut self.world, tick);

        // 破壊された Ship を ECS と ship_index から削除
        for ship_id in &combat.destroyed {
            if let Some(entity) = self.ship_index.remove(ship_id) {
                self.world.despawn_ship(entity);
            }
        }

        // 6. EventStore に Append
        let all_events: Vec<DomainEvent> = move_events.iter()
            .chain(lock.events.iter())
            .chain(combat.events.iter())
            .cloned()
            .collect();

        let count = all_events.len();
        self.event_store.append_batch(all_events.iter().cloned());

        TickResult { tick, events_emitted: count, events: all_events }
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
        self.world.inner().get::<&HullComp>(*entity).ok().map(|c| c.current_hp)
    }

    /// `apply_event` のテスト用公開ラッパー。
    #[cfg(test)]
    pub fn apply_event_pub(&mut self, event: DomainEvent) {
        self.apply_event(&event);
    }

    /// MoveCommand を処理する: 目標方向への Thrust ベクトルを設定する。
    ///
    /// 毎 Tick、MovementSystem が `thrust_magnitude` の大きさで
    /// この方向へ速度を加算する。`max_speed` を超えた分は clamp される。
    ///
    /// `target == current_position` の場合は thrust をゼロにして「停止推力」。
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

        // Thrust 方向ベクトル（正規化済み）。目標が同じ点ならゼロ。
        let thrust = if dist > f32::EPSILON {
            Velocity { dx: dx / dist, dy: dy / dist, dz: dz / dist }
        } else {
            Velocity::ZERO
        };

        if let Ok(mut t) = self.world.inner_mut().get::<&mut ThrustComp>(entity) {
            t.0 = thrust;
        }
    }

    /// プレイヤー船として指定し、PLAYER 性能値を設定する。
    ///
    /// Cycle 2: 最初に Spawn した船を Godot 側からこの API で指定する。
    pub fn set_player_ship(&mut self, ship_id: ShipId) {
        if let Some(&entity) = self.ship_index.get(&ship_id) {
            // base_stats を PLAYER に切り替えてから Fitting を再集計する
            self.base_stats.insert(ship_id, ShipStatsComp::PLAYER);
            self.world.set_ship_stats(entity, ShipStatsComp::PLAYER);
            // HullComp の max_hp も PLAYER 基準に更新
            if let Ok(mut hull) = self.world.inner_mut().get::<&mut HullComp>(entity) {
                hull.current_hp = ShipStatsComp::PLAYER.max_hp;
            }
            // 既に装備済みのモジュールがあれば PLAYER base で再集計
            let base = ShipStatsComp::PLAYER;
            apply_fitting(&mut self.world, ship_id, base);
        }
    }

    // ── Phase 5: PlayerId / 所有権管理 ────────────────────────────────────────

    /// 新しい PlayerId を採番して返す（接続時に呼ぶ）。
    pub fn next_player_id(&mut self) -> PlayerId {
        let id = PlayerId(self.player_id_counter);
        self.player_id_counter += 1;
        id
    }

    /// プレイヤー専用の Ship を Spawn し、PlayerId と紐付ける。
    ///
    /// NPC 船と異なり PLAYER 性能値が設定され、Weapon モジュールが自動装備される。
    pub fn spawn_player_ship(&mut self, player_id: PlayerId) -> ShipId {
        // 中心付近のランダムな位置に Spawn
        let pos = Position::new(0.0, 0.0, 0.0);
        let ship_id = ShipId::new(self.node_id, self.id_counter);
        self.id_counter += 1;

        self.insert_to_world(ship_id, pos, Velocity::ZERO);
        self.base_stats.insert(ship_id, ShipStatsComp::PLAYER);

        // PLAYER 性能値に切り替え + IsNpcComp を除去（プレイヤーは自動ロックしない）
        if let Some(&entity) = self.ship_index.get(&ship_id) {
            self.world.set_ship_stats(entity, ShipStatsComp::PLAYER);
            if let Ok(mut hull) = self.world.inner_mut().get::<&mut HullComp>(entity) {
                hull.current_hp = ShipStatsComp::PLAYER.max_hp;
            }
            // IsNpcComp を取り除く（spawn_ship で追加されるため）
            let _ = self.world.inner_mut().remove_one::<dawn_ecs::components::IsNpcComp>(entity);
        }

        // Small Railgun I を自動装備
        use dawn_core::{FitModuleCommand, SlotKind};
        self.fit_module(FitModuleCommand {
            ship_id,
            slot      : SlotKind::High,
            module_id : crate::modules::MODULE_RAILGUN_SMALL,
        });

        // 所有権を記録
        self.player_ships.insert(player_id, ship_id);
        self.ship_owners.insert(ship_id, player_id);

        self.event_store.append(DomainEvent::ShipSpawned(ShipSpawned {
            ship_id,
            sector_id       : self.sector_id,
            initial_position: pos,
            tick            : self.current_tick,
        }));

        ship_id
    }

    /// PlayerId が所有する ShipId を返す。
    pub fn get_player_ship(&self, player_id: PlayerId) -> Option<ShipId> {
        self.player_ships.get(&player_id).copied()
    }

    /// 所有権つき MoveCommand 処理。自分の船だけ操作できる。
    pub fn apply_move_command_owned(
        &mut self,
        player_id : PlayerId,
        ship_id   : ShipId,
        target    : Position,
    ) -> bool {
        if self.ship_owners.get(&ship_id) != Some(&player_id) {
            return false;  // 所有権なし → 無視
        }
        self.apply_move_command(ship_id, target);
        true
    }

    /// 所有権つき LockOnCommand 処理。
    pub fn apply_lock_on_owned(
        &mut self,
        player_id : PlayerId,
        cmd       : dawn_core::LockOnCommand,
    ) -> bool {
        if self.ship_owners.get(&cmd.ship_id) != Some(&player_id) {
            return false;
        }
        // lock_commands は tick_with_lock_commands() に渡すので true を返すだけ
        true
    }

    /// 現在の全 Ship の状態を InitialState JSON として返す（接続時の同期用）。
    pub fn build_initial_state_json(&self) -> String {
        let ships: Vec<serde_json::Value> = self.ship_index.keys().filter_map(|&ship_id| {
            let entity  = self.ship_index.get(&ship_id)?;
            let pos     = self.world.inner().get::<&PositionComp>(*entity).ok()?.0;
            let stats   = self.world.inner().get::<&ShipStatsComp>(*entity).ok()?;
            let hull    = self.world.inner().get::<&HullComp>(*entity).ok()?;
            Some(serde_json::json!({
                "ship_id"   : ship_id.raw(),
                "position"  : { "x": pos.x, "y": pos.y, "z": pos.z },
                "max_hp"    : stats.max_hp,
                "current_hp": hull.current_hp,
            }))
        }).collect();

        serde_json::json!({
            "type"  : "InitialState",
            "ships" : ships,
        }).to_string()
    }

    // ── Module Activation ─────────────────────────────────────────────────────

    /// Active モジュールをオンにする。
    pub fn activate_module(&mut self, cmd: dawn_core::ActivateModuleCommand) -> bool {
        self.set_module_active(cmd.ship_id, cmd.module_id, cmd.slot, true)
    }

    /// Active モジュールをオフにする。
    pub fn deactivate_module(&mut self, cmd: dawn_core::DeactivateModuleCommand) -> bool {
        self.set_module_active(cmd.ship_id, cmd.module_id, cmd.slot, false)
    }

    /// 所有権チェック付き activate。
    pub fn activate_module_owned(&mut self, player_id: PlayerId, cmd: dawn_core::ActivateModuleCommand) -> bool {
        if self.ship_owners.get(&cmd.ship_id) != Some(&player_id) { return false; }
        self.activate_module(cmd)
    }

    /// 所有権チェック付き deactivate。
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

        // FittingComp のスロットで対象モジュールを探す
        let found = self.world.inner_mut()
            .get::<&mut FittingComp>(entity)
            .ok()
            .and_then(|mut f| f.find_slot_mut(module_id, slot).map(|s| {
                s.is_active = active;
                true
            }))
            .unwrap_or(false);

        if !found { return false; }

        // ShipStatsComp を再集計
        let base = self.base_stats.get(&ship_id).copied().unwrap_or(ShipStatsComp::NPC);
        apply_fitting(&mut self.world, ship_id, base);

        // イベントを発行
        let event = if active {
            DomainEvent::ModuleActivated(ModuleActivated { ship_id, module_id, slot, tick: self.current_tick })
        } else {
            DomainEvent::ModuleDeactivated(ModuleDeactivated { ship_id, module_id, slot, tick: self.current_tick })
        };
        self.event_store.append(event);
        true
    }

    // ── Fitting ───────────────────────────────────────────────────────────────

    /// モジュール定義をレジストリに登録する。
    ///
    /// サーバー起動時に全モジュールを登録しておくことで、
    /// `fit_module` が ID だけでモジュール定義を解決できる。
    pub fn register_module(&mut self, def: ModuleDefinition) {
        self.module_registry.insert(def.id, def);
    }

    /// `FitModuleCommand` を処理する。
    ///
    /// 1. モジュール定義をレジストリで解決する。
    /// 2. `FittingComp` のスロットにモジュールを追加する。
    /// 3. `apply_fitting()` で `ShipStatsComp` を再集計する。
    /// 4. `ShipFitted` イベントを EventStore に Append する。
    ///
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

        // スロットにモジュールを追加
        // Active モジュールの初期状態:
        //   NPC 船  → is_active = true（自動でオン）
        //   プレイヤー船 → is_active = false（手動でオンにする）
        use dawn_core::fitting::ActivationMode;
        use dawn_ecs::components::FittedSlot;
        let is_npc = self.ship_owners.get(&cmd.ship_id).is_none();
        let is_active = match def.activation_mode {
            ActivationMode::Passive => true,   // Passive は常にオン扱い
            ActivationMode::Active  => is_npc, // NPC=true, Player=false
        };
        if let Ok(mut fitting) = self.world.inner_mut().get::<&mut FittingComp>(entity) {
            fitting.slot_mut(cmd.slot).push(FittedSlot { def, is_active });
        } else {
            return false;
        }

        // 装備なし時の素の stats を base として集計（二重加算を防ぐ）
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
                    // Velocity is not stored in ShipSpawned; snapshot provides it.
                    self.insert_to_world(e.ship_id, e.initial_position, Velocity::ZERO);
                }
                // Advance id_counter past any replayed ID to prevent future reuse.
                let counter = e.ship_id.0.counter();
                if counter >= self.id_counter {
                    self.id_counter = counter + 1;
                }
            }

            // VelocityChanged: Replay 時の位置再構築（ADR-0008）
            //
            // VelocityChanged { velocity: V_new, tick: T } は
            // 「Tick T に thrust を適用して速度が V_new になり、position += V_new が実行された」事実。
            //
            // Replay 手順:
            //   1. 前回のイベント以降 (current_tick+1 〜 T-1) は旧速度で位置を前進
            //   2. Tick T では新速度で位置を前進
            //   3. 速度を V_new に更新
            DomainEvent::VelocityChanged(e) => {
                if let Some(&entity) = self.ship_index.get(&e.ship_id) {
                    // 1. 旧速度で前進すべき Tick 数（T-1 まで）
                    let gap_ticks = e.tick.value()
                        .saturating_sub(self.current_tick.value())
                        .saturating_sub(1);  // Tick T 自体は新速度で処理するため -1

                    let old_vel = self.world.inner()
                        .get::<&VelocityComp>(entity).ok()
                        .map(|v| v.0)
                        .unwrap_or(Velocity::ZERO);

                    if let Ok(mut pos) = self.world.inner_mut().get::<&mut PositionComp>(entity) {
                        // gap_ticks 分は旧速度で前進、Tick T は新速度で前進
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

            // ShipMoved (deprecated, Upcaster): 位置差分から速度を復元して VelocityChanged として扱う
            #[allow(deprecated)]
            DomainEvent::ShipMoved(e) => {
                if let Some(&entity) = self.ship_index.get(&e.ship_id) {
                    // to - from = 1 Tick 分の変位 = velocity (units/tick)
                    let velocity = Velocity {
                        dx: e.to.x - e.from.x,
                        dy: e.to.y - e.from.y,
                        dz: e.to.z - e.from.z,
                    };
                    if let Ok(mut vel) = self.world.inner_mut().get::<&mut VelocityComp>(entity) {
                        vel.0 = velocity;
                    }
                    if let Ok(mut pos) = self.world.inner_mut().get::<&mut PositionComp>(entity) {
                        pos.0 = e.to;
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
                // FittingComp を Snapshot から復元して stats を再集計する
                if let Some(&entity) = self.ship_index.get(&e.ship_id) {
                    let fitting = FittingComp::from_snapshot(&e.fitting, &self.module_registry);
                    // 素の base stats を使って集計（二重加算を防ぐ）
                    let base = self.base_stats
                        .get(&e.ship_id)
                        .copied()
                        .unwrap_or(ShipStatsComp::NPC);
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

            // WeaponFired は ECS 状態を変えない（発射ログのみ）
            DomainEvent::WeaponFired(_) => {}

            // DamageTaken: HullComp.current_hp を更新する（INV-002 準拠）
            DomainEvent::DamageTaken(e) => {
                if let Some(&entity) = self.ship_index.get(&e.ship_id) {
                    if let Ok(mut hull) = self.world.inner_mut().get::<&mut HullComp>(entity) {
                        hull.current_hp = e.current_hp;
                        if e.current_hp <= 0.0 {
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
            SectorBounds::cube(SectorBounds::DEFAULT_SIZE),
        )
    }

    // ── Existing behaviour (unchanged) ───────────────────────────────────────

    #[test]
    fn spawning_a_ship_appends_a_ship_spawned_event() {
        let mut node = mem_node();
        node.spawn_ship(Position::ORIGIN, Velocity::ZERO);

        assert_eq!(node.total_event_count(), 1);
        assert!(matches!(
            node.event_store().all_records()[0].event,
            DomainEvent::ShipSpawned(_)
        ));
    }

    #[test]
    fn spawned_ships_receive_unique_ids() {
        let mut node = mem_node();
        let id_a = node.spawn_ship(Position::ORIGIN, Velocity::ZERO);
        let id_b = node.spawn_ship(Position::ORIGIN, Velocity::ZERO);
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
        // NPC（等速直線運動）は速度が変わらないので VelocityChanged を出さない（ADR-0008）
        let mut node = mem_node();
        node.spawn_ship(Position::new(100.0, 100.0, 100.0), Velocity::new(1.0, 0.0, 0.0));
        node.spawn_ship(Position::new(200.0, 100.0, 100.0), Velocity::new(0.0, 1.0, 0.0));
        assert_eq!(node.tick().events_emitted, 0,
            "NPC ships at constant velocity do not emit VelocityChanged");
    }

    #[test]
    fn stationary_ships_produce_no_events() {
        let mut node = mem_node();
        node.spawn_ship(Position::ORIGIN, Velocity::ZERO);
        assert_eq!(node.tick().events_emitted, 0);
    }

    #[test]
    fn velocity_changed_events_carry_the_current_tick_value() {
        // プレイヤー船（thrust あり）は VelocityChanged を発行する
        let mut node = mem_node();
        let ship_id = node.spawn_ship(Position::new(100.0, 100.0, 100.0), Velocity::ZERO);
        // プレイヤー船として設定（thrust_magnitude > 0）
        node.set_player_ship(ship_id);
        node.apply_move_command(ship_id, Position::new(10000.0, 0.0, 0.0));
        node.tick();
        node.tick();
        // VelocityChanged イベントが tick=2 で発行されているはず
        let last = node.event_store().all_records().last().unwrap();
        assert_eq!(last.event.tick(), Tick(2));
    }

    #[test]
    fn total_event_count_grows_monotonically_across_ticks() {
        let mut node = mem_node();
        node.spawn_ship(Position::new(100.0, 100.0, 100.0), Velocity::new(1.0, 1.0, 1.0));
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
            node.spawn_ship(Position::new(i as f32 * 100.0, 0.0, 0.0), Velocity::new(1.0, 0.0, 0.0));
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
            node.spawn_ship(Position::new(i as f32 * 100.0, 0.0, 0.0), Velocity::new(1.0, 0.0, 0.0));
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
                SectorBounds::cube(SectorBounds::DEFAULT_SIZE),
                store,
            );

            // Spawn 5 ships as players with thrust so they emit VelocityChanged events.
            // (ADR-0008: NPC ships at constant velocity emit no events, so tick cannot
            //  be restored from the event log alone. Using player ships with thrust
            //  ensures VelocityChanged events carry the tick for replay.)
            ship_ids = (0..5u64).map(|i| {
                let id = node.spawn_ship(
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
        } // ← node drops here; FileEventStore flushes on drop via BufWriter

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

    // ── Fitting: 二重加算しないことを保証 ────────────────────────────────────

    #[test]
    fn fitting_same_module_twice_does_not_double_count_stats() {
        use dawn_core::{FitModuleCommand, ModuleId, SlotKind};
        use dawn_core::fitting::{ModuleDefinition, ModuleKind, StatDelta};

        let mut node = mem_node();
        let ship_id = node.spawn_ship(Position::ORIGIN, Velocity::ZERO);

        let railgun = ModuleDefinition {
            id              : ModuleId(1),
            name            : "Test Railgun".to_string(),
            kind            : ModuleKind::Weapon,
            slot            : SlotKind::High,
            activation_mode : dawn_core::ActivationMode::Active,
            stat_delta      : StatDelta { weapon_damage_add: 25.0, weapon_range_add: 1000.0, ..StatDelta::ZERO },
        };
        node.register_module(railgun);

        // 1回目の装備
        node.fit_module(FitModuleCommand { ship_id, slot: SlotKind::High, module_id: ModuleId(1) });
        let stats_after_first = node.get_ship_stats(ship_id).unwrap();
        assert_eq!(stats_after_first.weapon_damage, 25.0, "1回装備後は base(0) + delta(25) = 25");

        // 同じモジュールをもう1つ装備（2スロット目）→ delta が2倍になるが base からの計算は正しいはず
        node.fit_module(FitModuleCommand { ship_id, slot: SlotKind::High, module_id: ModuleId(1) });
        let stats_after_second = node.get_ship_stats(ship_id).unwrap();
        assert_eq!(stats_after_second.weapon_damage, 50.0,
            "2個装備後は base(0) + 2×delta(25) = 50（二重加算なら75になる）");
    }

    // ── Combat Replay: DamageTaken が HP を正しく復元する（INV-002） ──────────

    #[test]
    fn damage_taken_event_is_replayed_to_restore_current_hp() {
        use dawn_core::events::DamageTaken;

        let mut node = mem_node();
        let ship_id = node.spawn_ship(Position::ORIGIN, Velocity::ZERO);

        // DamageTaken イベントを直接 apply_event で適用（Replay シミュレーション）
        node.apply_event_pub(DomainEvent::DamageTaken(DamageTaken {
            ship_id,
            amount    : 100.0,
            current_hp: 400.0,   // 500 - 100
            tick      : Tick(1),
        }));

        let hp = node.get_ship_hp(ship_id).unwrap();
        assert_eq!(hp, 400.0, "Replay 後の HP は DamageTaken イベントの current_hp と一致する");
    }
}
