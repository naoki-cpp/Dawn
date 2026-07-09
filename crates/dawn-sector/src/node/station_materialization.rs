//! Station-backed ship materialization rules.

use dawn_core::{
    events::{PackagedShipBuilt, ShipAssembled, ShipDisassembled},
    AssembleCommand, BuildPackagedShipCommand, DisassembleShipCommand, DomainEvent, ItemId,
    PlayerId, Position, ShipId, Velocity,
};
use dawn_ecs::components::{FittingComp, HullComp, InventoryComp, IsNpcComp, ShipStatsComp};
use dawn_event_store::store::EventStore;

use super::{
    station::{StationOperationOutcome, StationOperationRejection},
    SimulationNode,
};

impl<S: EventStore> SimulationNode<S> {
    pub const SCRAP_METAL_COST_PER_PACKAGED_SHIP: u64 = 1;

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
        if let Err(reason) =
            self.try_debit_station_item(player_id, cmd.station_id, ItemId::ScrapMetal, scrap_cost)
        {
            return StationOperationOutcome::Rejected {
                ship_id: cmd.ship_id,
                reason,
            };
        }
        self.credit_station_item(
            player_id,
            cmd.station_id,
            ItemId::PackagedShip(cmd.ship_type_id),
            1,
        );
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
        let cargo_items = self
            .world
            .inner()
            .get::<&InventoryComp>(entity)
            .map(|inv| inv.items.clone())
            .unwrap_or_default();

        self.credit_station_item(
            player_id,
            cmd.station_id,
            ItemId::PackagedShip(ship_type_id),
            1,
        );
        for (item_id, count) in cargo_items {
            self.credit_station_item(player_id, cmd.station_id, item_id, count);
        }
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
        self.try_debit_station_item(
            player_id,
            cmd.station_id,
            ItemId::PackagedShip(cmd.ship_type_id),
            1,
        )?;

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
    use dawn_core::{
        AssembleCommand, BuildPackagedShipCommand, DisassembleShipCommand, DockCommand,
        FitModuleCommand, ItemId, NodeId, SectorBounds, SectorId, ShipTypeId, SlotKind, StationId,
        UnfitModuleCommand,
    };
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
    fn build_packaged_ship_consumes_scrap_and_credits_the_packaged_hull() {
        let mut node = node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);
        let station_id = StationId(0);
        let station = node.station(station_id).expect("demo station exists");
        node.set_spawn_anchor_abs(ship_id, station.abs_m);
        assert!(accepted(node.dock_owned(
            player_id,
            ship_id,
            DockCommand { station_id }
        )));
        node.credit_station_item(player_id, station_id, ItemId::ScrapMetal, 3);

        assert!(accepted(node.build_packaged_ship_owned(
            player_id,
            BuildPackagedShipCommand {
                ship_id,
                station_id,
                ship_type_id: ShipTypeId(1),
            }
        )));

