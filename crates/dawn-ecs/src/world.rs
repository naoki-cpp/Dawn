//! `SimWorld` — the single owner of all ECS state within a Sector Node.

use crate::components::{
    AnchorComp, FittingComp, HullComp, IsNpcComp, LockComp, PositionComp, ShipIdComp,
    ShipStatsComp, ThrustComp, TransitComp, TransitState, VelocityComp, WeaponComp,
};
use dawn_core::{AnchorId, Position, SectorId, ShipId, Velocity};
use hecs::Entity;

/// Wraps `hecs::World` with a domain-aware API.
///
/// `get`/`get_mut`/`insert_one`/`remove_one`/`query` cover every access
/// pattern `dawn-sector` needs (confirmed by migrating all 226 prior
/// `inner()`/`inner_mut()` call sites there onto them). `inner()`/
/// `inner_mut()` stay `pub(crate)` for this crate's own systems (e.g.
/// `systems::movement`'s `query_mut` over a compound borrow, which the
/// single-component wrappers can't express) -- they are not part of the
/// public API other crates should reach for.
pub struct SimWorld {
    inner: hecs::World,
    sector_id: SectorId,
}

impl std::fmt::Debug for SimWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimWorld")
            .field("sector_id", &self.sector_id)
            .field("ship_count", &self.ship_count())
            .finish_non_exhaustive()
    }
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
        ship_id: ShipId,
        position: Position,
        velocity: Velocity,
    ) -> Entity {
        let stats = ShipStatsComp::NPC;
        self.inner.spawn((
            ShipIdComp(ship_id),
            PositionComp(position),
            // ADR-0029 step 2: anchor defaults to the Sector origin (AnchorId 0,
            // the star); the node overrides it per-Sector via set_ship_anchor.
            AnchorComp(AnchorId(0)),
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
        self.inner
            .get::<&TransitComp>(entity)
            .map(|c| c.0)
            .unwrap_or_default()
    }

    /// Set the Sector Transit state of a Ship.
    pub fn set_transit_state(&mut self, entity: Entity, state: TransitState) {
        if let Ok(mut comp) = self.inner.get::<&mut TransitComp>(entity) {
            comp.0 = state;
        }
    }

    /// Set the coordinate anchor a Ship's position offset is relative to
    /// (ADR-0029). Used at spawn/restore (Sector origin anchor) and on warp
    /// arrival (rebase to the destination body's anchor).
    pub fn set_ship_anchor(&mut self, entity: Entity, anchor: AnchorId) {
        if let Ok(mut comp) = self.inner.get::<&mut AnchorComp>(entity) {
            comp.0 = anchor;
        }
    }

    /// The coordinate anchor a Ship's position offset is relative to, or `None`
    /// if the entity has no `AnchorComp`.
    pub fn ship_anchor(&self, entity: Entity) -> Option<AnchorId> {
        self.inner.get::<&AnchorComp>(entity).ok().map(|c| c.0)
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

    /// Escape hatch to the underlying `hecs::World`, for this crate's own
    /// tests only (`get`/`get_mut`/`query` already cover every read-only
    /// need production systems have) -- external callers should use those
    /// wrappers instead.
    #[cfg(test)]
    pub(crate) fn inner(&self) -> &hecs::World {
        &self.inner
    }

    /// Mutable counterpart of [`Self::inner`], for compound-borrow queries
    /// (e.g. `query_mut` over several components at once) the single-
    /// component wrappers can't express.
    pub(crate) fn inner_mut(&mut self) -> &mut hecs::World {
        &mut self.inner
    }

    /// Whether `entity` is currently tackled (has a non-empty `TackledComp`).
    ///
    /// Used by `can_propose_warp` and `can_propose_jump` to reject commands from
    /// tackled ships (ADR-0024). Single query point so future tackle-type
    /// discrimination (disruptor vs scrambler) is added here only.
    pub fn is_tackled(&self, entity: Entity) -> bool {
        self.inner
            .get::<&crate::components::TackledComp>(entity)
            .is_ok()
    }

    /// Look up the ECS `Entity` handle for a given `ShipId`.
    ///
    /// Returns `None` if no ship with that ID exists in the world.
    /// O(n) linear scan — call once per operation, not per tick.
    pub fn find_entity(&self, ship_id: ShipId) -> Option<Entity> {
        self.inner
            .query::<(hecs::Entity, &ShipIdComp)>()
            .iter()
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

    /// Add a component to an existing entity, replacing it if already present.
    ///
    /// Returns `false` if the entity does not exist.
    pub fn insert_one<C: hecs::Component>(&mut self, entity: Entity, component: C) -> bool {
        self.inner.insert_one(entity, component).is_ok()
    }

    /// Remove a component from an entity and return it.
    ///
    /// Returns `None` if the entity does not exist or lacks the component.
    pub fn remove_one<C: hecs::Component>(&mut self, entity: Entity) -> Option<C> {
        self.inner.remove_one::<C>(entity).ok()
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
    fn make_ship_id(c: u64) -> ShipId {
        ShipId::new(NodeId(0), c)
    }

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
        let mut w = make_world();
        let target = Position::new(1.0, 2.0, 3.0);
        w.spawn_ship(make_ship_id(1), target, Velocity::ZERO);
        let mut found = false;
        for pos in w.inner().query::<&PositionComp>().iter() {
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
        assert_eq!(
            w.transit_state(e),
            TransitState::InTransit { to: SectorId(2) }
        );
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
    fn insert_one_adds_a_component_not_present_at_spawn() {
        let mut w = make_world();
        let e = w.spawn_ship(make_ship_id(1), Position::ORIGIN, Velocity::ZERO);
        assert!(!w.is_tackled(e), "ships spawn untackled");
        assert!(w.insert_one(e, crate::components::TackledComp { tacklers: vec![] }));
        assert!(w.is_tackled(e));
    }

    #[test]
    fn insert_one_returns_false_for_a_nonexistent_entity() {
        let mut w = make_world();
        let e = w.spawn_ship(make_ship_id(1), Position::ORIGIN, Velocity::ZERO);
        w.despawn_ship(e);
        assert!(!w.insert_one(e, crate::components::TackledComp { tacklers: vec![] }));
    }

    #[test]
    fn remove_one_takes_the_component_back_out() {
        let mut w = make_world();
        let e = w.spawn_ship(make_ship_id(1), Position::ORIGIN, Velocity::ZERO);
        w.insert_one(e, crate::components::TackledComp { tacklers: vec![] });
        let removed = w.remove_one::<crate::components::TackledComp>(e);
        assert!(removed.is_some());
        assert!(!w.is_tackled(e), "component is gone after remove_one");
    }

    #[test]
    fn remove_one_returns_none_when_the_component_is_absent() {
        let mut w = make_world();
        let e = w.spawn_ship(make_ship_id(1), Position::ORIGIN, Velocity::ZERO);
        assert!(w.remove_one::<crate::components::TackledComp>(e).is_none());
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
