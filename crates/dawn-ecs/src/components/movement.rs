//! Movement-related ECS components.

use dawn_core::{Position, Velocity};

/// Current world-space position of a Ship.
#[derive(Debug, Clone, Copy)]
pub struct PositionComp(pub Position);

/// Per-tick displacement vector.
/// The movement system adds `ThrustComp` to this each tick,
/// then clamps to `ShipStatsComp::max_speed`, then applies to `PositionComp`.
#[derive(Debug, Clone, Copy)]
pub struct VelocityComp(pub Velocity);

/// Acceleration vector applied to velocity every tick.
///
/// Set by `MoveCommand` (double-click in Godot).
/// Once set, the ship keeps accelerating in this direction until a new
/// command changes it.  Set to `Velocity::ZERO` to stop thrusting.
#[derive(Debug, Clone, Copy)]
pub struct ThrustComp(pub Velocity);

/// Configurable ship performance and combat stats.
///
/// Base values are set at spawn time.
/// When a Fitting is applied, `apply_fitting()` overwrites these with
/// base + sum(module StatDelta).  Movement and Combat systems read from here.
#[derive(Debug, Clone, Copy)]
pub struct ShipStatsComp {
    // ── Movement ──────────────────────────────────────────────────────────────
    /// Maximum speed magnitude (units/tick).  Velocity is clamped to this.
    pub max_speed        : f32,
    /// Thrust magnitude (units/tick²).  The direction comes from `ThrustComp`.
    pub thrust_magnitude : f32,

    // ── Combat ────────────────────────────────────────────────────────────────
    /// Maximum HP (shield + armor + hull combined).
    pub max_hp           : f32,
    /// Damage dealt per weapon shot.
    pub weapon_damage    : f32,
    /// Effective weapon range (units).  Target must be within this distance.
    pub weapon_range     : f32,
    /// Weapon reload time in ticks.  Ship cannot fire again until this many
    /// ticks have elapsed since `last_fired_tick`.
    pub weapon_cooldown  : u64,

    // ── Lock-on ───────────────────────────────────────────────────────────────
    /// Ticks required to complete a lock.  Reduced by sensor modules.
    pub lock_time        : u64,
    /// Maximum number of simultaneously locked targets.
    pub max_locks        : u32,
}

impl ShipStatsComp {
    /// Default NPC ship stats — constant velocity, no thrust, **no weapon**.
    ///
    /// weapon_damage = 0.0 means the ship cannot attack.
    /// Weapons are added exclusively via Fitting modules (StatDelta).
    pub const NPC: Self = Self {
        max_speed        : 400.0,
        thrust_magnitude : 0.0,
        max_hp           : 500.0,
        weapon_damage    : 0.0,   // ← no weapon until a module is fitted
        weapon_range     : 0.0,
        weapon_cooldown  : 1,
        lock_time        : 5,     // 5 ticks = 0.5 sec at 10 tick/sec
        max_locks        : 1,
    };

    /// Default player ship stats.
    ///
    /// Sector size = 10,000 units.  At 10 tick/sec:
    ///   max_speed=500  → crosses sector in 200 ticks (20 sec) — controllable
    ///   thrust=40      → reaches max_speed in ~13 ticks (1.3 sec) — clearly felt
    ///
    /// **No weapon** until a weapon module is fitted via Fitting system.
    pub const PLAYER: Self = Self {
        max_speed        : 500.0,
        thrust_magnitude : 40.0,
        max_hp           : 1_000.0,
        weapon_damage    : 0.0,   // ← no weapon until a module is fitted
        weapon_range     : 0.0,
        weapon_cooldown  : 1,
        lock_time        : 3,     // player は少し速くロックできる
        max_locks        : 2,
    };
}
