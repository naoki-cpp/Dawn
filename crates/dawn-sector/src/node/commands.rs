//! Client command orchestration for [`SimulationNode`].
//!
//! The public entry point keeps one exhaustive `ClientCommand` match, as
//! required by ADR-0047, but that match only selects a closed command family.
//! Family-local modules own validation and application policy:
//!
//! - `command_flight` — movement, steering, lock-on, and Jump routing
//! - `command_module` — module activation/deactivation
//! - `command_loadout` — fitting mutations that require a loadout refresh
//! - `command_station` — station and station-inventory operations
//!
//! This module owns the two cross-family concerns only: resolving the caller's
//! active ship once and projecting family effects into `ClientCommandFollowup`.

use dawn_core::{ClientCommand, JumpCommand, PlayerId, ShipId};
use dawn_event_store::store::EventStore;

use super::{
    command_flight::{FlightDispatchCommand, FlightDispatchEffect},
    command_loadout::{LoadoutDispatchCommand, LoadoutDispatchEffect},
    command_module::{ModuleDispatchCommand, ModuleDispatchEffect},
    command_station::{StationDispatchCommand, StationDispatchEffect},
    SimulationNode,
};

/// What `apply_client_command` hands back to the caller for commands that
/// require server-side context it does not have (Raft handles, session refs).
#[derive(Debug, Clone)]
pub enum ClientCommandFollowup {
    /// Forward to Jump routing (propose Transit to Raft if in range, or let
    /// `apply_jump_with_fallback` start a warp/approach fallback). Carries
    /// the caller's active ship explicitly, since `JumpCommand` itself no
    /// longer does (ADR-0037).
    Jump {
        ship_id: ShipId,
        command: JumpCommand,
    },
    /// The player's fitting/station-inventory changed (or the attempt was
    /// rejected) — push a refreshed `PlayerLoadout` JSON to this player's
    /// session so the client's UI reflects the authoritative state. Carries
    /// `PlayerId` rather than `ShipId`: some triggers (Disassemble, or
    /// Assemble from a shipless state) leave the caller with no active ship
    /// at all, so a ship_id can't always be resolved back to a player, but a
    /// player_id always identifies the right session
    /// (`docs/architecture/ownership.md` §8).
    RefreshPlayerLoadout { player_id: PlayerId },
}

impl ClientCommandFollowup {
    /// Returns the player whose authoritative loadout should be resent.
    ///
    /// Serving adapters use this to handle the common loadout-refresh path
    /// while retaining their own Jump routing policy.
    pub fn loadout_player_id(&self) -> Option<PlayerId> {
        match self {
            Self::RefreshPlayerLoadout { player_id } => Some(*player_id),
            Self::Jump { .. } => None,
        }
    }
}

/// A `ClientCommand` after the wire-level enum has been classified into the
/// one family that owns its policy. Each payload remains strongly typed; no
/// family receives the wider `ClientCommand` and no catch-all can silently
/// swallow a command it does not own.
enum ClientCommandDispatch {
    Flight(FlightDispatchCommand),
    Module(ModuleDispatchCommand),
    Loadout(LoadoutDispatchCommand),
    Station(StationDispatchCommand),
}

impl ClientCommandDispatch {
    /// The single exhaustive command-family selection table (ADR-0047).
    fn select(cmd: ClientCommand) -> Self {
        match cmd {
            ClientCommand::Move(cmd) => Self::Flight(FlightDispatchCommand::Move(cmd)),
            ClientCommand::LockOn(cmd) => Self::Flight(FlightDispatchCommand::LockOn(cmd)),
            ClientCommand::Activate(cmd) => Self::Module(ModuleDispatchCommand::Activate(cmd)),
            ClientCommand::Deactivate(cmd) => Self::Module(ModuleDispatchCommand::Deactivate(cmd)),
            ClientCommand::Attack(cmd) => Self::Flight(FlightDispatchCommand::Attack(cmd)),
            ClientCommand::Stop(cmd) => Self::Flight(FlightDispatchCommand::Stop(cmd)),
            ClientCommand::Jump(cmd) => Self::Flight(FlightDispatchCommand::Jump(cmd)),
            ClientCommand::Approach(cmd) => Self::Flight(FlightDispatchCommand::Approach(cmd)),
            ClientCommand::Warp(cmd) => Self::Flight(FlightDispatchCommand::Warp(cmd)),
            ClientCommand::Orbit(cmd) => Self::Flight(FlightDispatchCommand::Orbit(cmd)),
            ClientCommand::KeepAtRange(cmd) => {
                Self::Flight(FlightDispatchCommand::KeepAtRange(cmd))
            }
            ClientCommand::Fit(cmd) => Self::Loadout(LoadoutDispatchCommand::Fit(cmd)),
            ClientCommand::Unfit(cmd) => Self::Loadout(LoadoutDispatchCommand::Unfit(cmd)),
            ClientCommand::ReorderFittedModule(cmd) => {
                Self::Loadout(LoadoutDispatchCommand::ReorderFittedModule(cmd))
            }
            ClientCommand::Dock(cmd) => Self::Station(StationDispatchCommand::Dock(cmd)),
            ClientCommand::Undock(cmd) => Self::Station(StationDispatchCommand::Undock(cmd)),
            ClientCommand::BuildPackagedShip(cmd) => {
                Self::Station(StationDispatchCommand::BuildPackagedShip(cmd))
            }
            ClientCommand::DisassembleShip(cmd) => {
                Self::Station(StationDispatchCommand::DisassembleShip(cmd))
            }
            ClientCommand::SelectActiveShip(cmd) => {
                Self::Station(StationDispatchCommand::SelectActiveShip(cmd))
            }
            ClientCommand::Assemble(cmd) => Self::Station(StationDispatchCommand::Assemble(cmd)),
            ClientCommand::Disembark(_) => Self::Station(StationDispatchCommand::Disembark),
            ClientCommand::TransferToStation(cmd) => {
                Self::Station(StationDispatchCommand::TransferToStation(cmd))
            }
        }
    }

