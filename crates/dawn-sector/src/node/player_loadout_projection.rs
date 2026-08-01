//! PlayerLoadout wire projection for `SimulationNode`.
//!
//! This deep module owns the player-facing fitting/cargo/station-inventory
//! snapshot sent after handshake and after fitting/station operations.

use dawn_core::{ItemId, PlayerId, ShipId};
use dawn_ecs::components::{FittingComp, InventoryComp};
use dawn_event_store::store::EventStore;
#[cfg(test)]
use dawn_wire::ItemWire;
use dawn_wire::{
    ItemRowWire, ModuleRowWire, OwnedShipRowWire, PlayerLoadoutWire, SlotCapacityWire,
};

use super::SimulationNode;

impl<S: EventStore> SimulationNode<S> {
    /// The one seam every `ItemRowWire` (ship cargo, station inventory) goes
    /// through. The Item variant remains typed; only presentation metadata is
    /// added here. `None` if the registry backing `item_id` no longer has a
    /// definition for it (stale/renamed module or ship type).
    fn item_id_to_row_json(&self, item_id: ItemId, count: u64) -> Option<ItemRowWire> {
        match item_id {
            ItemId::Module(module_id) => {
                self.module_registry.get(&module_id).map(|def| ItemRowWire {
                    item_id: item_id.into(),
                    name: def.name.clone(),
                    kind: format!("{:?}", def.kind),
                    slot: format!("{:?}", def.slot),
                    count,
                })
            }
            ItemId::PackagedShip(ship_type_id) => {
                self.ship_type_registry
                    .get(&ship_type_id)
                    .map(|def| ItemRowWire {
                        item_id: item_id.into(),
                        name: def.name.clone(),
                        kind: String::new(),
                        slot: String::new(),
                        count,
                    })
            }
            ItemId::ScrapMetal => Some(ItemRowWire {
                item_id: item_id.into(),
                name: "Scrap Metal".to_string(),
                kind: String::new(),
                slot: String::new(),
                count,
            }),
        }
    }

    /// Return the player ship's loadout state as a PlayerLoadout wire message.
    ///
    /// Sent after Welcome + InitialState on connect, and again after every
    /// Fit/Unfit (ADR-0032).
    pub fn build_player_loadout_json(&self, ship_id: ShipId) -> Option<PlayerLoadoutWire> {
        let entity = self.ships.index.get(&ship_id)?;
        let fitting = self.world.get::<FittingComp>(*entity)?;

        let mut modules: Vec<ModuleRowWire> = Vec::new();
        let slot_names = [
            ("High", &fitting.high),
            ("Mid", &fitting.mid),
            ("Low", &fitting.low),
            ("Rig", &fitting.rig),
        ];
        for (slot_name, slots) in &slot_names {
            for (i, slot) in slots.iter().enumerate() {
                modules.push(ModuleRowWire {
                    slot: slot_name.to_string(),
                    index: i as u32,
                    module_id: slot.def.id.0,
                    name: slot.def.name.clone(),
                    kind: slot.def.kind,
                    is_active: slot.is_active,
                    is_active_module: matches!(
                        slot.def.activation_mode,
                        dawn_core::ActivationMode::Active
                    ),
                    cap_cost_per_cycle: slot.def.cap_cost_per_cycle,
                    cycle_time_ticks: slot.def.cycle_time_ticks,
                    stat_delta: slot.def.stat_delta,
                });
            }
        }

        let inventory: Vec<ItemRowWire> = self
            .world
            .get::<InventoryComp>(*entity)
            .map(|inv| {
                inv.items
                    .iter()
                    .filter_map(|(&item_id, &count)| self.item_id_to_row_json(item_id, count))
                    .collect()
            })
            .unwrap_or_default();

        let player_id = self.ships.owners.get(&ship_id).copied();
        let docked_station_id = player_id.and_then(|pid| self.player_docked_station(pid));
        let docked_station_name = docked_station_id
            .and_then(|station_id| self.station(station_id))
            .map(|station| station.name.clone());
        let station_inventory = player_id
            .map(|pid| self.station_inventory_json(pid))
            .unwrap_or_default();
        let active_ship_id = player_id.and_then(|pid| self.ships.active_ship.get(&pid).copied());
        let owned_ships = player_id
            .map(|pid| self.owned_ships_json(pid))
            .unwrap_or_default();

        let layout = self
            .ships
            .type_ids
            .get(&ship_id)
            .and_then(|t| self.ship_type_registry.get(t))
            .map(|d| d.slot_layout);
        let slot_capacity = SlotCapacityWire {
            high: layout.map(|l| l.high).unwrap_or(0),
            mid: layout.map(|l| l.mid).unwrap_or(0),
            low: layout.map(|l| l.low).unwrap_or(0),
            rig: layout.map(|l| l.rig).unwrap_or(0),
        };

        Some(PlayerLoadoutWire {
            tick: self.current_tick.value(),
            modules,
            inventory,
            station_inventory,
            docked_station_id: docked_station_id.map(|id| id.0),
            docked_station_name,
            slot_capacity,
            active_ship_id: active_ship_id.map(|id| id.raw()),
            owned_ships,
        })
    }

