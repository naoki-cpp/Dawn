//! Movement-related ECS components.

use dawn_core::{ApproachTarget, JumpGateId, Position, Velocity};

/// Persistent "approach" steering target (semi-automatic piloting, ADR-0015).
///
/// While a ship carries this component, the node's `process_approach()` step
/// re-aims `ThrustComp` at `target`'s latest position every tick (before the
/// Movement System integrates position), braking once the ship arrives.
/// The target is either another Ship (dynamic position) or a static Jump Gate.
///
/// Like `ThrustComp`, this is derived steering intent — it is NOT persisted in
/// `ShipSnapshot` and never produces its own event (the resulting velocity
/// change is recorded by `VelocityChanged`, ADR-0008).
#[derive(Debug, Clone, Copy)]
pub struct ApproachComp {
    pub target: ApproachTarget,
}

/// Two-phase intra-Sector warp state (short-range Fold, ADR-0022).
///
/// `Aligning` is the interruptible spin-up (the tackle window, ADR-0023);
/// `Warping` is committed — the node's `process_warp()` step controls the
/// ship's position/velocity at warp speed and the Movement System skips it.
/// Like `ApproachComp`, this is derived steering state: NOT persisted in
/// `ShipSnapshot` and never its own event (motion is recorded by
/// `VelocityChanged`, ADR-0008).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WarpPhase {
    /// Aligning: the ship points at the target and accelerates; warp engages
    /// once it is moving at ≥ 75% of max speed toward the gate (EVE-style
    /// alignment, ADR-0022). Align time therefore emerges from ship agility.
    /// Interruptible by Move/Stop and (ADR-0023) tackle.
    Aligning,
    /// Committed; flying to the gate at warp speed. Not interruptible.
    Warping,
}

#[derive(Debug, Clone, Copy)]
pub struct WarpComp {
    pub gate_id: JumpGateId,
    pub phase  : WarpPhase,
}

impl WarpComp {
    /// Whether the ship is in the committed warping phase (Movement skips it,
    /// Move/Stop/二重 warp are rejected).
    pub fn is_warping(&self) -> bool {
        matches!(self.phase, WarpPhase::Warping)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PositionComp(pub Position);

#[derive(Debug, Clone, Copy)]
pub struct VelocityComp(pub Velocity);

/// Acceleration vector applied to velocity every tick.
///
/// When `is_braking` is true, the direction stored in the inner `Velocity` is
/// ignored. The movement system instead applies thrust opposite to the current
/// velocity, decelerating the ship until it stops.
#[derive(Debug, Clone, Copy)]
pub struct ThrustComp {
    pub direction  : Velocity,
    pub is_braking : bool,
}

impl ThrustComp {
    /// No thrust, not braking.
    pub const ZERO: Self = Self { direction: Velocity::ZERO, is_braking: false };
}

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
    /// Weapon optimal range (units). Full hit chance within this distance.
    pub weapon_range         : f32,
    /// Turret tracking speed (rad/tick). Hit chance falls with high angular velocity.
    pub weapon_tracking      : f32,
    /// Weapon falloff range (units). Hit chance halves at optimal + falloff.
    pub weapon_falloff       : f32,
    pub weapon_cooldown      : u64,
    /// Signature radius. Larger = easier to track and hit.
    pub sig_radius           : f32,

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
        weapon_tracking      : 0.0,
        weapon_falloff       : 0.0,
        weapon_cooldown      : 1,
        sig_radius           : 40.0,
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
        weapon_tracking      : 0.0,
        weapon_falloff       : 0.0,
        weapon_cooldown      : 1,
        sig_radius           : 40.0,
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
            weapon_tracking      : 0.0,
            weapon_falloff       : 0.0,
            weapon_cooldown      : 1,
            sig_radius           : base.sig_radius,
            lock_time            : base.lock_time,
            max_locks            : base.max_locks,
            cap_max              : base.cap_max,
            cap_recharge_per_tick: base.cap_recharge_per_tick,
        }
    }
}
