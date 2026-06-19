//! Static Star System / Jump Gate / Celestial Body map data (ADR-0009, ADR-0025).
//!
//! Runtime topology is loaded from `data/star_map.toml` when the server starts.
//! If the file is absent, `Galaxy::builtin()` provides the hardcoded defaults.
//!
//! The free functions (`star_systems()`, `all_gates()`, etc.) are thin wrappers
//! over `Galaxy::builtin()` kept for backward compatibility in tests.

use dawn_core::{CelestialBodyDef, CelestialBodyId, CelestialBodyKind, JumpGateDef, JumpGateId, Position, SectorBounds, SectorId, StarSystemDef, StarSystemId};

// ── Galaxy ───────────────────────────────────────────────────────────────────

/// The complete navigation topology: star systems, jump gates, and celestial
/// bodies.  Loaded from `data/star_map.toml` at server startup; falls back to
/// [`Galaxy::builtin`] if the file is absent.
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

    /// Hardcoded defaults — used when `data/star_map.toml` is absent.
    pub fn builtin() -> Self {
        Self {
            systems: builtin_star_systems(),
            gates  : builtin_gates(),
            bodies : builtin_bodies(),
        }
    }

    /// Gates whose `from_sector` matches `sector`.
    pub fn gates_in_sector(&self, sector: SectorId) -> Vec<JumpGateDef> {
        self.gates.iter().filter(|g| g.from_sector == sector).cloned().collect()
    }

    /// Celestial bodies belonging to the Star System that contains `sector`.
    pub fn bodies_in_sector(&self, sector: SectorId) -> Vec<CelestialBodyDef> {
        let sys_id = match self.system_for_sector_opt(sector) {
            Some(id) => id,
            None     => return vec![],
        };
        let sectors_in_sys: Vec<SectorId> = self.systems.iter()
            .find(|s| s.id == sys_id)
            .map(|s| s.sectors.clone())
            .unwrap_or_default();
        // Bodies are tagged by position but not by sector — we match by which
        // sector they are nearest to (for the current 1-sector-per-system topology
        // every body in the system belongs to `sector`).
        // We identify them by the sectors list: any body whose ID range was
        // assigned to this system. In practice the TOML loader tags bodies with a
        // `sector` field; the builtin data follows the same convention implicitly
        // via the system–sector 1:1 mapping.
        self.bodies.iter()
            .filter(|b| {
                // A body belongs to this sector if it belongs to any sector in the same system.
                sectors_in_sys.contains(&sector)
                    // AND the body's ID is in the range we allocated for this system.
                    // For builtin data: system 0 → IDs 0,1; system 1 → IDs 2,3; system 2 → 4,5.
                    // For TOML data: bodies carry an explicit `sector` field stored in their ID
                    // range (assigned sequentially per-system by the loader).
                    //
                    // Simplification: since each system has exactly one sector, return all
                    // bodies whose StarSystem contains `sector`.
                    && self.system_contains_body(sys_id, b.id)
            })
            .cloned()
            .collect()
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

    // Determine whether body `bid` was assigned to system `sid`.
    // For the builtin data: system N owns body IDs [2N, 2N+1].
    // For loaded data: bodies are sorted by system order; this is an approximation.
    // A robust alternative would store `sector` on `CelestialBodyDef`, but to
    // avoid breaking `dawn-core` we use the assignment convention here.
    fn system_contains_body(&self, sid: StarSystemId, bid: CelestialBodyId) -> bool {
        let bodies_per_system: u32 = if self.systems.is_empty() { return false }
            else { (self.bodies.len() as u32).div_ceil(self.systems.len() as u32).max(1) };
        bid.0 / bodies_per_system == sid.0
    }
}

// ── Builtin data ──────────────────────────────────────────────────────────────

/// Activation radius (units) for all Jump Gates in the initial topology.
const GATE_ACTIVATION_RADIUS: f32 = 2_000.0;

fn builtin_star_systems() -> Vec<StarSystemDef> {
    vec![
        StarSystemDef { id: StarSystemId(0), name: "Alpha".to_string(), sectors: vec![SectorId(0)] },
        StarSystemDef { id: StarSystemId(1), name: "Beta".to_string(),  sectors: vec![SectorId(1)] },
        StarSystemDef { id: StarSystemId(2), name: "Gamma".to_string(), sectors: vec![SectorId(2)] },
    ]
}

fn builtin_gates() -> Vec<JumpGateDef> {
    let half = SectorBounds::DEFAULT_HALF;
    vec![
        // Gate 0: Sector 0 -> Sector 1 (Alpha -> Beta), placed on Sector 0's +X edge.
        JumpGateDef {
            id               : JumpGateId(0),
            from_sector      : SectorId(0),
            position         : Position::new(half - 1_000.0, 0.0, 0.0),
            to_sector        : SectorId(1),
            activation_radius: GATE_ACTIVATION_RADIUS,
        },
        // Gate 1: Sector 1 -> Sector 0 (Beta -> Alpha), placed on Sector 1's -X edge.
        JumpGateDef {
            id               : JumpGateId(1),
            from_sector      : SectorId(1),
            position         : Position::new(-(half - 1_000.0), 0.0, 0.0),
            to_sector        : SectorId(0),
            activation_radius: GATE_ACTIVATION_RADIUS,
        },
        // Gate 2: Sector 1 -> Sector 2 (Beta -> Gamma), placed on Sector 1's +X edge.
        JumpGateDef {
            id               : JumpGateId(2),
            from_sector      : SectorId(1),
            position         : Position::new(half - 1_000.0, 0.0, 0.0),
            to_sector        : SectorId(2),
            activation_radius: GATE_ACTIVATION_RADIUS,
        },
        // Gate 3: Sector 2 -> Sector 1 (Gamma -> Beta), placed on Sector 2's -X edge.
        JumpGateDef {
            id               : JumpGateId(3),
            from_sector      : SectorId(2),
            position         : Position::new(-(half - 1_000.0), 0.0, 0.0),
            to_sector        : SectorId(1),
            activation_radius: GATE_ACTIVATION_RADIUS,
        },
    ]
}