    /// Same wire message as [`Self::build_player_loadout_json`], keyed by
    /// `player_id` instead of a specific ship.
    pub fn build_player_loadout_json_for_player(
        &self,
        player_id: PlayerId,
    ) -> Option<PlayerLoadoutWire> {
        if let Some(ship_id) = self.ships.active_ship.get(&player_id).copied() {
            return self.build_player_loadout_json(ship_id);
        }
        let docked_station_id = self.player_docked_station(player_id);
        let docked_station_name = docked_station_id
            .and_then(|station_id| self.station(station_id))
            .map(|station| station.name.clone());
        let station_inventory = self.station_inventory_json(player_id);
        let owned_ships = self.owned_ships_json(player_id);

        Some(PlayerLoadoutWire {
            tick: self.current_tick.value(),
            modules: Vec::new(),
            inventory: Vec::new(),
            station_inventory,
            docked_station_id: docked_station_id.map(|id| id.0),
            docked_station_name,
            slot_capacity: SlotCapacityWire {
                high: 0,
                mid: 0,
                low: 0,
                rig: 0,
            },
            active_ship_id: None,
            owned_ships,
        })
    }

    /// Every ship `player_id` owns, active or not.
    fn owned_ships_json(&self, player_id: PlayerId) -> Vec<OwnedShipRowWire> {
        let active_ship_id = self.ships.active_ship.get(&player_id).copied();
        self.ships
            .owners
            .iter()
            .filter(|(_, owner)| **owner == player_id)
            .map(|(&ship_id, _)| {
                let ship_type_id = self.ships.type_ids.get(&ship_id).copied();
                let ship_type_name = ship_type_id
                    .and_then(|t| self.ship_type_registry.get(&t))
                    .map(|def| def.name.clone());
                OwnedShipRowWire {
                    ship_id: ship_id.raw(),
                    ship_type_id: ship_type_id.map(|t| t.0),
                    ship_type_name,
                    docked_station_id: self.docked_station(ship_id).map(|id| id.0),
                    is_active: Some(ship_id) == active_ship_id,
                }
            })
            .collect()
    }

