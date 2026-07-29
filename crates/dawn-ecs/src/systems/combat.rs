//! Combat system — ロック済みターゲットへの武器発射 / ダメージ / 破壊。
//!
//! # 処理順序（CLAUDE.md §6 — Lock System の後に呼ぶこと）
//!
//! 各 Tick で次を行う:
//!   1. LockComp で Locked 状態のターゲットを確認する
//!   2. fire_triggers にある Ship のみ発射判定を行う（CapacitorSystem 連動）
//!   3. 命中率を計算する（EVE Online トラッキング式）
//!      hit_chance = 0.5^( (angular/(tracking×sig))² + (max(0,d−opt)/falloff)² )
//!   4. 命中した場合のみダメージを適用し DamageTaken を生成する
//!      ダメージ倍率: ランダム（通常 50-149%、1% 確率で 300% ラッキーショット）
//!   5. HP ≤ 0 なら ShipDestroyed を生成し `destroyed` リストに積む
//!      （ECS からの削除は呼び出し元が行う）
//!
//! # Contract
//!
//! - 純粋計算: I/O なし、グローバル状態なし。
//! - Lock System が先に実行されていること（LockComp が最新状態であること）。
//! - 返り値の events を EventStore に Append するのは呼び出し元の責務。
//! - `destroyed` に含まれる ShipId は呼び出し元が ECS から削除すること。

use crate::{
    components::{
        AnchorComp, HullComp, LockComp, PositionComp, ShipIdComp, ShipStatsComp, TransitComp,
        VelocityComp,
    },
    SimWorld,
};
use dawn_core::{
    events::{DamageTaken, ShipDestroyed, WeaponFired},
    AbsolutePosition, AnchorId, DomainEvent, Position, ShipId, Tick, Velocity,
};
use rand::Rng;
use std::collections::HashMap;

// Internal snapshot type. `entity` is captured so write-back is O(1) per ship
// instead of a full-world scan (ADR-0019: server compute stays O(n)).
struct ShipSnapshot {
    entity: hecs::Entity,
    ship_id: ShipId,
    stats: ShipStatsComp,
    /// Absolute (Sector-frame) position in metres, kept as f64 so the pairwise
    /// difference between nearby ships stays precise even when anchors sit at
    /// true astronomical distances (ADR-0029). Hit-chance geometry is computed
    /// in f64 before combining it with the existing f32 combat coefficients.
    abs: AbsolutePosition,
    velocity: Velocity,
    current_shield: f32,
    current_armor: f32,
    current_hull: f32,
    is_dead: bool,
    locked_targets: Vec<ShipId>,
}

/// Combat System の実行結果。
#[derive(Debug)]
pub struct CombatResult {
    /// 今 Tick で生成されたイベント（WeaponFired / DamageTaken / ShipDestroyed）。
    pub events: Vec<DomainEvent>,
    /// 破壊された Ship の ID リスト。呼び出し元が ECS から削除する。
    pub destroyed: Vec<ShipId>,
}

/// A ship's absolute position (Sector-frame, metres, f64) = its anchor's
/// absolute position + its offset (ADR-0029). Falls back to the raw offset if
/// the anchor is unknown (tests / pre-anchor data).
fn absolute_position(
    offset: Position,
    anchor: AnchorId,
    anchor_abs: &HashMap<AnchorId, AbsolutePosition>,
) -> AbsolutePosition {
    match anchor_abs.get(&anchor) {
        Some(a) => AbsolutePosition::new(a[0] + offset.x, a[1] + offset.y, a[2] + offset.z),
        None => {
            // ADR-0029 R3: an anchor missing from a *populated* table is a data
            // bug — at true AU it silently misplaces the ship by the body's
            // absolute position. Tests pass an empty map on purpose (offset == the
            // absolute), so only assert when the caller actually provided a table.
            debug_assert!(
                anchor_abs.is_empty(),
                "combat: ship anchored on {anchor:?} absent from a populated anchor table \
                 — distance fell back to the raw offset (wrong frame at true AU)"
            );
            AbsolutePosition::new(offset.x, offset.y, offset.z)
        }
    }
}

