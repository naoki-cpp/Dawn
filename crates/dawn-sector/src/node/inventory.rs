//! Inventory and player-issued Fit/Unfit for `SimulationNode` (ADR-0032).
//!
//! The existing `fit_module` (in `node::commands`) stays the privileged,
//! unchecked path used internally to install a ship's starting loadout at
//! spawn -- unchanged, to protect its existing behavior and tests. This
//! module adds the player-facing pair that enforces slot capacity and
//! consumes from `InventoryComp`:
//!
//! - `fit_module_owned` -- Inventory -> `FittingComp` slot.
//! - `unfit_module_owned` -- `FittingComp` slot -> Inventory.
//!
//! Both emit the existing `ShipFitted` event (now carrying the resulting
//! inventory snapshot too, ADR-0032 §5) -- no new event type.

use dawn_core::{FitModuleCommand, PlayerId, TransferToStationCommand, UnfitModuleCommand};
use dawn_ecs::{
    components::{FittedSlot, FittingComp, InventoryComp},
    Entity,
};
use dawn_event_store::store::EventStore;

use super::SimulationNode;

impl<S: EventStore> SimulationNode<S> {
    /// Seed `entity`'s starting inventory: one of every currently registered
    /// module (ADR-0032 §1 -- a fixed initial set, no replenishment, until an
    /// Economy/Loot phase adds other supply sources). Called from both the
    /// live spawn path (`spawn_player_ship_at`) and `ShipSpawned` replay, so
    /// it must be a pure function of `module_registry` (loaded once at node
    /// startup, identically on every replay) to stay INV-002 compliant
    /// without a dedicated event.
    pub(super) fn seed_player_inventory(&mut self, entity: Entity) {
        let mut inventory = InventoryComp::empty();
        for module_id in self.module_registry.keys().copied() {
            inventory.add(module_id);
        }
        let _ = self.world.inner_mut().insert_one(entity, inventory);
    }

