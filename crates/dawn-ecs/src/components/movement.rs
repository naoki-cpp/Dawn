//! Movement-related ECS components.

use dawn_core::{Position, Velocity};

#[derive(Debug, Clone, Copy)]
pub struct PositionComp(pub Position);

#[derive(Debug, Clone, Copy)]
pub struct VelocityComp(pub Velocity);

/// Acceleration vector applied to velocity every tick.
#[derive(Debug, Clone, Copy)]
pub struct ThrustComp(pub Velocity);

/// Runtime ship stats after applying all active module deltas.
///
/// Base values come from `ShipTypeDefinition.base_stats` at spawn time.
/// `apply_fitting()` overwrites these with base + Σ(active module StatDelta).
#[derive(Debug, Clone, Copy)]
pub struct ShipStatsComp {
    // ── Movement ──────────────────────────────────────────────────────────────
    pub max_speed            : f32,
    pub thrust_magnitude     : f32,

    // ── HP (3-layer) ──────────────────────────────────────────────────────────
    pub max_shield           : f32,
    pub max_armor            : f32,
    pub max_hull             : f32,

    // ── Combat ────────────────────────────────────────────────────────────────
    /// Weapon damage per shot (0 = no weapon; supplied by modules only).
    pub weapon_damage        : f32,
    pub weapon_range         : f32,
    pub weapon_cooldown      : u64,

    // ── Lock-on ───────────────────────────────────────────────────────────────
    pub lock_time            : u64,
    pub max_locks            : u32,

    // ── Capacitor ─────────────────────────────────────────────────────────────
    /// Maximum capacitor pool size (GJ).
    pub cap_max              : f32,
    /// Capacitor regenerated per tick (GJ/tick).
    pub cap_recharge_per_tick: f32,
}

impl ShipStatsComp {
    /// Fallback NPC default (tests and missing ship-type registry).
    /// Production code must use ShipTypeDefinition instead.
    pub const NPC: Self = Self {
        max_speed            : 400.0,
        thrust_magnitude     : 0.0,
        max_shield           : 200.0,
        max_armor            : 150.0,
        max_hull             : 150.0,
        weapon_damage        : 0.0,
        weapon_range         : 0.0,
        weapon_cooldown      : 1,
        lock_time            : 5,
        max_locks            : 1,
        cap_max              : 300.0,
        cap_recharge_per_tick: 6.0,   // 2 %/tick → full in 50 ticks (5 s)
    };

    /// Fallback player default (tests and missing ship-type registry).
    pub const PLAYER: Self = Self {
        max_speed            : 500.0,
        thrust_magnitude     : 40.0,
        max_shield           : 500.0,
        max_armor            : 300.0,
        max_hull             : 200.0,
        weapon_damage        : 0.0,
        weapon_range         : 0.0,
        weapon_cooldown      : 1,
        lock_time            : 3,
        max_locks            : 2,
        cap_max              : 500.0,
        cap_recharge_per_tick: 10.0,  // 2 %/tick → full in 50 ticks (5 s)
    };

    /// Build from `ShipBaseStats` (weapon stats start at zero).
    pub fn from_base(base: &dawn_core::ShipBaseStats) -> Self {
        Self {
            max_speed            : base.max_speed,
            thrust_magnitude     : base.thrust_magnitude,
            max_shield           : base.max_shield,
            max_armor            : base.max_armor,
            max_hull             : base.max_hull,
            weapon_damage        : 0.0,
            weapon_range         : 0.0,
            weapon_cooldown      : 1,
            lock_time            : base.lock_time,
            max_locks            : base.max_locks,
            cap_max              : base.cap_max,
            cap_recharge_per_tick: base.cap_recharge_per_tick,
        }
    }
}
