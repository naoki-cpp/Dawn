//! Player-issued Fit/Unfit/Reorder for `SimulationNode` (ADR-0032).
//!
//! Ship cargo ownership and item movement live in `ship_cargo.rs`. This
//! module owns the fitting-specific validation and the `FittingComp` changes;
//! both modules use the existing `ShipFitted` snapshot event.

use dawn_core::{FitModuleCommand, PlayerId, ReorderFittedModuleCommand, UnfitModuleCommand};
use dawn_ecs::{
    components::{FittedSlot, FittingComp},
    Entity,
};
use dawn_event_store::store::EventStore;

use super::SimulationNode;

impl<S: EventStore> SimulationNode<S> {
    /// Move one instance of `cmd.module_id` from the owning player's
    /// inventory into `cmd.slot` (ADR-0032). Unlike `fit_module` (the
    /// internal spawn-time path), this enforces the module's own slot kind,
    /// the ship type's slot capacity, and that the module is actually present
    /// in `InventoryComp` -- rejecting (no state change) otherwise.
    pub fn fit_module_owned(&mut self, player_id: PlayerId, cmd: FitModuleCommand) -> bool {
        if !self.owns_ship(player_id, cmd.ship_id) {
            return false;
        }
        // ADR-0032 amendment (2026-07-08): the §2 "anywhere, MVP" trigger
        // fired now that Station exists (ADR-0034/0037) -- refitting
        // requires being docked.
        if !self.is_ship_docked(cmd.ship_id) {
            return false;
        }
        let Some(def) = self.module_registry.get(&cmd.module_id).cloned() else {
            return false;
        };
        if def.slot != cmd.slot {
            return false;
        }
        let Some(&entity) = self.ships.index.get(&cmd.ship_id) else {
            return false;
        };
        let Some(&type_id) = self.ships.type_ids.get(&cmd.ship_id) else {
            return false;
        };
        let capacity = self
            .ship_type_registry
            .get(&type_id)
            .map(|d| d.slot_layout.capacity_for(cmd.slot))
            .unwrap_or(0);
        let current_count = self
            .world
            .get::<FittingComp>(entity)
            .map(|f| f.slot(cmd.slot).len())
            .unwrap_or(0);
        if current_count >= capacity as usize {
            return false;
        }
        let took = self.take_ship_module(entity, cmd.module_id);
        if !took {
            return false;
        }

        use dawn_core::ActivationMode;
        let is_active = matches!(def.activation_mode, ActivationMode::Passive);
        let fitted = self
            .world
            .get_mut::<FittingComp>(entity)
            .map(|mut fitting| {
                fitting.slot_mut(cmd.slot).push(FittedSlot {
                    def,
                    is_active,
                    cycle_remaining: 0,
                    target_ship_id: None,
                });
            })
            .is_some();
        if !fitted {
            // FittingComp is expected on every spawned ship; if it's somehow
            // missing, undo the inventory take so the module isn't lost.
            self.add_ship_item(entity, dawn_core::ItemId::Module(cmd.module_id), 1);
            return false;
        }

        self.apply_fitting_and_emit(cmd.ship_id, entity);
        true
    }

    /// Move one fitted instance of `cmd.module_id` out of `cmd.slot` and back
    /// into the owning player's inventory (ADR-0032).
    pub fn unfit_module_owned(&mut self, player_id: PlayerId, cmd: UnfitModuleCommand) -> bool {
        if !self.owns_ship(player_id, cmd.ship_id) {
            return false;
        }
        // ADR-0032 amendment (2026-07-08): see fit_module_owned above.
        if !self.is_ship_docked(cmd.ship_id) {
            return false;
        }
        let Some(&entity) = self.ships.index.get(&cmd.ship_id) else {
            return false;
        };
        let removed = if let Some(mut fitting) = self.world.get_mut::<FittingComp>(entity) {
            let slots = fitting.slot_mut(cmd.slot);
            match slots.iter().position(|s| s.def.id == cmd.module_id) {
                Some(pos) => {
                    slots.remove(pos);
                    true
                }
                None => false,
            }
        } else {
            false
        };
        if !removed {
            return false;
        }

        // `ship_cargo` owns the defensive component creation and item
        // mutation so every cargo entry point shares the same semantics.
        self.add_ship_item(entity, dawn_core::ItemId::Module(cmd.module_id), 1);

        self.apply_fitting_and_emit(cmd.ship_id, entity);
        true
    }

