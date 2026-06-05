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
    events::{ShipFitted, ShipMoved, ShipSpawned},
    DomainEvent, FitModuleCommand, ModuleDefinition, ModuleId, NodeId, Position,
    SectorBounds, SectorId, ShipId, Tick, Velocity,
};
use dawn_ecs::{
    components::{FittingComp, HullComp, LockComp, PositionComp, ShipStatsComp, ThrustComp, VelocityComp},
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
    /// spawn 時に記録し、以後変更しない。
    base_stats      : HashMap<ShipId, ShipStatsComp>,
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
            world           : SimWorld::new(sector_id),
            event_store     : store,
            current_tick    : Tick::ZERO,
            id_counter      : 0,
            ship_index      : HashMap::new(),
            module_registry : HashMap::new(),
            base_stats      : HashMap::new(),
        }
    }

    /// Restore a node from a `StateSnapshot` and replay subsequent events.
    ///
    /// `store` must already contain all events (loaded from disk).
    /// Events before `snapshot.log_index` are covered by the snapshot;
    /// events from `snapshot.log_index` onward are replayed.
    pub fn restore_from(store: S, snapshot: &StateSnapshot) -> Self {
        let mut node = Self {
            node_id         : snapshot.node_id,
            sector_id       : snapshot.sector_id,
            bounds          : snapshot.bounds,
            world           : SimWorld::new(snapshot.sector_id),
            event_store     : store,
            current_tick    : snapshot.tick,
            id_counter      : snapshot.id_counter,
            ship_index      : HashMap::new(),
            module_registry : HashMap::new(),
            base_stats      : HashMap::new(),
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
        if let Ok(mut fitting) = self.world.inner_mut().get::<&mut FittingComp>(entity) {
            fitting.slot_mut(cmd.slot).push(def);
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

            DomainEvent::ShipMoved(ShipMoved { ship_id, to, tick, .. }) => {
                if let Some(&entity) = self.ship_index.get(ship_id) {
                    if let Ok(mut pos) = self.world.inner_mut().get::<&mut PositionComp>(entity) {
                        pos.0 = *to;
                    }
                }
                if *tick > self.current_tick {
                    self.current_tick = *tick;
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
    fn each_moving_ship_produces_one_ship_moved_event_per_tick() {
        let mut node = mem_node();
        node.spawn_ship(Position::new(100.0, 100.0, 100.0), Velocity::new(1.0, 0.0, 0.0));
        node.spawn_ship(Position::new(200.0, 100.0, 100.0), Velocity::new(0.0, 1.0, 0.0));
        assert_eq!(node.tick().events_emitted, 2);
    }

    #[test]
    fn stationary_ships_produce_no_ship_moved_events() {
        let mut node = mem_node();
        node.spawn_ship(Position::ORIGIN, Velocity::ZERO);
        assert_eq!(node.tick().events_emitted, 0);
    }

    #[test]
    fn ship_moved_events_carry_the_current_tick_value() {
        let mut node = mem_node();
        node.spawn_ship(Position::new(100.0, 100.0, 100.0), Velocity::new(1.0, 0.0, 0.0));
        node.tick();
        node.tick();
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

            // Spawn 5 ships.
            ship_ids = (0..5u64).map(|i| {
                node.spawn_ship(
                    Position::new(i as f32 * 100.0, 0.0, 0.0),
                    Velocity::new(1.0 + i as f32, 0.0, 0.0),
                )
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
        let snap   = StateSnapshot::load(&snapshot_path).unwrap();
        let store2 = FileEventStore::open(&event_path).unwrap();
        let node2  = SimulationNode::restore_from(store2, &snap);

        assert_eq!(
            node2.current_tick(), final_tick,
            "tick must match after restore"
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
                "position of ship {} must match after restore", id
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
            id        : ModuleId(1),
            name      : "Test Railgun".to_string(),
            kind      : ModuleKind::Weapon,
            slot      : SlotKind::High,
            stat_delta: StatDelta { weapon_damage_add: 25.0, weapon_range_add: 1000.0, ..StatDelta::ZERO },
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
