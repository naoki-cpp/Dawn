//! Spatial value types.

use serde::{Deserialize, Serialize};

// ── Position ─────────────────────────────────────────────────────────────────

/// 3-D coordinates of an entity in world space.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Position {
    pub const ORIGIN: Self = Self { x: 0.0, y: 0.0, z: 0.0 };

    pub fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub fn distance_squared(self, other: Self) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        let dz = self.z - other.z;
        dx * dx + dy * dy + dz * dz
    }

    pub fn distance(self, other: Self) -> f32 {
        self.distance_squared(other).sqrt()
    }
}

impl std::fmt::Display for Position {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({:.1}, {:.1}, {:.1})", self.x, self.y, self.z)
    }
}

// ── Velocity ─────────────────────────────────────────────────────────────────

/// Per-tick displacement vector.  Units are world-space units per tick.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Velocity {
    pub dx: f32,
    pub dy: f32,
    pub dz: f32,
}

impl Velocity {
    pub const ZERO: Self = Self { dx: 0.0, dy: 0.0, dz: 0.0 };

    pub fn new(dx: f32, dy: f32, dz: f32) -> Self {
        Self { dx, dy, dz }
    }

    /// Magnitude of the velocity vector.
    pub fn speed(self) -> f32 {
        (self.dx * self.dx + self.dy * self.dy + self.dz * self.dz).sqrt()
    }

    /// Return a velocity with all components negated on axes where the
    /// boundary was exceeded.  Used for wall-bounce in the movement system.
    pub fn reflect_x(self) -> Self { Self { dx: -self.dx, ..self } }
    pub fn reflect_y(self) -> Self { Self { dy: -self.dy, ..self } }
    pub fn reflect_z(self) -> Self { Self { dz: -self.dz, ..self } }
}

impl std::fmt::Display for Velocity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "v({:.2}, {:.2}, {:.2})", self.dx, self.dy, self.dz)
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_between_identical_positions_is_zero() {
        let p = Position::new(1.0, 2.0, 3.0);
        assert_eq!(p.distance(p), 0.0);
    }

    #[test]
    fn distance_satisfies_pythagorean_theorem_in_3d() {
        let a = Position::ORIGIN;
        let b = Position::new(3.0, 4.0, 0.0);
        assert!((b.distance(a) - 5.0).abs() < 1e-5);
    }

    #[test]
    fn velocity_reflect_x_negates_only_dx() {
        let v = Velocity::new(1.0, 2.0, 3.0);
        let r = v.reflect_x();
        assert_eq!(r.dx, -1.0);
        assert_eq!(r.dy, 2.0);
        assert_eq!(r.dz, 3.0);
    }
}
