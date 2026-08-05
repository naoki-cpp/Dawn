//! Shared player-command resolution seam for `SimulationNode` (candidate 1,
//! `/improve-codebase-architecture` review 2026-07-27).
//!
//! Every player-issued ship command needs to answer two questions before its
//! own logic runs: *does this player have authority over this ship*, and
//! *is the ship in the right dock state for this kind of command*. Both
//! answers used to be re-derived by hand in every `apply_*_command_owned`
//! wrapper (`is_active_ship`/`owns_ship` plus a docked check whose polarity
//! flips between the two command families). `resolve_flight_command` and
//! `resolve_fitting_command` are the one place that logic lives now.
//!
//! Once a `resolve_*_command` call succeeds, nothing downstream re-checks
//! ownership or dock state. The fitting methods (`fit_module_owned`,
//! `unfit_module_owned`, `reorder_fitted_module_owned`) use the returned
//! [`ResolvedShip`] directly for its `entity`. The flight `*_owned` wrappers
//! (`apply_warp_command_owned`, etc.) only need the pass/fail outcome --
//! `ship_id` is already in hand from their own parameter, and they delegate
//! to the matching unchecked `apply_*_command` (`apply_warp_command`, ...),
//! which re-derives `Entity` from `ship_id` itself. That's a cheap,
//! side-effect-free HashMap lookup, not a re-check of anything
//! `resolve_flight_command` already decided.

use dawn_core::{PlayerId, ShipId};
use dawn_ecs::Entity;
use dawn_event_store::store::EventStore;

use super::SimulationNode;

/// Why a player command was rejected before it could act on a ship.
///
/// Named so a rejection is a value that can be logged, tested, and matched
/// on, rather than collapsing to a bare `bool` at the call boundary --
/// mirrors `ModuleActivationRejection` (`command_module.rs`), the existing
/// precedent for this in the crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ShipCommandRejection {
    /// `player_id` does not own `ship_id` at all.
    NotOwned,
    /// `player_id` owns `ship_id`, but it isn't their *active* ship
    /// (ADR-0037) -- only relevant to flight/steering commands.
    NotActiveShip,
    /// `ship_id` has no entity in this Sector.
    ShipNotFound,
    /// The command requires the ship to be docked, but it isn't.
    MustBeDocked,
    /// The command requires the ship to be undocked, but it is docked.
    MustBeUndocked,
}

/// A ship a player command has been cleared to act on: ownership and dock
/// state are already verified, so the command body only needs to do its own
/// command-specific work (transit/tackle/target validation, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ResolvedShip {
    pub entity: Entity,
    pub ship_id: ShipId,
}

impl<S: EventStore> SimulationNode<S> {
    /// Resolve a flight/steering command (Move, Stop, Warp, Orbit,
    /// KeepAtRange, Approach): `ship_id` must be `player_id`'s active ship
    /// (ADR-0037), and it must not be docked.
    pub(super) fn resolve_flight_command(
        &self,
        player_id: PlayerId,
        ship_id: ShipId,
    ) -> Result<ResolvedShip, ShipCommandRejection> {
        if !self.owns_ship(player_id, ship_id) {
            return Err(ShipCommandRejection::NotOwned);
        }
        if !self.is_active_ship(player_id, ship_id) {
            return Err(ShipCommandRejection::NotActiveShip);
        }
        if self.is_ship_docked(ship_id) {
            return Err(ShipCommandRejection::MustBeUndocked);
        }
        let &entity = self
            .ships
            .index
            .get(&ship_id)
            .ok_or(ShipCommandRejection::ShipNotFound)?;
        Ok(ResolvedShip { entity, ship_id })
    }

