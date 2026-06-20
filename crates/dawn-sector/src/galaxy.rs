//! Static Star System / Jump Gate / Celestial Body map data (ADR-0009, ADR-0025).
//!
//! Production servers load `data/galaxy.toml` at startup. Tests and demos use
//! the embedded `data/galaxy.demo.toml` fixture via [`Galaxy::demo`].

use dawn_core::{
    CelestialBodyDef, CelestialBodyId, CelestialBodyKind, JumpGateDef, JumpGateId,
    Position, SectorId, StarSystemDef, StarSystemId,
};
use serde::Deserialize;
use std::{error::Error, fmt};

// -- Galaxy ------------------------------------------------------------------

/// The complete navigation topology: star systems, jump gates, and celestial
/// bodies.
#[derive(Debug, Clone)]
pub struct Galaxy {
    pub systems: Vec<StarSystemDef>,
    pub gates  : Vec<JumpGateDef>,
    pub bodies : Vec<CelestialBodyDef>,
}

impl Galaxy {
    /// Construct from explicitly provided data (used by `DataLoader`).
    pub fn new(systems: Vec<StarSystemDef>, gates: Vec<JumpGateDef>, bodies: Vec<CelestialBodyDef>) -> Self {
        Self { systems, gates, bodies }
    }

    /// Parse a `Galaxy` from the shared star-map TOML schema.
    pub fn from_toml_str(input: &str) -> Result<Self, GalaxyTomlError> {
        let file = toml::from_str::<StarMapFile>(input)?;
        Ok(Self {
            systems: file.star_systems.into_iter().map(entry_to_system).collect(),
            gates  : file.jump_gates.into_iter().map(entry_to_gate).collect(),
            bodies : file.celestial_bodies.into_iter().map(entry_to_body).collect(),
        })
    }

    /// Demo topology used by tests, benchmarks, and in-memory demos.
    pub fn demo() -> Self {
        Self::from_toml_str(include_str!("../../../data/galaxy.demo.toml"))
            .expect("embedded demo galaxy map must parse")
    }

    /// Gates whose `from_sector` matches `sector`.
    pub fn gates_in_sector(&self, sector: SectorId) -> Vec<JumpGateDef> {
        self.gates.iter().filter(|g| g.from_sector == sector).cloned().collect()
    }

    /// Celestial bodies explicitly assigned to `sector`.
    pub fn bodies_in_sector(&self, sector: SectorId) -> Vec<CelestialBodyDef> {
        self.bodies.iter().filter(|b| b.sector == sector).cloned().collect()
    }

    /// `StarSystemId` of the system that contains `sector`, or `None`.
    pub fn system_for_sector_opt(&self, sector: SectorId) -> Option<StarSystemId> {
        self.systems.iter().find(|s| s.sectors.contains(&sector)).map(|s| s.id)
    }

    /// `StarSystemId` of the system that contains `sector`.
    ///
    /// Panics if `sector` is not part of any known Star System.
    pub fn system_for_sector(&self, sector: SectorId) -> StarSystemId {
        self.system_for_sector_opt(sector)
            .unwrap_or_else(|| panic!("Sector {sector:?} does not belong to any known Star System"))
    }

}

// -- TOML schema -------------------------------------------------------------

#[derive(Debug)]
pub enum GalaxyTomlError {
    Parse(toml::de::Error),
}

impl fmt::Display for GalaxyTomlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "{e}"),
        }
    }
}

impl Error for GalaxyTomlError {}

impl From<toml::de::Error> for GalaxyTomlError {
    fn from(value: toml::de::Error) -> Self {
        Self::Parse(value)
    }
}

#[derive(Deserialize)]
struct StarMapFile {
    #[serde(default)] star_systems    : Vec<StarSystemEntry>,
    #[serde(default)] jump_gates      : Vec<JumpGateEntry>,
    #[serde(default)] celestial_bodies: Vec<CelestialBodyEntry>,
}