/// 1 Tick 分の Combat を実行する。
///
/// Lock System の後に呼ぶこと（LockComp が最新状態であること）。
/// `fire_triggers` は CapacitorSystem から渡される「この Tick に武器サイクルが
/// 開始した Ship の ID リスト」。リストにある Ship のみ発射する。
pub fn run(
    world: &mut SimWorld,
    tick: Tick,
    fire_triggers: &[ShipId],
    anchor_abs: &HashMap<AnchorId, AbsolutePosition>,
) -> CombatResult {
    // ── 1. 全 Ship をスナップショット ────────────────────────────────────────
    //
    // ADR-0029: distance/tracking depend on relative geometry, which is
    // frame-invariant only if both ships are expressed in the same frame. Once
    // ships can anchor on different bodies, resolve each ship's ABSOLUTE position
    // (anchor_abs + offset) so pairwise differences are correct. At compressed
    // Keep the absolute positions in f64 so pairwise geometry is precise at
    // true-AU distances.

    let mut ships: Vec<ShipSnapshot> = world
        .query::<(
            hecs::Entity,
            &ShipIdComp,
            &ShipStatsComp,
            &PositionComp,
            &VelocityComp,
            &HullComp,
            &LockComp,
            &AnchorComp,
            Option<&TransitComp>,
        )>()
        .iter()
        // A Ship pending Sector Transit is excluded from Combat entirely
        // (ADR-0014, issue #204): it can neither fire (its own weapon cycles
        // don't matter -- it isn't in `ships` to be looked up as an attacker)
        // nor be damaged (nothing here can resolve a lock onto it as a
        // target, since it never enters the snapshot list locks are matched
        // against). Frozen the same way in `MovementSystem`.
        .filter(|(_, _, _, _, _, _, _, _, transit)| !transit.is_some_and(|t| t.0.is_in_transit()))
        .map(
            |(entity, id, stats, pos, vel, hull, lock, anchor, _transit)| ShipSnapshot {
                entity,
                ship_id: id.0,
                stats: *stats,
                abs: absolute_position(pos.0, anchor.0, anchor_abs),
                velocity: vel.0,
                current_shield: hull.shield(),
                current_armor: hull.armor(),
                current_hull: hull.hull(),
                is_dead: hull.is_destroyed(),
                locked_targets: lock.locked_targets().collect(),
            },
        )
        .collect();

    // ── 2. ロック済みターゲットへ発射・命中判定 ──────────────────────────────

    let mut rng = rand::thread_rng();
    let mut events: Vec<DomainEvent> = Vec::new();
    let mut destroyed: Vec<ShipId> = Vec::new();
    let mut damage_accum: Vec<(usize, f32, ShipId)> = Vec::new();

    let n = ships.len();
    for i in 0..n {
        if ships[i].is_dead {
            continue;
        }
        if ships[i].stats.weapon_damage <= 0.0 {
            continue;
        }
        // Fire only when the capacitor cycle started this tick.
        if !fire_triggers.contains(&ships[i].ship_id) {
            continue;
        }

        // Locked ターゲットの中から最初の生存 Ship を選択
        let target_id = ships[i]
            .locked_targets
            .iter()
            .find(|&&tid| ships.iter().any(|s| s.ship_id == tid && !s.is_dead))
            .copied();
        let Some(tid) = target_id else { continue };
        let Some(j) = ships.iter().position(|s| s.ship_id == tid) else {
            continue;
        };

        // ── 命中率計算（EVE Online トラッキング式）─────────────────────────
        let hit_chance = calc_hit_chance(&ships[i], &ships[j]);

        // 命中チェック
        let roll: f32 = rng.gen();
        if roll > hit_chance {
            // ミス：WeaponFired は発行しない
            continue;
        }

        // ダメージ倍率（EVE 準拠: 通常 50-149%、1% で 300% ラッキーショット）
        let multiplier = damage_multiplier(&mut rng);
        let damage = (ships[i].stats.weapon_damage * multiplier).max(0.0);

        events.push(DomainEvent::WeaponFired(WeaponFired {
            attacker_id: ships[i].ship_id,
            target_id: tid,
            damage,
            tick,
        }));
        damage_accum.push((j, damage, ships[i].ship_id));
    }

    // ── 3. Apply damage ──────────────────────────────────────────────────────

    // Indices of ships whose HP/destroyed flag changed this tick (write-back set).
    let mut changed: Vec<usize> = Vec::new();

    for (j, damage, attacker_id) in damage_accum {
        changed.push(j);
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
            ship_id: ships[j].ship_id,
            damage,
            current_shield: ships[j].current_shield,
            current_armor: ships[j].current_armor,
            current_hull: ships[j].current_hull,
            tick,
        }));

        if ships[j].current_hull <= 0.0 && !ships[j].is_dead {
            ships[j].is_dead = true;
            events.push(DomainEvent::ShipDestroyed(ShipDestroyed {
                ship_id: ships[j].ship_id,
                killer_id: attacker_id,
                tick,
            }));
            destroyed.push(ships[j].ship_id);
        }
    }

    // ── 4. Write back to ECS (only ships that took damage this tick) ─────────

    changed.sort_unstable();
    changed.dedup();
    for j in changed {
        let s = &ships[j];
        if let Some(mut hull) = world.get_mut::<HullComp>(s.entity) {
            hull.set_hp(s.current_shield, s.current_armor, s.current_hull);
        }
    }

    CombatResult { events, destroyed }
}

