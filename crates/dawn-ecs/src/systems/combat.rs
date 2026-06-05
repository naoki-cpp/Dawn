//! Combat system — ロック済みターゲットへの武器発射 / ダメージ / 破壊。
//!
//! # 処理順序（CLAUDE.md §6 — Lock System の後に呼ぶこと）
//!
//! 各 Tick で次を行う:
//!   1. LockComp で Locked 状態のターゲットを確認する
//!   2. クールダウンが明けていれば WeaponFired を生成する
//!   3. ターゲットの HullComp にダメージを適用し DamageTaken を生成する
//!   4. HP ≤ 0 なら ShipDestroyed を生成し `destroyed` リストに積む
//!      （ECS からの削除は呼び出し元が行う）
//!
//! # Contract
//!
//! - 純粋計算: I/O なし、グローバル状態なし。
//! - Lock System が先に実行されていること（LockComp が最新状態であること）。
//! - 返り値の events を EventStore に Append するのは呼び出し元の責務。
//! - `destroyed` に含まれる ShipId は呼び出し元が ECS から削除すること。

use crate::{
    components::{HullComp, LockComp, ShipIdComp, ShipStatsComp, WeaponComp},
    SimWorld,
};
use dawn_core::{
    events::{DamageTaken, ShipDestroyed, WeaponFired},
    DomainEvent, ShipId, Tick,
};

// スナップショット用内部型
struct ShipSnapshot {
    ship_id        : ShipId,
    stats          : ShipStatsComp,
    last_fired     : Tick,
    current_shield : f32,
    current_armor  : f32,
    current_hull   : f32,
    is_dead        : bool,
    locked_targets : Vec<ShipId>,
}

/// Combat System の実行結果。
pub struct CombatResult {
    /// 今 Tick で生成されたイベント（WeaponFired / DamageTaken / ShipDestroyed）。
    pub events    : Vec<DomainEvent>,
    /// 破壊された Ship の ID リスト。呼び出し元が ECS から削除する。
    pub destroyed : Vec<ShipId>,
}

/// 1 Tick 分の Combat を実行する。
///
/// Lock System の後に呼ぶこと（LockComp が最新状態であること）。
pub fn run(world: &mut SimWorld, tick: Tick) -> CombatResult {
    // ── 1. 全 Ship をスナップショット ────────────────────────────────────────

    let mut ships: Vec<ShipSnapshot> = {
        let mut v = Vec::new();
        for (_, (id, stats, weapon, hull, lock)) in world
            .inner()
            .query::<(&ShipIdComp, &ShipStatsComp, &WeaponComp, &HullComp, &LockComp)>()
            .iter()
        {
            v.push(ShipSnapshot {
                ship_id        : id.0,
                stats          : *stats,
                last_fired     : weapon.last_fired_tick,
                current_shield : hull.current_shield,
                current_armor  : hull.current_armor,
                current_hull   : hull.current_hull,
                is_dead        : hull.is_destroyed,
                locked_targets : lock.locked_targets().collect(),
            });
        }
        v
    };

    // ── 2. ロック済みターゲットへ発射 ────────────────────────────────────────

    let mut events: Vec<DomainEvent>          = Vec::new();
    let mut destroyed: Vec<ShipId>            = Vec::new();
    let mut damage_accum: Vec<(usize, f32, ShipId)> = Vec::new();
    let mut fired: Vec<(usize, Tick)>         = Vec::new();

    let n = ships.len();
    for i in 0..n {
        if ships[i].is_dead { continue; }
        if ships[i].stats.weapon_damage <= 0.0 { continue; }
        if !can_fire(ships[i].last_fired, tick, ships[i].stats.weapon_cooldown) { continue; }

        // Locked ターゲットの中から最初の生存 Ship を選択
        let target_id = ships[i].locked_targets.iter()
            .find(|&&tid| ships.iter().any(|s| s.ship_id == tid && !s.is_dead))
            .copied();
        let Some(tid) = target_id else { continue };
        let Some(j)   = ships.iter().position(|s| s.ship_id == tid) else { continue };

        let damage = ships[i].stats.weapon_damage;
        events.push(DomainEvent::WeaponFired(WeaponFired {
            attacker_id: ships[i].ship_id,
            target_id  : tid,
            damage,
            tick,
        }));
        fired.push((i, tick));
        damage_accum.push((j, damage, ships[i].ship_id));
    }

    // ── 3. ダメージ適用 ──────────────────────────────────────────────────────

    for (j, damage, attacker_id) in damage_accum {
        // Shield → Armor → Hull の順にダメージを適用
        let mut remaining = damage;
        let shield_absorbed = remaining.min(ships[j].current_shield);
        ships[j].current_shield -= shield_absorbed;
        remaining -= shield_absorbed;
        if remaining > 0.0 {
            let armor_absorbed = remaining.min(ships[j].current_armor);
            ships[j].current_armor -= armor_absorbed;
            remaining -= armor_absorbed;
        }
        if remaining > 0.0 {
            ships[j].current_hull = (ships[j].current_hull - remaining).max(0.0);
        }

        events.push(DomainEvent::DamageTaken(DamageTaken {
            ship_id        : ships[j].ship_id,
            damage,
            current_shield : ships[j].current_shield,
            current_armor  : ships[j].current_armor,
            current_hull   : ships[j].current_hull,
            tick,
        }));

        if ships[j].current_hull <= 0.0 && !ships[j].is_dead {
            ships[j].is_dead = true;
            events.push(DomainEvent::ShipDestroyed(ShipDestroyed {
                ship_id  : ships[j].ship_id,
                killer_id: attacker_id,
                tick,
            }));
            destroyed.push(ships[j].ship_id);
        }
    }

    // ── 4. ECS へ書き戻し ────────────────────────────────────────────────────

    for (i, new_tick) in fired {
        let target_id = ships[i].ship_id;
        for (_, (id, weapon)) in world
            .inner_mut()
            .query_mut::<(&ShipIdComp, &mut WeaponComp)>()
        {
            if id.0 == target_id { weapon.last_fired_tick = new_tick; break; }
        }
    }

    let hp_updates: Vec<(ShipId, f32, f32, f32, bool)> = ships.iter()
        .map(|s| (s.ship_id, s.current_shield, s.current_armor, s.current_hull, s.is_dead))
        .collect();
    for (target_id, shield, armor, hull_hp, dead) in hp_updates {
        for (_, (id, hull)) in world
            .inner_mut()
            .query_mut::<(&ShipIdComp, &mut HullComp)>()
        {
            if id.0 == target_id {
                hull.current_shield = shield;
                hull.current_armor  = armor;
                hull.current_hull   = hull_hp;
                hull.is_destroyed   = dead;
                break;
            }
        }
    }

    CombatResult { events, destroyed }
}

