//! Ship-facing station lifecycle rules.

use dawn_core::{DockCommand, DomainEvent, PlayerId, ShipId, StationId};
use dawn_ecs::components::{LockComp, ThrustComp, VelocityComp, WarpComp};

use super::{
    station::{StationOperationOutcome, StationOperationRejection},
    station_operation_execution::{StationOperationExecution, StationOperationPlan},
    SimulationNode,
};

impl SimulationNode {
    /// True when `player_id`'s ship is currently docked at `station_id`.
    pub fn can_use_station(&self, player_id: PlayerId, station_id: StationId) -> bool {
        self.stations.docked_station_for_player(player_id) == Some(station_id)
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
        self.stations.docked_station_for_ship(ship_id)
    }

    pub fn is_ship_docked(&self, ship_id: ShipId) -> bool {
        self.stations.is_ship_docked(ship_id)
    }

    pub fn player_docked_station(&self, player_id: PlayerId) -> Option<StationId> {
        self.stations.docked_station_for_player(player_id)
    }

    /// Re-adopt a restored ship for a resumed player and reconcile Station access.
    ///
    /// Ship ownership is restored from the snapshot or Transit handoff before
    /// a resume begins. `docked_ships` is still authoritative after replay; use
    /// it here to repair the player-facing Station context once identity is
    /// re-established.
    pub fn resume_player_ship(&mut self, ship_id: ShipId, player_id: PlayerId) -> bool {
        if !self.adopt_player_ship(ship_id, player_id) {
            return false;
        }

        if let Some(station_id) = self.stations.docked_station_for_ship(ship_id) {
            self.stations.dock_player(player_id, station_id);
        } else {
            self.stations.undock_player(player_id);
        }
        true
    }

    pub(super) fn settle_ship_into_station(&mut self, ship_id: ShipId, station_id: StationId) {
        let Some(station_abs) = self.station(station_id).map(|station| station.abs_m) else {
            return;
        };
        let Some(&entity) = self.simulation.ships.index.get(&ship_id) else {
            return;
        };
        let _ = self.simulation.world.remove_one::<WarpComp>(entity);
        self.clear_steering_modes(entity);
        if let Some(mut thrust) = self.simulation.world.get_mut::<ThrustComp>(entity) {
            *thrust = ThrustComp::ZERO;
        }
        if let Some(mut velocity) = self.simulation.world.get_mut::<VelocityComp>(entity) {
            velocity.0 = dawn_core::Velocity::ZERO;
        }
        self.place_entity_at_absolute(entity, station_abs);
    }

    /// Dock the caller's active ship (ADR-0037: `ship_id` is always the
    /// caller's resolved active ship — `apply_client_request` never calls
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
        if self.stations.is_ship_docked(ship_id) {
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
        match self.execute_station_operation(StationOperationPlan::Dock {
            player_id,
            ship_id,
            station_id: cmd.station_id,
        }) {
            Ok(StationOperationExecution::Outcome(outcome)) => outcome,
            Ok(StationOperationExecution::Assembled(_)) => {
                unreachable!("Dock plan cannot assemble a ship")
            }
            Err(reason) => StationOperationOutcome::Rejected { ship_id, reason },
        }
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
        let Some(station_id) = self.stations.docked_station_for_ship(ship_id) else {
            return StationOperationOutcome::Rejected {
                ship_id,
                reason: StationOperationRejection::ShipNotDocked,
            };
        };
        match self.execute_station_operation(StationOperationPlan::Undock {
            player_id,
            ship_id,
            station_id,
        }) {
            Ok(StationOperationExecution::Outcome(outcome)) => outcome,
            Ok(StationOperationExecution::Assembled(_)) => {
                unreachable!("Undock plan cannot assemble a ship")
            }
            Err(reason) => StationOperationOutcome::Rejected { ship_id, reason },
        }
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
        if self.players.active_ship.get(&player_id) == Some(&cmd.ship_id) {
            return StationOperationOutcome::Rejected {
                ship_id: cmd.ship_id,
                reason: StationOperationRejection::AlreadyActive,
            };
        }
        let target_station = self.stations.docked_station_for_ship(cmd.ship_id);
        if target_station.is_none()
            || target_station != self.stations.docked_station_for_player(player_id)
        {
            return StationOperationOutcome::Rejected {
                ship_id: cmd.ship_id,
                reason: StationOperationRejection::ShipNotDockedHere,
            };
        }
        self.players.active_ship.insert(player_id, cmd.ship_id);
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
        let Some(ship_id) = self.players.active_ship.get(&player_id).copied() else {
            return Err(StationOperationRejection::NotOwned);
        };
        if !self.is_ship_docked(ship_id) {
            return Err(StationOperationRejection::ShipNotDocked);
        }
        self.players.active_ship.remove(&player_id);
        Ok(ship_id)
    }

