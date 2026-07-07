//! NPC station access and per-player station storage (ADR-0034 9B foundation).
//!
//! This module deliberately stops at the foundation layer:
//! - static station lookup
//! - "can this player use this station right now?" validation
//! - minimal per-player station inventory storage
//!
//! Assemble/Disassemble/build commands layer on top of these helpers in later
//! 9B slices.

use std::collections::BTreeMap;

use dawn_core::{
    events::{PackagedShipBuilt, ShipAssembled, ShipDisassembled, ShipDocked, ShipUndocked},
    AssembleCommand, BuildPackagedShipCommand, DisassembleShipCommand, DockCommand, DomainEvent,
    ItemId, PlayerId, Position, ShipId, StationId, Velocity,
};
use dawn_ecs::components::{
    FittingComp, HullComp, IsNpcComp, LockComp, ShipStatsComp, ThrustComp, VelocityComp, WarpComp,
};
use dawn_event_store::store::EventStore;

use super::SimulationNode;

/// The station command seam: callers learn whether the operation was accepted
/// and which ship should have its fitting/state resent to the client.
///
/// Callers no longer extract `ship_id` for a `RefreshFitting` followup --
/// `apply_client_command` already has `player_id` in scope and uses that
/// directly (a `ship_id` can't always be resolved back to a player, e.g.
/// after Disassemble removes it; see `docs/architecture/ownership.md` §8).
/// `ship_id` is kept because callers (tests, other station ops) still match
/// on which ship an operation targeted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StationOperationOutcome {
    Accepted {
        ship_id: ShipId,
    },
    Rejected {
        ship_id: ShipId,
        reason: StationOperationRejection,
    },
}

/// Why a station operation was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StationOperationRejection {
    NotOwned,
    AlreadyDocked,
    OutOfDockRange,
    ShipNotDocked,
    MissingDockedStationContext,
    WrongDockedStation,
    UnknownShipType,
    MissingStationItem,
    InsufficientStationItem,
    ShipNotFound,
    ShipIsFitted,
    ShipIsDamaged,
    /// `SelectActiveShipCommand` targeted the already-active ship (ADR-0037).
    AlreadyActive,
    /// `SelectActiveShipCommand` targeted a ship not docked at the caller's
    /// current docked station (ADR-0037; station-local switch only).
    ShipNotDockedHere,
}

impl<S: EventStore> SimulationNode<S> {
    pub const SCRAP_METAL_COST_PER_PACKAGED_SHIP: u64 = 1;

    /// True when `player_id`'s ship is currently docked at `station_id`.
    pub fn can_use_station(&self, player_id: PlayerId, station_id: StationId) -> bool {
        self.docked_players.get(&player_id).copied() == Some(station_id)
    }

    /// True when `ship_id` is spatially eligible to dock at `station_id`.
    pub fn can_dock_station(&self, ship_id: ShipId, station_id: StationId) -> bool {
        let station = match self.station(station_id) {
            Some(station) => station,
            None => return false,
        };
        let ship_abs = match self.ship_absolute(ship_id) {
            Some(ship_abs) => ship_abs,
            None => return false,
        };
        station.is_in_range_abs(ship_abs)
    }

    pub fn docked_station(&self, ship_id: ShipId) -> Option<StationId> {
        self.docked_ships.get(&ship_id).copied()
    }

    pub fn is_ship_docked(&self, ship_id: ShipId) -> bool {
        self.docked_ships.contains_key(&ship_id)
    }

    pub fn player_docked_station(&self, player_id: PlayerId) -> Option<StationId> {
        self.docked_players.get(&player_id).copied()
    }

    pub(super) fn settle_ship_into_station(&mut self, ship_id: ShipId, station_id: StationId) {
        let Some(station_abs) = self.station(station_id).map(|station| station.abs_m) else {
            return;
        };
        let Some(&entity) = self.ships.index.get(&ship_id) else {
            return;
        };
        let _ = self.world.inner_mut().remove_one::<WarpComp>(entity);
        self.clear_steering_modes(entity);
        if let Ok(mut thrust) = self.world.inner_mut().get::<&mut ThrustComp>(entity) {
            *thrust = ThrustComp::ZERO;
        }
        if let Ok(mut velocity) = self.world.inner_mut().get::<&mut VelocityComp>(entity) {
            velocity.0 = dawn_core::Velocity::ZERO;
        }
        self.place_entity_at_absolute(entity, station_abs);
    }

    /// Dock the caller's active ship (ADR-0037: `ship_id` is always the
    /// caller's resolved active ship — `apply_client_command` never calls
    /// this without one).
    pub(super) fn dock_owned(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
        cmd: DockCommand,
    ) -> StationOperationOutcome {
        if !self.is_active_ship(player_id, ship_id) {
            return StationOperationOutcome::Rejected {
                ship_id,
                reason: StationOperationRejection::NotOwned,
            };
        }
        if self.docked_ships.contains_key(&ship_id) {
            return StationOperationOutcome::Rejected {
                ship_id,
                reason: StationOperationRejection::AlreadyDocked,
            };
        }
        if !self.can_dock_station(ship_id, cmd.station_id) {
            return StationOperationOutcome::Rejected {
                ship_id,
                reason: StationOperationRejection::OutOfDockRange,
            };
        }
        self.settle_ship_into_station(ship_id, cmd.station_id);
        self.docked_ships.insert(ship_id, cmd.station_id);
        self.docked_players.insert(player_id, cmd.station_id);
        self.event_store.append(DomainEvent::ShipDocked(ShipDocked {
            ship_id,
            station_id: cmd.station_id,
            tick: self.current_tick,
        }));
        StationOperationOutcome::Accepted { ship_id }
    }

