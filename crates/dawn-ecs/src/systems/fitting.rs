//! Fitting system — FittingComp の StatDelta を ShipStatsComp に集計する。
//!
//! # 呼び出しタイミング
//! `FitModuleCommand` を処理した直後に `apply_fitting()` を呼び出すこと。
//! Tick ループ内では呼ばない（装備変更はイベント駆動）。

use crate::components::{FittingComp, HullComp, ShipStatsComp};
use crate::world::SimWorld;
use dawn_core::{ShipId, fitting::StatDelta};

/// 指定した Ship の `FittingComp` から `ShipStatsComp` を再集計する。
///
/// # ベース stats
/// 装備なしの base として `ShipStatsComp::NPC` または `PLAYER` を渡す。
/// モジュールの delta をその上に加算する。
///
/// # 戻り値
/// 集計後の `ShipStatsComp`（ECS への反映済み）。
/// Ship が存在しない場合は `None`。
pub fn apply_fitting(world: &mut SimWorld, ship_id: ShipId, base: ShipStatsComp) -> Option<ShipStatsComp> {
    // Ship entity を検索
    let entity = {
        let mut found = None;
        for (e, id) in world.inner().query::<&crate::components::ShipIdComp>().iter() {
            if id.0 == ship_id {
                found = Some(e);
                break;
            }
        }
        found?
    };

    // FittingComp の delta を集計
    let delta: StatDelta = world
        .inner()
        .get::<&FittingComp>(entity)
        .map(|f| f.total_delta())
        .unwrap_or(StatDelta::ZERO);

    // base + delta → 新しい stats
    let new_stats = apply_delta(base, &delta);

    // ShipStatsComp を更新
    world.set_ship_stats(entity, new_stats);

    // HullComp が存在すれば max_hp も更新（current_hp は比率で調整）
    if let Ok(mut hull) = world.inner_mut().get::<&mut HullComp>(entity) {
        let old_max = hull.current_hp.max(1.0);  // ゼロ除算防止
        let ratio   = hull.current_hp / old_max;
        hull.current_hp = (new_stats.max_hp * ratio).max(0.0);
    }

    Some(new_stats)
}

/// `base` stats に `delta` を加算した新しい stats を返す。
pub fn apply_delta(base: ShipStatsComp, delta: &StatDelta) -> ShipStatsComp {
    ShipStatsComp {
        max_speed        : (base.max_speed        + delta.max_speed_add).max(0.0),
        thrust_magnitude : (base.thrust_magnitude + delta.thrust_add).max(0.0),
        max_hp           : (base.max_hp           + delta.max_hp_add).max(1.0),
        weapon_damage    : (base.weapon_damage    + delta.weapon_damage_add).max(0.0),
        weapon_range     : (base.weapon_range     + delta.weapon_range_add).max(0.0),
        weapon_cooldown  : {
            let raw = base.weapon_cooldown as i64 + delta.weapon_cooldown_add as i64;
            raw.max(1) as u64
        },
        lock_time        : {
            let raw = base.lock_time as i64 + delta.lock_time_add as i64;
            raw.max(1) as u64  // 最低 1 Tick
        },
        max_locks        : {
            let raw = base.max_locks as i64 + delta.max_locks_add as i64;
            raw.max(0) as u32  // 0 = ロック不可
        },
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::FittingComp;
    use dawn_core::{NodeId, Position, ShipId, Velocity, fitting::{ModuleDefinition, ModuleId, ModuleKind, SlotKind, StatDelta}};
    use crate::world::SimWorld;
    use dawn_core::SectorId;

    fn ship_id() -> ShipId { ShipId::new(NodeId(0), 1) }

    fn make_world_with_ship() -> (SimWorld, ShipId) {
        let mut world = SimWorld::new(SectorId(0));
        let id = ship_id();
        let entity = world.spawn_ship(id, Position::ORIGIN, Velocity::ZERO);
        // FittingComp と HullComp を追加
        world.inner_mut().insert(entity, (
            FittingComp::empty(),
            HullComp::new(ShipStatsComp::NPC.max_hp),
        )).unwrap();
        (world, id)
    }

    #[test]
    fn apply_fitting_with_no_modules_leaves_base_stats_unchanged() {
        let (mut world, id) = make_world_with_ship();
        let result = apply_fitting(&mut world, id, ShipStatsComp::NPC);
        let stats = result.unwrap();
        assert_eq!(stats.max_speed, ShipStatsComp::NPC.max_speed);
        assert_eq!(stats.weapon_damage, ShipStatsComp::NPC.weapon_damage);
    }

    #[test]
    fn apply_fitting_adds_weapon_module_damage_to_base_stats() {
        let (mut world, id) = make_world_with_ship();

        // 武器モジュールを装備
        let entity = world.inner().query::<&crate::components::ShipIdComp>().iter()
            .find(|(_, s)| s.0 == id).map(|(e, _)| e).unwrap();
        let weapon_mod = ModuleDefinition {
            id: ModuleId(1), name: "Railgun".to_string(),
            kind: ModuleKind::Weapon, slot: SlotKind::High,
            stat_delta: StatDelta { weapon_damage_add: 15.0, ..StatDelta::ZERO },
        };
        world.inner_mut().get::<&mut FittingComp>(entity).unwrap().high.push(weapon_mod);

        let stats = apply_fitting(&mut world, id, ShipStatsComp::NPC).unwrap();
        assert_eq!(stats.weapon_damage, ShipStatsComp::NPC.weapon_damage + 15.0);
    }

    #[test]
    fn apply_delta_clamps_weapon_cooldown_to_minimum_one_tick() {
        let base = ShipStatsComp::NPC;  // cooldown = 5
        let delta = StatDelta { weapon_cooldown_add: -100, ..StatDelta::ZERO };
        let result = apply_delta(base, &delta);
        assert_eq!(result.weapon_cooldown, 1);
    }

    #[test]
    fn apply_delta_does_not_allow_negative_speed_or_damage() {
        let base = ShipStatsComp::NPC;
        let delta = StatDelta {
            max_speed_add     : -9999.0,
            weapon_damage_add : -9999.0,
            ..StatDelta::ZERO
        };
        let result = apply_delta(base, &delta);
        assert_eq!(result.max_speed, 0.0);
        assert_eq!(result.weapon_damage, 0.0);
    }

    #[test]
    fn apply_fitting_returns_none_for_nonexistent_ship() {
        let mut world = SimWorld::new(SectorId(0));
        let fake_id = ShipId::new(NodeId(0), 999);
        assert!(apply_fitting(&mut world, fake_id, ShipStatsComp::NPC).is_none());
    }
}
