//! `SimWorld` — the single owner of all ECS state within a Sector Node.

use crate::components::{PositionComp, ShipIdComp, VelocityComp};
use dawn_core::{Position, SectorId, ShipId, Velocity};
use hecs::Entity;

/// Wraps `hecs::World` with a domain-aware API.
///
/// Direct access to the inner `hecs::World` is available via `inner()` and
/// `inner_mut()` for use by systems that need query flexibility.
pub struct SimWorld {
    inner    : hecs::World,
    sector_id: SectorId,
}

impl SimWorld {
    pub fn new(sector_id: SectorId) -> Self {
        Self {
            inner: hecs::World::new(),
            sector_id,
        }
    }

    pub fn sector_id(&self) -> SectorId {
        self.sector_id
    }

    /// Spawn a Ship entity and return the ECS `Entity` handle.
    ///
    /// The caller is responsible for ensuring `ship_id` is globally unique.
    /// See CLAUDE.md INV-004 and FBD-005.
    pub fn spawn_ship(
        &mut self,
        ship_id : ShipId,
        position: Position,
        velocity: Velocity,
    ) -> Entity {
        self.inner.spawn((
            ShipIdComp(ship_id),
            PositionComp(position),
            VelocityComp(velocity),
        ))
    }

    /// Despawn a Ship entity.
    ///
    /// Returns `true` if the entity existed, `false` if it was already absent.
    /// The caller must append a `ShipDespawned` event before or after calling
    /// this — the ECS does not produce events.
    pub fn despawn_ship(&mut self, entity: Entity) -> bool {
        self.inner.despawn(entity).is_ok()
    }

    /// Number of Ship entities currently in the world.
    pub fn ship_count(&self) -> usize {
        self.inner.len() as usize
    }

    /// Immutable access to the underlying `hecs::World` for read-only queries.
    pub fn inner(&self) -> &hecs::World {
        &self.inner
    }

    /// Mutable access to the underlying `hecs::World` for systems.
    pub fn inner_mut(&mut self) -> &mut hecs::World {
        &mut self.inner
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{NodeId, Position, SectorId, Velocity};

    fn make_world() -> SimWorld {
        SimWorld::new(SectorId(0))
    }

    fn make_ship_id(counter: u64) -> ShipId {
        ShipId::new(NodeId(0), counter)
    }

    #[test]
    fn newly_created_world_contains_no_ships() {
        let world = make_world();
        assert_eq!(world.ship_count(), 0);
    }

    #[test]
    fn spawned_ship_increments_ship_count() {
        let mut world = make_world();
        world.spawn_ship(make_ship_id(1), Position::ORIGIN, Velocity::ZERO);
        assert_eq!(world.ship_count(), 1);
    }

    #[test]
    fn despawned_ship_decrements_ship_count() {
        let mut world = make_world();
        let entity = world.spawn_ship(make_ship_id(1), Position::ORIGIN, Velocity::ZERO);
        assert!(world.despawn_ship(entity));
        assert_eq!(world.ship_count(), 0);
    }

    #[test]
    fn despawning_nonexistent_entity_returns_false() {
        let mut world = make_world();
        let entity = world.spawn_ship(make_ship_id(1), Position::ORIGIN, Velocity::ZERO);
        world.despawn_ship(entity);
        assert!(!world.despawn_ship(entity), "second despawn must return false");
    }

    #[test]
    fn spawned_ship_position_is_retrievable_via_inner_world() {
        let mut world    = make_world();
        let target_pos   = Position::new(1.0, 2.0, 3.0);
        let _entity      = world.spawn_ship(make_ship_id(1), target_pos, Velocity::ZERO);

        let mut found = false;
        for (_e, pos) in world.inner().query::<&PositionComp>().iter() {
            assert_eq!(pos.0, target_pos);
            found = true;
        }
        assert!(found, "expected one ship to be found");
    }
}