    /// Undock the caller's active ship (ADR-0037: only the active ship may
    /// leave dock — `ship_id` is always the caller's resolved active ship).
    pub(super) fn undock_owned(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
    ) -> StationOperationOutcome {
        if !self.is_active_ship(player_id, ship_id) {
            return StationOperationOutcome::Rejected {
                ship_id,
                reason: StationOperationRejection::NotOwned,
            };
        }
        let Some(station_id) = self.docked_ships.remove(&ship_id) else {
            return StationOperationOutcome::Rejected {
                ship_id,
                reason: StationOperationRejection::ShipNotDocked,
            };
        };
        self.docked_players.remove(&player_id);
        self.event_store
            .append(DomainEvent::ShipUndocked(ShipUndocked {
                ship_id,
                station_id,
                tick: self.current_tick,
            }));
        StationOperationOutcome::Accepted { ship_id }
    }

    /// Switch the caller's active ship to another owned ship docked at the
    /// same station (ADR-0037). Station-local only for now: `ship_id` must
    /// be docked where the caller is currently docked.
    pub(super) fn select_active_ship_owned(
        &mut self,
        player_id: PlayerId,
        cmd: dawn_core::SelectActiveShipCommand,
    ) -> StationOperationOutcome {
        if !self.owns_ship(player_id, cmd.ship_id) {
            return StationOperationOutcome::Rejected {
                ship_id: cmd.ship_id,
                reason: StationOperationRejection::NotOwned,
            };
        }
        if self.ships.active_ship.get(&player_id) == Some(&cmd.ship_id) {
            return StationOperationOutcome::Rejected {
                ship_id: cmd.ship_id,
                reason: StationOperationRejection::AlreadyActive,
            };
        }
        let target_station = self.docked_ships.get(&cmd.ship_id).copied();
        if target_station.is_none()
            || target_station != self.docked_players.get(&player_id).copied()
        {
            return StationOperationOutcome::Rejected {
                ship_id: cmd.ship_id,
                reason: StationOperationRejection::ShipNotDockedHere,
            };
        }
        self.ships.active_ship.insert(player_id, cmd.ship_id);
        StationOperationOutcome::Accepted {
            ship_id: cmd.ship_id,
        }
    }

    /// Clear the caller's active ship while docked, without disassembling it
    /// or changing ownership (ADR-0037). Like `assemble_ship_owned`, there is
    /// no pre-existing ship_id to report on the "no active ship" rejection,
    /// so this returns `Result<ShipId, _>` rather than
    /// `StationOperationOutcome`. Session-local, not event-sourced (same tier
    /// as `select_active_ship_owned`) -- see `docs/architecture/ownership.md` §8.
    pub(super) fn disembark_owned(
        &mut self,
        player_id: PlayerId,
    ) -> Result<ShipId, StationOperationRejection> {
        let Some(ship_id) = self.ships.active_ship.get(&player_id).copied() else {
            return Err(StationOperationRejection::NotOwned);
        };
        if !self.is_ship_docked(ship_id) {
            return Err(StationOperationRejection::ShipNotDocked);
        }
        self.ships.active_ship.remove(&player_id);
        Ok(ship_id)
    }

    pub fn clear_docked_lock_targets(&mut self, tick: dawn_core::Tick) -> Vec<DomainEvent> {
        let mut events: Vec<DomainEvent> = Vec::new();
        let ship_ids: Vec<ShipId> = self.ships.index.keys().copied().collect();
        for ship_id in ship_ids {
            let Some(&entity) = self.ships.index.get(&ship_id) else {
                continue;
            };
            let locker_docked = self.is_ship_docked(ship_id);
            let lost_targets: Vec<ShipId> = match self.world.inner().get::<&LockComp>(entity) {
                Ok(lock) => lock
                    .entries
                    .iter()
                    .filter_map(|entry| {
                        if locker_docked || self.is_ship_docked(entry.target_id) {
                            Some(entry.target_id)
                        } else {
                            None
                        }
                    })
                    .collect(),
                Err(_) => Vec::new(),
            };
            if lost_targets.is_empty() {
                continue;
            }
            if let Ok(mut lock) = self.world.inner_mut().get::<&mut LockComp>(entity) {
                lock.entries
                    .retain(|entry| !lost_targets.contains(&entry.target_id));
            }
            for target_id in lost_targets {
                events.push(DomainEvent::LockLost(dawn_core::events::LockLost {
                    locker_id: ship_id,
                    target_id,
                    tick,
                }));
            }
        }
        events
    }

    /// Immutable view of the player's station inventory in this Sector.
    pub fn station_inventory(&self, player_id: PlayerId) -> Option<&BTreeMap<ItemId, u64>> {
        self.station_inventory_storage().get(&player_id)
    }

    /// Count one item stack inside the player's station inventory.
    pub fn station_item_count(&self, player_id: PlayerId, item_id: ItemId) -> u64 {
        self.station_inventory(player_id)
            .and_then(|inv| inv.get(&item_id).copied())
            .unwrap_or(0)
    }

    /// The current station-inventory storage backend.
    ///
    /// Today this is an in-memory `BTreeMap`, but callers go through this seam
    /// so the storage can later become lazy-loaded or DB-backed without every
    /// station command reaching into the raw field directly.
    pub fn station_inventory_storage(&self) -> &BTreeMap<PlayerId, BTreeMap<ItemId, u64>> {
        &self.station_inventories
    }

    pub(super) fn station_inventory_storage_mut(
        &mut self,
    ) -> &mut BTreeMap<PlayerId, BTreeMap<ItemId, u64>> {
        &mut self.station_inventories
    }

