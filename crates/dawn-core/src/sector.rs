//! Sector spatial boundaries.

use crate::position::Position;
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
/// Space is unbounded since Phase 4 Cycle 2 (walls were removed); these
/// bounds are used only as the spawn-placement range for new ships.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SectorBounds {
    pub min: Position,
    pub max: Position,
}

impl SectorBounds {
    /// Half-extent of the default sector in each axis (units).
    /// The sector spans `[-DEFAULT_HALF, +DEFAULT_HALF]` on all three axes,
    /// giving a total side length of 2 * DEFAULT_HALF = 1,400,000 units. Sized
    /// to contain a "wide" star system (planets at ~10^5 units, gates at the
    /// edge ~600,000) so bodies sit far enough that their bearing barely shifts
    /// during sublight travel — which reads as distance (f32-safe; see ADR-0028:
    /// f32 to ~10^7 units, i64 only for true AU). Bounds are soft (spawn-
    /// placement range only; space is unbounded since Phase 4).
    pub const DEFAULT_HALF: f32 = 700_000.0;

    pub fn new(min: Position, max: Position) -> Self {
        Self { min, max }
    }

    /// Create a cubic sector centred on `(0,0,0)` with half-extent `half`,
    /// so that the spawn origin (0,0,0) sits in the middle of the playfield.
    pub fn centered(half: f32) -> Self {
        Self {
            min: Position::new(-half, -half, -half),
            max: Position::new( half,  half,  half),
        }
    }

    pub fn contains(&self, pos: Position) -> bool {
        pos.x >= self.min.x && pos.x <= self.max.x
            && pos.y >= self.min.y && pos.y <= self.max.y
            && pos.z >= self.min.z && pos.z <= self.max.z
    }

}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn default_bounds() -> SectorBounds {
        SectorBounds::centered(SectorBounds::DEFAULT_HALF)
    }

    #[test]
    fn position_at_origin_is_inside_default_sector() {
        assert!(default_bounds().contains(Position::ORIGIN));
    }

    #[test]
    fn position_beyond_max_is_outside_sector() {
        // DEFAULT_HALF = 50_000; anything beyond that is outside.
        let p = Position::new(SectorBounds::DEFAULT_HALF + 1.0, 0.0, 0.0);
        assert!(!default_bounds().contains(p));
    }

    #[test]
    fn centered_bounds_are_symmetric_around_origin() {
        let b = SectorBounds::centered(100.0);
        assert_eq!(b.min, Position::new(-100.0, -100.0, -100.0));
        assert_eq!(b.max, Position::new( 100.0,  100.0,  100.0));
    }
}
