//! Station-family client command dispatch for `SimulationNode`.
//!
//! This module owns the station/inventory-management command family routing:
//! dock / undock / build / disassemble / select-active / assemble /
//! disembark / transfer-to-station. `node::commands` keeps the outer
//! `ClientCommand` entry point, resolves the caller's active ship once, and
//! converts this module's private effect into the public
//! `ClientCommandFollowup`.
//!
//! `StationDispatchCommand` deliberately re-encodes the eight station
//! variants of `ClientCommand` rather than this module matching
//! `ClientCommand` directly (ADR-0047): the private enum is what stops a
//! non-station command reaching here, and taking the wider type would need a
//! catch-all arm that silently treats anything unrecognised as "not mine".
//! The eight outer arms that pay for it are thin routing, not logic.

use dawn_core::{
    AssembleCommand, BuildPackagedShipCommand, DisassembleShipCommand, DockCommand, PlayerId,
    SelectActiveShipCommand, ShipId, TransferToStationCommand, UndockCommand,
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

/// What the server should do *after* a station command has been applied.
///
/// Named an effect, not an outcome, to keep it distinct from
/// `StationOperationOutcome` (`node/station.rs`), which reports whether the
/// domain operation itself was accepted or rejected and why. This type is a
/// layer above that: the operation's own success is already handled inside
/// the `*_owned` methods, and what survives to here is only the follow-up the
/// client connection owes as a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StationDispatchEffect {
    NoFollowup,
    RefreshPlayerLoadout,
}

impl<S: EventStore> SimulationNode<S> {
    /// Route one station-family command.
    ///
    /// `active_ship` is resolved once by `apply_client_command` at the command
    /// entry point and passed down rather than re-read here: which ship a
    /// player's commands route to (ADR-0037) is a property of receiving a
    /// command, not of being a station command, so it belongs to the caller.
    /// Dock/Undock used to look it up again out of `self.ships.active_ship`,
    /// which read the same value twice and left the rule in two places to
    /// drift apart. The other six variants carry their own target and ignore
    /// it.
    pub(super) fn dispatch_station_command(
        &mut self,
        player_id: PlayerId,
        active_ship: Option<ShipId>,
        cmd: StationDispatchCommand,
    ) -> StationDispatchEffect {
        match cmd {
            StationDispatchCommand::Dock(cmd) => {
                let Some(ship_id) = active_ship else {
                    return StationDispatchEffect::NoFollowup;
                };
                self.dock_owned(player_id, ship_id, cmd);
                StationDispatchEffect::RefreshPlayerLoadout
            }
            StationDispatchCommand::Undock(_) => {
                let Some(ship_id) = active_ship else {
                    return StationDispatchEffect::NoFollowup;
                };
                self.undock_owned(player_id, ship_id);
                StationDispatchEffect::RefreshPlayerLoadout
            }
            StationDispatchCommand::BuildPackagedShip(cmd) => {
                self.build_packaged_ship_owned(player_id, cmd);
                StationDispatchEffect::RefreshPlayerLoadout
            }
            StationDispatchCommand::DisassembleShip(cmd) => {
                self.disassemble_ship_owned(player_id, cmd);
                StationDispatchEffect::RefreshPlayerLoadout
            }
            StationDispatchCommand::SelectActiveShip(cmd) => {
                self.select_active_ship_owned(player_id, cmd);
                StationDispatchEffect::RefreshPlayerLoadout
            }
            StationDispatchCommand::Assemble(cmd) => {
                let _ = self.assemble_ship_owned(player_id, cmd);
                StationDispatchEffect::RefreshPlayerLoadout
            }
            StationDispatchCommand::Disembark => {
                let _ = self.disembark_owned(player_id);
                StationDispatchEffect::RefreshPlayerLoadout
            }
            StationDispatchCommand::TransferToStation(cmd) => {
                self.transfer_to_station_owned(player_id, cmd);
                StationDispatchEffect::RefreshPlayerLoadout
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

        let effect = node.dispatch_station_command(
            player_id,
            None,
            StationDispatchCommand::Dock(DockCommand {
                station_id: StationId(0),
            }),
        );

        assert_eq!(effect, StationDispatchEffect::NoFollowup);
    }

    #[test]
    fn packaged_ship_build_dispatch_refreshes_player_loadout_after_station_operation() {
        let mut node = node();
        let (player_id, ship_id) = docked_owned_player(&mut node);
        node.credit_station_item(player_id, StationId(0), ItemId::ScrapMetal, 10);

        let effect = node.dispatch_station_command(
            player_id,
            Some(ship_id),
            StationDispatchCommand::BuildPackagedShip(BuildPackagedShipCommand {
                ship_id,
                station_id: StationId(0),
                ship_type_id: ShipTypeId(1),
            }),
        );

        assert_eq!(effect, StationDispatchEffect::RefreshPlayerLoadout);
        assert_eq!(
            node.station_item_count(player_id, StationId(0), ItemId::PackagedShip(ShipTypeId(1)),),
            1,
            "station dispatch must still perform the underlying packaged ship build"
        );
    }

    /// The active ship now arrives as an argument instead of being re-read
    /// here, so the entry point is the single place that decides which ship a
    /// player's commands route to. This covers that wiring through the public
    /// API rather than the private dispatcher: `SelectActiveShip` switches the
    /// active ship, and the *next* command must reach the newly selected one.
    #[test]
    fn an_undock_after_select_active_ship_routes_to_the_newly_selected_ship() {
        use dawn_core::{ClientCommand, SelectActiveShipCommand, UndockCommand};

        let mut node = node();
        let (player_id, first_ship) = docked_owned_player(&mut node);

        // A second owned ship, docked at the same station.
        let second_ship = node.spawn_player_ship(player_id);
        let station = node.station(StationId(0)).expect("demo station exists");
        node.set_spawn_anchor_abs(second_ship, station.abs_m);
        node.dock_owned(
            player_id,
            second_ship,
            DockCommand {
                station_id: StationId(0),
            },
        );
        assert_ne!(first_ship, second_ship);

        // Spawning already makes a ship active (`spawn_player_ship_at`), so
        // hand the active slot back to the first ship first. Without this the
        // `SelectActiveShip` below is rejected as `AlreadyActive` and switches
        // nothing, leaving the test green for the wrong reason.
        node.select_active_ship_owned(
            player_id,
            SelectActiveShipCommand {
                ship_id: first_ship,
            },
        );
        assert!(
            node.is_active_ship(player_id, first_ship),
            "fixture precondition: the first ship is the active one"
        );

        let mut locks = Vec::new();
        node.apply_client_command(
            player_id,
            ClientCommand::SelectActiveShip(SelectActiveShipCommand {
                ship_id: second_ship,
            }),
            &mut locks,
        );
        assert!(
            node.is_active_ship(player_id, second_ship),
            "SelectActiveShip must actually switch the active ship"
        );

        // Undock routes to the active ship. If the entry point were still
        // handing down a stale value, this would undock `first_ship`.
        node.apply_client_command(
            player_id,
            ClientCommand::Undock(UndockCommand {}),
            &mut locks,
        );

        assert!(
            !node.is_ship_docked(second_ship),
            "Undock must reach the ship SelectActiveShip just switched to"
        );
        assert!(
            node.is_ship_docked(first_ship),
            "the previously active ship must stay docked"
        );
    }
}