    /// Reorder two fitted modules within the same slot kind (drag-and-drop
    /// reorder in the FITTED column, ADR-0032 amendment). Persisted (not
    /// merely a client display order) since iteration order assigns weapon
    /// hotkey F-numbers -- reuses `ShipFitted` (via `apply_fitting_and_emit`)
    /// the same way Fit/Unfit do, since that event already carries the full
    /// `FittingSnapshot` and a pure reorder doesn't need its own event type.
    pub fn reorder_fitted_module_owned(
        &mut self,
        player_id: PlayerId,
        cmd: ReorderFittedModuleCommand,
    ) -> bool {
        if !self.owns_ship(player_id, cmd.ship_id) {
            return false;
        }
        if !self.is_ship_docked(cmd.ship_id) {
            return false;
        }
        let Some(&entity) = self.ships.index.get(&cmd.ship_id) else {
            return false;
        };
        let reordered = if let Some(mut fitting) = self.world.get_mut::<FittingComp>(entity) {
            let slots = fitting.slot_mut(cmd.slot);
            let (from, to) = (cmd.from_index as usize, cmd.to_index as usize);
            if from >= slots.len() || to >= slots.len() {
                false
            } else {
                let moved = slots.remove(from);
                slots.insert(to, moved);
                true
            }
        } else {
            false
        };
        if !reordered {
            return false;
        }
        // Stats are unaffected by pure reordering, but ShipFitted still
        // needs to carry the new order for replay -- emit without the
        // reapply_fitting half of apply_fitting_and_emit's usual tail.
        self.emit_ship_fitted(cmd.ship_id, entity);
        true
    }

