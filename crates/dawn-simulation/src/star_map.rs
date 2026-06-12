//! Static Star System / Jump Gate topology (ADR-0009 §6).
//!
//! This is map data, not ECS state — it never changes at runtime and is not
//! persisted as events (only the *fact* that a Ship used a gate, i.e.
//! `JumpGateUsed`, is an event).
//!
//! Initial topology: 3 Star Systems, each containing exactly one Sector.
//!
//! ```text
//! Alpha (Sector 0) <-> Beta (Sector 1) <-> Gamma (Sector 2)
//!
//! Gate 0: Sector 0 -> Sector 1 (Alpha -> Beta)
//! Gate 1: Sector 1 -> Sector 0 (Beta -> Alpha)
//! Gate 2: Sector 1 -> Sector 2 (Beta -> Gamma)
//! Gate 3: Sector 2 -> Sector 1 (Gamma -> Beta)
//! ```

use dawn_core::{JumpGateDef, JumpGateId, Position, SectorBounds, SectorId, StarSystemDef, StarSystemId};

/// Activation radius (units) for all Jump Gates in the initial topology.
const GATE_ACTIVATION_RADIUS: f32 = 2_000.0;

/// All Star Systems in the initial topology (ADR-0009 §6).
pub fn star_systems() -> Vec<StarSystemDef> {
    vec![
        StarSystemDef { id: StarSystemId(0), name: "Alpha".to_string(), sectors: vec![SectorId(0)] },
        StarSystemDef { id: StarSystemId(1), name: "Beta".to_string(),  sectors: vec![SectorId(1)] },
        StarSystemDef { id: StarSystemId(2), name: "Gamma".to_string(), sectors: vec![SectorId(2)] },
    ]
}

/// All Jump Gates in the initial topology (ADR-0009 §6).
///
/// Each gate sits near the edge of its `from_sector`, opposite the
/// direction of its destination.
pub fn all_gates() -> Vec<JumpGateDef> {
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

/// All Jump Gates whose `from_sector` is `sector` — the gates a node for
/// `sector` needs to know about to validate `JumpCommand`s.
pub fn gates_in_sector(sector: SectorId) -> Vec<JumpGateDef> {
    all_gates().into_iter().filter(|g| g.from_sector == sector).collect()
}

/// The `StarSystemId` that `sector` belongs to.
///
/// Panics if `sector` is not part of any known Star System — the initial
/// topology covers every Sector that can exist (Sector 0..=2).
pub fn system_for_sector(sector: SectorId) -> StarSystemId {
    star_systems()
        .into_iter()
        .find(|sys| sys.sectors.contains(&sector))
        .map(|sys| sys.id)
        .unwrap_or_else(|| panic!("Sector {sector:?} does not belong to any known Star System"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_sector_in_initial_topology_belongs_to_a_distinct_star_system() {
        assert_eq!(system_for_sector(SectorId(0)), StarSystemId(0));
        assert_eq!(system_for_sector(SectorId(1)), StarSystemId(1));
        assert_eq!(system_for_sector(SectorId(2)), StarSystemId(2));
    }

    #[test]
    fn gates_in_sector_returns_only_gates_originating_in_that_sector() {
        let gates = gates_in_sector(SectorId(1));
        assert_eq!(gates.len(), 2);
        assert!(gates.iter().all(|g| g.from_sector == SectorId(1)));
        assert!(gates.iter().any(|g| g.id == JumpGateId(1) && g.to_sector == SectorId(0)));
        assert!(gates.iter().any(|g| g.id == JumpGateId(2) && g.to_sector == SectorId(2)));
    }

    #[test]
    fn all_gates_are_within_sector_bounds_and_have_positive_activation_radius() {
        let bounds = SectorBounds::centered(SectorBounds::DEFAULT_HALF);
        for gate in all_gates() {
            assert!(bounds.contains(gate.position), "gate {:?} position out of bounds", gate.id);
            assert!(gate.activation_radius > 0.0);
        }
    }
}