    /// Station inventory as wire rows for `player_id`, empty if the player
    /// isn't currently docked anywhere.
    fn station_inventory_json(&self, player_id: PlayerId) -> Vec<ItemRowWire> {
        let Some(station_id) = self.player_docked_station(player_id) else {
            return Vec::new();
        };
        self.station_inventory(player_id, station_id)
            .map(|inventory| {
                inventory
                    .iter()
                    .filter_map(|(&item_id, &count)| self.item_id_to_row_json(item_id, count))
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::station::StationOperationOutcome;
    use dawn_core::{
        AssembleCommand, DockCommand, NodeId, Position, SectorBounds, SectorId, StationId,
        UnfitModuleCommand,
    };

    fn mem_node() -> SimulationNode {
        SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        )
    }

    #[test]
    fn player_loadout_json_includes_scrap_metal_inventory_rows() {
        let mut node = mem_node();
        for def in crate::game_data::test_catalog().modules().to_vec() {
            node.register_module(def);
        }
        for def in crate::game_data::test_catalog().ship_types().to_vec() {
            node.register_ship_type(def);
        }

        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);
        let entity = *node.ships.index.get(&ship_id).unwrap();
        node.world
            .get_mut::<InventoryComp>(entity)
            .unwrap()
            .add_item(ItemId::ScrapMetal, 3);

        let payload = node.build_player_loadout_json(ship_id).unwrap();
        let scrap = payload
            .inventory
            .iter()
            .find(|row| row.item_id == ItemWire::ScrapMetal)
            .unwrap();
        assert_eq!(scrap.name, "Scrap Metal");
        assert_eq!(scrap.count, 3);
    }

    /// `ItemRowWire` always carries every field by construction (the type
    /// system enforces this now, not a runtime shape check) -- this test
    /// only confirms the three `ItemId` variants each produce a row.
    #[test]
    fn every_item_id_variant_produces_a_row_in_the_loadout_payload() {
        let mut node = mem_node();
        for def in crate::game_data::test_catalog().modules().to_vec() {
            node.register_module(def);
        }
        for def in crate::game_data::test_catalog().ship_types().to_vec() {
            node.register_ship_type(def);
        }

        let player_id = node.next_player_id();
        let station = node.station(StationId(0)).unwrap().clone();
        let ship_id = node.spawn_player_ship_at_pub(player_id, station.position);
        assert!(matches!(
            node.dock_owned(
                player_id,
                ship_id,
                DockCommand {
                    station_id: StationId(0)
                }
            ),
            StationOperationOutcome::Accepted { .. }
        ));

        let entity = *node.ships.index.get(&ship_id).unwrap();
        node.world
            .get_mut::<InventoryComp>(entity)
            .unwrap()
            .add_item(ItemId::ScrapMetal, 1);
        node.unfit_module_owned(
            player_id,
            UnfitModuleCommand {
                ship_id,
                slot: dawn_core::SlotKind::High,
                module_id: crate::modules::MODULE_RAILGUN_SMALL,
            },
        );

        let payload = node.build_player_loadout_json(ship_id).unwrap();

        let mut rows: Vec<&ItemRowWire> = payload.inventory.iter().collect();
        rows.extend(payload.station_inventory.iter());
        assert!(rows
            .iter()
            .any(|r| matches!(r.item_id, ItemWire::Module { .. })));
        assert!(rows.iter().any(|r| r.item_id == ItemWire::ScrapMetal));
        assert!(rows
            .iter()
            .any(|r| matches!(r.item_id, ItemWire::PackagedShip { .. })));
    }

    #[test]
    fn item_id_to_row_json_carries_the_count_for_every_item_id_variant() {
        let mut node = mem_node();
        for def in crate::game_data::test_catalog().modules().to_vec() {
            node.register_module(def);
        }
        for def in crate::game_data::test_catalog().ship_types().to_vec() {
            node.register_ship_type(def);
        }

        let module_id = crate::modules::MODULE_RAILGUN_SMALL;
        let ship_type_id = crate::ship_types::SHIP_TYPE_MAGPIE;
        for item_id in [
            ItemId::Module(module_id),
            ItemId::PackagedShip(ship_type_id),
            ItemId::ScrapMetal,
        ] {
            let row = node.item_id_to_row_json(item_id, 3).unwrap();
            assert_eq!(row.count, 3);
        }
    }

    #[test]
    fn item_id_to_row_json_returns_none_for_an_item_id_with_no_registry_definition() {
        let node = mem_node();
        assert!(node
            .item_id_to_row_json(ItemId::Module(dawn_core::ModuleId(999)), 1)
            .is_none());
        assert!(node
            .item_id_to_row_json(ItemId::PackagedShip(dawn_core::ShipTypeId(999)), 1)
            .is_none());
    }

    #[test]
    fn player_loadout_json_carries_docked_station_context_and_station_inventory() {
        let mut node = mem_node();
        for def in crate::game_data::test_catalog().modules().to_vec() {
            node.register_module(def);
        }
        for def in crate::game_data::test_catalog().ship_types().to_vec() {
            node.register_ship_type(def);
        }

        let player_id = node.next_player_id();
        let station = node.station(StationId(0)).unwrap().clone();
        let ship_id = node.spawn_player_ship_at_pub(player_id, station.position);
        node.credit_station_item(player_id, StationId(0), ItemId::ScrapMetal, 5);
        assert!(matches!(
            node.dock_owned(
                player_id,
                ship_id,
                DockCommand {
                    station_id: StationId(0)
                }
            ),
            StationOperationOutcome::Accepted { .. }
        ));

        let payload = node.build_player_loadout_json(ship_id).unwrap();
        assert_eq!(payload.docked_station_id, Some(0));
        assert_eq!(
            payload.docked_station_name.as_deref(),
            Some("Forge Station")
        );
        let scrap = payload
            .station_inventory
            .iter()
            .find(|row| row.item_id == ItemWire::ScrapMetal)
            .unwrap();
        assert_eq!(scrap.count, 5);
    }

    #[test]
    fn player_loadout_json_uses_null_dock_context_after_undock() {
        let mut node = mem_node();
        let player_id = node.next_player_id();
        let station = node.station(StationId(0)).unwrap().clone();
        let ship_id = node.spawn_player_ship_at_pub(player_id, station.position);
        assert!(matches!(
            node.dock_owned(
                player_id,
                ship_id,
                DockCommand {
                    station_id: StationId(0)
                }
            ),
            StationOperationOutcome::Accepted { .. }
        ));
        assert!(matches!(
            node.undock_owned(player_id, ship_id),
            StationOperationOutcome::Accepted { .. }
        ));

        let payload = node.build_player_loadout_json(ship_id).unwrap();
        assert_eq!(payload.docked_station_id, None);
        assert_eq!(payload.docked_station_name, None);
    }

    #[test]
    fn player_loadout_json_reports_the_true_active_ship_id() {
        let mut node = mem_node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);

        let payload = node.build_player_loadout_json(ship_id).unwrap();
        assert_eq!(payload.active_ship_id, Some(ship_id.raw()));
    }