#[derive(Deserialize)]
struct StarSystemEntry {
    id     : u32,
    name   : String,
    sectors: Vec<u8>,
}

#[derive(Deserialize)]
struct JumpGateEntry {
    id               : u32,
    from_sector      : u8,
    to_sector        : u8,
    position         : [f32; 3],
    activation_radius: f32,
}

#[derive(Deserialize)]
struct CelestialBodyEntry {
    id           : u32,
    sector       : u8,
    kind         : String,
    name         : String,
    position     : [f32; 3],
    radius       : f32,
    #[serde(default)] spectral_type: f32,
}

fn parse_body_kind(s: &str) -> CelestialBodyKind {
    match s { "Star" => CelestialBodyKind::Star, _ => CelestialBodyKind::Planet }
}

fn entry_to_system(e: StarSystemEntry) -> StarSystemDef {
    StarSystemDef {
        id     : StarSystemId(e.id),
        name   : e.name,
        sectors: e.sectors.into_iter().map(SectorId).collect(),
    }
}

fn entry_to_gate(e: JumpGateEntry) -> JumpGateDef {
    JumpGateDef {
        id               : JumpGateId(e.id),
        from_sector      : SectorId(e.from_sector),
        position         : Position::new(e.position[0], e.position[1], e.position[2]),
        to_sector        : SectorId(e.to_sector),
        activation_radius: e.activation_radius,
    }
}

fn entry_to_body(e: CelestialBodyEntry) -> CelestialBodyDef {
    CelestialBodyDef {
        id           : CelestialBodyId(e.id),
        sector       : SectorId(e.sector),
        kind         : parse_body_kind(&e.kind),
        name         : e.name,
        position     : Position::new(e.position[0], e.position[1], e.position[2]),
        radius       : e.radius,
        spectral_type: e.spectral_type,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_sector_in_initial_topology_belongs_to_a_distinct_star_system() {
        let map = Galaxy::demo();
        assert_eq!(map.system_for_sector(SectorId(0)), StarSystemId(0));
        assert_eq!(map.system_for_sector(SectorId(1)), StarSystemId(1));
        assert_eq!(map.system_for_sector(SectorId(2)), StarSystemId(2));
    }

    #[test]
    fn gates_in_sector_returns_only_gates_originating_in_that_sector() {
        let map   = Galaxy::demo();
        let gates = map.gates_in_sector(SectorId(1));
        assert_eq!(gates.len(), 2);
        assert!(gates.iter().all(|g| g.from_sector == SectorId(1)));
        assert!(gates.iter().any(|g| g.id == JumpGateId(1) && g.to_sector == SectorId(0)));
        assert!(gates.iter().any(|g| g.id == JumpGateId(2) && g.to_sector == SectorId(2)));
    }

    #[test]
    fn demo_map_parses_from_embedded_toml() {
        let map = Galaxy::demo();
        assert_eq!(map.systems.len(), 3);
        assert_eq!(map.gates.len(), 4);
        assert_eq!(map.bodies.len(), 7);
    }

    #[test]
    fn bodies_in_sector_returns_star_and_planet_for_each_builtin_sector() {
        let map = Galaxy::demo();
        // Sector 0 (Alpha) has an extra planet (Meridian) alongside Helios + Forge.
        let expected_counts = [(SectorId(0), 3), (SectorId(1), 2), (SectorId(2), 2)];
        for (sid, expected) in expected_counts {
            let bodies = map.bodies_in_sector(sid);
            assert_eq!(bodies.len(), expected, "sector {:?} should have {} bodies", sid, expected);
            assert!(bodies.iter().all(|b| b.sector == sid), "body assigned to wrong sector");
            assert!(bodies.iter().any(|b| b.kind == CelestialBodyKind::Star),  "no star in {:?}", sid);
            assert!(bodies.iter().any(|b| b.kind == CelestialBodyKind::Planet), "no planet in {:?}", sid);
        }
    }
}
