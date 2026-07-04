//! Static Star System / Jump Gate / Celestial Body map data (ADR-0009, ADR-0025).
//!
//! Production servers load `data/galaxy.toml` at startup. Tests and demos use
//! the embedded `data/galaxy.demo.toml` fixture via [`Galaxy::demo`].

use dawn_core::{
    CelestialBodyDef, CelestialBodyId, CelestialBodyKind, JumpGateDef, JumpGateId, Position,
    SectorId, StarSystemDef, StarSystemId,
};
use serde::Deserialize;
use std::{error::Error, fmt};

// -- Galaxy ------------------------------------------------------------------

/// The complete navigation topology: star systems, jump gates, and celestial
/// bodies.
#[derive(Debug, Clone)]
pub struct Galaxy {
    pub systems: Vec<StarSystemDef>,
    pub gates: Vec<JumpGateDef>,
    pub bodies: Vec<CelestialBodyDef>,
}

/// Game units per astronomical unit. Celestial body orbits are authored in AU in
/// `data/galaxy*.toml` and converted to units at load (1 unit = 1 m). Kept f64 so
/// the anchor source (`CelestialBodyDef.abs_m`) stays precise — this is forward-
/// compatible with the true-AU value (1.495978707e11).
///
/// **True AU, reactivated 2026-06-23** (ADR-0029): the residual checklist is
/// clear (gate f64, gate re-authoring, warp-transit f64, AoI f64, anchor-miss
/// guards, gates repositioned near a body so rebase keeps the offset small).
/// `WARP_SPEED` (node/mod.rs) is scaled by the same factor so warp durations
/// (in ticks) are unchanged from the compressed era; visual constants in the
/// client (`BODY_MARKER_CLAMP_DISTANCE` / `SUN_EFFECTIVE_DISTANCE`) were left
/// untouched on the reasoning that they are camera-relative rendering
/// placeholders, not AU-coupled — needs a human playtest to confirm.
pub const UNITS_PER_AU: f64 = 1.495978707e11;

impl Galaxy {
    /// Construct from explicitly provided data (used by `DataLoader`).
    pub fn new(
        systems: Vec<StarSystemDef>,
        gates: Vec<JumpGateDef>,
        bodies: Vec<CelestialBodyDef>,
    ) -> Self {
        Self {
            systems,
            gates,
            bodies,
        }
    }

    /// Parse a `Galaxy` from the shared star-map TOML schema.
    pub fn from_toml_str(input: &str) -> Result<Self, GalaxyTomlError> {
        let file = toml::from_str::<StarMapFile>(input)?;
        Ok(Self {
            systems: file.star_systems.into_iter().map(entry_to_system).collect(),
            gates: file.jump_gates.into_iter().map(entry_to_gate).collect(),
            bodies: file
                .celestial_bodies
                .into_iter()
                .map(entry_to_body)
                .collect(),
        })
    }

    /// Demo topology used by tests, benchmarks, and in-memory demos.
    pub fn demo() -> Self {
        Self::from_toml_str(include_str!("../../../data/galaxy.demo.toml"))
            .expect("embedded demo galaxy map must parse")
    }

    /// Read and parse a `Galaxy` from a TOML file on disk (the production
    /// `data/galaxy.toml` path). Shared by `dawn-simulation` and
    /// `dawn-sector-node`, which used to each hand-roll an identical
    /// read-then-parse-then-panic helper.
    ///
    /// Returns `Err` rather than panicking (library code shouldn't decide to
    /// crash the process) -- callers that want today's "fail fast on startup
    /// misconfiguration" behavior panic on the `Err` themselves.
    pub fn load_from_file(path: &str) -> Result<Self, GalaxyTomlError> {
        let content = std::fs::read_to_string(path).map_err(|source| GalaxyTomlError::Io {
            path: path.to_string(),
            source,
        })?;
        Self::from_toml_str(&content)
    }

    /// Gates whose `from_sector` matches `sector`.
    pub fn gates_in_sector(&self, sector: SectorId) -> Vec<JumpGateDef> {
        self.gates
            .iter()
            .filter(|g| g.from_sector == sector)
            .cloned()
            .collect()
    }

    /// Celestial bodies explicitly assigned to `sector`.
    pub fn bodies_in_sector(&self, sector: SectorId) -> Vec<CelestialBodyDef> {
        self.bodies
            .iter()
            .filter(|b| b.sector == sector)
            .cloned()
            .collect()
    }