// ── ヘルパー ──────────────────────────────────────────────────────────────────

/// EVE Online 準拠の命中率計算。
///
/// ```text
/// hit_chance = 0.5 ^ ( (angular / (tracking * sig))^2 + (max(0, d-opt) / falloff)^2 )
/// ```
///
/// - ω (angular velocity) = transversal_velocity / distance
/// - tracking = turret tracking speed (rad/tick)
/// - sig = target signature radius
/// - d = distance to target
/// - opt = weapon optimal range
/// - falloff = weapon falloff range (hit chance halves at opt+falloff)
fn calc_hit_chance(attacker: &ShipSnapshot, target: &ShipSnapshot) -> f32 {
    // Keep spatial separation and relative velocity in f64. Combat tuning
    // values remain f32, so only the final probability crosses that boundary.
    let dx = target.abs[0] - attacker.abs[0];
    let dy = target.abs[1] - attacker.abs[1];
    let dz = target.abs[2] - attacker.abs[2];
    let dist = (dx * dx + dy * dy + dz * dz).sqrt();

    // Avoid division by zero when ships overlap
    if dist < f64::EPSILON {
        return 1.0;
    }

    // ── Tracking term ─────────────────────────────────────────────────────────
    let tracking_term = if attacker.stats.weapon_tracking > f32::EPSILON
        && target.stats.sig_radius > f32::EPSILON
    {
        // Relative velocity of target w.r.t. attacker
        let rvx = target.velocity.dx - attacker.velocity.dx;
        let rvy = target.velocity.dy - attacker.velocity.dy;
        let rvz = target.velocity.dz - attacker.velocity.dz;

        // Radial component (along attacker→target)
        let radial = (rvx * dx + rvy * dy + rvz * dz) / dist;

        // Transversal speed = sqrt(|rel_vel|² − radial²)
        let rel_speed_sq = rvx * rvx + rvy * rvy + rvz * rvz;
        let transversal = (rel_speed_sq - radial * radial).max(0.0).sqrt();

        // Angular velocity (rad/tick)
        let angular = transversal / dist;

        angular / (f64::from(attacker.stats.weapon_tracking) * f64::from(target.stats.sig_radius))
    } else {
        0.0
    };

    // ── Range term ────────────────────────────────────────────────────────────
    let range_term = if attacker.stats.weapon_falloff > f32::EPSILON {
        let excess = (dist - f64::from(attacker.stats.weapon_range)).max(0.0);
        excess / f64::from(attacker.stats.weapon_falloff)
    } else if dist > f64::from(attacker.stats.weapon_range) {
        // No falloff defined: binary drop-off beyond optimal
        f64::INFINITY
    } else {
        0.0
    };

    // ── Combined hit chance ───────────────────────────────────────────────────
    let exponent = tracking_term * tracking_term + range_term * range_term;
    (0.5f64).powf(exponent) as f32
}

