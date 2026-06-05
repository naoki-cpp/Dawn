//! Movement-related ECS components.

use dawn_core::{Position, Velocity};

#[derive(Debug, Clone, Copy)]
pub struct PositionComp(pub Position);

#[derive(Debug, Clone, Copy)]
pub struct VelocityComp(pub Velocity);

/// Acceleration vector applied to velocity every tick.
#[derive(Debug, Clone, Copy)]
pub struct ThrustComp(pub Velocity);

/// Configurable ship performance and combat stats.
///
/// Base values come from `ShipTypeDefinition.base_stats` at spawn time.
/// `apply_fitting()` overwrites these with base + Σ(active module StatDelta).
#[derive(Debug, Clone, Copy)]
pub struct ShipStatsComp {
    // ── Movement ──────────────────────────────────────────────────────────────
    pub max_speed        : f32,
    pub thrust_magnitude : f32,

    // ── HP（3層）─────────────────────────────────────────────────────────────
    pub max_shield       : f32,
    pub max_armor        : f32,
    pub max_hull         : f32,

    // ── Combat ────────────────────────────────────────────────────────────────
    /// 武器ダメージ（0 = 武器なし。モジュールのみで供給）
    pub weapon_damage    : f32,
    pub weapon_range     : f32,
    pub weapon_cooldown  : u64,

    // ── Lock-on ───────────────────────────────────────────────────────────────
    pub lock_time        : u64,
    pub max_locks        : u32,
}

impl ShipStatsComp {
    /// テスト・フォールバック用 NPC デフォルト。
    /// 本番コードは ShipTypeDefinition を使うこと。
    pub const NPC: Self = Self {
        max_speed        : 400.0,
        thrust_magnitude : 0.0,
        max_shield       : 200.0,
        max_armor        : 150.0,
        max_hull         : 150.0,
        weapon_damage    : 0.0,
        weapon_range     : 0.0,
        weapon_cooldown  : 1,
        lock_time        : 5,
        max_locks        : 1,
    };

    /// テスト・フォールバック用プレイヤーデフォルト。
    pub const PLAYER: Self = Self {
        max_speed        : 500.0,
        thrust_magnitude : 40.0,
        max_shield       : 500.0,
        max_armor        : 300.0,
        max_hull         : 200.0,
        weapon_damage    : 0.0,
        weapon_range     : 0.0,
        weapon_cooldown  : 1,
        lock_time        : 3,
        max_locks        : 2,
    };

    /// `ShipBaseStats` から `ShipStatsComp` を生成する（武器スタットはゼロ）。
    pub fn from_base(base: &dawn_core::ShipBaseStats) -> Self {
        Self {
            max_speed        : base.max_speed,
            thrust_magnitude : base.thrust_magnitude,
            max_shield       : base.max_shield,
            max_armor        : base.max_armor,
            max_hull         : base.max_hull,
            weapon_damage    : 0.0,
            weapon_range     : 0.0,
            weapon_cooldown  : 1,
            lock_time        : base.lock_time,
            max_locks        : base.max_locks,
        }
    }
}