    #[test]
    fn player_loadout_json_for_player_reports_null_active_ship_id_when_shipless() {
        let mut node = mem_node();
        let player_id = node.next_player_id();
        node.docked_players.insert(player_id, StationId(0));

        let payload = node
            .build_player_loadout_json_for_player(player_id)
            .unwrap();
        assert_eq!(payload.active_ship_id, None);
        assert!(payload.modules.is_empty());
    }

    #[test]
    fn player_loadout_json_for_player_reports_active_ship_id_when_disembarked_ship_still_owned() {
        let mut node = mem_node();
        let player_id = node.next_player_id();
        let station = node.station(StationId(0)).unwrap().clone();
        let ship_id = node.spawn_player_ship_at_pub(player_id, station.position);
        assert!(matches!(
            node.dock_owned(
                player_id,
                ship_id,
                DockCommand {
                    station_id: StationId(0)
                }
            ),
            StationOperationOutcome::Accepted { .. }
        ));
        node.disembark_owned(player_id).unwrap();

        let payload = node
            .build_player_loadout_json_for_player(player_id)
            .unwrap();
        assert_eq!(payload.active_ship_id, None);
    }

    #[test]
    fn player_loadout_json_lists_every_owned_ship_with_active_and_docked_flags() {
        let mut node = mem_node();
        for def in crate::game_data::test_catalog().ship_types().to_vec() {
            node.register_ship_type(def);
        }
        let player_id = node.next_player_id();
        let station = node.station(StationId(0)).unwrap().clone();
        let ship_id = node.spawn_player_ship_at_pub(player_id, station.position);
        assert!(matches!(
            node.dock_owned(
                player_id,
                ship_id,
                DockCommand {
                    station_id: StationId(0)
                }
            ),
            StationOperationOutcome::Accepted { .. }
        ));

        let second_ship_id = node
            .assemble_ship_owned(
                player_id,
                AssembleCommand {
                    station_id: StationId(0),
                    ship_type_id: crate::ship_types::SHIP_TYPE_MAGPIE,
                },
            )
            .unwrap();

        let payload = node.build_player_loadout_json(ship_id).unwrap();
        assert_eq!(payload.owned_ships.len(), 2);

        let first_row = payload
            .owned_ships
            .iter()
            .find(|row| row.ship_id == ship_id.raw())
            .unwrap();
        assert!(first_row.is_active);
        assert_eq!(first_row.docked_station_id, Some(0));

        let second_row = payload
            .owned_ships
            .iter()
            .find(|row| row.ship_id == second_ship_id.raw())
            .unwrap();
        assert!(!second_row.is_active);
        assert_eq!(second_row.docked_station_id, Some(0));
    }
}