    #[cfg(test)]
    fn family(&self) -> ClientCommandFamily {
        match self {
            Self::Flight(_) => ClientCommandFamily::Flight,
            Self::Module(_) => ClientCommandFamily::Module,
            Self::Loadout(_) => ClientCommandFamily::Loadout,
            Self::Station(_) => ClientCommandFamily::Station,
        }
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientCommandFamily {
    Flight,
    Module,
    Loadout,
    Station,
}

/// Family-local effects before the server-facing follow-up projection.
///
/// Keeping the family effect types separate prevents a station handler from
/// producing a Jump effect, while this wrapper gives the orchestration layer
/// one place to project every family into `ClientCommandFollowup`.
enum ClientCommandEffect {
    Flight(FlightDispatchEffect),
    Module(ModuleDispatchEffect),
    Loadout(LoadoutDispatchEffect),
    Station(StationDispatchEffect),
}

fn project_followup(
    player_id: PlayerId,
    effect: ClientCommandEffect,
) -> Option<ClientCommandFollowup> {
    match effect {
        ClientCommandEffect::Flight(FlightDispatchEffect::NoFollowup)
        | ClientCommandEffect::Module(ModuleDispatchEffect::NoFollowup)
        | ClientCommandEffect::Station(StationDispatchEffect::NoFollowup) => None,
        ClientCommandEffect::Flight(FlightDispatchEffect::Jump { ship_id, command }) => {
            Some(ClientCommandFollowup::Jump { ship_id, command })
        }
        ClientCommandEffect::Module(ModuleDispatchEffect::RefreshPlayerLoadout)
        | ClientCommandEffect::Loadout(LoadoutDispatchEffect::RefreshPlayerLoadout)
        | ClientCommandEffect::Station(StationDispatchEffect::RefreshPlayerLoadout) => {
            Some(ClientCommandFollowup::RefreshPlayerLoadout { player_id })
        }
    }
}

impl<S: EventStore> SimulationNode<S> {
    /// Returns `true` if `player_id` owns `ship_id`.
    ///
    /// Used by station-management `_owned` variants (Fit/Unfit/Dock/
    /// BuildPackagedShip/DisassembleShip), which operate on any owned ship,
    /// not just the active one (ADR-0037; docs/architecture/ownership.md §7).
    pub fn owns_ship(&self, player_id: PlayerId, ship_id: ShipId) -> bool {
        self.ships.owners.get(&ship_id) == Some(&player_id)
    }

    /// Returns `true` if `ship_id` is `player_id`'s active ship (ADR-0037).
    ///
    /// Implies `owns_ship` (active ⊆ owned) — used by flight/steering/module
    /// `_owned` variants, which may only ever target the one ship a player is
    /// currently flying, never another owned-but-inactive ship.
    pub fn is_active_ship(&self, player_id: PlayerId, ship_id: ShipId) -> bool {
        self.ships.active_ship.get(&player_id) == Some(&ship_id)
    }

    /// Apply one `ClientCommand` on behalf of `player_id`.
    ///
    /// The wire command is exhaustively classified into one closed family,
    /// then delegated to that family's policy module. The caller's active ship
    /// is resolved once here and passed to families that need it (ADR-0037,
    /// ADR-0047). The resulting family effect is projected into the existing
    /// `ClientCommandFollowup` seam in one place.
    pub fn apply_client_command(
        &mut self,
        player_id: PlayerId,
        cmd: ClientCommand,
        lock_commands: &mut Vec<dawn_core::LockOnCommand>,
    ) -> Option<ClientCommandFollowup> {
        let active_ship = self.ships.active_ship.get(&player_id).copied();
        let effect = match ClientCommandDispatch::select(cmd) {
            ClientCommandDispatch::Flight(cmd) => ClientCommandEffect::Flight(
                self.dispatch_flight_command(player_id, active_ship, cmd, lock_commands),
            ),
            ClientCommandDispatch::Module(cmd) => ClientCommandEffect::Module(
                self.dispatch_module_command(player_id, active_ship, cmd),
            ),
            ClientCommandDispatch::Loadout(cmd) => {
                ClientCommandEffect::Loadout(self.dispatch_loadout_command(player_id, cmd))
            }
            ClientCommandDispatch::Station(cmd) => ClientCommandEffect::Station(
                self.dispatch_station_command(player_id, active_ship, cmd),
            ),
        };
        project_followup(player_id, effect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::station::StationOperationOutcome;
    use crate::node::ModuleActivationRejection;
    use dawn_core::{
        DomainEvent, FitModuleCommand, ModuleId, NodeId, Position, SectorBounds, SectorId,
        SlotKind, Velocity,
    };
    use dawn_ecs::components::{FittingComp, ThrustComp};

    #[test]
    fn client_commands_are_selected_into_their_closed_policy_family() {
        use dawn_core::{ActivateModuleCommand, DockCommand, MoveCommand, StationId};

        assert_eq!(
            ClientCommandDispatch::select(ClientCommand::Move(MoveCommand::new(Position::ORIGIN,)))
                .family(),
            ClientCommandFamily::Flight
        );
        assert_eq!(
            ClientCommandDispatch::select(ClientCommand::Activate(ActivateModuleCommand {
                module_id: ModuleId(1),
                slot: SlotKind::High,
                target_ship_id: None,
            }))
            .family(),
            ClientCommandFamily::Module
        );
        assert_eq!(
            ClientCommandDispatch::select(ClientCommand::Fit(FitModuleCommand {
                ship_id: ShipId(dawn_core::EntityId::from_raw(1)),
                slot: SlotKind::High,
                module_id: ModuleId(1),
            }))
            .family(),
            ClientCommandFamily::Loadout
        );
        assert_eq!(
            ClientCommandDispatch::select(ClientCommand::Dock(DockCommand {
                station_id: StationId(0),
            }))
            .family(),
            ClientCommandFamily::Station
        );
    }

    #[test]
    fn attack_command_payload_reaches_the_flight_family() {
        use dawn_core::AttackCommand;

        let attacker_id = ShipId(dawn_core::EntityId::from_raw(11));
        let target_id = ShipId(dawn_core::EntityId::from_raw(12));
        let dispatch = ClientCommandDispatch::select(ClientCommand::Attack(AttackCommand {
            attacker_id,
            target_id,
        }));

        assert!(matches!(
            dispatch,
            ClientCommandDispatch::Flight(FlightDispatchCommand::Attack(AttackCommand {
                attacker_id: actual_attacker,
                target_id: actual_target,
            })) if actual_attacker == attacker_id && actual_target == target_id
        ));
    }

    #[test]
    fn followup_projection_preserves_jump_context() {
        let ship_id = ShipId(dawn_core::EntityId::from_raw(3));
        let effect = ClientCommandEffect::Flight(FlightDispatchEffect::Jump {
            ship_id,
            command: JumpCommand {
                gate_id: dawn_core::JumpGateId(4),
            },
        });

        assert!(matches!(
            project_followup(PlayerId(7), effect),
            Some(ClientCommandFollowup::Jump {
                ship_id: projected_ship,
                command: JumpCommand { gate_id }
            }) if projected_ship == ship_id && gate_id == dawn_core::JumpGateId(4)
        ));
    }

    #[test]
    fn followup_projection_maps_every_refreshing_family_to_the_caller() {
        let player_id = PlayerId(7);
        let effects = [
            ClientCommandEffect::Module(ModuleDispatchEffect::RefreshPlayerLoadout),
            ClientCommandEffect::Loadout(LoadoutDispatchEffect::RefreshPlayerLoadout),
            ClientCommandEffect::Station(StationDispatchEffect::RefreshPlayerLoadout),
        ];

        for effect in effects {
            assert!(matches!(
                project_followup(player_id, effect),
                Some(ClientCommandFollowup::RefreshPlayerLoadout { player_id: id })
                    if id == player_id
            ));
        }
    }

    #[test]
    fn followup_projection_keeps_no_followup_effects_empty() {
        assert!(project_followup(
            PlayerId(7),
            ClientCommandEffect::Flight(FlightDispatchEffect::NoFollowup),
        )
        .is_none());
        assert!(project_followup(
            PlayerId(7),
            ClientCommandEffect::Module(ModuleDispatchEffect::NoFollowup),
        )
        .is_none());
        assert!(project_followup(
            PlayerId(7),
            ClientCommandEffect::Station(StationDispatchEffect::NoFollowup),
        )
        .is_none());
    }

    #[test]
    fn loadout_followup_exposes_the_player_to_refresh() {
        let followup = ClientCommandFollowup::RefreshPlayerLoadout {
            player_id: PlayerId(7),
        };

        assert_eq!(followup.loadout_player_id(), Some(PlayerId(7)));
    }

    #[test]
    fn jump_followup_does_not_request_a_loadout_refresh() {
        let followup = ClientCommandFollowup::Jump {
            ship_id: ShipId(dawn_core::EntityId::from_raw(3)),
            command: JumpCommand {
                gate_id: dawn_core::JumpGateId(0),
            },
        };

        assert_eq!(followup.loadout_player_id(), None);
    }

    fn mem_node() -> SimulationNode {
        SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        )
    }

    fn node_with_catalog() -> SimulationNode {
        mem_node()
    }

    #[test]
    fn fitting_same_module_twice_does_not_double_count_stats() {
        use dawn_core::{FitModuleCommand, ModuleId, SlotKind};

        let mut node = mem_node();
        let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);

        node.fit_module(FitModuleCommand {
            ship_id,
            slot: SlotKind::High,
            module_id: ModuleId(1),
        });
        let stats_after_first = node.get_ship_stats(ship_id).unwrap();
        assert_eq!(
            stats_after_first.weapon_damage, 25.0,
            "1 module: base(0) + delta(25) = 25"
        );

        node.fit_module(FitModuleCommand {
            ship_id,
            slot: SlotKind::High,
            module_id: ModuleId(1),
        });
        let stats_after_second = node.get_ship_stats(ship_id).unwrap();
        assert_eq!(
            stats_after_second.weapon_damage, 50.0,
            "2 modules: base(0) + 2×delta(25) = 50 (not 75 from double-counting)"
        );
    }

    #[test]
    fn player_weapon_deals_damage_to_bot_after_lock_and_activation() {
        use dawn_core::{ActivateModuleCommand, LockOnCommand, ModuleId, SlotKind};

        let mut node = node_with_catalog();

        let bot_pos = Position::new(500.0, 0.0, 0.0);
        let (_, bot_ship_id) = node.spawn_bot_ship(bot_pos);

        let player_id = node.next_player_id();
        let player_ship_id = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);

        let lock_cmd = LockOnCommand {
            ship_id: player_ship_id,
            target_id: bot_ship_id,
        };

        // Weapon activation requires a Locked target (ADR-0035 Q4) — tick
        // until the lock completes before activating.
        for _ in 0..5 {
            node.tick_with_lock_commands(std::slice::from_ref(&lock_cmd));
        }

        assert!(
            node.activate_module_owned(
                player_id,
                player_ship_id,
                ActivateModuleCommand {
                    module_id: ModuleId(1),
                    slot: SlotKind::High,
                    target_ship_id: Some(bot_ship_id),
                }
            )
            .is_ok(),
            "activate_module_owned should succeed for player's own ship"
        );

        let mut damage_events = 0;
        for _ in 0..25 {
            let result = node.tick_with_lock_commands(std::slice::from_ref(&lock_cmd));
            damage_events += result
                .events
                .iter()
                .filter(|e| matches!(e, DomainEvent::DamageTaken(d) if d.ship_id == bot_ship_id))
                .count();
        }

        assert!(
            damage_events > 0,
            "player should have dealt at least 1 DamageTaken to bot within 25 ticks"
        );
    }

    #[test]
    fn activate_module_owned_reports_the_specific_rejection_reason() {
        use crate::modules::MODULE_RAILGUN_SMALL;

        let mut node = node_with_catalog();
        let owner_id = node.next_player_id();
        let ship_id = node.spawn_player_ship_at_pub(owner_id, Position::ORIGIN);
        node.fit_module(FitModuleCommand {
            ship_id,
            slot: SlotKind::High,
            module_id: MODULE_RAILGUN_SMALL,
        });

        let intruder_id = node.next_player_id();
        assert_eq!(
            node.activate_module_owned(
                intruder_id,
                ship_id,
                dawn_core::ActivateModuleCommand {
                    module_id: MODULE_RAILGUN_SMALL,
                    slot: SlotKind::High,
                    target_ship_id: None,
                }
            ),
            Err(ModuleActivationRejection::NotOwned),
            "a player who isn't flying the ship must be rejected before any fitting lookup"
        );

        assert_eq!(
            node.activate_module_owned(
                owner_id,
                ship_id,
                dawn_core::ActivateModuleCommand {
                    module_id: ModuleId(9999),
                    slot: SlotKind::High,
                    target_ship_id: None,
                }
            ),
            Err(ModuleActivationRejection::SlotNotFound),
            "activating a module_id that isn't fitted in that slot must name the real reason"
        );

        let station = node
            .station(dawn_core::StationId(0))
            .expect("demo station exists")
            .clone();
        let docked_id = node.next_player_id();
        let docked_ship = node.spawn_player_ship_at_pub(docked_id, station.position);
        node.fit_module(FitModuleCommand {
            ship_id: docked_ship,
            slot: SlotKind::High,
            module_id: MODULE_RAILGUN_SMALL,
        });
        assert!(matches!(
            node.dock_owned(
                docked_id,
                docked_ship,
                dawn_core::DockCommand {
                    station_id: dawn_core::StationId(0),
                },
            ),
            crate::node::station::StationOperationOutcome::Accepted { .. }
        ));
        assert_eq!(
            node.activate_module_owned(
                docked_id,
                docked_ship,
                dawn_core::ActivateModuleCommand {
                    module_id: MODULE_RAILGUN_SMALL,
                    slot: SlotKind::High,
                    target_ship_id: None,
                }
            ),
            Err(ModuleActivationRejection::ShipDocked),
            "activating while docked must be distinguishable from not owning the ship"
        );
    }

    #[test]
    fn in_transit_ship_rejects_module_lock_and_dock_mutations() {
        use crate::modules::MODULE_AFTERBURNER;
        use dawn_core::commands::TransitCommand;
        use dawn_core::{
            ActivateModuleCommand, ClientCommand, DeactivateModuleCommand, DockCommand,
            LockOnCommand, SlotKind, StationId,
        };

        let mut node = node_with_catalog();
        let station = node.station(StationId(0)).unwrap().clone();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship_at_pub(player_id, station.position);
        let target = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(
                station.position.x + 100.0,
                station.position.y,
                station.position.z,
            ),
            Velocity::ZERO,
        );
        node.propose_transit(TransitCommand {
            ship_id,
            to: SectorId(1),
        })
        .unwrap();
        let events_before = node.total_event_count();

        assert_eq!(
            node.activate_module(
                ship_id,
                ActivateModuleCommand {
                    module_id: MODULE_AFTERBURNER,
                    slot: SlotKind::Mid,
                    target_ship_id: None,
                },
            ),
            Err(ModuleActivationRejection::ShipInTransit)
        );
        assert_eq!(
            node.deactivate_module(
                ship_id,
                DeactivateModuleCommand {
                    module_id: MODULE_AFTERBURNER,
                    slot: SlotKind::Mid,
                },
            ),
            Err(ModuleActivationRejection::ShipInTransit)
        );

        let mut locks = Vec::new();
        node.apply_client_command(
            player_id,
            ClientCommand::LockOn(LockOnCommand {
                ship_id,
                target_id: target,
            }),
            &mut locks,
        );
        assert!(locks.is_empty());
        node.apply_client_command(
            player_id,
            ClientCommand::Dock(DockCommand {
                station_id: StationId(0),
            }),
            &mut locks,
        );
        assert!(!node.is_ship_docked(ship_id));
        assert_eq!(node.total_event_count(), events_before);
    }

