//! Navigation domain types: star systems, jump gates, celestial bodies,
//! and warp targets (ADR-0009, ADR-0025).
//!
//! These types describe the *static* navigation topology. They are not ECS
//! entities and are not persisted as events.

use crate::position::{AbsolutePosition, Position};
use crate::sector::SectorId;
use serde::{Deserialize, Serialize};

// -- Shared navigation rules -----------------------------------------------

/// Minimum server-space distance required before a ship may enter warp.
/// Closer targets must be approached under sublight movement instead.
pub const MIN_WARP_DISTANCE: f64 = 3_000.0;

// -- CelestialBodyId / Kind / Def -------------------------------------------

/// Identifies a star, planet, or other celestial body within a Sector (ADR-0025).
/// Values are globally unique across all Sectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CelestialBodyId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CelestialBodyKind {
    Star,
    Planet,
}

/// Static definition of a celestial body (ADR-0025).
#[derive(Debug, Clone, PartialEq)]
pub struct CelestialBodyDef {
    pub id: CelestialBodyId,
    pub sector: SectorId,
    pub kind: CelestialBodyKind,
    pub name: String,
    pub position: Position,
    /// Absolute position in metres as f64 — the authoritative anchor source
    /// (ADR-0029). Equals `position` numerically at compressed scale, but stays
    /// precise at true-AU distances where the old f32 `position` would lose ~tens of
    /// km. `AnchorTable` is built from this, not from `position`.
    pub abs_m: AbsolutePosition,
    /// Logical radius (units). Warp arrival stops at `radius * 1.5` from centre.
    pub radius: f64,
    /// Blackbody spectral type [0=O/blue, 0.6=G/Sun-yellow, 1=M/red]. Planets: 0.0.
    pub spectral_type: f32,
}

// -- AnchorId ----------------------------------------------------------------

/// Identifies a coordinate *anchor* — a celestial body that serves as a local
/// origin for ship positions (ADR-0029). Anchors are per-body (§2), so an
/// `AnchorId` is one-to-one with a [`CelestialBodyId`]: a ship's authoritative
/// position is `(anchor, f64 offset)` and its absolute position is
/// `anchor_abs(f64) + offset`. Keeping the offset small (the ship stays near
/// its anchor) preserves precision even when the anchor sits at a true
/// astronomical distance from the Sector origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct AnchorId(pub u32);

impl From<CelestialBodyId> for AnchorId {
    fn from(b: CelestialBodyId) -> Self {
        AnchorId(b.0)
    }
}

// -- WarpTarget --------------------------------------------------------------

/// The destination of a `WarpCommand` (ADR-0025).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WarpTarget {
    Gate(JumpGateId),
    Body(CelestialBodyId),
}

// -- StarSystemId ------------------------------------------------------------

/// Identifies a Star System (a group of one or more Sectors).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StarSystemId(pub u32);

/// Static definition of a Star System: which Sectors belong to it.
#[derive(Debug, Clone, PartialEq)]
pub struct StarSystemDef {
    pub id: StarSystemId,
    pub name: String,
    pub sectors: Vec<SectorId>,
}

// -- JumpGateId --------------------------------------------------------------

/// Identifies a Jump Gate (a fixed navigation point within a Sector).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JumpGateId(pub u32);

/// Static definition of a Jump Gate: its location and destination.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JumpGateDef {
    pub id: JumpGateId,
    pub from_sector: SectorId,
    pub position: Position,
    /// Absolute gate position in metres as f64 — the authoritative source for
    /// range/warp checks (ADR-0029 review R1). Equals `position` numerically at
    /// compressed scale, but stays precise at true-AU distances where the old f32
    /// `position` is ~tens of km coarse (one f32 ulp at ~10^11 m). Gates are
    /// Sector-frame fixtures, so this *is* their absolute position.
    pub abs_m: AbsolutePosition,
    pub to_sector: SectorId,
    /// A Ship within this distance of `position` may use the gate.
    pub activation_radius: f64,
}

impl JumpGateDef {
    /// Whether `ship_pos` is close enough to this gate to use it.
    pub fn is_in_range(&self, ship_pos: Position) -> bool {
        ship_pos.distance(self.position) <= self.activation_radius
    }