    /// Resolve a station-inventory command (Fit, Unfit,
    /// ReorderFittedModule): `ship_id` must be owned by `player_id` (any
    /// owned ship, not just the active one -- ADR-0037), and it must be
    /// docked.
    pub(super) fn resolve_fitting_command(
        &self,
        player_id: PlayerId,
        ship_id: ShipId,
    ) -> Result<ResolvedShip, ShipCommandRejection> {
        if !self.owns_ship(player_id, ship_id) {
            return Err(ShipCommandRejection::NotOwned);
        }
        if !self.is_ship_docked(ship_id) {
            return Err(ShipCommandRejection::MustBeDocked);
        }
        let &entity = self
            .ships
            .index
            .get(&ship_id)
            .ok_or(ShipCommandRejection::ShipNotFound)?;
        Ok(ResolvedShip { entity, ship_id })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{DockCommand, NodeId, Position, SectorBounds, SectorId, StationId};
    use dawn_event_store::memory::InMemoryEventStore;

    fn mem_node() -> SimulationNode<InMemoryEventStore> {
        SimulationNode::with_store(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            InMemoryEventStore::new(),
        )
    }

    fn spawn_owned_player_at(
        node: &mut SimulationNode<InMemoryEventStore>,
        pos: Position,
    ) -> (PlayerId, ShipId) {
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship_at_pub(player_id, pos);
        (player_id, ship_id)
    }

    /// Spawns and docks an owned player ship at the demo station
    /// (`StationId(0)`), mirroring `inventory.rs`'s `spawn_owned_player`.
    fn spawn_docked_owned_player(
        node: &mut SimulationNode<InMemoryEventStore>,
    ) -> (PlayerId, ShipId) {
        let station = node
            .station(StationId(0))
            .expect("demo station exists")
            .clone();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship_at_pub(player_id, station.position);
        assert!(matches!(
            node.dock_owned(
                player_id,
                ship_id,
                DockCommand {
                    station_id: StationId(0),
                },
            ),
            crate::node::station::StationOperationOutcome::Accepted { .. }
        ));
        (player_id, ship_id)
    }

    #[test]
    fn flight_command_rejects_unowned_ship() {
        let mut node = mem_node();
        let (_owner, ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let stranger = node.next_player_id();
        assert_eq!(
            node.resolve_flight_command(stranger, ship_id),
            Err(ShipCommandRejection::NotOwned)
        );
    }

    #[test]
    fn flight_command_rejects_owned_but_inactive_ship() {
        let mut node = mem_node();
        let player_id = node.next_player_id();
        let first = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);
        let _second = node.spawn_player_ship_at_pub(player_id, Position::new(10.0, 0.0, 0.0));
        // spawn_player_ship_at_pub sets each newly spawned ship active, so
        // `first` is no longer the active ship once `_second` exists.
        assert_eq!(
            node.resolve_flight_command(player_id, first),
            Err(ShipCommandRejection::NotActiveShip)
        );
    }

    #[test]
    fn flight_command_rejects_docked_ship() {
        let mut node = mem_node();
        let (player_id, ship_id) = spawn_docked_owned_player(&mut node);
        assert_eq!(
            node.resolve_flight_command(player_id, ship_id),
            Err(ShipCommandRejection::MustBeUndocked)
        );
    }

    #[test]
    fn flight_command_accepts_active_undocked_owned_ship() {
        let mut node = mem_node();
        let (player_id, ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let resolved = node
            .resolve_flight_command(player_id, ship_id)
            .expect("active undocked owned ship should resolve");
        assert_eq!(resolved.ship_id, ship_id);
    }

    #[test]
    fn fitting_command_rejects_undocked_ship() {
        let mut node = mem_node();
        let (player_id, ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        assert_eq!(
            node.resolve_fitting_command(player_id, ship_id),
            Err(ShipCommandRejection::MustBeDocked)
        );
    }

    #[test]
    fn fitting_command_accepts_any_owned_docked_ship_even_if_inactive() {
        let mut node = mem_node();
        let (player_id, docked_ship) = spawn_docked_owned_player(&mut node);
        // A second owned ship becomes active, so `docked_ship` is owned but
        // no longer the active ship -- fitting commands must still accept
        // it (ADR-0037: Fit/Unfit act on any owned ship, not just active).
        let _active = node.spawn_player_ship_at_pub(player_id, Position::new(10.0, 0.0, 0.0));
        let resolved = node
            .resolve_fitting_command(player_id, docked_ship)
            .expect("owned docked ship should resolve even when inactive");
        assert_eq!(resolved.ship_id, docked_ship);
    }

    #[test]
    fn fitting_command_rejects_unowned_ship() {
        let mut node = mem_node();
        let (_owner, ship_id) = spawn_docked_owned_player(&mut node);
        let stranger = node.next_player_id();
        assert_eq!(
            node.resolve_fitting_command(stranger, ship_id),
            Err(ShipCommandRejection::NotOwned)
        );
    }
}