    /// `StarSystemId` of the system that contains `sector`, or `None`.
    pub fn system_for_sector_opt(&self, sector: SectorId) -> Option<StarSystemId> {
        self.systems
            .iter()
            .find(|s| s.sectors.contains(&sector))
            .map(|s| s.id)
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
    /// Reading the galaxy map file itself failed (e.g. `load_from_file`).
    /// Carries the path so callers can build a useful panic/log message
    /// without re-deriving it themselves.
    Io {
        path: String,
        source: std::io::Error,
    },
}

impl fmt::Display for GalaxyTomlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "{e}"),
            Self::Io { path, source } => write!(f, "cannot read galaxy map '{path}': {source}"),
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
    #[serde(default)]
    star_systems: Vec<StarSystemEntry>,
    #[serde(default)]
    jump_gates: Vec<JumpGateEntry>,
    #[serde(default)]
    celestial_bodies: Vec<CelestialBodyEntry>,
}

#[derive(Deserialize)]
struct StarSystemEntry {
    id: u32,
    name: String,
    sectors: Vec<u8>,
}

#[derive(Deserialize)]
struct JumpGateEntry {
    id: u32,
    from_sector: u8,
    to_sector: u8,
    /// Gate position in AU, converted to metres by `UNITS_PER_AU` on load —
    /// same convention as `CelestialBodyEntry.position` (ADR-0029 residual:
    /// gates used to be authored as fixed units, decoupled from `UNITS_PER_AU`,
    /// so flipping to true AU would have left them sitting on top of the star).
    /// Parsed as f64 so the authoring precision survives at true-AU scale —
    /// the f64 `abs_m` source (ADR-0029 R1).
    position: [f64; 3],
    activation_radius: f32,
}

#[derive(Deserialize)]
struct CelestialBodyEntry {
    id: u32,
    sector: u8,
    kind: String,
    name: String,
    /// Orbit position in AU (converted to metres by `UNITS_PER_AU` on load).
    /// Parsed as f64 so the authoring precision survives at true-AU scale.
    position: [f64; 3],
    /// Visual radius in units (exaggerated for gameplay; not an AU distance).
    radius: f32,
    #[serde(default)]
    spectral_type: f32,
}

fn parse_body_kind(s: &str) -> CelestialBodyKind {
    match s {
        "Star" => CelestialBodyKind::Star,
        _ => CelestialBodyKind::Planet,
    }
}

fn entry_to_system(e: StarSystemEntry) -> StarSystemDef {
    StarSystemDef {
        id: StarSystemId(e.id),
        name: e.name,
        sectors: e.sectors.into_iter().map(SectorId).collect(),
    }
}

fn entry_to_gate(e: JumpGateEntry) -> JumpGateDef {
    // Authored in AU, scaled to metres by UNITS_PER_AU (same conversion as
    // `entry_to_body`) so gate placement tracks the sector scale instead of
    // sitting at a fixed unit offset (ADR-0029 residual). `abs_m` is the
    // authoritative f64 gate position; `position` is its f32 view (coarse at
    // true AU, fine at compressed scale) — ADR-0029 R1.
    let factor = UNITS_PER_AU;
    let abs_m = [
        e.position[0] * factor,
        e.position[1] * factor,
        e.position[2] * factor,
    ];
    JumpGateDef {
        id: JumpGateId(e.id),
        from_sector: SectorId(e.from_sector),
        position: Position::new(abs_m[0] as f32, abs_m[1] as f32, abs_m[2] as f32),
        abs_m,
        to_sector: SectorId(e.to_sector),
        activation_radius: e.activation_radius,
    }
}

