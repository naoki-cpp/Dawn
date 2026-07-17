//! Execution seam for accepted Station operations.
//!
//! The sibling Station modules validate commands and build a plan. This
//! module owns the ordered live side effects: durable inventory writes first,
//! ECS/index updates next, and the corresponding DomainEvent last. SQLite
//! remains the durable Station inventory authority (ADR-0038); this seam does
//! not attempt to make SQLite and the append-only Event Store one transaction.

use dawn_core::{
    events::{PackagedShipBuilt, ShipAssembled, ShipDisassembled, ShipDocked, ShipUndocked},
    DomainEvent, ItemId, PlayerId, ShipId, ShipTypeId, StationId, Velocity,
};
use dawn_ecs::components::{InventoryComp, IsNpcComp};
use dawn_event_store::store::EventStore;

use super::{
    station::{StationOperationOutcome, StationOperationRejection},
    SimulationNode,
};

/// Validated work handed from a Station operation module to the execution
/// seam. Keeping this as data makes the side-effect order visible without
/// exposing the runtime's ECS or SQLite handles to command dispatch.
pub(super) enum StationOperationPlan {
    Dock {
        player_id: PlayerId,
        ship_id: ShipId,
        station_id: StationId,
    },
    Undock {
        player_id: PlayerId,
        ship_id: ShipId,
        station_id: StationId,
    },
    BuildPackagedShip {
        player_id: PlayerId,
        ship_id: ShipId,
        station_id: StationId,
        ship_type_id: ShipTypeId,
        scrap_cost: u64,
    },
    DisassembleShip {
        player_id: PlayerId,
        ship_id: ShipId,
        station_id: StationId,
        ship_type_id: ShipTypeId,
    },
    AssembleShip {
        player_id: PlayerId,
        station_id: StationId,
        ship_type_id: ShipTypeId,
    },
}

/// Result of executing a plan. Assemble has no pre-existing ship to report,
/// so it returns the newly allocated ship separately from the usual outcome.
pub(super) enum StationOperationExecution {
    Outcome(StationOperationOutcome),
    Assembled(ShipId),
}

impl<S: EventStore> SimulationNode<S> {
    /// Execute one already-validated Station plan.
    ///
    /// A fallible inventory debit happens before any other live mutation. Once
    /// that succeeds, each plan applies its complete runtime state change and
    /// appends exactly one event as the final step. This preserves ADR-0038's
    /// accepted SQLite/Event Store crash window while making the order a single
    /// local invariant for all event-sourced Station operations.
    pub(super) fn execute_station_operation(
        &mut self,
        plan: StationOperationPlan,
    ) -> Result<StationOperationExecution, StationOperationRejection> {
        match plan {
            StationOperationPlan::Dock {
                player_id,
                ship_id,
                station_id,
            } => {
                self.settle_ship_into_station(ship_id, station_id);
                self.docked_ships.insert(ship_id, station_id);
                self.docked_players.insert(player_id, station_id);
                self.append_station_event(DomainEvent::ShipDocked(ShipDocked {
                    ship_id,
                    station_id,
                    tick: self.current_tick,
                }));
                Ok(StationOperationExecution::Outcome(
                    StationOperationOutcome::Accepted { ship_id },
                ))
            }
            StationOperationPlan::Undock {
                player_id,
                ship_id,
                station_id,
            } => {
                self.docked_ships.remove(&ship_id);
                self.docked_players.remove(&player_id);
                self.append_station_event(DomainEvent::ShipUndocked(ShipUndocked {
                    ship_id,
                    station_id,
                    tick: self.current_tick,
                }));
                Ok(StationOperationExecution::Outcome(
                    StationOperationOutcome::Accepted { ship_id },
                ))
            }
            StationOperationPlan::BuildPackagedShip {
                player_id,
                ship_id,
                station_id,
                ship_type_id,
                scrap_cost,
            } => {
                self.try_debit_station_item(player_id, station_id, ItemId::ScrapMetal, scrap_cost)?;
                self.credit_station_item(
                    player_id,
                    station_id,
                    ItemId::PackagedShip(ship_type_id),
                    1,
                );
                self.append_station_event(DomainEvent::PackagedShipBuilt(PackagedShipBuilt {
                    ship_id,
                    player_id,
                    station_id,
                    ship_type_id,
                    scrap_cost,
                    tick: self.current_tick,
                }));
                Ok(StationOperationExecution::Outcome(
                    StationOperationOutcome::Accepted { ship_id },
                ))
            }
            StationOperationPlan::DisassembleShip {
                player_id,
                ship_id,
                station_id,
                ship_type_id,
            } => {
                let Some(&entity) = self.ships.index.get(&ship_id) else {
                    return Err(StationOperationRejection::ShipNotFound);
                };

                let salvaged_cargo: Vec<(ItemId, u64)> = self
                    .world
                    .get_mut::<InventoryComp>(entity)
                    .map(|mut inventory| std::mem::take(&mut inventory.items).into_iter().collect())
                    .unwrap_or_default();
                for (item_id, count) in salvaged_cargo {
                    self.credit_station_item(player_id, station_id, item_id, count);
                }
                self.credit_station_item(
                    player_id,
                    station_id,
                    ItemId::PackagedShip(ship_type_id),
                    1,
                );
                self.remove_ship(ship_id);
                self.append_station_event(DomainEvent::ShipDisassembled(ShipDisassembled {
                    ship_id,
                    player_id,
                    station_id,
                    ship_type_id,
                    tick: self.current_tick,
                }));
                Ok(StationOperationExecution::Outcome(
                    StationOperationOutcome::Accepted { ship_id },
                ))
            }
            StationOperationPlan::AssembleShip {
                player_id,
                station_id,
                ship_type_id,
            } => {
                self.try_debit_station_item(
                    player_id,
                    station_id,
                    ItemId::PackagedShip(ship_type_id),
                    1,
                )?;

                let ship_id = ShipId::new(self.node_id, self.id_counter);
                self.id_counter += 1;
                self.insert_ship_entity(
                    ship_id,
                    ship_type_id,
                    dawn_core::Position::ORIGIN,
                    Velocity::ZERO,
                );
                if let Some(&entity) = self.ships.index.get(&ship_id) {
                    let _ = self.world.remove_one::<IsNpcComp>(entity);
                }
                self.settle_ship_into_station(ship_id, station_id);
                self.docked_ships.insert(ship_id, station_id);
                self.ships.owners.insert(ship_id, player_id);

                self.append_station_event(DomainEvent::ShipAssembled(ShipAssembled {
                    ship_id,
                    player_id,
                    station_id,
                    ship_type_id,
                    tick: self.current_tick,
                }));
                Ok(StationOperationExecution::Assembled(ship_id))
            }
        }
    }

