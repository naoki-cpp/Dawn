//! Station-family client command dispatch for `SimulationNode`.
//!
//! This module owns the station/inventory-management command family routing:
//! dock / undock / build / disassemble / select-active / assemble /
//! disembark / transfer-to-station. `node::commands` keeps the outer
//! `ClientCommand` entry point and converts this module's private outcome
//! into the public `ClientCommandFollowup`.

use dawn_core::{
    AssembleCommand, BuildPackagedShipCommand, DisassembleShipCommand, DockCommand, PlayerId,
    SelectActiveShipCommand, TransferToStationCommand, UndockCommand,
};
use dawn_event_store::store::EventStore;

use super::SimulationNode;

pub(super) enum StationDispatchCommand {
    Dock(DockCommand),
    Undock(UndockCommand),
    BuildPackagedShip(BuildPackagedShipCommand),
    DisassembleShip(DisassembleShipCommand),
    SelectActiveShip(SelectActiveShipCommand),
    Assemble(AssembleCommand),
    Disembark,
    TransferToStation(TransferToStationCommand),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StationDispatchOutcome {
    NoFollowup,
    RefreshPlayerLoadout,
}

impl<S: EventStore> SimulationNode<S> {
    pub(super) fn dispatch_station_command(
        &mut self,
        player_id: PlayerId,
        cmd: StationDispatchCommand,
    ) -> StationDispatchOutcome {
        match cmd {
            StationDispatchCommand::Dock(cmd) => {
                let Some(ship_id) = self.ships.active_ship.get(&player_id).copied() else {
                    return StationDispatchOutcome::NoFollowup;
                };
                self.dock_owned(player_id, ship_id, cmd);
                StationDispatchOutcome::RefreshPlayerLoadout
            }
            StationDispatchCommand::Undock(_) => {
                let Some(ship_id) = self.ships.active_ship.get(&player_id).copied() else {
                    return StationDispatchOutcome::NoFollowup;
                };
                self.undock_owned(player_id, ship_id);
                StationDispatchOutcome::RefreshPlayerLoadout
            }
            StationDispatchCommand::BuildPackagedShip(cmd) => {
                self.build_packaged_ship_owned(player_id, cmd);
                StationDispatchOutcome::RefreshPlayerLoadout
            }
            StationDispatchCommand::DisassembleShip(cmd) => {
                self.disassemble_ship_owned(player_id, cmd);
                StationDispatchOutcome::RefreshPlayerLoadout
            }
            StationDispatchCommand::SelectActiveShip(cmd) => {
                self.select_active_ship_owned(player_id, cmd);
                StationDispatchOutcome::RefreshPlayerLoadout
            }
            StationDispatchCommand::Assemble(cmd) => {
                let _ = self.assemble_ship_owned(player_id, cmd);
                StationDispatchOutcome::RefreshPlayerLoadout
            }
            StationDispatchCommand::Disembark => {
                let _ = self.disembark_owned(player_id);
                StationDispatchOutcome::RefreshPlayerLoadout
            }
            StationDispatchCommand::TransferToStation(cmd) => {
                self.transfer_to_station_owned(player_id, cmd);
                StationDispatchOutcome::RefreshPlayerLoadout
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use dawn_core::{
        BuildPackagedShipCommand, DockCommand, ItemId, NodeId, SectorBounds, SectorId, ShipTypeId,
        StationId,
    };
    use dawn_event_store::InMemoryEventStore;

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

    fn docked_owned_player(
        node: &mut SimulationNode<InMemoryEventStore>,
    ) -> (PlayerId, dawn_core::ShipId) {
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);
        let station = node.station(StationId(0)).expect("demo station exists");
        node.set_spawn_anchor_abs(ship_id, station.abs_m);
        node.dock_owned(
            player_id,
            ship_id,
            DockCommand {
                station_id: StationId(0),
            },
        );
        (player_id, ship_id)
    }

    #[test]
    fn dock_dispatch_without_active_ship_returns_no_followup() {
        let mut node = node();
        let player_id = node.next_player_id();

        let outcome = node.dispatch_station_command(
            player_id,
            StationDispatchCommand::Dock(DockCommand {
                station_id: StationId(0),
            }),
        );

        assert_eq!(outcome, StationDispatchOutcome::NoFollowup);
    }

    #[test]
    fn packaged_ship_build_dispatch_refreshes_player_loadout_after_station_operation() {
        let mut node = node();
        let (player_id, ship_id) = docked_owned_player(&mut node);
        node.credit_station_item(player_id, StationId(0), ItemId::ScrapMetal, 10);

        let outcome = node.dispatch_station_command(
            player_id,
            StationDispatchCommand::BuildPackagedShip(BuildPackagedShipCommand {
                ship_id,
                station_id: StationId(0),
                ship_type_id: ShipTypeId(1),
            }),
        );

        assert_eq!(outcome, StationDispatchOutcome::RefreshPlayerLoadout);
        assert_eq!(
            node.station_item_count(player_id, StationId(0), ItemId::PackagedShip(ShipTypeId(1)),),
            1,
            "station dispatch must still perform the underlying packaged ship build"
        );
    }
}