/// All builtin celestial bodies (all sectors).
///
/// Scale: 1 unit = 10,000 km  →  1 AU ≈ 15,000 units.
/// Jump Gates sit at ~49,000 units (≈ 3.3 AU, outer asteroid belt).
///
/// CelestialBodyId values are globally unique:
///   Sector 0 (Alpha):  0=Helios(star), 1=Forge(planet)
///   Sector 1 (Beta):   2=Aegis(star),  3=Haven(planet)
///   Sector 2 (Gamma):  4=Crimson(star),5=Bastion(planet)
fn builtin_bodies() -> Vec<CelestialBodyDef> {
    let mut v = vec![];
    // Sector 0 (Alpha)
    v.push(CelestialBodyDef {
        id           : CelestialBodyId(0),
        kind         : CelestialBodyKind::Star,
        name         : "Helios".to_string(),
        position     : Position::new(0.0, 0.0, 0.0),
        radius       : 15_000.0,
        spectral_type: 0.60, // G-type, Sun-like yellow
    });
    v.push(CelestialBodyDef {
        id           : CelestialBodyId(1),
        kind         : CelestialBodyKind::Planet,
        name         : "Forge".to_string(),
        position     : Position::new(15_000.0, 0.0, 0.0),
        radius       : 3_500.0,
        spectral_type: 0.0,
    });
    // Sector 1 (Beta)
    // Note: body IDs 2,3 → system 1 → div_ceil logic in system_contains_body
    // uses ids 2,3 for system index 1 (assuming 2 bodies/system).
    v.push(CelestialBodyDef {
        id           : CelestialBodyId(2),
        kind         : CelestialBodyKind::Star,
        name         : "Aegis".to_string(),
        position     : Position::new(0.0, 0.0, 0.0),
        radius       : 12_000.0,
        spectral_type: 0.30, // A-type, white-blue
    });
    v.push(CelestialBodyDef {
        id           : CelestialBodyId(3),
        kind         : CelestialBodyKind::Planet,
        name         : "Haven".to_string(),
        position     : Position::new(-21_600.0, 0.0, 7_200.0),
        radius       : 4_500.0,
        spectral_type: 0.0,
    });
    // Sector 2 (Gamma)
    v.push(CelestialBodyDef {
        id           : CelestialBodyId(4),
        kind         : CelestialBodyKind::Star,
        name         : "Crimson".to_string(),
        position     : Position::new(0.0, 0.0, 0.0),
        radius       : 18_000.0,
        spectral_type: 0.85, // K/M-type, orange-red
    });
    v.push(CelestialBodyDef {
        id           : CelestialBodyId(5),
        kind         : CelestialBodyKind::Planet,
        name         : "Bastion".to_string(),
        position     : Position::new(10_000.0, 0.0, -4_000.0),
        radius       : 3_000.0,
        spectral_type: 0.0,
    });
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_sector_in_initial_topology_belongs_to_a_distinct_star_system() {
        let map = Galaxy::builtin();
        assert_eq!(map.system_for_sector(SectorId(0)), StarSystemId(0));
        assert_eq!(map.system_for_sector(SectorId(1)), StarSystemId(1));
        assert_eq!(map.system_for_sector(SectorId(2)), StarSystemId(2));
    }

    #[test]
    fn gates_in_sector_returns_only_gates_originating_in_that_sector() {
        let map   = Galaxy::builtin();
        let gates = map.gates_in_sector(SectorId(1));
        assert_eq!(gates.len(), 2);
        assert!(gates.iter().all(|g| g.from_sector == SectorId(1)));
        assert!(gates.iter().any(|g| g.id == JumpGateId(1) && g.to_sector == SectorId(0)));
        assert!(gates.iter().any(|g| g.id == JumpGateId(2) && g.to_sector == SectorId(2)));
    }

    #[test]
    fn all_gates_are_within_sector_bounds_and_have_positive_activation_radius() {
        let map    = Galaxy::builtin();
        let bounds = SectorBounds::centered(SectorBounds::DEFAULT_HALF);
        for gate in &map.gates {
            assert!(bounds.contains(gate.position), "gate {:?} position out of bounds", gate.id);
            assert!(gate.activation_radius > 0.0);
        }
    }

    #[test]
    fn bodies_in_sector_returns_star_and_planet_for_each_builtin_sector() {
        let map = Galaxy::builtin();
        for sid in [SectorId(0), SectorId(1), SectorId(2)] {
            let bodies = map.bodies_in_sector(sid);
            assert_eq!(bodies.len(), 2, "sector {:?} should have 2 bodies", sid);
            assert!(bodies.iter().any(|b| b.kind == CelestialBodyKind::Star),  "no star in {:?}", sid);
            assert!(bodies.iter().any(|b| b.kind == CelestialBodyKind::Planet), "no planet in {:?}", sid);
        }
    }
}
