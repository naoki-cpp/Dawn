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

/// Configurable ship performance stats.
///
/// Future: overridden by equipment loadout.
/// Default values are set at spawn time via `SimulationNode::spawn_ship`.
#[derive(Debug, Clone, Copy)]
pub struct ShipStatsComp {
    /// Maximum speed magnitude (units/tick).  Velocity is clamped to this.
    pub max_speed: f32,
    /// Thrust magnitude (units/tick²).  The direction comes from `ThrustComp`.
    pub thrust_magnitude: f32,
}

impl ShipStatsComp {
    /// Default NPC ship stats — constant velocity, no thrust.
    pub const NPC: Self = Self { max_speed: 400.0, thrust_magnitude: 0.0 };

    /// Default player ship stats.
    ///
    /// Sector size = 10,000 units.  At 10 tick/sec:
    ///   max_speed=500  → Godot 50 u/tick  → crosses sector in 200 ticks (20 sec) — controllable
    ///   thrust=40      → reaches max_speed in ~13 ticks (1.3 sec) — clearly felt
    pub const PLAYER: Self = Self { max_speed: 500.0, thrust_magnitude: 40.0 };
}