/// EVE Online 準拠のダメージ倍率。
///
/// - x < 0.01 (1%): ラッキーショット → 3.0×
/// - それ以外: x + 0.49 ∈ [0.49, 1.49) ≈ 50-149%
fn damage_multiplier(rng: &mut impl Rng) -> f32 {
    let x: f32 = rng.gen();
    if x < 0.01 {
        3.0
    } else {
        x + 0.49
    }
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

    fn ship_id(n: u64) -> ShipId {
        ShipId::new(NodeId(0), n)
    }

    fn armed_stats(damage: f32) -> ShipStatsComp {
        ShipStatsComp {
            weapon_damage: damage,
            weapon_range: 5_000.0,
            ..ShipStatsComp::NPC
        }
    }

    /// ship_id(1) に対して ship_id(2) を Locked 状態にセットアップする
    fn setup_with_lock(damage: f32) -> SimWorld {
        let mut world = SimWorld::new(SectorId(0));
        let ea = world.spawn_ship(ship_id(1), Position::ORIGIN, Velocity::ZERO);
        world.spawn_ship(ship_id(2), Position::new(100.0, 0.0, 0.0), Velocity::ZERO);
        world.set_ship_stats(ea, armed_stats(damage));
        // Set ship_id(1) lock to Locked state.
        if let Some(mut lock) = world.get_mut::<LockComp>(ea) {
            lock.entries.push(LockEntry {
                target_id: ship_id(2),
                state: LockState::Locked,
            });
        }
        world
    }

    #[test]
    fn weapon_fires_when_target_is_locked_and_in_fire_triggers() {
        let mut world = setup_with_lock(25.0);
        let triggers = [ship_id(1)];
        let result = run(&mut world, Tick(1), &triggers, &HashMap::new());
        let fired = result
            .events
            .iter()
            .filter(|e| matches!(e, DomainEvent::WeaponFired(_)))
            .count();
        assert_eq!(fired, 1);
    }

    #[test]
    fn no_fire_without_lock() {
        let mut world = SimWorld::new(SectorId(0));
        let ea = world.spawn_ship(ship_id(1), Position::ORIGIN, Velocity::ZERO);
        world.spawn_ship(ship_id(2), Position::new(100.0, 0.0, 0.0), Velocity::ZERO);
        world.set_ship_stats(ea, armed_stats(25.0));
        // LockComp starts empty.
        let triggers = [ship_id(1)];
        let result = run(&mut world, Tick(1), &triggers, &HashMap::new());
        assert!(result.events.is_empty(), "no fire without lock");
    }

    #[test]
    fn ship_destroyed_when_hp_reaches_zero() {
        let mut world = setup_with_lock(99999.0);
        let triggers = [ship_id(1)];
        let result = run(&mut world, Tick(1), &triggers, &HashMap::new());
        let destroyed = result
            .events
            .iter()
            .filter(|e| matches!(e, DomainEvent::ShipDestroyed(_)))
            .count();
        assert_eq!(destroyed, 1);
        assert_eq!(result.destroyed.len(), 1);
    }

    #[test]
    fn weapon_does_not_fire_when_not_in_fire_triggers() {
        let mut world = setup_with_lock(1.0);
        // ship_id(1) is NOT in fire_triggers this tick
        let result = run(&mut world, Tick(1), &[], &HashMap::new());
        let fired = result
            .events
            .iter()
            .filter(|e| matches!(e, DomainEvent::WeaponFired(_)))
            .count();
        assert_eq!(fired, 0, "no fire when ship is absent from fire_triggers");
    }

    #[test]
    fn weapon_fires_only_when_in_fire_triggers() {
        let mut world = setup_with_lock(1.0);
        // Tick 1: not triggered
        let r1 = run(&mut world, Tick(1), &[], &HashMap::new());
        assert_eq!(
            r1.events
                .iter()
                .filter(|e| matches!(e, DomainEvent::WeaponFired(_)))
                .count(),
            0
        );
        // Tick 2: triggered (stationary target within optimal → hit_chance = 1.0)
        let r2 = run(&mut world, Tick(2), &[ship_id(1)], &HashMap::new());
        assert_eq!(
            r2.events
                .iter()
                .filter(|e| matches!(e, DomainEvent::WeaponFired(_)))
                .count(),
            1,
            "fire_triggersにあれば発射"
        );
    }

    // ── Tracking formula unit tests ───────────────────────────────────────────

    fn make_snap(id: ShipId, pos: Position, vel: Velocity, stats: ShipStatsComp) -> ShipSnapshot {
        ShipSnapshot {
            entity: hecs::Entity::DANGLING,
            ship_id: id,
            stats,
            abs: AbsolutePosition::new(pos.x, pos.y, pos.z),
            velocity: vel,
            current_shield: 100.0,
            current_armor: 100.0,
            current_hull: 100.0,
            is_dead: false,
            locked_targets: vec![],
        }
    }

    fn tracking_stats(tracking: f32, optimal: f32, falloff: f32) -> ShipStatsComp {
        ShipStatsComp {
            weapon_damage: 25.0,
            weapon_range: optimal,
            weapon_tracking: tracking,
            weapon_falloff: falloff,
            sig_radius: 40.0,
            ..ShipStatsComp::NPC
        }
    }

    #[test]
    fn stationary_target_within_optimal_always_hits() {
        let atk = make_snap(
            ship_id(1),
            Position::ORIGIN,
            Velocity::ZERO,
            tracking_stats(0.05, 5000.0, 2000.0),
        );
        let tgt = make_snap(
            ship_id(2),
            Position::new(1000.0, 0.0, 0.0),
            Velocity::ZERO,
            ShipStatsComp {
                sig_radius: 40.0,
                ..ShipStatsComp::NPC
            },
        );
        let h = calc_hit_chance(&atk, &tgt);
        assert!(
            (h - 1.0).abs() < 1e-5,
            "stationary target at optimal: hit_chance must be 1.0, got {h}"
        );
    }

    #[test]
    fn hit_chance_is_half_at_optimal_plus_falloff() {
        // dist = optimal + falloff → range_term = 1.0 → hit_chance = 0.5^1 = 0.5
        let atk = make_snap(
            ship_id(1),
            Position::ORIGIN,
            Velocity::ZERO,
            tracking_stats(0.05, 1000.0, 1000.0),
        );
        let tgt = make_snap(
            ship_id(2),
            Position::new(2000.0, 0.0, 0.0),
            Velocity::ZERO,
            ShipStatsComp {
                sig_radius: 40.0,
                ..ShipStatsComp::NPC
            },
        );
        let h = calc_hit_chance(&atk, &tgt);
        assert!(
            (h - 0.5).abs() < 1e-4,
            "at optimal+falloff hit_chance must be 0.5, got {h}"
        );
    }

    #[test]
    fn hit_chance_is_half_when_angular_equals_tracking_times_sig() {
        // tracking_term = 1.0 → hit_chance = 0.5^1 = 0.5
        // angular = transversal / dist
        // transversal = tracking × sig × dist / dist = tracking × sig
        // → target moving at tracking × sig perpendicular at dist=1000
        let tracking = 0.05_f32;
        let sig = 40.0_f32;
        let dist = 1000.0_f64;
        // angular = transversal / dist; choose a transversal velocity that
        // makes angular equal to tracking × signature radius.
        let perp_speed = f64::from(tracking) * f64::from(sig) * dist;

        let atk = make_snap(
            ship_id(1),
            Position::ORIGIN,
            Velocity::ZERO,
            tracking_stats(tracking, 5000.0, 5000.0),
        );
        let tgt = make_snap(
            ship_id(2),
            Position::new(dist, 0.0, 0.0),
            Velocity::new(0.0, perp_speed, 0.0), // pure transversal
            ShipStatsComp {
                sig_radius: sig,
                ..ShipStatsComp::NPC
            },
        );
        let h = calc_hit_chance(&atk, &tgt);
        assert!(
            (h - 0.5).abs() < 1e-4,
            "angular = tracking×sig → hit_chance must be 0.5, got {h}"
        );
    }

    #[test]
    fn radially_approaching_target_has_full_hit_chance() {
        // Target moving directly toward attacker: transversal = 0 → angular = 0
        let atk = make_snap(
            ship_id(1),
            Position::ORIGIN,
            Velocity::ZERO,
            tracking_stats(0.001, 5000.0, 5000.0),
        ); // very slow tracking
        let tgt = make_snap(
            ship_id(2),
            Position::new(2000.0, 0.0, 0.0),
            Velocity::new(-500.0, 0.0, 0.0), // heading straight at attacker
            ShipStatsComp {
                sig_radius: 40.0,
                ..ShipStatsComp::NPC
            },
        );
        let h = calc_hit_chance(&atk, &tgt);
        assert!(
            (h - 1.0).abs() < 1e-4,
            "radial approach → zero angular → hit_chance must be 1.0, got {h}"
        );
    }
}
