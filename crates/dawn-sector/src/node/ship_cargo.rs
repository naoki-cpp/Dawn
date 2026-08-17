//! Ship cargo ownership and item movement for `SimulationNode`.
//!
//! This module owns the places where an item enters or leaves a player ship:
//! fitting delegates one-item mutations here, Station transfer moves whole
//! stacks, and Market bridge commands use the same ownership-checked cargo
//! mutation. The existing `ShipFitted` snapshot event remains the single
//! persistence seam for cargo changes.

use dawn_core::{
    CreditItemCommand, ItemId, ModuleId, PlayerId, RemoveItemCommand, ReturnItemCommand,
    TransferDirection, TransferToStationCommand,
};
use dawn_ecs::{components::InventoryComp, Entity};

use super::SimulationNode;

impl SimulationNode {
    /// Seed `entity` with one of every registered module.
    ///
    /// The live player-spawn path calls this deterministic seed, so no separate
    /// public fact is needed for the fixed starter cargo.
    pub(super) fn seed_player_inventory(&mut self, entity: Entity) {
        let mut inventory = InventoryComp::empty();
        for module_id in self.game_data.module_registry.keys().copied() {
            inventory.add(module_id);
        }
        let _ = self.simulation.world.insert_one(entity, inventory);
    }

    /// Consume one module from a ship cargo stack for player fitting.
    pub(super) fn take_ship_module(&mut self, entity: Entity, module_id: ModuleId) -> bool {
        self.simulation
            .world
            .get_mut::<InventoryComp>(entity)
            .map(|mut inventory| inventory.take(module_id))
            .unwrap_or(false)
    }

    /// Add an item to a ship cargo, creating the component defensively when a
    /// player ship was restored without one by an older snapshot.
    pub(super) fn add_ship_item(&mut self, entity: Entity, item_id: ItemId, count: u64) {
        if count == 0 {
            return;
        }
        if let Some(mut inventory) = self.simulation.world.get_mut::<InventoryComp>(entity) {
            inventory.add_item(item_id, count);
            return;
        }

        let mut inventory = InventoryComp::empty();
        inventory.add_item(item_id, count);
        let _ = self.simulation.world.insert_one(entity, inventory);
    }

    /// Remove `cmd.quantity` from the owned ship's cargo for a Market Ask.
    ///
    /// Market owns matching and settlement; Sector owns the authoritative item
    /// mutation and records the resulting cargo through `ShipFitted`.
    pub fn remove_item_owned(&mut self, cmd: RemoveItemCommand) -> bool {
        self.apply_market_item_mutation(
            cmd.player_id,
            cmd.ship_id,
            cmd.item_id,
            cmd.quantity,
            cmd.settlement_id,
            MarketItemMutation::Remove,
        )
    }

    /// Return the remaining quantity of a cancelled Market Ask to ship cargo.
    pub fn return_item_owned(&mut self, cmd: ReturnItemCommand) -> bool {
        self.apply_market_item_mutation(
            cmd.player_id,
            cmd.ship_id,
            cmd.item_id,
            cmd.quantity,
            cmd.settlement_id,
            MarketItemMutation::Add,
        )
    }

    /// Credit a filled Market purchase to ship cargo.
    pub fn credit_item_owned(&mut self, cmd: CreditItemCommand) -> bool {
        self.apply_market_item_mutation(
            cmd.player_id,
            cmd.ship_id,
            cmd.item_id,
            cmd.quantity,
            cmd.settlement_id,
            MarketItemMutation::Add,
        )
    }