    /// Move one instance of `cmd.module_id` from the owning player's
    /// inventory into `cmd.slot` (ADR-0032). Unlike `fit_module` (the
    /// internal spawn-time path), this enforces the module's own slot kind,
    /// the ship type's slot capacity, and that the module is actually present
    /// in `InventoryComp` -- rejecting (no state change) otherwise.
    pub fn fit_module_owned(&mut self, player_id: PlayerId, cmd: FitModuleCommand) -> bool {
        if !self.owns_ship(player_id, cmd.ship_id) {
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
            .inner()
            .get::<&FittingComp>(entity)
            .map(|f| f.slot(cmd.slot).len())
            .unwrap_or(0);
        if current_count >= capacity as usize {
            return false;
        }
        let took = self
            .world
            .inner_mut()
            .get::<&mut InventoryComp>(entity)
            .map(|mut inv| inv.take(cmd.module_id))
            .unwrap_or(false);
        if !took {
            return false;
        }

        use dawn_core::ActivationMode;
        let is_active = matches!(def.activation_mode, ActivationMode::Passive);
        let fitted = self
            .world
            .inner_mut()
            .get::<&mut FittingComp>(entity)
            .map(|mut fitting| {
                fitting.slot_mut(cmd.slot).push(FittedSlot {
                    def,
                    is_active,
                    cycle_remaining: 0,
                    target_ship_id: None,
                });
            })
            .is_ok();
        if !fitted {
            // FittingComp is expected on every spawned ship; if it's somehow
            // missing, undo the inventory take so the module isn't lost.
            if let Ok(mut inv) = self.world.inner_mut().get::<&mut InventoryComp>(entity) {
                inv.add(cmd.module_id);
            }
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
        let Some(&entity) = self.ships.index.get(&cmd.ship_id) else {
            return false;
        };
        let removed =
            if let Ok(mut fitting) = self.world.inner_mut().get::<&mut FittingComp>(entity) {
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

        // Return to inventory. Ships without InventoryComp (NPCs) never reach
        // here in practice (unowned), but insert a fresh one defensively
        // rather than silently dropping the module.
        let added = self
            .world
            .inner_mut()
            .get::<&mut InventoryComp>(entity)
            .map(|mut inv| inv.add(cmd.module_id))
            .is_ok();
        if !added {
            let _ = self.world.inner_mut().insert_one(entity, {
                let mut inv = InventoryComp::empty();
                inv.add(cmd.module_id);
                inv
            });
        }

        self.apply_fitting_and_emit(cmd.ship_id, entity);
        true
    }

    /// Move the entire stack of `cmd.item_id` out of the docked ship's own
    /// cargo (`InventoryComp`) into the caller's station inventory
    /// (ADR-0034 section 9B). Whole-stack only -- no partial-count transfer.
    /// Rejected if the caller doesn't own `cmd.ship_id`, isn't currently
    /// docked at `cmd.station_id`, or the ship's cargo has none of
    /// `cmd.item_id`.
    pub fn transfer_to_station_owned(
        &mut self,
        player_id: PlayerId,
        cmd: TransferToStationCommand,
    ) -> bool {
        if !self.owns_ship(player_id, cmd.ship_id) {
            return false;
        }
        if !self.can_use_station(player_id, cmd.station_id) {
            return false;
        }
        let Some(&entity) = self.ships.index.get(&cmd.ship_id) else {
            return false;
        };
        let taken = self
            .world
            .inner_mut()
            .get::<&mut InventoryComp>(entity)
            .map(|mut inv| inv.take_all(cmd.item_id))
            .unwrap_or(0);
        if taken == 0 {
            return false;
        }
        self.credit_station_item(player_id, cmd.item_id, taken);
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
    use dawn_core::{NodeId, Position, SectorBounds, SectorId, SlotKind};

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

    fn spawn_owned_player(node: &mut SimulationNode) -> (dawn_core::PlayerId, dawn_core::ShipId) {
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);
        (player_id, ship_id)
    }

    #[test]
    fn player_ship_starts_with_one_of_every_registered_module() {
        let mut node = node_with_modules();
        let (_player, ship_id) = spawn_owned_player(&mut node);
        let entity = *node.ships.index.get(&ship_id).unwrap();
        let inv = node.world.inner().get::<&InventoryComp>(entity).unwrap();
        assert_eq!(inv.items.len(), modules::all_modules().len());
    }

    #[test]
    fn fit_module_owned_moves_an_item_from_inventory_into_an_empty_slot() {
        let mut node = node_with_modules();
        let (player, ship_id) = spawn_owned_player(&mut node);
        let entity = *node.ships.index.get(&ship_id).unwrap();
        let before_inv_len =
            total_items(&node.world.inner().get::<&InventoryComp>(entity).unwrap());

        assert!(node.fit_module_owned(
            player,
            FitModuleCommand {
                ship_id,
                slot: SlotKind::High,
                module_id: modules::MODULE_RAILGUN_MEDIUM,
            }
        ));

        let fitting = node.world.inner().get::<&FittingComp>(entity).unwrap();
        assert!(fitting
            .slot(SlotKind::High)
            .iter()
            .any(|s| s.def.id == modules::MODULE_RAILGUN_MEDIUM));
        let inv = node.world.inner().get::<&InventoryComp>(entity).unwrap();
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
            .inner_mut()
            .get::<&mut InventoryComp>(entity)
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
            .inner_mut()
            .get::<&mut InventoryComp>(entity)
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
            .inner()
            .get::<&InventoryComp>(entity)
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

        let fitting = node.world.inner().get::<&FittingComp>(entity).unwrap();
        assert!(!fitting
            .slot(SlotKind::High)
            .iter()
            .any(|s| s.def.id == modules::MODULE_RAILGUN_SMALL));
        let inv = node.world.inner().get::<&InventoryComp>(entity).unwrap();
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

    #[test]
    fn fit_then_unfit_round_trips_inventory_count() {
        let mut node = node_with_modules();
        let (player, ship_id) = spawn_owned_player(&mut node);
        let entity = *node.ships.index.get(&ship_id).unwrap();
        let start_len = node
            .world
            .inner()
            .get::<&InventoryComp>(entity)
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

        let inv = node.world.inner().get::<&InventoryComp>(entity).unwrap();
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
            .inner_mut()
            .get::<&mut InventoryComp>(entity)
            .unwrap()
            .add_item(dawn_core::ItemId::ScrapMetal, 4);

        assert!(node.transfer_to_station_owned(
            player,
            TransferToStationCommand {
                ship_id,
                station_id,
                item_id: dawn_core::ItemId::ScrapMetal,
            }
        ));

        let inv = node.world.inner().get::<&InventoryComp>(entity).unwrap();
        assert_eq!(inv.item_count(dawn_core::ItemId::ScrapMetal), 0);
        assert_eq!(
            node.station_inventory(player)
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
            }
        ));
    }

    #[test]
    fn transfer_to_station_owned_is_rejected_when_not_docked_at_the_station() {
        let mut node = node_with_modules();
        let (player, ship_id) = spawn_owned_player(&mut node);
        let entity = *node.ships.index.get(&ship_id).unwrap();
        node.world
            .inner_mut()
            .get::<&mut InventoryComp>(entity)
            .unwrap()
            .add_item(dawn_core::ItemId::ScrapMetal, 4);

        assert!(!node.transfer_to_station_owned(
            player,
            TransferToStationCommand {
                ship_id,
                station_id: dawn_core::StationId(0),
                item_id: dawn_core::ItemId::ScrapMetal,
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
            }
        ));
    }
}