fn entry_to_body(e: CelestialBodyEntry) -> CelestialBodyDef {
    // `position` is authored in AU; convert to game units (see UNITS_PER_AU).
    // `abs_m` is the same conversion done in f64 — the authoritative anchor
    // source that stays precise at true-AU scale (ADR-0029). At compressed scale
    // it equals `position` numerically.
    let factor = UNITS_PER_AU;
    let abs_m = [
        e.position[0] * factor,
        e.position[1] * factor,
        e.position[2] * factor,
    ];
    CelestialBodyDef {
        id: CelestialBodyId(e.id),
        sector: SectorId(e.sector),
        kind: parse_body_kind(&e.kind),
        name: e.name,
        position: Position::new(abs_m[0] as f32, abs_m[1] as f32, abs_m[2] as f32),
        abs_m,
        radius: e.radius,
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
        let map = Galaxy::demo();
        let gates = map.gates_in_sector(SectorId(1));
        assert_eq!(gates.len(), 2);
        assert!(gates.iter().all(|g| g.from_sector == SectorId(1)));
        assert!(gates
            .iter()
            .any(|g| g.id == JumpGateId(1) && g.to_sector == SectorId(0)));
        assert!(gates
            .iter()
            .any(|g| g.id == JumpGateId(2) && g.to_sector == SectorId(2)));
    }

    #[test]
    fn gate_positions_are_converted_from_au_to_units_like_celestial_bodies() {
        // ADR-0029 residual: gates used to be authored as a fixed unit offset
        // (decoupled from UNITS_PER_AU), which would have put them on top of
        // the star once UNITS_PER_AU flips to true AU. Gate 0 is authored at
        // [-0.72, 0.0, -1.32] AU in galaxy.demo.toml -- confirm the loader
        // scales it the same way it scales body positions.
        let map = Galaxy::demo();
        let gate0 = map
            .gates
            .iter()
            .find(|g| g.id == JumpGateId(0))
            .expect("gate 0 exists");
        assert_eq!(gate0.abs_m[0], -0.72 * UNITS_PER_AU);
        assert_eq!(gate0.abs_m[1], 0.0);
        assert_eq!(gate0.abs_m[2], -1.32 * UNITS_PER_AU);
        // f32 ulp bound at this magnitude, not an exactness check (true AU only).
        let ulp_bound = (0.72 * UNITS_PER_AU * f32::EPSILON as f64).abs().max(1.0);
        assert!(
            (gate0.position.x as f64 - (-0.72) * UNITS_PER_AU).abs() < ulp_bound,
            "x = {}",
            gate0.position.x
        );
    }

    #[test]
    fn demo_map_parses_from_embedded_toml() {
        let map = Galaxy::demo();
        assert_eq!(map.systems.len(), 3);
        assert_eq!(map.gates.len(), 4);
        assert_eq!(map.bodies.len(), 7);
    }

    #[test]
    fn celestial_body_positions_are_converted_from_au_to_units() {
        let map = Galaxy::demo();
        // Forge (id 1) is authored at [0.8, 0.0, 0.5] AU in galaxy.demo.toml;
        // the loader scales it by UNITS_PER_AU into metres. The f64 anchor source
        // (abs_m) is exact; the f32 `position` is only ulp-precise at ~10^11 m
        // (~16 km), which is why anchors use abs_m, not position (ADR-0029).
        let forge = map
            .bodies
            .iter()
            .find(|b| b.id == CelestialBodyId(1))
            .expect("Forge exists");
        assert_eq!(forge.abs_m[0], 0.8 * UNITS_PER_AU);
        assert_eq!(forge.abs_m[2], 0.5 * UNITS_PER_AU);
        // At true AU the f32 `position` is only ulp-precise (~tens of km at
        // ~10^11 m), which is why anchors use abs_m, not position (ADR-0029) --
        // this bound is the f32 ulp at Forge's magnitude, not an exactness check.
        let ulp_bound = (0.8 * UNITS_PER_AU * f32::EPSILON as f64).abs().max(1.0);
        assert!(
            (forge.position.x as f64 - 0.8 * UNITS_PER_AU).abs() < ulp_bound,
            "x = {}",
            forge.position.x
        );
        assert_eq!(forge.position.y, 0.0);
        // Stars at [0,0,0] AU stay at the origin (0 * factor = 0).
        let helios = map
            .bodies
            .iter()
            .find(|b| b.id == CelestialBodyId(0))
            .expect("Helios exists");
        assert_eq!(helios.position, Position::ORIGIN);
    }

    #[test]
    fn bodies_in_sector_returns_star_and_planet_for_each_builtin_sector() {
        let map = Galaxy::demo();
        // Sector 0 (Alpha) has an extra planet (Meridian) alongside Helios + Forge.
        let expected_counts = [(SectorId(0), 3), (SectorId(1), 2), (SectorId(2), 2)];
        for (sid, expected) in expected_counts {
            let bodies = map.bodies_in_sector(sid);
            assert_eq!(
                bodies.len(),
                expected,
                "sector {:?} should have {} bodies",
                sid,
                expected
            );
            assert!(
                bodies.iter().all(|b| b.sector == sid),
                "body assigned to wrong sector"
            );
            assert!(
                bodies.iter().any(|b| b.kind == CelestialBodyKind::Star),
                "no star in {:?}",
                sid
            );
            assert!(
                bodies.iter().any(|b| b.kind == CelestialBodyKind::Planet),
                "no planet in {:?}",
                sid
            );
        }
    }

    #[test]
    fn load_from_file_reads_and_parses_a_real_galaxy_toml() {
        // Relative to the crate root (cargo test's cwd), same demo fixture
        // Galaxy::demo() embeds at compile time via include_str!.
        let map = Galaxy::load_from_file("../../data/galaxy.demo.toml")
            .expect("demo galaxy map file must load");
        assert_eq!(map.systems.len(), 3);
    }

    #[test]
    fn load_from_file_returns_io_error_for_a_missing_path_instead_of_panicking() {
        let err = Galaxy::load_from_file("does/not/exist.toml").unwrap_err();
        assert!(
            matches!(err, GalaxyTomlError::Io { .. }),
            "expected an Io variant, got {err:?}"
        );
    }
}