    /// Shared tail of `fit_module_owned`/`unfit_module_owned`: recompute
    /// `ShipStatsComp` from the new `FittingComp` (`reapply_fitting`), then
    /// tell the world about it (`emit_ship_fitted`, shared with
    /// `commands.rs::fit_module`'s privileged path -- ADR-0032 §5).
    fn apply_fitting_and_emit(&mut self, ship_id: dawn_core::ShipId, entity: Entity) {
        self.reapply_fitting(ship_id);
        self.emit_ship_fitted(ship_id, entity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{modules, ship_types};
    use dawn_core::{
        CreditItemCommand, NodeId, Position, RemoveItemCommand, ReturnItemCommand, SectorBounds,
        SectorId, SlotKind, TransferToStationCommand,
    };
    use dawn_ecs::components::InventoryComp;

    fn total_items(inv: &InventoryComp) -> u64 {
        inv.items.values().copied().sum()
    }

    fn node_with_modules() -> SimulationNode {
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

    /// Spawns and docks an owned player ship at the demo station
    /// (`StationId(0)`). Fit/Unfit require being docked (ADR-0032
    /// amendment, 2026-07-08), so this is the default fixture for every
    /// fit/unfit test in this module -- the handful that only test an
    /// ownership/inventory/capacity rejection don't care whether the ship
    /// is also docked, since `owns_ship` is checked first either way.
    fn spawn_owned_player(node: &mut SimulationNode) -> (dawn_core::PlayerId, dawn_core::ShipId) {
        use dawn_core::{DockCommand, StationId};
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

    /// Spawns and docks a second owned player ship for docked-fit rejection
    /// tests that also need an explicitly *undocked* ship.
    fn spawn_owned_player_undocked(
        node: &mut SimulationNode,
    ) -> (dawn_core::PlayerId, dawn_core::ShipId) {
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);
        (player_id, ship_id)
    }

    #[test]
    fn player_ship_starts_with_one_of_every_registered_module() {
        let mut node = node_with_modules();
        let (_player, ship_id) = spawn_owned_player(&mut node);
        let entity = *node.ships.index.get(&ship_id).unwrap();
        let inv = node.world.get::<InventoryComp>(entity).unwrap();
        assert_eq!(inv.items.len(), modules::all_modules().len());
    }

    #[test]
    fn market_remove_item_owned_debits_exact_quantity_and_emits_snapshot() {
        let mut node = node_with_modules();
        let (player, ship_id) = spawn_owned_player(&mut node);
        let entity = *node.ships.index.get(&ship_id).unwrap();
        node.world
            .get_mut::<InventoryComp>(entity)
            .unwrap()
            .add_item(dawn_core::ItemId::ScrapMetal, 5);

        assert!(node.remove_item_owned(RemoveItemCommand {
            player_id: player,
            ship_id,
            item_id: dawn_core::ItemId::ScrapMetal,
            quantity: 2,
        }));
        assert_eq!(
            node.world
                .get::<InventoryComp>(entity)
                .unwrap()
                .item_count(dawn_core::ItemId::ScrapMetal),
            3
        );

        let event = &node.event_store().all_records().last().unwrap().event;
        let dawn_core::DomainEvent::ShipFitted(event) = event else {
            panic!("market item removal must emit a ShipFitted snapshot");
        };
        assert_eq!(event.ship_id, ship_id);
        assert_eq!(
            event
                .inventory
                .iter()
                .filter(|item| **item == dawn_core::ItemId::ScrapMetal)
                .count(),
            3
        );
    }

    #[test]
    fn market_return_and_credit_item_owned_add_to_the_same_cargo() {
        let mut node = node_with_modules();
        let (player, ship_id) = spawn_owned_player(&mut node);

        assert!(node.return_item_owned(ReturnItemCommand {
            player_id: player,
            ship_id,
            item_id: dawn_core::ItemId::ScrapMetal,
            quantity: 2,
        }));
        assert!(node.credit_item_owned(CreditItemCommand {
            player_id: player,
            ship_id,
            item_id: dawn_core::ItemId::ScrapMetal,
            quantity: 3,
        }));

        let entity = *node.ships.index.get(&ship_id).unwrap();
        assert_eq!(
            node.world
                .get::<InventoryComp>(entity)
                .unwrap()
                .item_count(dawn_core::ItemId::ScrapMetal),
            5
        );
    }

    #[test]
    fn market_item_mutations_reject_invalid_owner_quantity_and_balance() {
        let mut node = node_with_modules();
        let (player, ship_id) = spawn_owned_player(&mut node);
        let stranger = node.next_player_id();
        let entity = *node.ships.index.get(&ship_id).unwrap();
        node.world
            .get_mut::<InventoryComp>(entity)
            .unwrap()
            .add_item(dawn_core::ItemId::ScrapMetal, 2);

        let command = |player_id, quantity| RemoveItemCommand {
            player_id,
            ship_id,
            item_id: dawn_core::ItemId::ScrapMetal,
            quantity,
        };
        assert!(!node.remove_item_owned(command(stranger, 1)));
        assert!(!node.remove_item_owned(command(player, 0)));
        assert!(!node.remove_item_owned(command(player, 3)));
        assert_eq!(
            node.world
                .get::<InventoryComp>(entity)
                .unwrap()
                .item_count(dawn_core::ItemId::ScrapMetal),
            2
        );

        node.world
            .get_mut::<InventoryComp>(entity)
            .unwrap()
            .items
            .insert(dawn_core::ItemId::ScrapMetal, u64::MAX);
        assert!(!node.credit_item_owned(CreditItemCommand {
            player_id: player,
            ship_id,
            item_id: dawn_core::ItemId::ScrapMetal,
            quantity: 1,
        }));
        assert_eq!(
            node.world
                .get::<InventoryComp>(entity)
                .unwrap()
                .item_count(dawn_core::ItemId::ScrapMetal),
            u64::MAX
        );
    }

    #[test]
    fn market_item_snapshot_replay_restores_the_credited_cargo() {
        let mut node = node_with_modules();
        let (player, ship_id) = spawn_owned_player(&mut node);
        assert!(node.credit_item_owned(CreditItemCommand {
            player_id: player,
            ship_id,
            item_id: dawn_core::ItemId::ScrapMetal,
            quantity: 4,
        }));
        let event = node
            .event_store()
            .all_records()
            .last()
            .unwrap()
            .event
            .clone();

        let entity = *node.ships.index.get(&ship_id).unwrap();
        node.world
            .get_mut::<InventoryComp>(entity)
            .unwrap()
            .items
            .clear();
        node.apply_event_pub(event);

        assert_eq!(
            node.world
                .get::<InventoryComp>(entity)
                .unwrap()
                .item_count(dawn_core::ItemId::ScrapMetal),
            4
        );
    }

    #[test]
    fn fit_module_owned_moves_an_item_from_inventory_into_an_empty_slot() {
        let mut node = node_with_modules();
        let (player, ship_id) = spawn_owned_player(&mut node);
        let entity = *node.ships.index.get(&ship_id).unwrap();
        let before_inv_len = total_items(&node.world.get::<InventoryComp>(entity).unwrap());

        assert!(node.fit_module_owned(
            player,
            FitModuleCommand {
                ship_id,
                slot: SlotKind::High,
                module_id: modules::MODULE_RAILGUN_MEDIUM,
            }
        ));

        let fitting = node.world.get::<FittingComp>(entity).unwrap();
        assert!(fitting
            .slot(SlotKind::High)
            .iter()
            .any(|s| s.def.id == modules::MODULE_RAILGUN_MEDIUM));
        let inv = node.world.get::<InventoryComp>(entity).unwrap();
        assert_eq!(total_items(&inv), before_inv_len - 1);
    }

    #[test]
    fn fit_module_owned_is_rejected_for_a_ship_the_player_does_not_own() {
        let mut node = node_with_modules();
        let (_owner, ship_id) = spawn_owned_player(&mut node);
        let stranger = node.next_player_id();

        assert!(!node.fit_module_owned(
            stranger,
            FitModuleCommand {
                ship_id,
                slot: SlotKind::High,
                module_id: modules::MODULE_RAILGUN_MEDIUM,
            }
        ));
    }

    #[test]
    fn fit_module_owned_is_rejected_when_the_module_is_not_in_inventory() {
        let mut node = node_with_modules();
        let (player, ship_id) = spawn_owned_player(&mut node);
        let entity = *node.ships.index.get(&ship_id).unwrap();
        // Drain the inventory of this module first.
        node.world
            .get_mut::<InventoryComp>(entity)
            .unwrap()
            .take(modules::MODULE_RAILGUN_MEDIUM);

        assert!(!node.fit_module_owned(
            player,
            FitModuleCommand {
                ship_id,
                slot: SlotKind::High,
                module_id: modules::MODULE_RAILGUN_MEDIUM,
            }
        ));
    }

    #[test]
    fn fit_module_owned_is_rejected_when_the_slot_kind_does_not_match() {
        let mut node = node_with_modules();
        let (player, ship_id) = spawn_owned_player(&mut node);

        // MODULE_RAILGUN_MEDIUM is a High-slot module.
        assert!(!node.fit_module_owned(
            player,
            FitModuleCommand {
                ship_id,
                slot: SlotKind::Mid,
                module_id: modules::MODULE_RAILGUN_MEDIUM,
            }
        ));
    }

    #[test]
    fn fit_module_owned_is_rejected_once_the_slot_kind_is_at_capacity() {
        let mut node = node_with_modules();
        let (player, ship_id) = spawn_owned_player(&mut node);
        let entity = *node.ships.index.get(&ship_id).unwrap();
        let capacity = node
            .ship_type_registry
            .get(&ship_types::SHIP_TYPE_MAGPIE)
            .unwrap()
            .slot_layout
            .capacity_for(SlotKind::High);
        // The default loadout already occupies 1 High slot (a small
        // railgun); give the ship just enough spares to fill the rest.
        let remaining = capacity as usize - 1;
        node.world
            .get_mut::<InventoryComp>(entity)
            .unwrap()
            .items
            .entry(dawn_core::ItemId::Module(modules::MODULE_RAILGUN_MEDIUM))
            .and_modify(|count| *count += remaining as u64)
            .or_insert(remaining as u64);

        for _ in 0..remaining {
            assert!(node.fit_module_owned(
                player,
                FitModuleCommand {
                    ship_id,
                    slot: SlotKind::High,
                    module_id: modules::MODULE_RAILGUN_MEDIUM,
                }
            ));
        }
        // The High slot is now full (1 default-loadout railgun + `remaining`
        // more) so the next Fit must be rejected regardless of inventory.
        assert!(!node.fit_module_owned(
            player,
            FitModuleCommand {
                ship_id,
                slot: SlotKind::High,
                module_id: modules::MODULE_RAILGUN_MEDIUM,
            }
        ));
    }

    #[test]
    fn unfit_module_owned_moves_a_fitted_item_back_into_inventory() {
        let mut node = node_with_modules();
        let (player, ship_id) = spawn_owned_player(&mut node);
        let entity = *node.ships.index.get(&ship_id).unwrap();
        let before_inv_len = node
            .world
            .get::<InventoryComp>(entity)
            .map(|inv| total_items(&inv))
            .unwrap();

        assert!(node.unfit_module_owned(
            player,
            UnfitModuleCommand {
                ship_id,
                slot: SlotKind::High,
                module_id: modules::MODULE_RAILGUN_SMALL,
            }
        ));

        let fitting = node.world.get::<FittingComp>(entity).unwrap();
        assert!(!fitting
            .slot(SlotKind::High)
            .iter()
            .any(|s| s.def.id == modules::MODULE_RAILGUN_SMALL));
        let inv = node.world.get::<InventoryComp>(entity).unwrap();
        assert_eq!(total_items(&inv), before_inv_len + 1);
    }

    #[test]
    fn unfit_module_owned_is_rejected_when_no_such_module_is_fitted_in_that_slot() {
        let mut node = node_with_modules();
        let (player, ship_id) = spawn_owned_player(&mut node);

        assert!(!node.unfit_module_owned(
            player,
            UnfitModuleCommand {
                ship_id,
                slot: SlotKind::Low,
                module_id: modules::MODULE_RAILGUN_SMALL,
            }
        ));
    }

    #[test]
    fn unfit_module_owned_is_rejected_for_a_ship_the_player_does_not_own() {
        let mut node = node_with_modules();
        let (_owner, ship_id) = spawn_owned_player(&mut node);
        let stranger = node.next_player_id();

        assert!(!node.unfit_module_owned(
            stranger,
            UnfitModuleCommand {
                ship_id,
                slot: SlotKind::High,
                module_id: modules::MODULE_RAILGUN_SMALL,
            }
        ));
    }

    /// ADR-0032 amendment (2026-07-08): the §2 "anywhere, MVP" trigger fired
    /// now that Station exists.
    #[test]
    fn fit_module_owned_is_rejected_when_the_ship_is_undocked() {
        let mut node = node_with_modules();
        let (player, ship_id) = spawn_owned_player_undocked(&mut node);

        assert!(!node.fit_module_owned(
            player,
            FitModuleCommand {
                ship_id,
                slot: SlotKind::High,
                module_id: modules::MODULE_RAILGUN_MEDIUM,
            }
        ));
    }

    #[test]
    fn unfit_module_owned_is_rejected_when_the_ship_is_undocked() {
        let mut node = node_with_modules();
        let (player, ship_id) = spawn_owned_player_undocked(&mut node);

        assert!(!node.unfit_module_owned(
            player,
            UnfitModuleCommand {
                ship_id,
                slot: SlotKind::High,
                module_id: modules::MODULE_RAILGUN_SMALL,
            }
        ));
    }

    #[test]
    fn fit_then_unfit_round_trips_inventory_count() {
        let mut node = node_with_modules();
        let (player, ship_id) = spawn_owned_player(&mut node);
        let entity = *node.ships.index.get(&ship_id).unwrap();
        let start_len = node
            .world
            .get::<InventoryComp>(entity)
            .map(|inv| total_items(&inv))
            .unwrap();

        assert!(node.fit_module_owned(
            player,
            FitModuleCommand {
                ship_id,
                slot: SlotKind::High,
                module_id: modules::MODULE_RAILGUN_MEDIUM,
            }
        ));
        assert!(node.unfit_module_owned(
            player,
            UnfitModuleCommand {
                ship_id,
                slot: SlotKind::High,
                module_id: modules::MODULE_RAILGUN_MEDIUM,
            }
        ));

        let inv = node.world.get::<InventoryComp>(entity).unwrap();
        assert_eq!(total_items(&inv), start_len);
    }

    /// Spawns a fresh owned player ship at the demo station (`StationId(0)`,
    /// present in every `node_with_modules()` fixture) and docks it there.
    fn spawn_and_dock_owned_player(
        node: &mut SimulationNode,
    ) -> (dawn_core::PlayerId, dawn_core::ShipId, dawn_core::StationId) {
        use dawn_core::{DockCommand, StationId};

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
                }
            ),
            crate::node::station::StationOperationOutcome::Accepted { .. }
        ));
        (player_id, ship_id, StationId(0))
    }

    #[test]
    fn transfer_to_station_owned_moves_the_whole_stack_of_scrap_metal() {
        let mut node = node_with_modules();
        let (player, ship_id, station_id) = spawn_and_dock_owned_player(&mut node);
        let entity = *node.ships.index.get(&ship_id).unwrap();
        node.world
            .get_mut::<InventoryComp>(entity)
            .unwrap()
            .add_item(dawn_core::ItemId::ScrapMetal, 4);

        assert!(node.transfer_to_station_owned(
            player,
            TransferToStationCommand {
                ship_id,
                station_id,
                item_id: dawn_core::ItemId::ScrapMetal,
                direction: dawn_core::TransferDirection::ToStation,
            }
        ));

        let inv = node.world.get::<InventoryComp>(entity).unwrap();
        assert_eq!(inv.item_count(dawn_core::ItemId::ScrapMetal), 0);
        assert_eq!(
            node.station_inventory(player, dawn_core::StationId(0))
                .and_then(|inv| inv.get(&dawn_core::ItemId::ScrapMetal).copied())
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
                item_id: dawn_core::ItemId::ScrapMetal,
                direction: dawn_core::TransferDirection::ToStation,
            }
        ));
    }

    /// security-review.md SEC-3: `can_use_station` only checks the
    /// *player's* docked context, so under the multi-owned-ship model
    /// (ADR-0037) a player docked with their active ship must not be able
    /// to transfer cargo from a *different* owned ship that isn't itself
    /// docked at that station (e.g. still in open space, or docked
    /// elsewhere).
    #[test]
    fn transfer_to_station_owned_is_rejected_when_the_ship_itself_is_not_docked_at_the_station() {
        let mut node = node_with_modules();
        let (player, active_ship, station_id) = spawn_and_dock_owned_player(&mut node);
        let _ = active_ship;

        // A second owned ship, never docked anywhere.
        let other_ship = node.spawn_player_ship_at_pub(player, Position::ORIGIN);
        assert!(node.owns_ship(player, other_ship));
        let entity = *node.ships.index.get(&other_ship).unwrap();
        node.world
            .get_mut::<InventoryComp>(entity)
            .unwrap()
            .add_item(dawn_core::ItemId::ScrapMetal, 4);

        // The player is docked (via `active_ship`) at `station_id`, so
        // `can_use_station` alone would pass -- but `other_ship` itself
        // isn't docked there.
        assert!(!node.transfer_to_station_owned(
            player,
            TransferToStationCommand {
                ship_id: other_ship,
                station_id,
                item_id: dawn_core::ItemId::ScrapMetal,
                direction: dawn_core::TransferDirection::ToStation,
            }
        ));
        let inv = node.world.get::<InventoryComp>(entity).unwrap();
        assert_eq!(
            inv.item_count(dawn_core::ItemId::ScrapMetal),
            4,
            "rejected transfer must not move the cargo"
        );
    }

    #[test]
    fn transfer_to_station_owned_is_rejected_when_not_docked_at_the_station() {
        let mut node = node_with_modules();
        let (player, ship_id) = spawn_owned_player_undocked(&mut node);
        let entity = *node.ships.index.get(&ship_id).unwrap();
        node.world
            .get_mut::<InventoryComp>(entity)
            .unwrap()
            .add_item(dawn_core::ItemId::ScrapMetal, 4);

        assert!(!node.transfer_to_station_owned(
            player,
            TransferToStationCommand {
                ship_id,
                station_id: dawn_core::StationId(0),
                item_id: dawn_core::ItemId::ScrapMetal,
                direction: dawn_core::TransferDirection::ToStation,
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
                item_id: dawn_core::ItemId::ScrapMetal,
                direction: dawn_core::TransferDirection::ToStation,
            }
        ));
    }

    #[test]
    fn transfer_to_station_owned_to_ship_moves_the_whole_stack_back_into_cargo() {
        let mut node = node_with_modules();
        let (player, ship_id, station_id) = spawn_and_dock_owned_player(&mut node);
        node.credit_station_item(player, station_id, dawn_core::ItemId::ScrapMetal, 7);

        assert!(node.transfer_to_station_owned(
            player,
            TransferToStationCommand {
                ship_id,
                station_id,
                item_id: dawn_core::ItemId::ScrapMetal,
                direction: dawn_core::TransferDirection::ToShip,
            }
        ));

        let entity = *node.ships.index.get(&ship_id).unwrap();
        let inv = node.world.get::<InventoryComp>(entity).unwrap();
        assert_eq!(inv.item_count(dawn_core::ItemId::ScrapMetal), 7);
        assert_eq!(
            node.station_item_count(player, station_id, dawn_core::ItemId::ScrapMetal),
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
                item_id: dawn_core::ItemId::ScrapMetal,
                direction: dawn_core::TransferDirection::ToShip,
            }
        ));
    }

    /// The default loadout (`spawn_player_ship_at`) fits two Mid modules
    /// (Afterburner then Fold Disruptor, in that order) -- exactly the
    /// fixture a reorder test needs, with no extra Fit calls.
    #[test]
    fn reorder_fitted_module_owned_swaps_two_modules_within_the_same_slot_kind() {
        let mut node = node_with_modules();
        let (player, ship_id) = spawn_owned_player(&mut node);
        let entity = *node.ships.index.get(&ship_id).unwrap();
        let before: Vec<dawn_core::ModuleId> = node
            .world
            .get::<FittingComp>(entity)
            .unwrap()
            .slot(SlotKind::Mid)
            .iter()
            .map(|s| s.def.id)
            .collect();
        assert_eq!(
            before.len(),
            2,
            "default loadout fits exactly 2 Mid modules"
        );

        assert!(node.reorder_fitted_module_owned(
            player,
            ReorderFittedModuleCommand {
                ship_id,
                slot: SlotKind::Mid,
                from_index: 0,
                to_index: 1,
            }
        ));

        let after: Vec<dawn_core::ModuleId> = node
            .world
            .get::<FittingComp>(entity)
            .unwrap()
            .slot(SlotKind::Mid)
            .iter()
            .map(|s| s.def.id)
            .collect();
        assert_eq!(after, vec![before[1], before[0]]);
    }

    #[test]
    fn reorder_fitted_module_owned_is_rejected_when_the_ship_is_undocked() {
        let mut node = node_with_modules();
        let (player, ship_id) = spawn_owned_player_undocked(&mut node);

        assert!(!node.reorder_fitted_module_owned(
            player,
            ReorderFittedModuleCommand {
                ship_id,
                slot: SlotKind::Mid,
                from_index: 0,
                to_index: 1,
            }
        ));
    }

    #[test]
    fn reorder_fitted_module_owned_is_rejected_for_an_out_of_bounds_index() {
        let mut node = node_with_modules();
        let (player, ship_id) = spawn_owned_player(&mut node);

        assert!(!node.reorder_fitted_module_owned(
            player,
            ReorderFittedModuleCommand {
                ship_id,
                slot: SlotKind::Mid,
                from_index: 0,
                to_index: 99,
            }
        ));
    }

    #[test]
    fn reorder_fitted_module_owned_is_rejected_for_a_ship_the_player_does_not_own() {
        let mut node = node_with_modules();
        let (_owner, ship_id) = spawn_owned_player(&mut node);
        let stranger = node.next_player_id();

        assert!(!node.reorder_fitted_module_owned(
            stranger,
            ReorderFittedModuleCommand {
                ship_id,
                slot: SlotKind::Mid,
                from_index: 0,
                to_index: 1,
            }
        ));
    }
}
