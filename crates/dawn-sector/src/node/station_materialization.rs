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

        // Salvage the ship's unfitted cargo (ADR-0032/0034 InventoryComp)
        // into the station before the entity is despawned below --
        // Disassemble only requires an unfitted *hull* (the `is_fitted`
        // check above), not an empty cargo hold, so any Module/ScrapMetal
        // stacks riding along would otherwise silently vanish with the
        // entity instead of following the ship into its packaged form.
        let salvaged_cargo: Vec<(ItemId, u64)> = self
            .world
            .inner_mut()
            .get::<&mut InventoryComp>(entity)
            .map(|mut inventory| std::mem::take(&mut inventory.items).into_iter().collect())
            .unwrap_or_default();
        for (item_id, count) in salvaged_cargo {
            self.credit_station_item(player_id, cmd.station_id, item_id, count);
        }

        self.credit_station_item(
            player_id,
            cmd.station_id,
            ItemId::PackagedShip(ship_type_id),
            1,
        );
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

        // Regression: the three modules unfit above (now sitting in the
        // ship's cargo hold, not fitted) must follow the ship into station
        // inventory rather than vanishing with the despawned entity. Count
        // is 2, not 1: `spawn_player_ship` seeds one of every registered
        // module into cargo (ADR-0032) *and separately* fits one more of
        // each via the privileged, non-inventory-consuming `fit_module`
        // path -- unfitting returns that second copy to the same cargo
        // stack the seed already populated.
        assert_eq!(
            node.station_item_count(
                player_id,
                station_id,
                ItemId::Module(crate::modules::MODULE_RAILGUN_SMALL)
            ),
            2
        );
        assert_eq!(
            node.station_item_count(
                player_id,
                station_id,
                ItemId::Module(crate::modules::MODULE_AFTERBURNER)
            ),
            2
        );
        assert_eq!(
            node.station_item_count(
                player_id,
                station_id,
                ItemId::Module(crate::modules::MODULE_FOLD_DISRUPTOR)
            ),
            2
        );
    }

    #[test]
    fn disassemble_ship_salvages_scrap_metal_cargo_into_the_station() {
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
        // Fully unfit so the ship qualifies for Disassemble.
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
        // Scrap Metal earned from a kill, sitting in cargo alongside the
        // unfit modules -- not just a Module stack.
        let entity = *node.ships.index.get(&ship_id).expect("ship exists");
        node.world
            .inner_mut()
            .get::<&mut dawn_ecs::components::InventoryComp>(entity)
            .expect("player ship has an InventoryComp")
            .add_item(ItemId::ScrapMetal, 5);

        assert!(accepted(node.disassemble_ship_owned(
            player_id,
            DisassembleShipCommand {
                ship_id,
                station_id,
            }
        )));

        assert_eq!(
            node.station_item_count(player_id, station_id, ItemId::ScrapMetal),
            5
        );
    }

    #[test]
    fn player_can_assemble_a_new_ship_right_after_disassembling_their_only_ship() {
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

        // The player now owns zero live ships -- the exact "shipless docked
        // player" state ownership.md §8 exists to make recoverable. Assemble
        // should succeed using the PackagedShip Disassemble just credited.
        assert!(
            node.can_use_station(player_id, station_id),
            "docked_players context must survive the ship entity being despawned"
        );
        let new_ship = node.assemble_ship_owned(
            player_id,
            AssembleCommand {
                station_id,
                ship_type_id,
            },
        );
        assert!(
            new_ship.is_ok(),
            "Assemble must succeed right after Disassemble, got {new_ship:?}"
        );
    }

    #[test]
    fn disassembling_one_owned_ship_does_not_affect_a_second_owned_ship() {
        let mut node = node();
        let player_id = node.next_player_id();
        let ship_a = node.spawn_player_ship(player_id);
        let station_id = StationId(0);
        let station = node.station(station_id).expect("demo station exists");
        node.set_spawn_anchor_abs(ship_a, station.abs_m);
        assert!(accepted(node.dock_owned(
            player_id,
            ship_a,
            DockCommand { station_id }
        )));

        // Give the player a second, independent owned ship via Assemble
        // (ADR-0037), same as a real "player has more than one ship" state.
        node.credit_station_item(
            player_id,
            station_id,
            ItemId::PackagedShip(ShipTypeId(1)),
            1,
        );
        let ship_b = node
            .assemble_ship_owned(
                player_id,
                AssembleCommand {
                    station_id,
                    ship_type_id: ShipTypeId(1),
                },
            )
            .expect("assemble succeeds with a packaged hull in station inventory");

        // Fully unfit ship_a so it qualifies for Disassemble.
        assert!(node.unfit_module_owned(
            player_id,
            UnfitModuleCommand {
                ship_id: ship_a,
                slot: SlotKind::High,
                module_id: crate::modules::MODULE_RAILGUN_SMALL,
            }
        ));
        assert!(node.unfit_module_owned(
            player_id,
            UnfitModuleCommand {
                ship_id: ship_a,
                slot: SlotKind::Mid,
                module_id: crate::modules::MODULE_AFTERBURNER,
            }
        ));
        assert!(node.unfit_module_owned(
            player_id,
            UnfitModuleCommand {
                ship_id: ship_a,
                slot: SlotKind::Mid,
                module_id: crate::modules::MODULE_FOLD_DISRUPTOR,
            }
        ));

        assert!(accepted(node.disassemble_ship_owned(
            player_id,
            DisassembleShipCommand {
                ship_id: ship_a,
                station_id,
            }
        )));

        // ship_b must still be a live, owned ship -- untouched by ship_a's
        // disassembly.
        assert!(
            node.ships.index.contains_key(&ship_b),
            "ship_b's ECS entity must still exist"
        );
        assert_eq!(
            node.ships.owners.get(&ship_b),
            Some(&player_id),
            "ship_b must still be owned by the same player"
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
