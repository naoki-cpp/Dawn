//! Star System / Jump Gate static map data (ADR-0009).
//!
//! `StarSystemDef` and `JumpGateDef` describe the *static* navigation
//! topology. They are not ECS entities (CLAUDE.md §1 scope: Ship only) and
//! are not persisted as events — only the *fact* that a Ship used a gate
//! (`JumpGateUsed`) is an event.

use crate::position::Position;
use crate::sector::SectorId;
use serde::{Deserialize, Serialize};

// ── StarSystemId ─────────────────────────────────────────────────────────────

/// Identifies a Star System (a group of one or more Sectors).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StarSystemId(pub u32);

/// Static definition of a Star System: which Sectors belong to it.
#[derive(Debug, Clone, PartialEq)]
pub struct StarSystemDef {
    pub id      : StarSystemId,
    pub name    : String,
    pub sectors : Vec<SectorId>,
}

// ── JumpGateId ───────────────────────────────────────────────────────────────

/// Identifies a Jump Gate (a fixed navigation point within a Sector).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JumpGateId(pub u32);

/// Static definition of a Jump Gate: its location and destination.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JumpGateDef {
    pub id               : JumpGateId,
    pub from_sector      : SectorId,
    pub position         : Position,
    pub to_sector        : SectorId,
    /// A Ship within this distance of `position` may use the gate.
    pub activation_radius: f32,
}

impl JumpGateDef {
    /// Whether `ship_pos` is close enough to this gate to use it.
    pub fn is_in_range(&self, ship_pos: Position) -> bool {
        ship_pos.distance(self.position) <= self.activation_radius
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn gate() -> JumpGateDef {
        JumpGateDef {
            id               : JumpGateId(0),
            from_sector      : SectorId(0),
            position         : Position::new(100.0, 0.0, 0.0),
            to_sector        : SectorId(1),
            activation_radius: 50.0,
        }
    }

    #[test]
    fn ship_within_activation_radius_is_in_range() {
        let g = gate();
        assert!(g.is_in_range(Position::new(120.0, 0.0, 0.0)));
    }

    #[test]
    fn ship_beyond_activation_radius_is_not_in_range() {
        let g = gate();
        assert!(!g.is_in_range(Position::new(1000.0, 0.0, 0.0)));
    }

    #[test]
    fn star_system_def_holds_its_member_sectors() {
        let sys = StarSystemDef {
            id     : StarSystemId(0),
            name   : "Alpha".to_string(),
            sectors: vec![SectorId(0)],
        };
        assert_eq!(sys.sectors, vec![SectorId(0)]);
    }
}