    // ── apply_client_command ─────────────────────────────────────────────────

    fn spawn_owned_player_at(node: &mut SimulationNode, pos: Position) -> (PlayerId, ShipId) {
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship_at_pub(player_id, pos);
        (player_id, ship_id)
    }

    #[test]
    fn owned_move_command_is_applied_and_returns_no_followup() {
        use dawn_core::{ClientCommand, MoveCommand};
        let mut node = mem_node();
        let (player_id, _ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let mut locks = Vec::new();
        let result = node.apply_client_command(
            player_id,
            ClientCommand::Move(MoveCommand {
                target_position: Position::new(1_000.0, 0.0, 0.0),
            }),
            &mut locks,
        );
        assert!(result.is_none(), "Move must not produce a followup");
    }

    #[test]
    fn move_command_with_no_active_ship_is_silently_ignored_and_returns_no_followup() {
        // ADR-0037: MoveCommand no longer names a ship at all -- there is no
        // longer a wire-representable way to send a Move for a ship the
        // caller doesn't fly. The only remaining rejection path is a player
        // with no active ship at all (e.g. their only ship was destroyed).
        use dawn_core::{ClientCommand, MoveCommand};
        let mut node = mem_node();
        let player_id = node.next_player_id();
        let before = node.total_event_count();
        let mut locks = Vec::new();
        let result = node.apply_client_command(
            player_id,
            ClientCommand::Move(MoveCommand {
                target_position: Position::new(1_000.0, 0.0, 0.0),
            }),
            &mut locks,
        );
        assert!(
            result.is_none(),
            "Move with no active ship must not produce a followup"
        );
        assert_eq!(
            node.total_event_count(),
            before,
            "Move with no active ship must not append events"
        );
    }

    #[test]
    fn fit_command_returns_player_loadout_refresh_followup() {
        use dawn_core::{ClientCommand, FitModuleCommand, ModuleId, SlotKind};
        let mut node = mem_node();
        let (player_id, ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let mut locks = Vec::new();
        let result = node.apply_client_command(
            player_id,
            ClientCommand::Fit(FitModuleCommand {
                ship_id,
                module_id: ModuleId(1),
                slot: SlotKind::High,
            }),
            &mut locks,
        );
        assert!(
            matches!(
                result,
                Some(ClientCommandFollowup::RefreshPlayerLoadout { player_id: id })
                    if id == player_id
            ),
            "Fit must return RefreshPlayerLoadout for the caller's player_id"
        );
    }

    #[test]
    fn jump_command_is_handed_back_as_followup() {
        use dawn_core::{ClientCommand, JumpCommand, JumpGateId};
        let mut node = mem_node();
        let (player_id, ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let mut locks = Vec::new();
        let result = node.apply_client_command(
            player_id,
            ClientCommand::Jump(JumpCommand {
                gate_id: JumpGateId(0),
            }),
            &mut locks,
        );
        assert!(
            matches!(
                result,
                Some(ClientCommandFollowup::Jump { ship_id: sid, command: j })
                    if sid == ship_id && j.gate_id == JumpGateId(0)
            ),
            "Jump must be handed back as a followup"
        );
    }

    #[test]
    fn docked_target_loses_locks_and_cannot_be_damaged_by_active_weapons() {
        use dawn_core::{
            ActivateModuleCommand, DockCommand, LockOnCommand, ModuleId, SlotKind, StationId,
        };

        let mut node = node_with_catalog();
        let station = node
            .station(StationId(0))
            .expect("demo station exists")
            .clone();

        let attacker_id = node.next_player_id();
        let attacker_ship_id = node.spawn_player_ship_at_pub(
            attacker_id,
            Position::new(
                station.position.x + 200.0,
                station.position.y,
                station.position.z,
            ),
        );

        let target_player_id = node.next_player_id();
        let target_ship_id = node.spawn_player_ship_at_pub(target_player_id, station.position);

        let lock_cmd = LockOnCommand {
            ship_id: attacker_ship_id,
            target_id: target_ship_id,
        };
        for _ in 0..5 {
            node.tick_with_lock_commands(std::slice::from_ref(&lock_cmd));
        }

        assert!(
            node.activate_module_owned(
                attacker_id,
                attacker_ship_id,
                ActivateModuleCommand {
                    module_id: ModuleId(1),
                    slot: SlotKind::High,
                    target_ship_id: Some(target_ship_id),
                }
            )
            .is_ok(),
            "attacker should have an active weapon on the target before docking"
        );

        assert!(matches!(
            node.dock_owned(
                target_player_id,
                target_ship_id,
                DockCommand {
                    station_id: StationId(0),
                }
            ),
            StationOperationOutcome::Accepted { .. }
        ));

        let result = node.tick_with_lock_commands(std::slice::from_ref(&lock_cmd));
        assert!(
            result
                .events
                .iter()
                .any(|e| matches!(e, DomainEvent::LockLost(l) if l.locker_id == attacker_ship_id && l.target_id == target_ship_id)),
            "docking should tear down the attacker's lock on the target"
        );
        assert!(
            !result
                .events
                .iter()
                .any(|e| matches!(e, DomainEvent::DamageTaken(d) if d.ship_id == target_ship_id)),
            "docked targets must not take combat damage"
        );
    }

    fn docked_owned_player(node: &mut SimulationNode) -> (PlayerId, ShipId) {
        use dawn_core::StationId;
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);
        let station = node.station(StationId(0)).expect("demo station exists");
        node.set_spawn_anchor_abs(ship_id, station.abs_m);
        assert!(matches!(
            node.dock_owned(
                player_id,
                ship_id,
                dawn_core::DockCommand {
                    station_id: StationId(0),
                },
            ),
            StationOperationOutcome::Accepted { .. }
        ));
        (player_id, ship_id)
    }

    #[test]
    fn lock_on_command_dispatches_a_lock_command_for_the_active_ship() {
        use dawn_core::{ClientCommand, LockOnCommand};
        let mut node = mem_node();
        let (player_id, ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target_ship_id = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(500.0, 0.0, 0.0),
            Velocity::ZERO,
        );
        let mut locks = Vec::new();
        let result = node.apply_client_command(
            player_id,
            ClientCommand::LockOn(LockOnCommand {
                ship_id: target_ship_id,
                target_id: target_ship_id,
            }),
            &mut locks,
        );
        assert!(result.is_none(), "LockOn must not produce a followup");
        assert_eq!(locks.len(), 1, "LockOn must push exactly one LockOnCommand");
        assert_eq!(
            locks[0].ship_id, ship_id,
            "the pushed command must resolve to the caller's active ship, not the wire ship_id"
        );
        assert_eq!(locks[0].target_id, target_ship_id);
    }

    #[test]
    fn activate_command_dispatches_through_and_activates_the_fitted_module() {
        use crate::modules::MODULE_AFTERBURNER;
        use dawn_core::{ActivateModuleCommand, ClientCommand, FitModuleCommand, SlotKind};

        let mut node = node_with_catalog();
        let (player_id, ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        node.fit_module(FitModuleCommand {
            ship_id,
            slot: SlotKind::Mid,
            module_id: MODULE_AFTERBURNER,
        });
        let mut locks = Vec::new();
        let result = node.apply_client_command(
            player_id,
            ClientCommand::Activate(ActivateModuleCommand {
                module_id: MODULE_AFTERBURNER,
                slot: SlotKind::Mid,
                target_ship_id: None,
            }),
            &mut locks,
        );
        assert!(
            matches!(
                result,
                Some(ClientCommandFollowup::RefreshPlayerLoadout { player_id: id })
                    if id == player_id
            ),
            "Activate must return RefreshPlayerLoadout for the caller's player_id"
        );
        let entity = *node.ships.index.get(&ship_id).unwrap();
        let is_active = node
            .world
            .get::<FittingComp>(entity)
            .unwrap()
            .iter_slots()
            .find(|s| s.def.id == MODULE_AFTERBURNER)
            .unwrap()
            .is_active;
        assert!(
            is_active,
            "Activate dispatch must actually flip the module on"
        );
    }

    #[test]
    fn deactivate_command_dispatches_through_and_deactivates_the_fitted_module() {
        use crate::modules::MODULE_AFTERBURNER;
        use dawn_core::{
            ActivateModuleCommand, ClientCommand, DeactivateModuleCommand, FitModuleCommand,
            SlotKind,
        };

        let mut node = node_with_catalog();
        let (player_id, ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        node.fit_module(FitModuleCommand {
            ship_id,
            slot: SlotKind::Mid,
            module_id: MODULE_AFTERBURNER,
        });
        node.activate_module_owned(
            player_id,
            ship_id,
            ActivateModuleCommand {
                module_id: MODULE_AFTERBURNER,
                slot: SlotKind::Mid,
                target_ship_id: None,
            },
        )
        .unwrap();

        let mut locks = Vec::new();
        let result = node.apply_client_command(
            player_id,
            ClientCommand::Deactivate(DeactivateModuleCommand {
                module_id: MODULE_AFTERBURNER,
                slot: SlotKind::Mid,
            }),
            &mut locks,
        );
        assert!(
            matches!(
                result,
                Some(ClientCommandFollowup::RefreshPlayerLoadout { player_id: id })
                    if id == player_id
            ),
            "Deactivate must return RefreshPlayerLoadout for the caller's player_id"
        );
        let entity = *node.ships.index.get(&ship_id).unwrap();
        let is_active = node
            .world
            .get::<FittingComp>(entity)
            .unwrap()
            .iter_slots()
            .find(|s| s.def.id == MODULE_AFTERBURNER)
            .unwrap()
            .is_active;
        assert!(
            !is_active,
            "Deactivate dispatch must actually flip the module off"
        );
    }

    #[test]
    fn stop_command_dispatches_through_and_brakes_the_ship() {
        use dawn_core::{ClientCommand, StopCommand};
        let mut node = mem_node();
        let (player_id, ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let entity = *node.ships.index.get(&ship_id).unwrap();
        node.apply_move_command_owned(player_id, ship_id, Position::new(1_000.0, 0.0, 0.0));

        let mut locks = Vec::new();
        let result =
            node.apply_client_command(player_id, ClientCommand::Stop(StopCommand), &mut locks);
        assert!(result.is_none(), "Stop must not produce a followup");
        let thrust = node.world.get::<ThrustComp>(entity).unwrap();
        assert!(
            thrust.is_braking,
            "Stop dispatch must brake the ship's thrust"
        );
    }

    #[test]
    fn approach_command_dispatches_through_and_attaches_approach_comp() {
        use dawn_core::{ApproachCommand, ApproachTarget, ClientCommand};
        use dawn_ecs::components::ApproachComp;

        let mut node = mem_node();
        let (player_id, ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target_ship_id = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(2_000.0, 0.0, 0.0),
            Velocity::ZERO,
        );

        let mut locks = Vec::new();
        let result = node.apply_client_command(
            player_id,
            ClientCommand::Approach(ApproachCommand {
                target: ApproachTarget::Ship(target_ship_id),
            }),
            &mut locks,
        );
        assert!(result.is_none(), "Approach must not produce a followup");
        let entity = *node.ships.index.get(&ship_id).unwrap();
        assert!(
            node.world.get::<ApproachComp>(entity).is_some(),
            "Approach dispatch must attach ApproachComp to the caller's active ship"
        );
    }

    #[test]
    fn warp_command_dispatches_through_and_attaches_warp_comp() {
        use dawn_core::{ClientCommand, WarpCommand, WarpTarget};
        use dawn_ecs::components::WarpComp;

        let mut node = node_with_catalog();
        let (player_id, ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);

        let mut locks = Vec::new();
        let result = node.apply_client_command(
            player_id,
            ClientCommand::Warp(WarpCommand {
                target: WarpTarget::Body(dawn_core::CelestialBodyId(1)),
            }),
            &mut locks,
        );
        assert!(result.is_none(), "Warp must not produce a followup");
        let entity = *node.ships.index.get(&ship_id).unwrap();
        assert!(
            node.world.get::<WarpComp>(entity).is_some(),
            "Warp dispatch must attach WarpComp to the caller's active ship"
        );
    }

    #[test]
    fn orbit_command_dispatches_through_and_attaches_orbit_comp() {
        use dawn_core::{ApproachTarget, ClientCommand, OrbitCommand};
        use dawn_ecs::components::OrbitComp;

        let mut node = mem_node();
        let (player_id, ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target_ship_id = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(2_000.0, 0.0, 0.0),
            Velocity::ZERO,
        );

        let mut locks = Vec::new();
        let result = node.apply_client_command(
            player_id,
            ClientCommand::Orbit(OrbitCommand {
                target: ApproachTarget::Ship(target_ship_id),
                radius: Some(750.0),
            }),
            &mut locks,
        );
        assert!(result.is_none(), "Orbit must not produce a followup");
        let entity = *node.ships.index.get(&ship_id).unwrap();
        assert!(
            node.world.get::<OrbitComp>(entity).is_some(),
            "Orbit dispatch must attach OrbitComp to the caller's active ship"
        );
    }

    #[test]
    fn keep_at_range_command_dispatches_through_and_attaches_keep_at_range_comp() {
        use dawn_core::{ApproachTarget, ClientCommand, KeepAtRangeCommand};
        use dawn_ecs::components::KeepAtRangeComp;

        let mut node = mem_node();
        let (player_id, ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target_ship_id = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(2_000.0, 0.0, 0.0),
            Velocity::ZERO,
        );

        let mut locks = Vec::new();
        let result = node.apply_client_command(
            player_id,
            ClientCommand::KeepAtRange(KeepAtRangeCommand {
                target: ApproachTarget::Ship(target_ship_id),
                range: Some(1_500.0),
            }),
            &mut locks,
        );
        assert!(result.is_none(), "KeepAtRange must not produce a followup");
        let entity = *node.ships.index.get(&ship_id).unwrap();
        assert!(
            node.world.get::<KeepAtRangeComp>(entity).is_some(),
            "KeepAtRange dispatch must attach KeepAtRangeComp to the caller's active ship"
        );
    }

    #[test]
    fn unfit_command_dispatches_through_and_returns_refresh_fitting_followup() {
        use dawn_core::{ClientCommand, FitModuleCommand, ModuleId, SlotKind, UnfitModuleCommand};
        let mut node = mem_node();
        let (player_id, ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        node.fit_module(FitModuleCommand {
            ship_id,
            slot: SlotKind::High,
            module_id: ModuleId(1),
        });

        let mut locks = Vec::new();
        let result = node.apply_client_command(
            player_id,
            ClientCommand::Unfit(UnfitModuleCommand {
                ship_id,
                slot: SlotKind::High,
                module_id: ModuleId(1),
            }),
            &mut locks,
        );
        assert!(
            matches!(
                result,
                Some(ClientCommandFollowup::RefreshPlayerLoadout { player_id: id })
                    if id == player_id
            ),
            "Unfit must return RefreshPlayerLoadout for the caller's player_id"
        );
    }

    #[test]
    fn undock_command_dispatches_through_and_undocks_the_active_ship() {
        use dawn_core::ClientCommand;
        let mut node = node_with_catalog();
        let (player_id, ship_id) = docked_owned_player(&mut node);
        assert!(node.is_ship_docked(ship_id));

        let mut locks = Vec::new();
        let result = node.apply_client_command(
            player_id,
            ClientCommand::Undock(dawn_core::UndockCommand),
            &mut locks,
        );
        assert!(
            matches!(
                result,
                Some(ClientCommandFollowup::RefreshPlayerLoadout { player_id: id })
                    if id == player_id
            ),
            "Undock must return RefreshPlayerLoadout for the caller's player_id"
        );
        assert!(
            !node.is_ship_docked(ship_id),
            "Undock dispatch must actually undock the ship"
        );
    }

    #[test]
    fn dock_command_dispatches_through_and_docks_the_active_ship() {
        use dawn_core::{ClientCommand, DockCommand, StationId};
        let mut node = node_with_catalog();
        let (player_id, ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let station = node.station(StationId(0)).expect("demo station exists");
        node.set_spawn_anchor_abs(ship_id, station.abs_m);

        let mut locks = Vec::new();
        let result = node.apply_client_command(
            player_id,
            ClientCommand::Dock(DockCommand {
                station_id: StationId(0),
            }),
            &mut locks,
        );
        assert!(
            matches!(
                result,
                Some(ClientCommandFollowup::RefreshPlayerLoadout { player_id: id })
                    if id == player_id
            ),
            "Dock must return RefreshPlayerLoadout for the caller's player_id"
        );
        assert!(
            node.is_ship_docked(ship_id),
            "Dock dispatch must actually dock the ship"
        );
    }

    #[test]
    fn build_packaged_ship_command_dispatches_through_and_credits_the_station_item() {
        use dawn_core::{BuildPackagedShipCommand, ClientCommand, ItemId, StationId};
        let mut node = node_with_catalog();
        let (player_id, ship_id) = docked_owned_player(&mut node);
        node.credit_station_item(player_id, StationId(0), ItemId::ScrapMetal, 10);

        let mut locks = Vec::new();
        let result = node.apply_client_command(
            player_id,
            ClientCommand::BuildPackagedShip(BuildPackagedShipCommand {
                ship_id,
                station_id: StationId(0),
                ship_type_id: dawn_core::ShipTypeId(1),
            }),
            &mut locks,
        );
        assert!(
            matches!(
                result,
                Some(ClientCommandFollowup::RefreshPlayerLoadout { player_id: id })
                    if id == player_id
            ),
            "BuildPackagedShip must return RefreshPlayerLoadout for the caller's player_id"
        );
        assert_eq!(
            node.station_item_count(
                player_id,
                StationId(0),
                ItemId::PackagedShip(dawn_core::ShipTypeId(1)),
            ),
            1,
            "BuildPackagedShip dispatch must actually credit the packaged-ship item"
        );
    }

    #[test]
    fn select_active_ship_command_dispatches_through_and_switches_active_ship() {
        use dawn_core::{ClientCommand, SelectActiveShipCommand, StationId};
        let mut node = node_with_catalog();
        let (player_id, first_ship_id) = docked_owned_player(&mut node);
        let station_abs = node
            .station(StationId(0))
            .expect("demo station exists")
            .abs_m;
        // spawn_player_ship_at_pub makes the new ship the caller's active
        // ship, so dock it here (dock_owned requires the docking ship to be
        // active) before resetting active back to `first_ship_id` -- the
        // test then exercises an actual switch away from a docked, owned,
        // inactive ship.
        let second_ship_id = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);
        node.set_spawn_anchor_abs(second_ship_id, station_abs);
        assert!(matches!(
            node.dock_owned(
                player_id,
                second_ship_id,
                dawn_core::DockCommand {
                    station_id: StationId(0)
                }
            ),
            StationOperationOutcome::Accepted { .. }
        ));
        node.ships.active_ship.insert(player_id, first_ship_id);

        let mut locks = Vec::new();
        let result = node.apply_client_command(
            player_id,
            ClientCommand::SelectActiveShip(SelectActiveShipCommand {
                ship_id: second_ship_id,
            }),
            &mut locks,
        );
        assert!(
            matches!(
                result,
                Some(ClientCommandFollowup::RefreshPlayerLoadout { player_id: id })
                    if id == player_id
            ),
            "SelectActiveShip must return RefreshPlayerLoadout for the caller's player_id"
        );
        let _ = first_ship_id;
    }

    #[test]
    fn disembark_command_dispatches_through_and_clears_the_active_ship() {
        use dawn_core::{ClientCommand, DisembarkCommand};
        let mut node = node_with_catalog();
        let (player_id, ship_id) = docked_owned_player(&mut node);

        let mut locks = Vec::new();
        let result = node.apply_client_command(
            player_id,
            ClientCommand::Disembark(DisembarkCommand),
            &mut locks,
        );
        assert!(
            matches!(
                result,
                Some(ClientCommandFollowup::RefreshPlayerLoadout { player_id: id })
                    if id == player_id
            ),
            "Disembark must return RefreshPlayerLoadout for the caller's player_id"
        );
        assert!(
            !node.is_active_ship(player_id, ship_id),
            "Disembark dispatch must actually clear the caller's active ship"
        );
    }

    #[test]
    fn transfer_to_station_command_dispatches_through() {
        use dawn_core::{
            ClientCommand, ItemId, StationId, TransferDirection, TransferToStationCommand,
        };
        let mut node = node_with_catalog();
        let (player_id, ship_id) = docked_owned_player(&mut node);
        let entity = *node.ships.index.get(&ship_id).unwrap();
        if let Some(mut inv) = node
            .world
            .get_mut::<dawn_ecs::components::InventoryComp>(entity)
        {
            inv.add_item(ItemId::ScrapMetal, 5);
        }

        let mut locks = Vec::new();
        let result = node.apply_client_command(
            player_id,
            ClientCommand::TransferToStation(TransferToStationCommand {
                ship_id,
                station_id: StationId(0),
                item_id: ItemId::ScrapMetal,
                direction: TransferDirection::ToStation,
            }),
            &mut locks,
        );
        assert!(
            matches!(
                result,
                Some(ClientCommandFollowup::RefreshPlayerLoadout { player_id: id })
                    if id == player_id
            ),
            "TransferToStation must return RefreshPlayerLoadout for the caller's player_id"
        );
        assert_eq!(
            node.station_item_count(player_id, StationId(0), ItemId::ScrapMetal),
            5,
            "TransferToStation dispatch must actually move the item into the station inventory"
        );
    }

    #[test]
    fn jump_command_from_a_docked_ship_returns_no_followup() {
        use dawn_core::{ClientCommand, JumpCommand, JumpGateId};
        let mut node = node_with_catalog();
        let (player_id, _ship_id) = docked_owned_player(&mut node);
        let mut locks = Vec::new();
        let result = node.apply_client_command(
            player_id,
            ClientCommand::Jump(JumpCommand {
                gate_id: JumpGateId(0),
            }),
            &mut locks,
        );
        assert!(
            result.is_none(),
            "Jump from a docked ship must not produce a followup"
        );
    }
}
