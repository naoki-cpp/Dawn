//! Spatial value types.

use serde::{Deserialize, Serialize};

// ── AbsolutePosition --------------------------------------------------------

/// A sector-frame absolute coordinate in metres.
///
/// This is distinct from [`Position`], which is the current anchor-relative
/// f64 simulation offset (ADR-0044).
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AbsolutePosition(pub [f64; 3]);

impl AbsolutePosition {
    pub const ORIGIN: Self = Self([0.0, 0.0, 0.0]);

    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self([x, y, z])
    }

    pub const fn as_array(self) -> [f64; 3] {
        self.0
    }

    pub fn distance_squared(self, other: Self) -> f64 {
        let dx = self.0[0] - other.0[0];
        let dy = self.0[1] - other.0[1];
        let dz = self.0[2] - other.0[2];
        dx * dx + dy * dy + dz * dz
    }

    pub fn distance(self, other: Self) -> f64 {
        self.distance_squared(other).sqrt()
    }
}

impl From<[f64; 3]> for AbsolutePosition {
    fn from(value: [f64; 3]) -> Self {
        Self(value)
    }
}

impl From<Position> for AbsolutePosition {
    fn from(value: Position) -> Self {
        Self::new(value.x, value.y, value.z)
    }
}

impl From<AbsolutePosition> for [f64; 3] {
    fn from(value: AbsolutePosition) -> Self {
        value.0
    }
}

impl std::ops::Index<usize> for AbsolutePosition {
    type Output = f64;

    fn index(&self, index: usize) -> &Self::Output {
        &self.0[index]
    }
}

// ── Position ─────────────────────────────────────────────────────────────────

/// 3-D coordinates of an entity in its current simulation frame.
///
/// Components are `f64` so anchor-relative motion does not reintroduce the
/// precision loss that absolute coordinates were moved to `AbsolutePosition`
/// to avoid (ADR-0044).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Position {
    pub const ORIGIN: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn distance_squared(self, other: Self) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        let dz = self.z - other.z;
        dx * dx + dy * dy + dz * dz
    }

    pub fn distance(self, other: Self) -> f64 {
        self.distance_squared(other).sqrt()
    }
}

impl std::fmt::Display for Position {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({:.1}, {:.1}, {:.1})", self.x, self.y, self.z)
    }
}

// ── Velocity ─────────────────────────────────────────────────────────────────

/// Per-tick displacement vector. Units are world-space units per tick.
///
/// Movement stays in `f64` through integration and is narrowed only at an
/// engine rendering boundary (ADR-0044/ADR-0045).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Velocity {
    pub dx: f64,
    pub dy: f64,
    pub dz: f64,
}

impl Velocity {
    pub const ZERO: Self = Self {
        dx: 0.0,
        dy: 0.0,
        dz: 0.0,
    };

    pub const fn new(dx: f64, dy: f64, dz: f64) -> Self {
        Self { dx, dy, dz }
    }

    /// Magnitude of the velocity vector.
    pub fn speed(self) -> f64 {
        (self.dx * self.dx + self.dy * self.dy + self.dz * self.dz).sqrt()
    }

    /// Return a velocity with all components negated on axes where the
    /// boundary was exceeded.  Used for wall-bounce in the movement system.
    pub fn reflect_x(self) -> Self {
        Self {
            dx: -self.dx,
            ..self
        }
    }
    pub fn reflect_y(self) -> Self {
        Self {
            dy: -self.dy,
            ..self
        }
    }
    pub fn reflect_z(self) -> Self {
        Self {
            dz: -self.dz,
            ..self
        }
    }
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
    fn absolute_position_distance_uses_f64_coordinates() {
        let a = AbsolutePosition::new(1.0e11, 0.0, 0.0);
        let b = AbsolutePosition::new(1.0e11 + 3.0, 4.0, 0.0);

        assert!((a.distance(b) - 5.0).abs() < 1e-9);
    }

    #[test]
    fn absolute_position_round_trips_through_array_representation() {
        let position = AbsolutePosition::new(1.0, 2.0, 3.0);

        assert_eq!(position.as_array(), [1.0, 2.0, 3.0]);
        assert_eq!(AbsolutePosition::from(position.as_array()), position);
    }

    #[test]
    fn distance_between_identical_positions_is_zero() {
        let p = Position::new(1.0, 2.0, 3.0);
        assert_eq!(p.distance(p), 0.0);
    }

    #[test]
    fn distance_satisfies_pythagorean_theorem_in_3d() {
        let a = Position::ORIGIN;
        let b = Position::new(3.0, 4.0, 0.0);
        assert!((b.distance(a) - 5.0).abs() < 1e-12);
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