    pub(super) fn replace_station_inventory_storage(
        &mut self,
        inventories: BTreeMap<PlayerId, BTreeMap<ItemId, u64>>,
    ) {
        self.station_inventories = inventories;
    }

    fn station_inventory_mut(&mut self, player_id: PlayerId) -> Option<&mut BTreeMap<ItemId, u64>> {
        self.station_inventory_storage_mut().get_mut(&player_id)
    }

    #[cfg(test)]
    pub(crate) fn replace_station_inventory(
        &mut self,
        player_id: PlayerId,
        inventory: BTreeMap<ItemId, u64>,
    ) {
        self.station_inventory_storage_mut()
            .insert(player_id, inventory);
    }

    /// Add `count` of `item_id` to the player's station inventory.
    pub fn credit_station_item(&mut self, player_id: PlayerId, item_id: ItemId, count: u64) {
        if count == 0 {
            return;
        }
        *self
            .station_inventory_storage_mut()
            .entry(player_id)
            .or_default()
            .entry(item_id)
            .or_default() += count;
    }

    pub(super) fn try_debit_station_item(
        &mut self,
        player_id: PlayerId,
        item_id: ItemId,
        count: u64,
    ) -> Result<(), StationOperationRejection> {
        if count == 0 {
            return Ok(());
        }
        let Some(inv) = self.station_inventory_mut(player_id) else {
            return Err(StationOperationRejection::MissingStationItem);
        };
        let Some(stack) = inv.get_mut(&item_id) else {
            return Err(StationOperationRejection::MissingStationItem);
        };
        if *stack < count {
            return Err(StationOperationRejection::InsufficientStationItem);
        }
        *stack -= count;
        if *stack == 0 {
            inv.remove(&item_id);
        }
        Ok(())
    }

    pub(super) fn build_packaged_ship_owned(
        &mut self,
        player_id: PlayerId,
        cmd: BuildPackagedShipCommand,
    ) -> StationOperationOutcome {
        if !self.owns_ship(player_id, cmd.ship_id) {
            return StationOperationOutcome::Rejected {
                ship_id: cmd.ship_id,
                reason: StationOperationRejection::NotOwned,
            };
        }
        if !self.can_use_station(player_id, cmd.station_id) {
            return StationOperationOutcome::Rejected {
                ship_id: cmd.ship_id,
                reason: StationOperationRejection::MissingDockedStationContext,
            };
        }
        if !self.ship_type_registry.contains_key(&cmd.ship_type_id) {
            return StationOperationOutcome::Rejected {
                ship_id: cmd.ship_id,
                reason: StationOperationRejection::UnknownShipType,
            };
        }
        let scrap_cost = Self::SCRAP_METAL_COST_PER_PACKAGED_SHIP;
        if let Err(reason) = self.try_debit_station_item(player_id, ItemId::ScrapMetal, scrap_cost)
        {
            return StationOperationOutcome::Rejected {
                ship_id: cmd.ship_id,
                reason,
            };
        }
        self.credit_station_item(player_id, ItemId::PackagedShip(cmd.ship_type_id), 1);
        self.event_store
            .append(DomainEvent::PackagedShipBuilt(PackagedShipBuilt {
                ship_id: cmd.ship_id,
                player_id,
                station_id: cmd.station_id,
                ship_type_id: cmd.ship_type_id,
                scrap_cost,
                tick: self.current_tick,
            }));
        StationOperationOutcome::Accepted {
            ship_id: cmd.ship_id,
        }
    }

    pub(super) fn disassemble_ship_owned(
        &mut self,
        player_id: PlayerId,
        cmd: DisassembleShipCommand,
    ) -> StationOperationOutcome {
        if !self.owns_ship(player_id, cmd.ship_id) {
            return StationOperationOutcome::Rejected {
                ship_id: cmd.ship_id,
                reason: StationOperationRejection::NotOwned,
            };
        }
        if self.player_docked_station(player_id) != Some(cmd.station_id) {
            return StationOperationOutcome::Rejected {
                ship_id: cmd.ship_id,
                reason: StationOperationRejection::MissingDockedStationContext,
            };
        }
        if self.docked_station(cmd.ship_id) != Some(cmd.station_id) {
            return StationOperationOutcome::Rejected {
                ship_id: cmd.ship_id,
                reason: StationOperationRejection::WrongDockedStation,
            };
        }
        let Some(&entity) = self.ships.index.get(&cmd.ship_id) else {
            return StationOperationOutcome::Rejected {
                ship_id: cmd.ship_id,
                reason: StationOperationRejection::ShipNotFound,
            };
        };
        let is_fitted = {
            let Ok(fitting) = self.world.inner().get::<&FittingComp>(entity) else {
                return StationOperationOutcome::Rejected {
                    ship_id: cmd.ship_id,
                    reason: StationOperationRejection::ShipNotFound,
                };
            };
            let has_any_fitted_module = fitting.iter_slots().next().is_some();
            has_any_fitted_module
        };
        if is_fitted {
            return StationOperationOutcome::Rejected {
                ship_id: cmd.ship_id,
                reason: StationOperationRejection::ShipIsFitted,
            };
        }
        let is_damaged = {
            let Ok(hull) = self.world.inner().get::<&HullComp>(entity) else {
                return StationOperationOutcome::Rejected {
                    ship_id: cmd.ship_id,
                    reason: StationOperationRejection::ShipNotFound,
                };
            };
            let Ok(stats) = self.world.inner().get::<&ShipStatsComp>(entity) else {
                return StationOperationOutcome::Rejected {
                    ship_id: cmd.ship_id,
                    reason: StationOperationRejection::ShipNotFound,
                };
            };
            hull.is_destroyed()
                || hull.shield() != stats.max_shield
                || hull.armor() != stats.max_armor
                || hull.hull() != stats.max_hull
        };
        if is_damaged {
            return StationOperationOutcome::Rejected {
                ship_id: cmd.ship_id,
                reason: StationOperationRejection::ShipIsDamaged,
            };
        }
        let Some(ship_type_id) = self.ships.type_ids.get(&cmd.ship_id).copied() else {
            return StationOperationOutcome::Rejected {
                ship_id: cmd.ship_id,
                reason: StationOperationRejection::ShipNotFound,
            };
        };

        self.credit_station_item(player_id, ItemId::PackagedShip(ship_type_id), 1);
        self.remove_ship(cmd.ship_id);
        self.event_store
            .append(DomainEvent::ShipDisassembled(ShipDisassembled {
                ship_id: cmd.ship_id,
                player_id,
                station_id: cmd.station_id,
                ship_type_id,
                tick: self.current_tick,
            }));
        StationOperationOutcome::Accepted {
            ship_id: cmd.ship_id,
        }
    }