    /// Dispatch one Market settlement mutation admitted through a `FrameInput`
    /// (issue #315). Called from within Tick preparation, before the write
    /// set is captured, so the mutation is part of the durable delta and is
    /// rolled back with everything else if preparation does not lead to a
    /// committed apply.
    ///
    /// Distinguishes "this Sector cannot act on it right now"
    /// (`Unavailable`, leave it pending) from "this Sector refuses it"
    /// (`Rejected`, compensate) so a ship that changed Sector between the
    /// caller's drain and this apply does not have its order reversed.
    pub(crate) fn apply_market_settlement_input(
        &mut self,
        input: crate::transition::MarketSettlementInput,
    ) -> crate::transition::MarketSettlementStatus {
        use crate::transition::{MarketSettlementInput, MarketSettlementStatus};

        let settlement_id = input.settlement_id();
        // An already-committed settlement stays Applied even if the ship has
        // since left: the mutation is durable, only the ACK was lost.
        if self
            .simulation
            .applied_market_settlements
            .contains(&settlement_id)
        {
            return MarketSettlementStatus::Applied;
        }

        // Transient: not this Sector's ship to mutate (handed off by a
        // committed Transit, in transit, or not materialized here). The
        // owning Sector picks it up on a later drain.
        let (player_id, ship_id) = input.identity();
        if !self.owns_ship(player_id, ship_id)
            || !self.simulation.ships.index.contains_key(&ship_id)
        {
            return MarketSettlementStatus::Unavailable;
        }

        let applied = match input {
            MarketSettlementInput::RemoveItem(cmd) => self.remove_item_owned(cmd),
            MarketSettlementInput::ReturnItem(cmd) => self.return_item_owned(cmd),
            MarketSettlementInput::CreditItem(cmd) => self.credit_item_owned(cmd),
        };
        if applied {
            MarketSettlementStatus::Applied
        } else {
            MarketSettlementStatus::Rejected
        }
    }

    /// Retire settlement identities the Market ledger has durably decided,
    /// so the idempotency guard does not grow without bound (issue #315).
    ///
    /// A decided settlement is never redelivered, so keeping its id only
    /// inflates every subsequent `TickRecoveryDelta` and checkpoint. Called
    /// from Tick preparation, inside the same rollback window as the rest of
    /// the write set.
    pub(crate) fn retire_acknowledged_settlements(&mut self, settlement_ids: &[u64]) {
        for settlement_id in settlement_ids {
            self.simulation
                .applied_market_settlements
                .remove(settlement_id);
        }
    }

    fn apply_market_item_mutation(
        &mut self,
        player_id: PlayerId,
        ship_id: dawn_core::ShipId,
        item_id: ItemId,
        quantity: u64,
        settlement_id: u64,
        mutation: MarketItemMutation,
    ) -> bool {
        if quantity == 0 || settlement_id == 0 || !self.owns_ship(player_id, ship_id) {
            return false;
        }
        if self
            .simulation
            .applied_market_settlements
            .contains(&settlement_id)
        {
            return true;
        }
        let Some(&entity) = self.simulation.ships.index.get(&ship_id) else {
            return false;
        };

        let changed =
            if let Some(mut inventory) = self.simulation.world.get_mut::<InventoryComp>(entity) {
                match mutation {
                    MarketItemMutation::Remove => {
                        let Some(current) = inventory.items.get(&item_id).copied() else {
                            return false;
                        };
                        if current < quantity {
                            return false;
                        }
                        if current == quantity {
                            inventory.items.remove(&item_id);
                        } else {
                            inventory.items.insert(item_id, current - quantity);
                        }
                        true
                    }
                    MarketItemMutation::Add => {
                        let current = inventory.item_count(item_id);
                        let Some(next) = current.checked_add(quantity) else {
                            return false;
                        };
                        inventory.items.insert(item_id, next);
                        true
                    }
                }
            } else {
                false
            };

        if !changed {
            return false;
        }

        self.simulation
            .applied_market_settlements
            .insert(settlement_id);

        // ShipFitted is the existing full fitting/inventory snapshot event.
        // Reuse it so Market settlement remains a complete public projection
        // fact without a second partial inventory source of truth.
        self.emit_ship_fitted_with_settlement(ship_id, entity, Some(settlement_id));
        true
    }

