//! Movement-related ECS components.

use dawn_core::{AnchorId, ApproachTarget, Position, Velocity, WarpTarget};

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
    pub target   : WarpTarget,
    pub phase    : WarpPhase,
    /// When true and `target` is `Gate`, automatically propose a Jump once warp
    /// completes and the ship arrives within the gate's activation radius.
    pub auto_jump: bool,
    /// Parametric warp plan (ADR-0022 amendment, 2026-06-21). Warp walks the
    /// segment from `warp_start` to the arrival point over `warp_total` ticks
    /// (smoothstep eased), reaching the destination exactly. These are
    /// meaningful only while `phase == Warping`; the `Aligning` phase leaves
    /// them at their engage-time defaults (start = ORIGIN, total/elapsed = 0).
    pub warp_start  : Position,
    pub warp_total  : u32,
    pub warp_elapsed: u32,
    /// Exact arrival point in absolute (Sector-frame) metres, f64 (ADR-0029).
    /// Set at engage for Body warps from the f64 anchor source so the arrival
    /// rebase is precise at true-AU distances (the f32 `PositionComp` near a
    /// 7.5e11 anchor would be ~65 km coarse). `[0,0,0]` for Gate warps (no rebase).
    pub warp_arrival_abs: [f64; 3],
}

impl WarpComp {
    /// Whether the ship is in the committed warping phase (Movement skips it,
    /// Move/Stop/二重 warp are rejected).
    pub fn is_warping(&self) -> bool {
        matches!(self.phase, WarpPhase::Warping)
    }
}

/// Ships currently tackling this ship (ADR-0024).
///
/// A ship is tackled while this component exists and `tacklers` is non-empty.
/// Tackled ships cannot warp or jump. Unlike `WarpComp`/`ApproachComp`, this
/// IS persisted in `ShipSnapshot` because losing tackle state on restart would
/// allow the tackled ship to escape.
#[derive(Debug, Clone)]
pub struct TackledComp {
    pub tacklers: Vec<dawn_core::ShipId>,
}

#[derive(Debug, Clone, Copy)]
pub struct PositionComp(pub Position);

/// The coordinate anchor a ship's [`PositionComp`] offset is relative to
/// (ADR-0029). The ship's absolute position is `anchor_abs(f64) + offset`,
/// where `anchor_abs` comes from the static `AnchorTable`. While all bodies
/// sit in the star-origin frame and ships anchor on the star (at the origin),
/// the offset equals the absolute position — so introducing this component is
/// a semantic no-op until anchors diverge at warp arrival (ADR-0029 §4 step 4).
#[derive(Debug, Clone, Copy)]
pub struct AnchorComp(pub AnchorId);

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

/// Runtime ship stats after applying all active module deltas (ADR-0023).
///
/// `max_speed`       = base_max_speed × Π(active speed_multipliers)
/// `mass`            = hull_mass + Σ(ALL fitted mass_add)  [passive — always]
/// τ (align time)    = mass × inertia_modifier / MASS_SCALE  [computed in MovementSystem]
#[derive(Debug, Clone, Copy)]
pub struct ShipStatsComp {
    // ── Movement (ADR-0023) ───────────────────────────────────────────────────
    /// Effective max speed (units/tick) after active propulsion modules.
    pub max_speed            : f32,
    /// Total mass including all fitted modules (kg). Always includes mass_add
    /// from all slots regardless of active/inactive state (passive behaviour).
    pub mass                 : f32,
    /// Inertia modifier (dimensionless). Controls align time; lower = more agile.
    pub inertia_modifier     : f32,

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

    // ── Tackle ────────────────────────────────────────────────────────────────
    /// Effective tackle range (units) after summing active Tackle module deltas.
    /// Zero means this ship has no tackle capability (ADR-0024).
    pub tackle_range         : f32,
}

impl ShipStatsComp {
    /// Fallback NPC default (tests and missing ship-type registry).
    /// Production code must use ShipTypeDefinition instead.
    pub const NPC: Self = Self {
        max_speed            : 400.0,
        mass                 : 12_000_000.0,
        inertia_modifier     : 0.3,
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
        cap_recharge_per_tick: 6.0,
        tackle_range         : 0.0,
    };

    /// Fallback player default (tests and missing ship-type registry).
    pub const PLAYER: Self = Self {
        max_speed            : 500.0,
        mass                 : 10_000_000.0,
        inertia_modifier     : 0.3,
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
        cap_recharge_per_tick: 10.0,
        tackle_range         : 0.0,
    };

    /// Build from `ShipBaseStats` (weapon stats start at zero).
    /// `apply_fitting()` will overwrite max_speed and mass after modules are applied.
    pub fn from_base(base: &dawn_core::ShipBaseStats) -> Self {
        Self {
            max_speed            : base.max_speed,
            mass                 : base.mass,
            inertia_modifier     : base.inertia_modifier,
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
            tackle_range         : 0.0,
        }
    }
}
