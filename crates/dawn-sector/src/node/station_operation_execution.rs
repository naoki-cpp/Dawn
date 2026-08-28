//! Execution seam for accepted Station operations.
//!
//! The sibling Station modules validate commands and build a plan. This
//! module owns the ordered live side effects: authoritative Station overlay
//! staging, runtime state mutation, and the corresponding `DomainEvent`
//! publication.
//!
//! ADR-0049 changes the target contract: the journal-owned Station aggregate is
//! durable before live apply, then the required SQLite projection is applied
//! idempotently before acknowledgement/publication. #277 owns the replacement
//! repository seam, so this module must not be treated as the final authority
//! ordering.

use dawn_core::{
    events::{PackagedShipBuilt, ShipAssembled, ShipDisassembled, ShipDocked, ShipUndocked},
    DomainEvent, ItemId, PlayerId, ShipId, ShipTypeId, StationId, Velocity,
};
use dawn_ecs::components::{InventoryComp, IsNpcComp};

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

/// Runtime-only Station state transition used by live execution.
///
/// These directives deliberately contain no Station inventory mutation and no
/// public-event output. The caller performs durable inventory work first and
/// emits the public event after applying this state change.
#[derive(Debug, Clone, Copy)]
pub(super) enum StationRuntimeState {
    Dock {
        ship_id: ShipId,
        station_id: StationId,
    },
    Undock {
        ship_id: ShipId,
    },
    Disassemble {
        ship_id: ShipId,
    },
    Assemble {
        player_id: PlayerId,
        ship_id: ShipId,
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

impl SimulationNode {
    /// Apply only the ECS/map/index portion of an accepted Station operation.
    ///
    /// This is the single owner of the runtime mutation. It must remain free
    /// of SQLite writes and event appends.
    pub(super) fn apply_station_runtime_state(&mut self, state: StationRuntimeState) {
        match state {
            StationRuntimeState::Dock {
                ship_id,
                station_id,
            } => {
                self.settle_ship_into_station(ship_id, station_id);
                self.stations.dock_ship(ship_id, station_id);
                if let Some(player_id) = self.players.owners.get(&ship_id).copied() {
                    self.stations.dock_player(player_id, station_id);
                }
            }
            StationRuntimeState::Undock { ship_id } => {
                if let Some(player_id) = self.players.owners.get(&ship_id).copied() {
                    self.stations.undock_player(player_id);
                }
                self.stations.undock_ship(ship_id);
            }
            StationRuntimeState::Disassemble { ship_id } => {
                self.remove_ship(ship_id);
            }
            StationRuntimeState::Assemble {
                player_id,
                ship_id,
                station_id,
                ship_type_id,
            } => {
                if !self.simulation.ships.index.contains_key(&ship_id) {
                    self.insert_ship_entity(
                        ship_id,
                        ship_type_id,
                        dawn_core::Position::ORIGIN,
                        Velocity::ZERO,
                    );
                    if let Some(&entity) = self.simulation.ships.index.get(&ship_id) {
                        let _ = self.simulation.world.remove_one::<IsNpcComp>(entity);
                    }
                    self.settle_ship_into_station(ship_id, station_id);
                }
                self.stations.dock_ship(ship_id, station_id);
                self.players.owners.insert(ship_id, player_id);
                let counter = ship_id.0.counter();
                if counter >= self.simulation.id_counter {
                    self.simulation.id_counter = counter + 1;
                }
            }
        }
    }

    /// Execute one already-validated Station plan.
    ///
    /// Inventory debits and credits update only the bounded authoritative
    /// overlay. Each plan then applies its complete runtime state change and
    /// emits exactly one public event as the final step. The enclosing durable
    /// runtime frame carries the ordered overlay mutations in its
    /// `RecoveryDelta` and applies the SQLite projection after live apply.
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
                debug_assert_eq!(self.players.owners.get(&ship_id).copied(), Some(player_id));
                self.apply_station_runtime_state(StationRuntimeState::Dock {
                    ship_id,
                    station_id,
                });
                self.append_station_event(DomainEvent::ShipDocked(ShipDocked {
                    ship_id,
                    station_id,
                    tick: self.simulation.current_tick,
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
                debug_assert_eq!(self.players.owners.get(&ship_id).copied(), Some(player_id));
                self.apply_station_runtime_state(StationRuntimeState::Undock { ship_id });
                self.append_station_event(DomainEvent::ShipUndocked(ShipUndocked {
                    ship_id,
                    station_id,
                    tick: self.simulation.current_tick,
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
                )
                .map_err(StationOperationRejection::projection_read)?;
                self.append_station_event(DomainEvent::PackagedShipBuilt(PackagedShipBuilt {
                    ship_id,
                    player_id,
                    station_id,
                    ship_type_id,
                    scrap_cost,
                    tick: self.simulation.current_tick,
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
                let Some(&entity) = self.simulation.ships.index.get(&ship_id) else {
                    return Err(StationOperationRejection::ShipNotFound);
                };

                let salvaged_cargo: Vec<(ItemId, u64)> = self
                    .simulation
                    .world
                    .get_mut::<InventoryComp>(entity)
                    .map(|mut inventory| std::mem::take(&mut inventory.items).into_iter().collect())
                    .unwrap_or_default();
                for (item_id, count) in salvaged_cargo {
                    self.credit_station_item(player_id, station_id, item_id, count)
                        .map_err(StationOperationRejection::projection_read)?;
                }
                self.credit_station_item(
                    player_id,
                    station_id,
                    ItemId::PackagedShip(ship_type_id),
                    1,
                )
                .map_err(StationOperationRejection::projection_read)?;
                self.apply_station_runtime_state(StationRuntimeState::Disassemble { ship_id });
                self.append_station_event(DomainEvent::ShipDisassembled(ShipDisassembled {
                    ship_id,
                    player_id,
                    station_id,
                    ship_type_id,
                    tick: self.simulation.current_tick,
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

                let ship_id = ShipId::new(self.node_id, self.simulation.id_counter);
                self.apply_station_runtime_state(StationRuntimeState::Assemble {
                    player_id,
                    ship_id,
                    station_id,
                    ship_type_id,
                });

                self.append_station_event(DomainEvent::ShipAssembled(ShipAssembled {
                    ship_id,
                    player_id,
                    station_id,
                    ship_type_id,
                    tick: self.simulation.current_tick,
                }));
                Ok(StationOperationExecution::Assembled(ship_id))
            }
        }
    }

    fn append_station_event(&mut self, event: DomainEvent) {
        self.emit_event(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{ItemId, NodeId, SectorBounds, SectorId, ShipTypeId, StationId};

    fn node() -> SimulationNode {
        SimulationNode::new_test(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        )
    }

    fn paired_player_nodes() -> (SimulationNode, SimulationNode, PlayerId, ShipId) {
        let mut live = node();
        let mut comparison = node();
        let player_id = live.next_player_id();
        assert_eq!(comparison.next_player_id(), player_id);
        let ship_id = live.spawn_player_ship(player_id);
        assert_eq!(comparison.spawn_player_ship(player_id), ship_id);
        (live, comparison, player_id, ship_id)
    }

    fn assert_same_runtime_state(live: &SimulationNode, comparison: &SimulationNode) {
        let mut live_snapshot = live.take_snapshot();
        let mut comparison_snapshot = comparison.take_snapshot();
        live_snapshot.covered_recovery_index = 0.into();
        comparison_snapshot.covered_recovery_index = 0.into();
        assert_eq!(
            postcard::to_stdvec(&live_snapshot).unwrap(),
            postcard::to_stdvec(&comparison_snapshot).unwrap()
        );
        assert_eq!(
            live.simulation.ships.type_ids,
            comparison.simulation.ships.type_ids
        );
        assert_eq!(live.players.owners, comparison.players.owners);
        assert_eq!(live.players.active_ship, comparison.players.active_ship);
    }

    #[test]
    fn rejected_station_debit_does_not_append_an_event_or_create_output() {
        let mut node = node();
        let before = node.pending_event_count();

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
        assert_eq!(node.pending_event_count(), before);
        assert_eq!(
            node.station_item_count(
                PlayerId(1),
                StationId(0),
                ItemId::PackagedShip(ShipTypeId(1))
            )
            .unwrap(),
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
        )
        .unwrap();
        let before = node.pending_event_count();

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
            node.station_item_count(player_id, StationId(0), ItemId::PackagedShip(ShipTypeId(1)))
                .unwrap(),
            0
        );
        assert_eq!(node.docked_station(ship_id), Some(StationId(0)));
        assert_eq!(node.pending_event_count(), before + 1);
        assert!(matches!(
            node.pending_events().last(),
            Some(DomainEvent::ShipAssembled(event)) if event.ship_id == ship_id
        ));
    }

    #[test]
    fn dock_execution_is_deterministic_across_equivalent_nodes() {
        let (mut live, mut comparison, player_id, ship_id) = paired_player_nodes();
        let station_id = StationId(0);

        live.execute_station_operation(StationOperationPlan::Dock {
            player_id,
            ship_id,
            station_id,
        })
        .unwrap();
        comparison
            .execute_station_operation(StationOperationPlan::Dock {
                player_id,
                ship_id,
                station_id,
            })
            .unwrap();

        assert_same_runtime_state(&live, &comparison);
    }

    #[test]
    fn undock_execution_is_deterministic_across_equivalent_nodes() {
        let (mut live, mut comparison, player_id, ship_id) = paired_player_nodes();
        let station_id = StationId(0);
        live.execute_station_operation(StationOperationPlan::Dock {
            player_id,
            ship_id,
            station_id,
        })
        .unwrap();
        comparison
            .execute_station_operation(StationOperationPlan::Dock {
                player_id,
                ship_id,
                station_id,
            })
            .unwrap();

        live.execute_station_operation(StationOperationPlan::Undock {
            player_id,
            ship_id,
            station_id,
        })
        .unwrap();
        comparison
            .execute_station_operation(StationOperationPlan::Undock {
                player_id,
                ship_id,
                station_id,
            })
            .unwrap();

        assert_same_runtime_state(&live, &comparison);
    }

    #[test]
    fn disassemble_live_execution_updates_runtime_state_and_projection() {
        let (mut live, mut comparison, player_id, ship_id) = paired_player_nodes();
        let station_id = StationId(0);
        let ship_type_id = live.simulation.ships.type_ids[&ship_id];

        live.execute_station_operation(StationOperationPlan::DisassembleShip {
            player_id,
            ship_id,
            station_id,
            ship_type_id,
        })
        .unwrap();
        comparison
            .execute_station_operation(StationOperationPlan::DisassembleShip {
                player_id,
                ship_id,
                station_id,
                ship_type_id,
            })
            .unwrap();
        assert_same_runtime_state(&live, &comparison);
    }

    #[test]
    fn assemble_live_execution_updates_runtime_state_and_projection() {
        let mut live = node();
        let mut comparison = node();
        let player_id = PlayerId(1);
        let station_id = StationId(0);
        let ship_type_id = ShipTypeId(1);
        let packaged = ItemId::PackagedShip(ship_type_id);
        live.credit_station_item(player_id, station_id, packaged, 1)
            .unwrap();
        comparison
            .credit_station_item(player_id, station_id, packaged, 1)
            .unwrap();

        let result = live
            .execute_station_operation(StationOperationPlan::AssembleShip {
                player_id,
                station_id,
                ship_type_id,
            })
            .unwrap();
        let StationOperationExecution::Assembled(live_ship_id) = result else {
            panic!("expected an assembled ship result");
        };
        let comparison_result = comparison
            .execute_station_operation(StationOperationPlan::AssembleShip {
                player_id,
                station_id,
                ship_type_id,
            })
            .unwrap();
        let StationOperationExecution::Assembled(comparison_ship_id) = comparison_result else {
            panic!("expected an assembled ship result");
        };

        assert_eq!(live_ship_id, comparison_ship_id);
        assert_eq!(
            comparison
                .station_item_count(player_id, station_id, packaged)
                .unwrap(),
            0
        );
        assert_eq!(
            comparison.docked_station(comparison_ship_id),
            Some(station_id)
        );
        assert_same_runtime_state(&live, &comparison);
    }
}