    pub fn clear_docked_lock_targets(&mut self, tick: dawn_core::Tick) -> Vec<DomainEvent> {
        let mut events: Vec<DomainEvent> = Vec::new();
        let ship_ids: Vec<ShipId> = self.simulation.ships.index.keys().copied().collect();
        for ship_id in ship_ids {
            let Some(&entity) = self.simulation.ships.index.get(&ship_id) else {
                continue;
            };
            let locker_docked = self.is_ship_docked(ship_id);
            let lost_targets: Vec<ShipId> = match self.simulation.world.get::<LockComp>(entity) {
                Some(lock) => lock
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
                None => Vec::new(),
            };
            if lost_targets.is_empty() {
                continue;
            }
            if let Some(mut lock) = self.simulation.world.get_mut::<LockComp>(entity) {
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
}

#[cfg(test)]
mod tests {
    use dawn_core::{
        events::{ShipDocked, ShipUndocked},
        DockCommand, NodeId, SectorBounds, SectorId, StationId, Tick, WarpTarget,
    };
    use dawn_ecs::components::{ThrustComp, VelocityComp};
    use dawn_event_store::{store::EventStore, InMemoryEventStore};

    use super::*;

    fn accepted(outcome: StationOperationOutcome) -> bool {
        matches!(outcome, StationOperationOutcome::Accepted { .. })
    }

    fn node() -> SimulationNode {
        SimulationNode::new_test(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        )
    }

    fn copied_store(node: &SimulationNode) -> InMemoryEventStore {
        let mut store = InMemoryEventStore::new();
        for event in node.pending_events() {
            store.append(event.clone());
        }
        store
    }

    #[test]
    fn resume_reconciles_a_dock_event_replayed_without_ownership() {
        let mut original = node();
        let player_id = original.next_player_id();
        let ship_id = original.spawn_player_ship(player_id);
        let snapshot = original.take_snapshot();
        let mut store = copied_store(&original);
        store.append(DomainEvent::ShipDocked(ShipDocked {
            ship_id,
            station_id: StationId(0),
            tick: Tick(1),
        }));

        let mut restored = SimulationNode::restore_from_test(
            store,
            &snapshot,
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            crate::game_data::test_catalog().modules(),
            crate::game_data::test_catalog().ship_types(),
        );
        assert_eq!(restored.docked_station(ship_id), Some(StationId(0)));
        assert_eq!(
            restored.player_docked_station(player_id),
            Some(StationId(0))
        );

        assert!(restored.resume_player_ship(ship_id, player_id));
        assert_eq!(
            restored.player_docked_station(player_id),
            Some(StationId(0))
        );
    }

    #[test]
    fn resume_clears_stale_player_context_after_undock_tail_replay() {
        let mut original = node();
        let player_id = original.next_player_id();
        let ship_id = original.spawn_player_ship(player_id);
        original.apply_event_pub(DomainEvent::ShipDocked(ShipDocked {
            ship_id,
            station_id: StationId(0),
            tick: Tick(1),
        }));
        let snapshot = original.take_snapshot();
        let mut store = copied_store(&original);
        store.append(DomainEvent::ShipUndocked(ShipUndocked {
            ship_id,
            station_id: StationId(0),
            tick: Tick(2),
        }));

        let mut restored = SimulationNode::restore_from_test(
            store,
            &snapshot,
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            crate::game_data::test_catalog().modules(),
            crate::game_data::test_catalog().ship_types(),
        );
        assert_eq!(restored.docked_station(ship_id), None);
        assert_eq!(restored.player_docked_station(player_id), None);

        assert!(restored.resume_player_ship(ship_id, player_id));
        assert_eq!(restored.player_docked_station(player_id), None);
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
                station.abs_m[0] + station.docking_radius + 5000.0,
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
        let entity = *node
            .simulation
            .ships
            .index
            .get(&ship_id)
            .expect("ship entity");
        if let Some(mut velocity) = node.simulation.world.get_mut::<VelocityComp>(entity) {
            velocity.0 = dawn_core::Velocity {
                dx: 10.0,
                dy: 0.0,
                dz: 0.0,
            };
        }
        if let Some(mut thrust) = node.simulation.world.get_mut::<ThrustComp>(entity) {
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

        let velocity = node.simulation.world.get::<VelocityComp>(entity).unwrap().0;
        let thrust = *node.simulation.world.get::<ThrustComp>(entity).unwrap();
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
        let entity = *node
            .simulation
            .ships
            .index
            .get(&ship_id)
            .expect("ship entity");
        assert!(accepted(node.dock_owned(
            player_id,
            ship_id,
            DockCommand {
                station_id: StationId(0),
            }
        )));

        let before = *node.simulation.world.get::<ThrustComp>(entity).unwrap();
        assert!(node.apply_move_command_owned(
            player_id,
            ship_id,
            dawn_core::Position::new(5000.0, 0.0, 0.0)
        ));
        let after = *node.simulation.world.get::<ThrustComp>(entity).unwrap();
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
}
