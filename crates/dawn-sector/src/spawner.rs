//! Deterministic dummy-ship generation for load testing.
//!
//! `rand` is intentionally confined to this crate.  No other crate
//! may depend on `rand` for production code paths.

use dawn_core::{NodeId, Position, SectorBounds, ShipId, Velocity};
use rand::Rng;

/// Parameters for generating a population of dummy Ships.
#[derive(Debug, Clone)]
pub struct SpawnConfig {
    /// Which node is creating these ships.
    pub node_id: NodeId,
    /// Spatial bounds within which ships are scattered.
    pub bounds: SectorBounds,
    /// Absolute value cap for each velocity component (units/tick).
    pub max_speed: f32,
    /// Seed for the PRNG, enabling deterministic reproduction.
    pub seed: u64,
}

impl SpawnConfig {
    pub fn default_for_node(node_id: NodeId) -> Self {
        Self {
            node_id,
            bounds: SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            max_speed: 5.0,
            seed: 0xDEAD_BEEF,
        }
    }
}

/// Generate `count` `(ShipId, Position, Velocity)` triples using a seeded
/// PRNG so that the same seed always produces the same population.
///
/// Counter starts at `counter_offset`, allowing multiple nodes to generate
/// non-overlapping ID ranges.
pub fn generate_ships(
    count: usize,
    config: &SpawnConfig,
    counter_offset: u64,
) -> Vec<(ShipId, Position, Velocity)> {
    use rand::SeedableRng;
    let mut rng = rand::rngs::SmallRng::seed_from_u64(config.seed ^ counter_offset);

    (0..count)
        .map(|i| {
            let ship_id = ShipId::new(config.node_id, counter_offset + i as u64);

            let pos = Position::new(
                rng.gen_range(config.bounds.min.x..=config.bounds.max.x),
                rng.gen_range(config.bounds.min.y..=config.bounds.max.y),
                rng.gen_range(config.bounds.min.z..=config.bounds.max.z),
            );

            // Ensure every ship has a non-zero velocity so it actually moves.
            let vel = loop {
                let dx = rng.gen_range(-config.max_speed..=config.max_speed);
                let dy = rng.gen_range(-config.max_speed..=config.max_speed);
                let dz = rng.gen_range(-config.max_speed..=config.max_speed);
                if dx.abs() > 0.1 || dy.abs() > 0.1 || dz.abs() > 0.1 {
                    break Velocity::new(dx, dy, dz);
                }
            };

            (ship_id, pos, vel)
        })
        .collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> SpawnConfig {
        SpawnConfig::default_for_node(NodeId(0))
    }

    #[test]
    fn generates_the_exact_number_of_ships_requested() {
        let ships = generate_ships(100, &config(), 0);
        assert_eq!(ships.len(), 100);
    }

    #[test]
    fn all_generated_positions_are_within_sector_bounds() {
        let cfg = config();
        let ships = generate_ships(1_000, &cfg, 0);
        for (_, pos, _) in &ships {
            assert!(
                cfg.bounds.contains(*pos),
                "position {pos} is outside bounds"
            );
        }
    }

    #[test]
    fn all_generated_ships_have_nonzero_velocity() {
        let ships = generate_ships(1_000, &config(), 0);
        for (_, _, vel) in &ships {
            assert!(
                vel.speed() > 0.0,
                "every dummy ship must have non-zero velocity so VelocityChanged events are produced"
            );
        }
    }

    #[test]
    fn ship_ids_are_unique_within_a_single_generation() {
        use std::collections::HashSet;
        let ships = generate_ships(10_000, &config(), 0);
        let ids: HashSet<_> = ships.iter().map(|(id, _, _)| id.raw()).collect();
        assert_eq!(
            ids.len(),
            ships.len(),
            "all generated ShipIds must be unique"
        );
    }

    #[test]
    fn same_seed_produces_identical_populations() {
        let a = generate_ships(100, &config(), 0);
        let b = generate_ships(100, &config(), 0);
        for (i, ((id_a, pos_a, vel_a), (id_b, pos_b, vel_b))) in a.iter().zip(b.iter()).enumerate()
        {
            assert_eq!(id_a, id_b, "id mismatch at index {i}");
            assert_eq!(pos_a, pos_b, "position mismatch at index {i}");
            assert_eq!(vel_a, vel_b, "velocity mismatch at index {i}");
        }
    }
}