    /// Whether an absolute (Sector-frame, f64) ship position is within range.
    /// The precise path: composes the separation in f64 against the f64 gate
    /// source, so it does not lose the ~16 km of f32 ulp at true-AU distances
    /// (ADR-0029 R1).
    pub fn is_in_range_abs(&self, ship_abs: AbsolutePosition) -> bool {
        self.distance_abs(ship_abs) <= self.activation_radius
    }

    /// True distance (metres, f64) from an absolute ship position to this gate.
    pub fn distance_abs(&self, ship_abs: AbsolutePosition) -> f64 {
        let d = [
            ship_abs[0] - self.abs_m[0],
            ship_abs[1] - self.abs_m[1],
            ship_abs[2] - self.abs_m[2],
        ];
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    }
}

// -- StationId ---------------------------------------------------------------

/// Identifies an NPC-provided station within a Sector (ADR-0034 9B foundation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StationId(pub u32);

/// Static definition of a station where players may assemble/disassemble ships
/// and access a station inventory.
#[derive(Debug, Clone, PartialEq)]
pub struct StationDef {
    pub id: StationId,
    pub sector: SectorId,
    pub name: String,
    pub position: Position,
    /// Absolute station position in metres as f64, matching gate/body authoring.
    pub abs_m: AbsolutePosition,
    /// A ship within this radius may use the station.
    pub docking_radius: f64,
}

impl StationDef {
    /// Whether `ship_pos` is close enough to use the station.
    pub fn is_in_range(&self, ship_pos: Position) -> bool {
        ship_pos.distance(self.position) <= self.docking_radius
    }

    /// Whether an absolute (Sector-frame, f64) ship position is within range.
    pub fn is_in_range_abs(&self, ship_abs: AbsolutePosition) -> bool {
        self.distance_abs(ship_abs) <= self.docking_radius
    }

    /// True distance (metres, f64) from an absolute ship position to this station.
    pub fn distance_abs(&self, ship_abs: AbsolutePosition) -> f64 {
        let d = [
            ship_abs[0] - self.abs_m[0],
            ship_abs[1] - self.abs_m[1],
            ship_abs[2] - self.abs_m[2],
        ];
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    }
}

// -- Tests -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn gate() -> JumpGateDef {
        JumpGateDef {
            id: JumpGateId(0),
            from_sector: SectorId(0),
            position: Position::new(100.0, 0.0, 0.0),
            abs_m: [100.0, 0.0, 0.0].into(),
            to_sector: SectorId(1),
            activation_radius: 50.0,
        }
    }

    #[test]
    fn is_in_range_abs_is_precise_at_true_au() {
        // A gate authored ~1 AU out (where f32 ulp is ~16 km): the f64 path
        // resolves a ship 40 m inside the 50 m ring correctly, which the f32
        // `position` could not distinguish (ADR-0029 R1).
        const AU_M: f64 = 1.495978707e11;
        let g = JumpGateDef {
            id: JumpGateId(0),
            from_sector: SectorId(0),
            position: Position::new(AU_M, 0.0, 0.0),
            abs_m: [AU_M, 0.0, 0.0].into(),
            to_sector: SectorId(1),
            activation_radius: 50.0,
        };
        assert!(
            g.is_in_range_abs([AU_M + 40.0, 0.0, 0.0].into()),
            "40 m out is within the 50 m ring"
        );
        assert!(
            !g.is_in_range_abs([AU_M + 60.0, 0.0, 0.0].into()),
            "60 m out is beyond the ring"
        );
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
            id: StarSystemId(0),
            name: "Alpha".to_string(),
            sectors: vec![SectorId(0)],
        };
        assert_eq!(sys.sectors, vec![SectorId(0)]);
    }

    #[test]
    fn station_in_range_abs_is_precise_at_true_au() {
        const AU_M: f64 = 1.495978707e11;
        let station = StationDef {
            id: StationId(0),
            sector: SectorId(0),
            name: "Forge Station".to_string(),
            position: Position::new(AU_M, 0.0, 0.0),
            abs_m: [AU_M, 0.0, 0.0].into(),
            docking_radius: 100.0,
        };
        assert!(station.is_in_range_abs([AU_M + 80.0, 0.0, 0.0].into()));
        assert!(!station.is_in_range_abs([AU_M + 120.0, 0.0, 0.0].into()));
    }
}
