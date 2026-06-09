//! Logical tick counter.
//!
//! A `Tick` is a monotonically increasing `u64` that represents one simulation
//! step within a single Sector Node.  It has **no relationship** to wall-clock
//! time.  See CLAUDE.md INV-005 and FBD-003.

use serde::{Deserialize, Serialize};

/// One logical time step within a Sector Node.
///
/// - Comparable only within the same Sector (cross-Sector ordering uses VectorClock).
/// - Every movement event (`VelocityChanged`) must carry the `Tick` at which it occurred.
/// - Never derived from `std::time::SystemTime`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Tick(pub u64);

impl Tick {
    pub const ZERO: Self = Self(0);

    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }

    pub fn value(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for Tick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Tick({})", self.0)
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_is_monotonically_increasing() {
        let t = Tick::ZERO;
        assert!(t.next() > t);
        assert!(t.next().next() > t.next());
    }

    #[test]
    fn tick_zero_is_the_smallest_possible_tick() {
        assert_eq!(Tick::ZERO, Tick(0));
        assert!(Tick(1) > Tick::ZERO);
    }
}