    /// Convert a station-inventory `PackagedShip` item into a new live docked
    /// ship, owned by `player_id` (ADR-0034 9B, ADR-0037). Unlike the other
    /// station operations, there is no pre-existing `ship_id` to reject
    /// against on failure, so this returns `Result<ShipId, _>` rather than
    /// `StationOperationOutcome`. Does not change `active_ship` -- a later
    /// `SelectActiveShipCommand` makes the new ship active (see
    /// `docs/architecture/ownership.md` §7-8).
    pub(super) fn assemble_ship_owned(
        &mut self,
        player_id: PlayerId,
        cmd: AssembleCommand,
    ) -> Result<ShipId, StationOperationRejection> {
        if !self.can_use_station(player_id, cmd.station_id) {
            return Err(StationOperationRejection::MissingDockedStationContext);
        }
        if !self.ship_type_registry.contains_key(&cmd.ship_type_id) {
            return Err(StationOperationRejection::UnknownShipType);
        }
        self.try_debit_station_item(player_id, ItemId::PackagedShip(cmd.ship_type_id), 1)?;

        let ship_id = ShipId::new(self.node_id, self.id_counter);
        self.id_counter += 1;
        self.insert_ship_entity(ship_id, cmd.ship_type_id, Position::ORIGIN, Velocity::ZERO);
        if let Some(&entity) = self.ships.index.get(&ship_id) {
            let _ = self.world.inner_mut().remove_one::<IsNpcComp>(entity);
        }
        self.settle_ship_into_station(ship_id, cmd.station_id);
        self.docked_ships.insert(ship_id, cmd.station_id);
        self.ships.owners.insert(ship_id, player_id);

        self.event_store
            .append(DomainEvent::ShipAssembled(ShipAssembled {
                ship_id,
                player_id,
                station_id: cmd.station_id,
                ship_type_id: cmd.ship_type_id,
                tick: self.current_tick,
            }));
        Ok(ship_id)
    }
}

#[cfg(test)]
mod tests {
    use dawn_core::{ItemId, NodeId, SectorBounds, SectorId, StationId, WarpTarget};
    use dawn_event_store::InMemoryEventStore;

    use crate::{modules, ship_types};

    use super::*;

    fn accepted(outcome: StationOperationOutcome) -> bool {
        matches!(outcome, StationOperationOutcome::Accepted { .. })
    }