// ── ヘルパー ──────────────────────────────────────────────────────────────────

fn can_fire(last_fired: Tick, current: Tick, cooldown: u64) -> bool {
    current.value() >= last_fired.value() + cooldown
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        components::{LockComp, LockEntry, LockState, ShipStatsComp},
        SimWorld,
    };
    use dawn_core::{NodeId, Position, SectorId, ShipId, Tick, Velocity};

    fn ship_id(n: u64) -> ShipId { ShipId::new(NodeId(0), n) }

    fn armed_stats(damage: f32, cooldown: u64) -> ShipStatsComp {
        ShipStatsComp { weapon_damage: damage, weapon_range: 5_000.0, weapon_cooldown: cooldown, ..ShipStatsComp::NPC }
    }

    /// ship_id(1) に対して ship_id(2) を Locked 状態にセットアップする
    fn setup_with_lock(damage: f32, cooldown: u64) -> SimWorld {
        let mut world = SimWorld::new(SectorId(0));
        let ea = world.spawn_ship(ship_id(1), Position::ORIGIN, Velocity::ZERO);
        world.spawn_ship(ship_id(2), Position::new(100.0, 0.0, 0.0), Velocity::ZERO);
        world.set_ship_stats(ea, armed_stats(damage, cooldown));
        // ship_id(1) のロックを Locked 状態にセット
        if let Ok(mut lock) = world.inner_mut().get::<&mut LockComp>(ea) {
            lock.entries.push(LockEntry { target_id: ship_id(2), state: LockState::Locked });
        }
        world
    }

    #[test]
    fn weapon_fires_when_target_is_locked_and_cooldown_cleared() {
        let mut world = setup_with_lock(25.0, 1);
        let result = run(&mut world, Tick(1));
        let fired = result.events.iter().filter(|e| matches!(e, DomainEvent::WeaponFired(_))).count();
        assert_eq!(fired, 1);
    }

    #[test]
    fn no_fire_without_lock() {
        let mut world = SimWorld::new(SectorId(0));
        let ea = world.spawn_ship(ship_id(1), Position::ORIGIN, Velocity::ZERO);
        world.spawn_ship(ship_id(2), Position::new(100.0, 0.0, 0.0), Velocity::ZERO);
        world.set_ship_stats(ea, armed_stats(25.0, 1));
        // LockComp は空のまま
        let result = run(&mut world, Tick(1));
        assert!(result.events.is_empty(), "ロックなしでは発射しない");
    }

    #[test]
    fn ship_destroyed_when_hp_reaches_zero() {
        let mut world = setup_with_lock(99999.0, 1);
        let result = run(&mut world, Tick(1));
        let destroyed = result.events.iter().filter(|e| matches!(e, DomainEvent::ShipDestroyed(_))).count();
        assert_eq!(destroyed, 1);
        assert_eq!(result.destroyed.len(), 1);
    }

    #[test]
    fn weapon_does_not_fire_during_cooldown() {
        let mut world = setup_with_lock(1.0, 5);
        run(&mut world, Tick(1));
        let r2 = run(&mut world, Tick(4));  // 1 + 5 = 6 > 4
        let fired = r2.events.iter().filter(|e| matches!(e, DomainEvent::WeaponFired(_))).count();
        assert_eq!(fired, 0, "クールダウン中は発射しない");
    }

    #[test]
    fn weapon_fires_again_after_cooldown() {
        let mut world = setup_with_lock(1.0, 5);
        run(&mut world, Tick(1));
        let r = run(&mut world, Tick(6));
        let fired = r.events.iter().filter(|e| matches!(e, DomainEvent::WeaponFired(_))).count();
        assert_eq!(fired, 1, "クールダウン後に再発射");
    }
}
