//! Sector spatial boundaries.

use crate::position::{Position, Velocity};
use serde::{Deserialize, Serialize};

// ── SectorId ──────────────────────────────────────────────────────────────────

/// Identifies a Sector (spatial partition of the world).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SectorId(pub u8);

impl std::fmt::Display for SectorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Sector({})", self.0)
    }
}

// ── SectorBounds ─────────────────────────────────────────────────────────────

/// Axis-aligned bounding box that defines the volume of a Sector.
///
/// Ships must remain within their assigned Sector's bounds.
/// The movement system enforces this via wall-bounce (velocity reflection).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SectorBounds {
    pub min: Position,
    pub max: Position,
}

impl SectorBounds {
    /// A default cubic sector of side 10,000 units starting at the origin.
    pub const DEFAULT_SIZE: f32 = 10_000.0;

    pub fn new(min: Position, max: Position) -> Self {
        Self { min, max }
    }

    /// Create a cubic sector of `size` units with its origin at `(0,0,0)`.
    pub fn cube(size: f32) -> Self {
        Self {
            min: Position::ORIGIN,
            max: Position::new(size, size, size),
        }
    }

    pub fn contains(&self, pos: Position) -> bool {
        pos.x >= self.min.x && pos.x <= self.max.x
            && pos.y >= self.min.y && pos.y <= self.max.y
            && pos.z >= self.min.z && pos.z <= self.max.z
    }

    /// Clamp `pos` to the boundary and reflect components of `vel` on any
    /// axis that was exceeded.  Mutates both in place.
    ///
    /// This implements elastic wall-bounce: a ship that crosses a wall is
    /// placed exactly on the wall and its velocity on that axis is reversed.
    pub fn clamp_and_reflect(&self, pos: &mut Position, vel: &mut Velocity) {
        if pos.x < self.min.x {
            pos.x = self.min.x;
            vel.dx = vel.dx.abs();
        } else if pos.x > self.max.x {
            pos.x = self.max.x;
            vel.dx = -vel.dx.abs();
        }

        if pos.y < self.min.y {
            pos.y = self.min.y;
            vel.dy = vel.dy.abs();
        } else if pos.y > self.max.y {
            pos.y = self.max.y;
            vel.dy = -vel.dy.abs();
        }

        if pos.z < self.min.z {
            pos.z = self.min.z;
            vel.dz = vel.dz.abs();
        } else if pos.z > self.max.z {
            pos.z = self.max.z;
            vel.dz = -vel.dz.abs();
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn default_bounds() -> SectorBounds {
        SectorBounds::cube(SectorBounds::DEFAULT_SIZE)
    }

    #[test]
    fn position_at_origin_is_inside_default_sector() {
        assert!(default_bounds().contains(Position::ORIGIN));
    }

    #[test]
    fn position_beyond_max_is_outside_sector() {
        let p = Position::new(10_001.0, 0.0, 0.0);
        assert!(!default_bounds().contains(p));
    }

    #[test]
    fn clamp_and_reflect_reverses_dx_when_x_exceeds_max() {
        let bounds = default_bounds();
        let mut pos = Position::new(10_050.0, 500.0, 500.0);
        let mut vel = Velocity::new(5.0, 0.0, 0.0);

        bounds.clamp_and_reflect(&mut pos, &mut vel);

        assert_eq!(pos.x, bounds.max.x);
        assert!(vel.dx < 0.0, "velocity should be reflected inward");
    }

    #[test]
    fn clamp_and_reflect_reverses_dx_when_x_is_below_min() {
        let bounds = default_bounds();
        let mut pos = Position::new(-10.0, 500.0, 500.0);
        let mut vel = Velocity::new(-5.0, 0.0, 0.0);

        bounds.clamp_and_reflect(&mut pos, &mut vel);

        assert_eq!(pos.x, bounds.min.x);
        assert!(vel.dx > 0.0, "velocity should be reflected inward");
    }

    #[test]
    fn position_stays_unchanged_when_fully_inside_bounds() {
        let bounds = default_bounds();
        let mut pos = Position::new(500.0, 500.0, 500.0);
        let mut vel = Velocity::new(1.0, 1.0, 1.0);
        let original_pos = pos;
        let original_vel = vel;

        bounds.clamp_and_reflect(&mut pos, &mut vel);

        assert_eq!(pos, original_pos);
        assert_eq!(vel, original_vel);
    }
}