    fn node() -> SimulationNode<InMemoryEventStore> {
        let mut node = SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        );
        for def in modules::all_modules() {
            node.register_module(def);
        }
        for def in ship_types::all_ship_types() {
            node.register_ship_type(def);
        }
        node
    }

    #[test]
    fn player_can_dock_a_station_when_inside_its_docking_radius() {
        let mut node = node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);
        let station = node.station(StationId(0)).expect("demo station exists");
        node.set_spawn_anchor_abs(ship_id, station.abs_m);

        assert!(node.can_dock_station(ship_id, StationId(0)));
    }

    #[test]
    fn player_cannot_dock_a_station_when_out_of_range() {
        let mut node = node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);
        let station = node.station(StationId(0)).expect("demo station exists");
        node.set_spawn_anchor_abs(
            ship_id,
            [
                station.abs_m[0] + station.docking_radius as f64 + 5000.0,
                station.abs_m[1],
                station.abs_m[2],
            ],
        );

        assert!(!node.can_dock_station(ship_id, StationId(0)));
    }

    #[test]
    fn warping_to_the_host_planet_lands_within_dock_range_of_the_local_station() {
        let mut node = node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);

        assert!(node.apply_warp_command_owned(
            player_id,
            ship_id,
            dawn_core::WarpCommand {
                target: WarpTarget::Body(dawn_core::CelestialBodyId(1)),
            }
        ));

        for _ in 0..5_000 {
            node.tick();
            if node.warp_phase(ship_id).is_none() {
                break;
            }
        }

        assert!(
            node.can_dock_station(ship_id, StationId(0)),
            "Forge warp arrival should be close enough to dock at Forge Station"
        );
    }

    #[test]
    fn docking_zeroes_velocity_and_thrust() {
        let mut node = node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);
        let station = node.station(StationId(0)).expect("demo station exists");
        node.set_spawn_anchor_abs(ship_id, station.abs_m);
        let entity = *node.ships.index.get(&ship_id).expect("ship entity");
        if let Ok(mut velocity) = node.world.inner_mut().get::<&mut VelocityComp>(entity) {
            velocity.0 = dawn_core::Velocity {
                dx: 10.0,
                dy: 0.0,
                dz: 0.0,
            };
        }
        if let Ok(mut thrust) = node.world.inner_mut().get::<&mut ThrustComp>(entity) {
            thrust.direction = dawn_core::Velocity {
                dx: 1.0,
                dy: 0.0,
                dz: 0.0,
            };
            thrust.is_braking = false;
        }

        assert!(accepted(node.dock_owned(
            player_id,
            ship_id,
            DockCommand {
                station_id: StationId(0),
            }
        )));

        let velocity = node.world.inner().get::<&VelocityComp>(entity).unwrap().0;
        let thrust = *node.world.inner().get::<&ThrustComp>(entity).unwrap();
        assert_eq!(velocity, dawn_core::Velocity::ZERO);
        assert_eq!(thrust.direction, ThrustComp::ZERO.direction);
        assert_eq!(thrust.is_braking, ThrustComp::ZERO.is_braking);
    }

    #[test]
    fn docked_ship_rejects_movement_commands() {
        let mut node = node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);
        let station = node.station(StationId(0)).expect("demo station exists");
        node.set_spawn_anchor_abs(ship_id, station.abs_m);
        let entity = *node.ships.index.get(&ship_id).expect("ship entity");
        assert!(accepted(node.dock_owned(
            player_id,
            ship_id,
            DockCommand {
                station_id: StationId(0),
            }
        )));

        let before = *node.world.inner().get::<&ThrustComp>(entity).unwrap();
        assert!(node.apply_move_command_owned(
            player_id,
            ship_id,
            dawn_core::Position::new(5000.0, 0.0, 0.0)
        ));
        let after = *node.world.inner().get::<&ThrustComp>(entity).unwrap();
        assert_eq!(
            before.direction, after.direction,
            "docked ships should ignore manual piloting"
        );
        assert_eq!(
            before.is_braking, after.is_braking,
            "docked ships should ignore manual piloting"
        );
    }

    #[test]
    fn docked_ship_rejects_warp_commands() {
        let mut node = node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);
        let station = node.station(StationId(0)).expect("demo station exists");
        node.set_spawn_anchor_abs(ship_id, station.abs_m);
        assert!(accepted(node.dock_owned(
            player_id,
            ship_id,
            DockCommand {
                station_id: StationId(0),
            }
        )));

        assert!(!node.apply_warp_command_owned(
            player_id,
            ship_id,
            dawn_core::WarpCommand {
                target: WarpTarget::Body(dawn_core::CelestialBodyId(1)),
            }
        ));
    }

    #[test]
    fn docking_snaps_the_ship_to_the_station_and_prevents_drift() {
        let mut node = node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);
        let station_abs = node
            .station(StationId(0))
            .expect("demo station exists")
            .abs_m;
        node.set_spawn_anchor_abs(
            ship_id,
            [station_abs[0] + 1000.0, station_abs[1], station_abs[2]],
        );
        let entity = *node.ships.index.get(&ship_id).expect("ship entity");
        if let Ok(mut velocity) = node.world.inner_mut().get::<&mut VelocityComp>(entity) {
            velocity.0 = dawn_core::Velocity {
                dx: 250.0,
                dy: 0.0,
                dz: 0.0,
            };
        }

        assert!(accepted(node.dock_owned(
            player_id,
            ship_id,
            DockCommand {
                station_id: StationId(0),
            }
        )));

        let after_dock = node
            .ship_absolute(ship_id)
            .expect("ship exists after docking");
        let dock_error = ((after_dock[0] - station_abs[0]).powi(2)
            + (after_dock[1] - station_abs[1]).powi(2)
            + (after_dock[2] - station_abs[2]).powi(2))
        .sqrt();
        assert!(
            dock_error < 1.0,
            "docking should snap the ship to the station position (error={dock_error}m)"
        );

        for _ in 0..5 {
            node.tick();
        }

        let after_ticks = node
            .ship_absolute(ship_id)
            .expect("ship exists after docked ticks");
        let tick_error = ((after_ticks[0] - station_abs[0]).powi(2)
            + (after_ticks[1] - station_abs[1]).powi(2)
            + (after_ticks[2] - station_abs[2]).powi(2))
        .sqrt();
        assert!(
            tick_error < 1.0,
            "docked ships should remain fixed at the station position (error={tick_error}m)"
        );
    }

    #[test]
    fn undocking_from_a_docked_station_allows_immediate_redock() {
        let mut node = node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);
        let station_abs = node
            .station(StationId(0))
            .expect("demo station exists")
            .abs_m;
        node.set_spawn_anchor_abs(
            ship_id,
            [station_abs[0] + 1000.0, station_abs[1], station_abs[2]],
        );

        assert!(accepted(node.dock_owned(
            player_id,
            ship_id,
            DockCommand {
                station_id: StationId(0),
            }
        )));
        node.tick();
        assert!(accepted(node.undock_owned(player_id, ship_id)));

        assert!(
            node.can_dock_station(ship_id, StationId(0)),
            "undocked ships should still be in docking range of the station they just left"
        );
        assert!(accepted(node.dock_owned(
            player_id,
            ship_id,
            DockCommand {
                station_id: StationId(0),
            }
        )));
    }

    #[test]
    fn can_use_station_requires_an_existing_dock_state() {
        let mut node = node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);
        let station = node.station(StationId(0)).expect("demo station exists");
        node.set_spawn_anchor_abs(ship_id, station.abs_m);

        assert!(!node.can_use_station(player_id, StationId(0)));
        assert!(accepted(node.dock_owned(
            player_id,
            ship_id,
            DockCommand {
                station_id: StationId(0),
            }
        )));
        assert!(node.can_use_station(player_id, StationId(0)));
    }

    #[test]
    fn undock_removes_station_access() {
        let mut node = node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);
        let station = node.station(StationId(0)).expect("demo station exists");
        node.set_spawn_anchor_abs(ship_id, station.abs_m);
        assert!(accepted(node.dock_owned(
            player_id,
            ship_id,
            DockCommand {
                station_id: StationId(0),
            }
        )));

        assert!(accepted(node.undock_owned(player_id, ship_id)));
        assert!(!node.can_use_station(player_id, StationId(0)));
    }

    #[test]
    fn station_access_is_anchored_to_player_docked_context() {
        let mut node = node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);
        let station = node.station(StationId(0)).expect("demo station exists");
        node.set_spawn_anchor_abs(ship_id, station.abs_m);
        assert!(accepted(node.dock_owned(
            player_id,
            ship_id,
            DockCommand {
                station_id: StationId(0),
            }
        )));

        node.ships.active_ship.remove(&player_id);

        assert_eq!(node.player_docked_station(player_id), Some(StationId(0)));
        assert!(node.can_use_station(player_id, StationId(0)));
    }

    #[test]
    fn station_inventory_tracks_items_per_player() {
        let mut node = node();
        let player_a = PlayerId(1);
        let player_b = PlayerId(2);

        node.credit_station_item(player_a, ItemId::ScrapMetal, 3);
        node.credit_station_item(player_a, ItemId::ScrapMetal, 2);
        node.credit_station_item(player_b, ItemId::PackagedShip(dawn_core::ShipTypeId(1)), 1);

        assert_eq!(node.station_item_count(player_a, ItemId::ScrapMetal), 5);
        assert_eq!(
            node.station_item_count(player_b, ItemId::PackagedShip(dawn_core::ShipTypeId(1))),
            1
        );
        assert_eq!(node.station_item_count(player_b, ItemId::ScrapMetal), 0);
    }

    #[test]
    fn try_debit_station_item_rejects_missing_or_insufficient_stacks() {
        let mut node = node();
        let player_id = PlayerId(1);
        node.credit_station_item(player_id, ItemId::ScrapMetal, 2);

        assert!(matches!(
            node.try_debit_station_item(
                player_id,
                ItemId::PackagedShip(dawn_core::ShipTypeId(1)),
                1
            ),
            Err(StationOperationRejection::MissingStationItem)
        ));
        assert!(matches!(
            node.try_debit_station_item(player_id, ItemId::ScrapMetal, 3),
            Err(StationOperationRejection::InsufficientStationItem)
        ));
        assert_eq!(node.station_item_count(player_id, ItemId::ScrapMetal), 2);
        assert!(node
            .try_debit_station_item(player_id, ItemId::ScrapMetal, 2)
            .is_ok());
        assert_eq!(node.station_item_count(player_id, ItemId::ScrapMetal), 0);
    }

    #[test]
    fn build_packaged_ship_consumes_scrap_and_credits_the_packaged_hull() {
        let mut node = node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);
        let station = node.station(StationId(0)).expect("demo station exists");
        node.set_spawn_anchor_abs(ship_id, station.abs_m);
        assert!(accepted(node.dock_owned(
            player_id,
            ship_id,
            DockCommand {
                station_id: StationId(0),
            }
        )));
        node.credit_station_item(
            player_id,
            ItemId::ScrapMetal,
            SimulationNode::<InMemoryEventStore>::SCRAP_METAL_COST_PER_PACKAGED_SHIP,
        );

        assert!(accepted(node.build_packaged_ship_owned(
            player_id,
            BuildPackagedShipCommand {
                ship_id,
                station_id: StationId(0),
                ship_type_id: crate::ship_types::SHIP_TYPE_MAGPIE,
            }
        )));
        assert_eq!(node.station_item_count(player_id, ItemId::ScrapMetal), 0);
        assert_eq!(
            node.station_item_count(
                player_id,
                ItemId::PackagedShip(crate::ship_types::SHIP_TYPE_MAGPIE)
            ),
            // 1 from the starter Packaged Ship every new player is granted
            // (spawn_player_ship_at) + 1 just built here.
            2
        );
    }

    #[test]
    fn build_packaged_ship_is_rejected_when_not_docked() {
        let mut node = node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);
        node.credit_station_item(player_id, ItemId::ScrapMetal, 10);

        assert!(matches!(
            node.build_packaged_ship_owned(
                player_id,
                BuildPackagedShipCommand {
                    ship_id,
                    station_id: StationId(0),
                    ship_type_id: crate::ship_types::SHIP_TYPE_MAGPIE,
                }
            ),
            StationOperationOutcome::Rejected {
                reason: StationOperationRejection::MissingDockedStationContext,
                ..
            }
        ));
    }

    #[test]
    fn disassemble_ship_credits_packaged_hull_and_removes_ship() {
        let mut node = node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);
        let station = node.station(StationId(0)).expect("demo station exists");
        node.set_spawn_anchor_abs(ship_id, station.abs_m);
        assert!(accepted(node.dock_owned(
            player_id,
            ship_id,
            DockCommand {
                station_id: StationId(0),
            }
        )));
        assert!(node.unfit_module_owned(
            player_id,
            dawn_core::UnfitModuleCommand {
                ship_id,
                slot: dawn_core::SlotKind::High,
                module_id: crate::modules::MODULE_RAILGUN_SMALL,
            }
        ));
        assert!(node.unfit_module_owned(
            player_id,
            dawn_core::UnfitModuleCommand {
                ship_id,
                slot: dawn_core::SlotKind::Mid,
                module_id: crate::modules::MODULE_AFTERBURNER,
            }
        ));
        assert!(node.unfit_module_owned(
            player_id,
            dawn_core::UnfitModuleCommand {
                ship_id,
                slot: dawn_core::SlotKind::Mid,
                module_id: crate::modules::MODULE_FOLD_DISRUPTOR,
            }
        ));

        assert!(accepted(node.disassemble_ship_owned(
            player_id,
            DisassembleShipCommand {
                ship_id,
                station_id: StationId(0),
            }
        )));
        assert_eq!(
            node.station_item_count(
                player_id,
                ItemId::PackagedShip(crate::ship_types::SHIP_TYPE_MAGPIE)
            ),
            // 1 from the starter Packaged Ship every new player is granted
            // (spawn_player_ship_at) + 1 just disassembled here.
            2
        );
        assert!(node.get_ship_position(ship_id).is_none());
        assert_eq!(node.player_docked_station(player_id), Some(StationId(0)));
    }

    #[test]
    fn disassembling_the_only_owned_ship_still_lets_the_client_see_the_updated_station_inventory() {
        // Regression: RefreshFitting used to carry the (now-removed) ship_id,
        // and build_player_loadout_json bailed out immediately once the ship
        // no longer existed in the ECS -- so a client that disassembled its
        // only ship never received the updated station inventory at all.
        // Fixed by keying RefreshFitting on player_id instead
        // (docs/architecture/ownership.md §8).
        let mut node = node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);
        let station = node.station(StationId(0)).expect("demo station exists");
        node.set_spawn_anchor_abs(ship_id, station.abs_m);
        assert!(accepted(node.dock_owned(
            player_id,
            ship_id,
            DockCommand {
                station_id: StationId(0),
            }
        )));
        node.unfit_module_owned(
            player_id,
            dawn_core::UnfitModuleCommand {
                ship_id,
                slot: dawn_core::SlotKind::High,
                module_id: crate::modules::MODULE_RAILGUN_SMALL,
            },
        );
        node.unfit_module_owned(
            player_id,
            dawn_core::UnfitModuleCommand {
                ship_id,
                slot: dawn_core::SlotKind::Mid,
                module_id: crate::modules::MODULE_AFTERBURNER,
            },
        );
        node.unfit_module_owned(
            player_id,
            dawn_core::UnfitModuleCommand {
                ship_id,
                slot: dawn_core::SlotKind::Mid,
                module_id: crate::modules::MODULE_FOLD_DISRUPTOR,
            },
        );

        let mut locks = Vec::new();
        let followup = node.apply_client_command(
            player_id,
            dawn_core::ClientCommand::DisassembleShip(DisassembleShipCommand {
                ship_id,
                station_id: StationId(0),
            }),
            &mut locks,
        );
        let crate::node::ClientCommandFollowup::RefreshFitting(refreshed_player) =
            followup.expect("Disassemble must return a followup")
        else {
            panic!("expected RefreshFitting followup");
        };
        assert_eq!(refreshed_player, player_id);

        let json = node
            .build_player_loadout_json_for_player(refreshed_player)
            .expect("payload must build even though no ship is left");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let station_inventory = v["station_inventory"].as_array().unwrap();
        assert!(
            station_inventory
                .iter()
                .any(|row| row["item_type"] == "PackagedShip"),
            "station_inventory must show the newly-created PackagedShip item: {station_inventory:?}"
        );
    }

    #[test]
    fn disassemble_ship_is_rejected_when_any_module_is_fitted() {
        let mut node = node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);
        let station = node.station(StationId(0)).expect("demo station exists");
        node.set_spawn_anchor_abs(ship_id, station.abs_m);
        assert!(accepted(node.dock_owned(
            player_id,
            ship_id,
            DockCommand {
                station_id: StationId(0),
            }
        )));

        assert!(matches!(
            node.disassemble_ship_owned(
                player_id,
                DisassembleShipCommand {
                    ship_id,
                    station_id: StationId(0),
                }
            ),
            StationOperationOutcome::Rejected {
                reason: StationOperationRejection::ShipIsFitted,
                ..
            }
        ));
    }

    #[test]
    fn assemble_ship_creates_a_new_docked_owned_ship_and_debits_the_packaged_ship_item() {
        let mut node = node();
        let player_id = node.next_player_id();
        node.credit_station_item(
            player_id,
            ItemId::PackagedShip(crate::ship_types::SHIP_TYPE_MAGPIE),
            1,
        );
        node.docked_players.insert(player_id, StationId(0));

        let new_ship_id = node
            .assemble_ship_owned(
                player_id,
                AssembleCommand {
                    station_id: StationId(0),
                    ship_type_id: crate::ship_types::SHIP_TYPE_MAGPIE,
                },
            )
            .expect("assemble should succeed with a packaged ship in inventory");

        assert!(node.owns_ship(player_id, new_ship_id));
        assert_eq!(node.docked_station(new_ship_id), Some(StationId(0)));
        assert_eq!(
            node.station_item_count(
                player_id,
                ItemId::PackagedShip(crate::ship_types::SHIP_TYPE_MAGPIE)
            ),
            0
        );
    }

    #[test]
    fn assemble_ship_does_not_change_active_ship() {
        // A player who disassembled their only ship (active_ship cleared,
        // still docked) must not have Assemble silently re-activate the new
        // ship -- ADR-0037 requires an explicit SelectActiveShipCommand.
        let mut node = node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);
        let station = node.station(StationId(0)).expect("demo station exists");
        node.set_spawn_anchor_abs(ship_id, station.abs_m);
        assert!(accepted(node.dock_owned(
            player_id,
            ship_id,
            DockCommand {
                station_id: StationId(0),
            }
        )));
        node.unfit_module_owned(
            player_id,
            dawn_core::UnfitModuleCommand {
                ship_id,
                slot: dawn_core::SlotKind::High,
                module_id: crate::modules::MODULE_RAILGUN_SMALL,
            },
        );
        node.unfit_module_owned(
            player_id,
            dawn_core::UnfitModuleCommand {
                ship_id,
                slot: dawn_core::SlotKind::Mid,
                module_id: crate::modules::MODULE_AFTERBURNER,
            },
        );
        node.unfit_module_owned(
            player_id,
            dawn_core::UnfitModuleCommand {
                ship_id,
                slot: dawn_core::SlotKind::Mid,
                module_id: crate::modules::MODULE_FOLD_DISRUPTOR,
            },
        );
        assert!(accepted(node.disassemble_ship_owned(
            player_id,
            DisassembleShipCommand {
                ship_id,
                station_id: StationId(0),
            }
        )));
        assert!(!node.is_active_ship(player_id, ship_id));

        let new_ship_id = node
            .assemble_ship_owned(
                player_id,
                AssembleCommand {
                    station_id: StationId(0),
                    ship_type_id: crate::ship_types::SHIP_TYPE_MAGPIE,
                },
            )
            .expect("assemble should succeed");

        assert!(
            !node.is_active_ship(player_id, new_ship_id),
            "Assemble must not auto-activate the new ship (ADR-0037)"
        );

        // The shipless dead end (docs/architecture/ownership.md §8) is now
        // resolvable: SelectActiveShip then Undock.
        assert!(matches!(
            node.select_active_ship_owned(
                player_id,
                dawn_core::SelectActiveShipCommand {
                    ship_id: new_ship_id
                }
            ),
            StationOperationOutcome::Accepted { .. }
        ));
        assert!(matches!(
            node.undock_owned(player_id, new_ship_id),
            StationOperationOutcome::Accepted { .. }
        ));
    }

    #[test]
    fn assemble_ship_is_rejected_when_caller_is_not_docked_at_the_target_station() {
        let mut node = node();
        let player_id = node.next_player_id();
        node.credit_station_item(
            player_id,
            ItemId::PackagedShip(crate::ship_types::SHIP_TYPE_MAGPIE),
            1,
        );

        assert!(matches!(
            node.assemble_ship_owned(
                player_id,
                AssembleCommand {
                    station_id: StationId(0),
                    ship_type_id: crate::ship_types::SHIP_TYPE_MAGPIE,
                }
            ),
            Err(StationOperationRejection::MissingDockedStationContext)
        ));
    }

    #[test]
    fn assemble_ship_is_rejected_when_no_packaged_ship_is_in_station_inventory() {
        let mut node = node();
        let player_id = node.next_player_id();
        node.docked_players.insert(player_id, StationId(0));

        assert!(matches!(
            node.assemble_ship_owned(
                player_id,
                AssembleCommand {
                    station_id: StationId(0),
                    ship_type_id: crate::ship_types::SHIP_TYPE_MAGPIE,
                }
            ),
            Err(StationOperationRejection::MissingStationItem)
        ));
    }

    #[test]
    fn disembark_clears_active_ship_without_touching_ownership_or_docked_state() {
        let mut node = node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);
        let station = node.station(StationId(0)).expect("demo station exists");
        node.set_spawn_anchor_abs(ship_id, station.abs_m);
        assert!(accepted(node.dock_owned(
            player_id,
            ship_id,
            DockCommand {
                station_id: StationId(0),
            }
        )));

        let disembarked_ship_id = node
            .disembark_owned(player_id)
            .expect("disembark should succeed while docked in an active ship");
        assert_eq!(disembarked_ship_id, ship_id);

        assert!(!node.is_active_ship(player_id, ship_id));
        assert!(node.owns_ship(player_id, ship_id), "ownership unaffected");
        assert_eq!(
            node.docked_station(ship_id),
            Some(StationId(0)),
            "the ship stays docked"
        );
        assert_eq!(
            node.player_docked_station(player_id),
            Some(StationId(0)),
            "the player's station context is unaffected"
        );
    }

    #[test]
    fn disembark_is_rejected_when_the_caller_has_no_active_ship() {
        let mut node = node();
        let player_id = node.next_player_id();

        assert!(matches!(
            node.disembark_owned(player_id),
            Err(StationOperationRejection::NotOwned)
        ));
    }

    #[test]
    fn disembark_is_rejected_when_the_active_ship_is_not_docked() {
        let mut node = node();
        let player_id = node.next_player_id();
        node.spawn_player_ship(player_id);

        assert!(matches!(
            node.disembark_owned(player_id),
            Err(StationOperationRejection::ShipNotDocked)
        ));
    }

    #[test]
    fn disembark_then_select_active_ship_then_undock_round_trips_back_to_flying() {
        // The whole point of Disembark: leave the ship voluntarily, then get
        // back into it (or another owned ship) and fly off again.
        let mut node = node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);
        let station = node.station(StationId(0)).expect("demo station exists");
        node.set_spawn_anchor_abs(ship_id, station.abs_m);
        assert!(accepted(node.dock_owned(
            player_id,
            ship_id,
            DockCommand {
                station_id: StationId(0),
            }
        )));
        node.disembark_owned(player_id).expect("disembark succeeds");
        assert!(!node.is_active_ship(player_id, ship_id));

        assert!(matches!(
            node.select_active_ship_owned(
                player_id,
                dawn_core::SelectActiveShipCommand { ship_id }
            ),
            StationOperationOutcome::Accepted { .. }
        ));
        assert!(matches!(
            node.undock_owned(player_id, ship_id),
            StationOperationOutcome::Accepted { .. }
        ));
    }
}
