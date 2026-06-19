//! `SimWorld` — the single owner of all ECS state within a Sector Node.

use crate::components::{FittingComp, HullComp, IsNpcComp, LockComp, PositionComp, ShipIdComp, ShipStatsComp, ThrustComp, TransitComp, TransitState, VelocityComp, WeaponComp};
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
    /// All ships receive `ThrustComp::ZERO` and `ShipStatsComp::NPC` by default.
    /// Use `set_ship_stats()` to override stats for the player ship.
    ///
    /// The caller is responsible for ensuring `ship_id` is globally unique.
    /// See CLAUDE.md INV-004 and FBD-005.
    pub fn spawn_ship(
        &mut self,
        ship_id : ShipId,
        position: Position,
        velocity: Velocity,
    ) -> Entity {
        let stats = ShipStatsComp::NPC;
        self.inner.spawn((
            ShipIdComp(ship_id),
            PositionComp(position),
            VelocityComp(velocity),
            ThrustComp::ZERO,
            stats,
            FittingComp::empty(),
            HullComp::new(stats.max_shield, stats.max_armor, stats.max_hull),
            WeaponComp::new(),
            LockComp::new(),
            IsNpcComp,
            TransitComp::default(),
        ))
    }

    /// Current Sector Transit state of a Ship (ADR-0014).
    ///
    /// Returns `TransitState::None` if the entity has no `TransitComp`
    /// (should not happen for ships spawned via `spawn_ship`).
    pub fn transit_state(&self, entity: Entity) -> TransitState {
        self.inner.get::<&TransitComp>(entity)
            .map(|c| c.0)
            .unwrap_or_default()
    }

    /// Set the Sector Transit state of a Ship.
    pub fn set_transit_state(&mut self, entity: Entity, state: TransitState) {
        if let Ok(mut comp) = self.inner.get::<&mut TransitComp>(entity) {
            comp.0 = state;
        }
    }

    /// Override the stats (max_speed, mass, inertia_modifier, etc.) for a specific ship.
    ///
    /// Used to designate the player ship with higher performance values.
    pub fn set_ship_stats(&mut self, entity: Entity, stats: ShipStatsComp) {
        if let Ok(mut comp) = self.inner.get::<&mut ShipStatsComp>(entity) {
            *comp = stats;
        }
    }

    /// Despawn a Ship entity.
    ///
    /// Returns `true` if the entity existed, `false` if it was already absent.
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

    /// Whether `entity` is currently tackled (has a non-empty `TackledComp`).
    ///
    /// Used by `can_propose_warp` and `can_propose_jump` to reject commands from
    /// tackled ships (ADR-0024). Single query point so future tackle-type
    /// discrimination (disruptor vs scrambler) is added here only.
    pub fn is_tackled(&self, entity: Entity) -> bool {
        self.inner.get::<&crate::components::TackledComp>(entity).is_ok()
    }

    /// Look up the ECS `Entity` handle for a given `ShipId`.
    ///
    /// Returns `None` if no ship with that ID exists in the world.
    /// O(n) linear scan — call once per operation, not per tick.
    pub fn find_entity(&self, ship_id: ShipId) -> Option<Entity> {
        self.inner.query::<&ShipIdComp>().iter()
            .find(|(_, id)| id.0 == ship_id)
            .map(|(e, _)| e)
    }

    /// Run a read-only ECS query over all entities.
    ///
    /// Prefer this over `inner()` for query access; it avoids exposing the raw
    /// `hecs::World` and keeps the API surface contained.
    pub fn query<Q: hecs::Query>(&self) -> hecs::QueryBorrow<'_, Q> {
        self.inner.query::<Q>()
    }

    /// Read a single component from an entity.
    ///
    /// Returns `None` if the entity does not exist or lacks the component.
    pub fn get<C: hecs::Component>(&self, entity: Entity) -> Option<hecs::Ref<'_, C>> {
        self.inner.get::<&C>(entity).ok()
    }

    /// Mutably access a single component on an entity.
    ///
    /// Returns `None` if the entity does not exist or lacks the component.
    pub fn get_mut<C: hecs::Component>(&mut self, entity: Entity) -> Option<hecs::RefMut<'_, C>> {
        self.inner.get::<&mut C>(entity).ok()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{NodeId, Position, SectorId, Velocity};

    fn make_world() -> SimWorld { SimWorld::new(SectorId(0)) }
    fn make_ship_id(c: u64) -> ShipId { ShipId::new(NodeId(0), c) }

    #[test]
    fn newly_created_world_contains_no_ships() {
        assert_eq!(make_world().ship_count(), 0);
    }

    #[test]
    fn spawned_ship_increments_ship_count() {
        let mut w = make_world();
        w.spawn_ship(make_ship_id(1), Position::ORIGIN, Velocity::ZERO);
        assert_eq!(w.ship_count(), 1);
    }

    #[test]
    fn despawned_ship_decrements_ship_count() {
        let mut w = make_world();
        let e = w.spawn_ship(make_ship_id(1), Position::ORIGIN, Velocity::ZERO);
        assert!(w.despawn_ship(e));
        assert_eq!(w.ship_count(), 0);
    }

    #[test]
    fn despawning_nonexistent_entity_returns_false() {
        let mut w = make_world();
        let e = w.spawn_ship(make_ship_id(1), Position::ORIGIN, Velocity::ZERO);
        w.despawn_ship(e);
        assert!(!w.despawn_ship(e));
    }

    #[test]
    fn spawned_ship_position_is_retrievable_via_inner_world() {
        let mut w  = make_world();
        let target = Position::new(1.0, 2.0, 3.0);
        w.spawn_ship(make_ship_id(1), target, Velocity::ZERO);
        let mut found = false;
        for (_e, pos) in w.inner().query::<&PositionComp>().iter() {
            assert_eq!(pos.0, target);
            found = true;
        }
        assert!(found);
    }

    #[test]
    fn spawned_ship_starts_with_no_transit_in_progress() {
        let mut w = make_world();
        let e = w.spawn_ship(make_ship_id(1), Position::ORIGIN, Velocity::ZERO);
        assert_eq!(w.transit_state(e), TransitState::None);
    }

    #[test]
    fn set_transit_state_marks_ship_in_transit_to_destination_sector() {
        let mut w = make_world();
        let e = w.spawn_ship(make_ship_id(1), Position::ORIGIN, Velocity::ZERO);
        w.set_transit_state(e, TransitState::InTransit { to: SectorId(2) });
        assert_eq!(w.transit_state(e), TransitState::InTransit { to: SectorId(2) });
    }

    #[test]
    fn find_entity_returns_entity_for_known_ship_id() {
        let mut w = make_world();
        let id = make_ship_id(1);
        w.spawn_ship(id, Position::ORIGIN, Velocity::ZERO);
        assert!(w.find_entity(id).is_some());
    }

    #[test]
    fn find_entity_returns_none_for_unknown_ship_id() {
        let w = make_world();
        assert!(w.find_entity(make_ship_id(99)).is_none());
    }

    #[test]
    fn get_returns_component_for_existing_entity() {
        let mut w = make_world();
        let e = w.spawn_ship(make_ship_id(1), Position::ORIGIN, Velocity::ZERO);
        assert!(w.get::<ShipStatsComp>(e).is_some());
    }

    #[test]
    fn get_mut_allows_mutating_component() {
        let mut w = make_world();
        let e = w.spawn_ship(make_ship_id(1), Position::ORIGIN, Velocity::ZERO);
        if let Some(mut stats) = w.get_mut::<ShipStatsComp>(e) {
            stats.max_speed = 9999.0;
        }
        assert_eq!(w.get::<ShipStatsComp>(e).unwrap().max_speed, 9999.0);
    }

    #[test]
    fn ship_stats_can_be_overridden_for_player_ship() {
        let mut w = make_world();
        let e = w.spawn_ship(make_ship_id(1), Position::ORIGIN, Velocity::ZERO);
        w.set_ship_stats(e, ShipStatsComp::PLAYER);
        let stats = *w.inner().get::<&ShipStatsComp>(e).unwrap();
        assert_eq!(stats.max_speed, ShipStatsComp::PLAYER.max_speed);
    }
}