    fn append_station_event(&mut self, event: DomainEvent) {
        self.event_store.append(event);
    }
}

#[cfg(test)]
mod tests {
    use dawn_core::{ItemId, NodeId, SectorBounds, SectorId, ShipTypeId, StationId};
    use dawn_event_store::{store::EventStore, InMemoryEventStore};

    use crate::{modules, ship_types};

    use super::*;

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
    fn rejected_station_debit_does_not_append_an_event_or_create_output() {
        let mut node = node();
        let before = node.event_store.len();

        let result = node.execute_station_operation(StationOperationPlan::BuildPackagedShip {
            player_id: PlayerId(1),
            ship_id: ShipId::new(NodeId(0), 10),
            station_id: StationId(0),
            ship_type_id: ShipTypeId(1),
            scrap_cost: 1,
        });

        assert!(matches!(
            result,
            Err(StationOperationRejection::MissingStationItem)
        ));
        assert_eq!(node.event_store.len(), before);
        assert_eq!(
            node.station_item_count(
                PlayerId(1),
                StationId(0),
                ItemId::PackagedShip(ShipTypeId(1))
            ),
            0
        );
    }

    #[test]
    fn assembled_plan_debits_inventory_before_appending_its_event() {
        let mut node = node();
        let player_id = PlayerId(1);
        node.credit_station_item(
            player_id,
            StationId(0),
            ItemId::PackagedShip(ShipTypeId(1)),
            1,
        );
        let before = node.event_store.len();

        let result = node
            .execute_station_operation(StationOperationPlan::AssembleShip {
                player_id,
                station_id: StationId(0),
                ship_type_id: ShipTypeId(1),
            })
            .expect("packaged ship should assemble");

        let StationOperationExecution::Assembled(ship_id) = result else {
            panic!("expected an assembled ship result");
        };
        assert_eq!(
            node.station_item_count(player_id, StationId(0), ItemId::PackagedShip(ShipTypeId(1))),
            0
        );
        assert_eq!(node.docked_station(ship_id), Some(StationId(0)));
        assert_eq!(node.event_store.len(), before + 1);
        assert!(matches!(
            node.event_store.all_records().last().map(|record| &record.event),
            Some(DomainEvent::ShipAssembled(event)) if event.ship_id == ship_id
        ));
    }
}