        assert_eq!(
            node.station_item_count(player_id, station_id, ItemId::ScrapMetal),
            2
        );
        assert_eq!(
            node.station_item_count(player_id, station_id, ItemId::PackagedShip(ShipTypeId(1))),
            1
        );
    }

    #[test]
    fn build_packaged_ship_is_rejected_when_not_docked() {
        let mut node = node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);

        assert_eq!(
            node.build_packaged_ship_owned(
                player_id,
                BuildPackagedShipCommand {
                    ship_id,
                    station_id: StationId(0),
                    ship_type_id: ShipTypeId(1),
                }
            ),
            StationOperationOutcome::Rejected {
                ship_id,
                reason: StationOperationRejection::MissingDockedStationContext,
            }
        );
    }

    #[test]
    fn disassemble_ship_credits_packaged_hull_and_removes_ship() {
        let mut node = node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);
        let station_id = StationId(0);
        let station = node.station(station_id).expect("demo station exists");
        node.set_spawn_anchor_abs(ship_id, station.abs_m);
        assert!(accepted(node.dock_owned(
            player_id,
            ship_id,
            DockCommand { station_id }
        )));
        assert!(node.unfit_module_owned(
            player_id,
            UnfitModuleCommand {
                ship_id,
                slot: SlotKind::High,
                module_id: crate::modules::MODULE_RAILGUN_SMALL,
            }
        ));
        assert!(node.unfit_module_owned(
            player_id,
            UnfitModuleCommand {
                ship_id,
                slot: SlotKind::Mid,
                module_id: crate::modules::MODULE_AFTERBURNER,
            }
        ));
        assert!(node.unfit_module_owned(
            player_id,
            UnfitModuleCommand {
                ship_id,
                slot: SlotKind::Mid,
                module_id: crate::modules::MODULE_FOLD_DISRUPTOR,
            }
        ));

        let ship_type_id = *node
            .ships
            .type_ids
            .get(&ship_id)
            .expect("player ship type is registered");

        assert!(accepted(node.disassemble_ship_owned(
            player_id,
            DisassembleShipCommand {
                ship_id,
                station_id,
            }
        )));

        assert_eq!(
            node.station_item_count(player_id, station_id, ItemId::PackagedShip(ship_type_id)),
            2
        );
        assert!(!node.ships.index.contains_key(&ship_id));
    }

    #[test]
    fn disassemble_ship_moves_ship_cargo_into_station_inventory() {
        let mut node = node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);
        let station_id = StationId(0);
        let station = node.station(station_id).expect("demo station exists");
        node.set_spawn_anchor_abs(ship_id, station.abs_m);
        assert!(accepted(node.dock_owned(
            player_id,
            ship_id,
            DockCommand { station_id }
        )));
        assert!(node.unfit_module_owned(
            player_id,
            UnfitModuleCommand {
                ship_id,
                slot: SlotKind::High,
                module_id: crate::modules::MODULE_RAILGUN_SMALL,
            }
        ));
        assert!(node.unfit_module_owned(
            player_id,
            UnfitModuleCommand {
                ship_id,
                slot: SlotKind::Mid,
                module_id: crate::modules::MODULE_AFTERBURNER,
            }
        ));
        assert!(node.unfit_module_owned(
            player_id,
            UnfitModuleCommand {
                ship_id,
                slot: SlotKind::Mid,
                module_id: crate::modules::MODULE_FOLD_DISRUPTOR,
            }
        ));
        let entity = *node.ships.index.get(&ship_id).expect("ship exists before disassemble");
        node.world
            .inner_mut()
            .get::<&mut InventoryComp>(entity)
            .expect("player ship keeps cargo inventory")
            .add_item(ItemId::ScrapMetal, 7);
        let expected_railguns = node
            .world
            .inner()
            .get::<&InventoryComp>(entity)
            .expect("player ship keeps cargo inventory")
            .item_count(ItemId::Module(crate::modules::MODULE_RAILGUN_SMALL));

        assert!(accepted(node.disassemble_ship_owned(
            player_id,
            DisassembleShipCommand {
                ship_id,
                station_id,
            }
        )));

        assert_eq!(
            node.station_item_count(player_id, station_id, ItemId::ScrapMetal),
            7,
            "disassemble must preserve ship cargo by moving it into station inventory"
        );
        assert_eq!(
            node.station_item_count(
                player_id,
                station_id,
                ItemId::Module(crate::modules::MODULE_RAILGUN_SMALL)
            ),
            expected_railguns,
            "disassemble must preserve the full cargo stack for each item"
        );
    }

    #[test]
    fn disassemble_ship_is_rejected_when_any_module_is_fitted() {
        let mut node = node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);
        let station_id = StationId(0);
        let station = node.station(station_id).expect("demo station exists");
        node.set_spawn_anchor_abs(ship_id, station.abs_m);
        assert!(accepted(node.dock_owned(
            player_id,
            ship_id,
            DockCommand { station_id }
        )));
        node.fit_module(FitModuleCommand {
            ship_id,
            slot: SlotKind::High,
            module_id: crate::modules::MODULE_RAILGUN_SMALL,
        });

        assert_eq!(
            node.disassemble_ship_owned(
                player_id,
                DisassembleShipCommand {
                    ship_id,
                    station_id,
                }
            ),
            StationOperationOutcome::Rejected {
                ship_id,
                reason: StationOperationRejection::ShipIsFitted,
            }
        );
    }

    #[test]
    fn assemble_ship_creates_a_new_docked_owned_ship_and_debits_the_packaged_ship_item() {
        let mut node = node();
        let player_id = node.next_player_id();
        let active_ship_id = node.spawn_player_ship(player_id);
        let station_id = StationId(0);
        let station = node.station(station_id).expect("demo station exists");
        node.set_spawn_anchor_abs(active_ship_id, station.abs_m);
        assert!(accepted(node.dock_owned(
            player_id,
            active_ship_id,
            DockCommand { station_id }
        )));
        node.credit_station_item(
            player_id,
            station_id,
            ItemId::PackagedShip(ShipTypeId(1)),
            1,
        );

        let new_ship_id = node
            .assemble_ship_owned(
                player_id,
                AssembleCommand {
                    station_id,
                    ship_type_id: ShipTypeId(1),
                },
            )
            .expect("assemble succeeds");

        assert_eq!(
            node.station_item_count(player_id, station_id, ItemId::PackagedShip(ShipTypeId(1))),
            0
        );
        assert_eq!(node.docked_station(new_ship_id), Some(station_id));
        assert_eq!(node.ships.owners.get(&new_ship_id), Some(&player_id));
    }
}