    /// Move a whole item stack between a docked ship and its Station inventory.
    ///
    /// The ship itself must be owned by `player_id` and docked at the requested
    /// Station. The transfer is intentionally whole-stack only, matching the
    /// existing `TransferToStationCommand` contract.
    pub(crate) fn transfer_to_station_owned(
        &mut self,
        player_id: PlayerId,
        cmd: TransferToStationCommand,
    ) -> bool {
        if !self.owns_ship(player_id, cmd.ship_id)
            || !self.can_use_station(player_id, cmd.station_id)
            || self.docked_station(cmd.ship_id) != Some(cmd.station_id)
        {
            return false;
        }
        let Some(&entity) = self.simulation.ships.index.get(&cmd.ship_id) else {
            return false;
        };

        match cmd.direction {
            TransferDirection::ToStation => {
                let taken = self
                    .simulation
                    .world
                    .get_mut::<InventoryComp>(entity)
                    .map(|mut inventory| inventory.take_all(cmd.item_id))
                    .unwrap_or(0);
                if taken == 0 {
                    return false;
                }
                self.credit_station_item(player_id, cmd.station_id, cmd.item_id, taken);
                true
            }
            TransferDirection::ToShip => {
                let count = self.station_item_count(player_id, cmd.station_id, cmd.item_id);
                if count == 0
                    || self
                        .try_debit_station_item(player_id, cmd.station_id, cmd.item_id, count)
                        .is_err()
                {
                    return false;
                }
                self.add_ship_item(entity, cmd.item_id, count);
                true
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum MarketItemMutation {
    Remove,
    Add,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules;
    use dawn_core::{DockCommand, NodeId, SectorBounds, SectorId, StationId};

    fn node_with_modules() -> SimulationNode {
        SimulationNode::new_test(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        )
    }

    fn spawn_owned_player(node: &mut SimulationNode) -> (PlayerId, dawn_core::ShipId) {
        let station = node.station(StationId(0)).unwrap().clone();
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

    fn spawn_and_dock_owned_player(
        node: &mut SimulationNode,
    ) -> (PlayerId, dawn_core::ShipId, StationId) {
        let (player_id, ship_id) = spawn_owned_player(node);
        (player_id, ship_id, StationId(0))
    }

    #[test]
    fn player_ship_starts_with_one_of_every_registered_module() {
        let mut node = node_with_modules();
        let (_player, ship_id) = spawn_owned_player(&mut node);
        let entity = *node.simulation.ships.index.get(&ship_id).unwrap();
        let inventory = node.simulation.world.get::<InventoryComp>(entity).unwrap();
        assert_eq!(
            inventory.items.len(),
            crate::game_data::test_catalog().modules().to_vec().len()
        );
    }

    #[test]
    fn ship_cargo_module_mutation_takes_and_restores_one_item() {
        let mut node = node_with_modules();
        let (_player, ship_id) = spawn_owned_player(&mut node);
        let entity = *node.simulation.ships.index.get(&ship_id).unwrap();
        let module_id = modules::MODULE_RAILGUN_MEDIUM;
        let before = node
            .simulation
            .world
            .get::<InventoryComp>(entity)
            .unwrap()
            .item_count(ItemId::Module(module_id));

        assert!(node.take_ship_module(entity, module_id));
        assert_eq!(
            node.simulation
                .world
                .get::<InventoryComp>(entity)
                .unwrap()
                .item_count(ItemId::Module(module_id)),
            before - 1
        );

        node.add_ship_item(entity, ItemId::Module(module_id), 1);
        assert_eq!(
            node.simulation
                .world
                .get::<InventoryComp>(entity)
                .unwrap()
                .item_count(ItemId::Module(module_id)),
            before
        );
    }

    #[test]
    fn market_bridge_records_the_resulting_cargo_snapshot() {
        let mut node = node_with_modules();
        let (player, ship_id) = spawn_owned_player(&mut node);
        let entity = *node.simulation.ships.index.get(&ship_id).unwrap();
        node.add_ship_item(entity, ItemId::ScrapMetal, 3);

        assert!(node.remove_item_owned(RemoveItemCommand {
            player_id: player,
            ship_id,
            item_id: ItemId::ScrapMetal,
            quantity: 2,
            settlement_id: 1,
        }));

        let event = node.pending_events().last().unwrap();
        let dawn_core::DomainEvent::ShipFitted(event) = event else {
            panic!("cargo mutation must emit a ShipFitted snapshot");
        };
        assert_eq!(
            event
                .inventory
                .iter()
                .filter(|item| **item == ItemId::ScrapMetal)
                .count(),
            1
        );
        assert_eq!(event.market_settlement_id, Some(1));
    }

    #[test]
    fn market_return_and_credit_item_owned_add_to_the_same_cargo() {
        let mut node = node_with_modules();
        let (player, ship_id) = spawn_owned_player(&mut node);

        assert!(node.return_item_owned(ReturnItemCommand {
            player_id: player,
            ship_id,
            item_id: ItemId::ScrapMetal,
            quantity: 2,
            settlement_id: 2,
        }));
        assert!(node.credit_item_owned(CreditItemCommand {
            player_id: player,
            ship_id,
            item_id: ItemId::ScrapMetal,
            quantity: 3,
            settlement_id: 3,
        }));

        let entity = *node.simulation.ships.index.get(&ship_id).unwrap();
        assert_eq!(
            node.simulation
                .world
                .get::<InventoryComp>(entity)
                .unwrap()
                .item_count(ItemId::ScrapMetal),
            5
        );
    }

    #[test]
    fn market_item_mutations_reject_invalid_owner_quantity_and_balance() {
        let mut node = node_with_modules();
        let (player, ship_id) = spawn_owned_player(&mut node);
        let stranger = node.next_player_id();
        let entity = *node.simulation.ships.index.get(&ship_id).unwrap();
        node.add_ship_item(entity, ItemId::ScrapMetal, 2);

        let command = |player_id, quantity| RemoveItemCommand {
            player_id,
            ship_id,
            item_id: ItemId::ScrapMetal,
            quantity,
            settlement_id: 4,
        };
        assert!(!node.remove_item_owned(command(stranger, 1)));
        assert!(!node.remove_item_owned(command(player, 0)));
        assert!(!node.remove_item_owned(RemoveItemCommand {
            settlement_id: 0,
            ..command(player, 1)
        }));
        assert!(!node.remove_item_owned(command(player, 3)));
        assert_eq!(
            node.simulation
                .world
                .get::<InventoryComp>(entity)
                .unwrap()
                .item_count(ItemId::ScrapMetal),
            2
        );

        node.simulation
            .world
            .get_mut::<InventoryComp>(entity)
            .unwrap()
            .items
            .insert(ItemId::ScrapMetal, u64::MAX);
        assert!(!node.credit_item_owned(CreditItemCommand {
            player_id: player,
            ship_id,
            item_id: ItemId::ScrapMetal,
            quantity: 1,
            settlement_id: 5,
        }));
        assert_eq!(
            node.simulation
                .world
                .get::<InventoryComp>(entity)
                .unwrap()
                .item_count(ItemId::ScrapMetal),
            u64::MAX
        );
    }

    #[test]
    fn duplicate_market_settlement_does_not_repeat_the_cargo_mutation() {
        let mut node = node_with_modules();
        let (player, ship_id) = spawn_owned_player(&mut node);
        let entity = *node.simulation.ships.index.get(&ship_id).unwrap();
        node.add_ship_item(entity, ItemId::ScrapMetal, 5);
        let command = RemoveItemCommand {
            player_id: player,
            ship_id,
            item_id: ItemId::ScrapMetal,
            quantity: 2,
            settlement_id: 99,
        };
        let events_before = node.pending_events().len();

        assert!(node.remove_item_owned(command));
        assert!(node.remove_item_owned(command));
        assert_eq!(
            node.simulation
                .world
                .get::<InventoryComp>(entity)
                .unwrap()
                .item_count(ItemId::ScrapMetal),
            3
        );
        assert_eq!(node.pending_events().len(), events_before + 1);
    }

    #[test]
    fn transfer_to_station_owned_moves_the_whole_stack_of_scrap_metal() {
        let mut node = node_with_modules();
        let (player, ship_id, station_id) = spawn_and_dock_owned_player(&mut node);
        let entity = *node.simulation.ships.index.get(&ship_id).unwrap();
        node.add_ship_item(entity, ItemId::ScrapMetal, 4);

        assert!(node.transfer_to_station_owned(
            player,
            TransferToStationCommand {
                ship_id,
                station_id,
                item_id: ItemId::ScrapMetal,
                direction: TransferDirection::ToStation,
            }
        ));

        let inventory = node.simulation.world.get::<InventoryComp>(entity).unwrap();
        assert_eq!(inventory.item_count(ItemId::ScrapMetal), 0);
        assert_eq!(
            node.station_inventory(player, station_id)
                .and_then(|items| items.get(&ItemId::ScrapMetal).copied())
                .unwrap_or(0),
            4
        );
    }

    #[test]
    fn transfer_to_station_owned_is_rejected_for_a_ship_the_player_does_not_own() {
        let mut node = node_with_modules();
        let (_owner, ship_id, station_id) = spawn_and_dock_owned_player(&mut node);
        let stranger = node.next_player_id();

        assert!(!node.transfer_to_station_owned(
            stranger,
            TransferToStationCommand {
                ship_id,
                station_id,
                item_id: ItemId::ScrapMetal,
                direction: TransferDirection::ToStation,
            }
        ));
    }

    #[test]
    fn transfer_to_station_owned_is_rejected_when_the_ship_itself_is_not_docked_at_the_station() {
        let mut node = node_with_modules();
        let (player, _active_ship, station_id) = spawn_and_dock_owned_player(&mut node);
        let other_ship = node.spawn_player_ship_at_pub(player, dawn_core::Position::ORIGIN);
        let entity = *node.simulation.ships.index.get(&other_ship).unwrap();
        node.add_ship_item(entity, ItemId::ScrapMetal, 4);

        assert!(!node.transfer_to_station_owned(
            player,
            TransferToStationCommand {
                ship_id: other_ship,
                station_id,
                item_id: ItemId::ScrapMetal,
                direction: TransferDirection::ToStation,
            }
        ));
        assert_eq!(
            node.simulation
                .world
                .get::<InventoryComp>(entity)
                .unwrap()
                .item_count(ItemId::ScrapMetal),
            4,
            "rejected transfer must not move the cargo"
        );
    }

    #[test]
    fn transfer_to_station_owned_is_rejected_when_not_docked_at_the_station() {
        let mut node = node_with_modules();
        let player = node.next_player_id();
        let ship_id = node.spawn_player_ship_at_pub(player, dawn_core::Position::ORIGIN);
        let entity = *node.simulation.ships.index.get(&ship_id).unwrap();
        node.add_ship_item(entity, ItemId::ScrapMetal, 4);

        assert!(!node.transfer_to_station_owned(
            player,
            TransferToStationCommand {
                ship_id,
                station_id: StationId(0),
                item_id: ItemId::ScrapMetal,
                direction: TransferDirection::ToStation,
            }
        ));
    }

    #[test]
    fn transfer_to_station_owned_is_rejected_when_the_ship_cargo_has_none_of_the_item() {
        let mut node = node_with_modules();
        let (player, ship_id, station_id) = spawn_and_dock_owned_player(&mut node);

        assert!(!node.transfer_to_station_owned(
            player,
            TransferToStationCommand {
                ship_id,
                station_id,
                item_id: ItemId::ScrapMetal,
                direction: TransferDirection::ToStation,
            }
        ));
    }

    #[test]
    fn transfer_to_station_owned_to_ship_moves_the_whole_stack_back_into_cargo() {
        let mut node = node_with_modules();
        let (player, ship_id, station_id) = spawn_and_dock_owned_player(&mut node);
        node.credit_station_item(player, station_id, ItemId::ScrapMetal, 7);

        assert!(node.transfer_to_station_owned(
            player,
            TransferToStationCommand {
                ship_id,
                station_id,
                item_id: ItemId::ScrapMetal,
                direction: TransferDirection::ToShip,
            }
        ));

        let entity = *node.simulation.ships.index.get(&ship_id).unwrap();
        assert_eq!(
            node.simulation
                .world
                .get::<InventoryComp>(entity)
                .unwrap()
                .item_count(ItemId::ScrapMetal),
            7
        );
        assert_eq!(
            node.station_item_count(player, station_id, ItemId::ScrapMetal),
            0
        );
    }

    #[test]
    fn transfer_to_station_owned_to_ship_is_rejected_when_station_inventory_has_none_of_the_item() {
        let mut node = node_with_modules();
        let (player, ship_id, station_id) = spawn_and_dock_owned_player(&mut node);

        assert!(!node.transfer_to_station_owned(
            player,
            TransferToStationCommand {
                ship_id,
                station_id,
                item_id: ItemId::ScrapMetal,
                direction: TransferDirection::ToShip,
            }
        ));
    }
}
