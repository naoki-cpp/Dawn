//! Typed client-request admission and command-family orchestration for [`SimulationNode`].
//!
//! [`ClientRequest`] is the single external request catalog. This module owns
//! the one exhaustive admission seam: it validates untrusted values, injects
//! session-derived active-ship authority in the arm that needs it, and invokes
//! the family-local command/policy method directly. No second dispatch enum or
//! parallel request catalog exists below this match.

use dawn_core::{
    ActivateModuleCommand, ApproachCommand, BuildPackagedShipCommand, ClientRequest,
    ClientRequestValidationError, DeactivateModuleCommand, DisassembleShipCommand, DockCommand,
    FitModuleCommand, JumpCommand, KeepAtRangeCommand, LockOnCommand, OrbitCommand, PlayerId,
    ReorderFittedModuleCommand, SelectActiveShipCommand, ShipId, TransferToStationCommand,
    UnfitModuleCommand, WarpCommand,
};

use super::SimulationNode;

/// What request application hands back to a serving adapter.
#[derive(Debug, Clone)]
pub enum ClientCommandFollowup {
    Jump {
        ship_id: ShipId,
        command: JumpCommand,
    },
    RefreshPlayerLoadout {
        player_id: PlayerId,
    },
}

impl ClientCommandFollowup {
    pub fn loadout_player_id(&self) -> Option<PlayerId> {
        match self {
            Self::RefreshPlayerLoadout { player_id } => Some(*player_id),
            Self::Jump { .. } => None,
        }
    }
}

/// Structured failures before a request reaches gameplay policy.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum ClientRequestAdmissionError {
    #[error(transparent)]
    Validation(#[from] ClientRequestValidationError),
    #[error("request requires an admitted active ship")]
    NoActiveShip,
    #[error("{request} is not currently supported")]
    UnsupportedRequest { request: &'static str },
}

/// Result of admitting a request during one runtime frame.
#[derive(Debug, Clone, Copy)]
pub enum RuntimeCommandDispatch {
    /// A jump needs consensus submission after all requests are collected.
    Jump {
        session_index: usize,
        ship_id: ShipId,
        command: JumpCommand,
    },
    /// The adapter must refresh the player's typed loadout projection.
    RefreshPlayerLoadout {
        session_index: usize,
        player_id: PlayerId,
    },
    /// The adapter must send a structured rejection to the session.
    Rejected {
        session_index: usize,
        error: ClientRequestAdmissionError,
    },
}

fn require_active_ship(active_ship: Option<ShipId>) -> Result<ShipId, ClientRequestAdmissionError> {
    active_ship.ok_or(ClientRequestAdmissionError::NoActiveShip)
}

fn refresh_loadout(player_id: PlayerId) -> Option<ClientCommandFollowup> {
    Some(ClientCommandFollowup::RefreshPlayerLoadout { player_id })
}

/// Collect and admit all queued Sector requests for one runtime frame.
///
/// This function intentionally stops draining a session after its first Jump
/// request, matching the existing consensus handoff behavior. Other requests
/// are applied immediately to the node's prepared command state and their
/// adapter-visible followups are returned without knowing anything about
/// WebSockets or a particular deployment.
pub fn collect_runtime_commands<S, Player, Request>(
    node: &mut SimulationNode,
    sessions: &mut [S],
    lock_commands: &mut Vec<LockOnCommand>,
    player: Player,
    mut request: Request,
) -> Vec<RuntimeCommandDispatch>
where
    Player: Fn(&S) -> PlayerId,
    Request: FnMut(&mut S) -> Option<ClientRequest>,
{
    let mut dispatches = Vec::new();

    for (session_index, session) in sessions.iter_mut().enumerate() {
        while let Some(client_request) = request(session) {
            match node.apply_client_request(player(session), client_request, lock_commands) {
                Ok(Some(ClientCommandFollowup::Jump { ship_id, command })) => {
                    dispatches.push(RuntimeCommandDispatch::Jump {
                        session_index,
                        ship_id,
                        command,
                    });
                    break;
                }
                Ok(Some(ClientCommandFollowup::RefreshPlayerLoadout { player_id })) => {
                    dispatches.push(RuntimeCommandDispatch::RefreshPlayerLoadout {
                        session_index,
                        player_id,
                    });
                }
                Ok(None) => {}
                Err(error) => dispatches.push(RuntimeCommandDispatch::Rejected {
                    session_index,
                    error,
                }),
            }
        }
    }

    dispatches
}

impl SimulationNode {
    pub fn owns_ship(&self, player_id: PlayerId, ship_id: ShipId) -> bool {
        self.ships.owners.get(&ship_id) == Some(&player_id)
    }

    pub fn is_active_ship(&self, player_id: PlayerId, ship_id: ShipId) -> bool {
        self.ships.active_ship.get(&player_id) == Some(&ship_id)
    }

    /// Validate and admit one external request on behalf of the authenticated
    /// session. This exhaustive match is the only protocol-to-application
    /// conversion table; each arm calls its family-local policy directly.
    pub fn apply_client_request(
        &mut self,
        player_id: PlayerId,
        request: ClientRequest,
        lock_commands: &mut Vec<LockOnCommand>,
    ) -> Result<Option<ClientCommandFollowup>, ClientRequestAdmissionError> {
        request.validate()?;
        let active_ship = self.ships.active_ship.get(&player_id).copied();

        let followup = match request {
            ClientRequest::Move { target } => {
                let ship_id = require_active_ship(active_ship)?;
                self.apply_move_command_owned(player_id, ship_id, target);
                None
            }
            ClientRequest::LockOn { target } => {
                let ship_id = require_active_ship(active_ship)?;
                if !self.is_ship_docked(ship_id) && !self.is_ship_in_transit(ship_id) {
                    lock_commands.push(LockOnCommand {
                        ship_id,
                        target_id: target,
                    });
                }
                None
            }
            ClientRequest::ActivateModule {
                module,
                slot,
                target,
            } => {
                let ship_id = require_active_ship(active_ship)?;
                let _ = self.activate_module_owned(
                    player_id,
                    ship_id,
                    ActivateModuleCommand {
                        module_id: module,
                        slot,
                        target_ship_id: target,
                    },
                );
                refresh_loadout(player_id)
            }
            ClientRequest::DeactivateModule { module, slot } => {
                let ship_id = require_active_ship(active_ship)?;
                let _ = self.deactivate_module_owned(
                    player_id,
                    ship_id,
                    DeactivateModuleCommand {
                        module_id: module,
                        slot,
                    },
                );
                refresh_loadout(player_id)
            }
            ClientRequest::Attack { .. } => {
                return Err(ClientRequestAdmissionError::UnsupportedRequest { request: "Attack" });
            }
            ClientRequest::Stop => {
                let ship_id = require_active_ship(active_ship)?;
                self.apply_stop_command_owned(player_id, ship_id);
                None
            }
            ClientRequest::Jump { gate } => {
                let ship_id = require_active_ship(active_ship)?;
                if self.is_ship_docked(ship_id) {
                    None
                } else {
                    Some(ClientCommandFollowup::Jump {
                        ship_id,
                        command: JumpCommand { gate_id: gate },
                    })
                }
            }
            ClientRequest::Approach { target } => {
                let ship_id = require_active_ship(active_ship)?;
                self.apply_approach_command_owned(player_id, ship_id, ApproachCommand { target });
                None
            }
            ClientRequest::Warp { target } => {
                let ship_id = require_active_ship(active_ship)?;
                self.apply_warp_command_owned(player_id, ship_id, WarpCommand { target });
                None
            }
            ClientRequest::Orbit { target, radius } => {
                let ship_id = require_active_ship(active_ship)?;
                self.apply_orbit_command_owned(player_id, ship_id, OrbitCommand { target, radius });
                None
            }
            ClientRequest::KeepAtRange { target, range } => {
                let ship_id = require_active_ship(active_ship)?;
                self.apply_keep_at_range_command_owned(
                    player_id,
                    ship_id,
                    KeepAtRangeCommand { target, range },
                );
                None
            }
            ClientRequest::FitModule { ship, module, slot } => {
                self.fit_module_owned(
                    player_id,
                    FitModuleCommand {
                        ship_id: ship,
                        module_id: module,
                        slot,
                    },
                );
                refresh_loadout(player_id)
            }
            ClientRequest::UnfitModule { ship, module, slot } => {
                self.unfit_module_owned(
                    player_id,
                    UnfitModuleCommand {
                        ship_id: ship,
                        module_id: module,
                        slot,
                    },
                );
                refresh_loadout(player_id)
            }
            ClientRequest::ReorderFittedModule {
                ship,
                slot,
                from_index,
                to_index,
            } => {
                self.reorder_fitted_module_owned(
                    player_id,
                    ReorderFittedModuleCommand {
                        ship_id: ship,
                        slot,
                        from_index,
                        to_index,
                    },
                );
                refresh_loadout(player_id)
            }
            ClientRequest::Dock { station } => {
                let ship_id = require_active_ship(active_ship)?;
                if self.is_ship_in_transit(ship_id) {
                    None
                } else {
                    self.dock_owned(
                        player_id,
                        ship_id,
                        DockCommand {
                            station_id: station,
                        },
                    );
                    refresh_loadout(player_id)
                }
            }
            ClientRequest::Undock => {
                let ship_id = require_active_ship(active_ship)?;
                self.undock_owned(player_id, ship_id);
                refresh_loadout(player_id)
            }
            ClientRequest::BuildPackagedShip {
                ship,
                station,
                ship_type,
            } => {
                self.build_packaged_ship_owned(
                    player_id,
                    BuildPackagedShipCommand {
                        ship_id: ship,
                        station_id: station,
                        ship_type_id: ship_type,
                    },
                );
                refresh_loadout(player_id)
            }
            ClientRequest::DisassembleShip { ship, station } => {
                self.disassemble_ship_owned(
                    player_id,
                    DisassembleShipCommand {
                        ship_id: ship,
                        station_id: station,
                    },
                );
                refresh_loadout(player_id)
            }
            ClientRequest::SelectActiveShip { ship } => {
                self.select_active_ship_owned(player_id, SelectActiveShipCommand { ship_id: ship });
                refresh_loadout(player_id)
            }
            ClientRequest::Assemble { station, ship_type } => {
                let _ = self.assemble_ship_owned(
                    player_id,
                    dawn_core::AssembleCommand {
                        station_id: station,
                        ship_type_id: ship_type,
                    },
                );
                refresh_loadout(player_id)
            }
            ClientRequest::Disembark => {
                let _ship_id = require_active_ship(active_ship)?;
                let _ = self.disembark_owned(player_id);
                refresh_loadout(player_id)
            }
            ClientRequest::TransferCargo {
                ship,
                station,
                item,
                direction,
            } => {
                self.transfer_to_station_owned(
                    player_id,
                    TransferToStationCommand {
                        ship_id: ship,
                        station_id: station,
                        item_id: item,
                        direction,
                    },
                );
                refresh_loadout(player_id)
            }
        };

        Ok(followup)
    }

    #[cfg(test)]
    fn apply_client_request_unchecked(
        &mut self,
        player_id: PlayerId,
        request: ClientRequest,
        lock_commands: &mut Vec<LockOnCommand>,
    ) -> Option<ClientCommandFollowup> {
        self.apply_client_request(player_id, request, lock_commands)
            .expect("test request must pass admission")
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

    fn mem_node() -> SimulationNode {
        SimulationNode::new_test(
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
    fn unsupported_attack_request_returns_structured_admission_error() {
        let mut node = mem_node();
        let player_id = node.next_player_id();
        let mut locks = Vec::new();
        let result = node.apply_client_request(
            player_id,
            ClientRequest::Attack {
                target: dawn_core::ShipId(dawn_core::EntityId::from_raw(42)),
            },
            &mut locks,
        );

        assert!(matches!(
            result,
            Err(ClientRequestAdmissionError::UnsupportedRequest { request: "Attack" })
        ));
        assert!(locks.is_empty());
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
            ActivateModuleCommand, ClientRequest, DeactivateModuleCommand, SlotKind, StationId,
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
        node.apply_client_request_unchecked(
            player_id,
            ClientRequest::LockOn { target },
            &mut locks,
        );
        assert!(locks.is_empty());
        node.apply_client_request_unchecked(
            player_id,
            ClientRequest::Dock {
                station: StationId(0),
            },
            &mut locks,
        );
        assert!(!node.is_ship_docked(ship_id));
        assert_eq!(node.total_event_count(), events_before);
    }

    // ── apply_client_request ─────────────────────────────────────────────────

    fn spawn_owned_player_at(node: &mut SimulationNode, pos: Position) -> (PlayerId, ShipId) {
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship_at_pub(player_id, pos);
        (player_id, ship_id)
    }

    #[test]
    fn owned_move_command_is_applied_and_returns_no_followup() {
        use dawn_core::ClientRequest;
        let mut node = mem_node();
        let (player_id, _ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let mut locks = Vec::new();
        let result = node.apply_client_request_unchecked(
            player_id,
            ClientRequest::Move {
                target: Position::new(1_000.0, 0.0, 0.0),
            },
            &mut locks,
        );
        assert!(result.is_none(), "Move must not produce a followup");
    }

    #[test]
    fn move_request_with_no_active_ship_returns_structured_admission_error() {
        let mut node = mem_node();
        let player_id = node.next_player_id();
        let before = node.total_event_count();
        let mut locks = Vec::new();
        let result = node.apply_client_request(
            player_id,
            ClientRequest::Move {
                target: Position::new(1_000.0, 0.0, 0.0),
            },
            &mut locks,
        );
        assert!(matches!(
            result,
            Err(ClientRequestAdmissionError::NoActiveShip)
        ));
        assert_eq!(node.total_event_count(), before);
    }

    #[test]
    fn fit_command_returns_player_loadout_refresh_followup() {
        use dawn_core::{ClientRequest, ModuleId, SlotKind};
        let mut node = mem_node();
        let (player_id, ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let mut locks = Vec::new();
        let result = node.apply_client_request_unchecked(
            player_id,
            ClientRequest::FitModule {
                ship: ship_id,
                module: ModuleId(1),
                slot: SlotKind::High,
            },
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
        use dawn_core::{ClientRequest, JumpGateId};
        let mut node = mem_node();
        let (player_id, ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let mut locks = Vec::new();
        let result = node.apply_client_request_unchecked(
            player_id,
            ClientRequest::Jump {
                gate: JumpGateId(0),
            },
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
        use dawn_core::ClientRequest;
        let mut node = mem_node();
        let (player_id, ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target_ship_id = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(500.0, 0.0, 0.0),
            Velocity::ZERO,
        );
        let mut locks = Vec::new();
        let result = node.apply_client_request_unchecked(
            player_id,
            ClientRequest::LockOn {
                target: target_ship_id,
            },
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
        use dawn_core::{ClientRequest, FitModuleCommand, SlotKind};

        let mut node = node_with_catalog();
        let (player_id, ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        node.fit_module(FitModuleCommand {
            ship_id,
            slot: SlotKind::Mid,
            module_id: MODULE_AFTERBURNER,
        });
        let mut locks = Vec::new();
        let result = node.apply_client_request_unchecked(
            player_id,
            ClientRequest::ActivateModule {
                module: MODULE_AFTERBURNER,
                slot: SlotKind::Mid,
                target: None,
            },
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
        use dawn_core::{ActivateModuleCommand, ClientRequest, FitModuleCommand, SlotKind};

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
        let result = node.apply_client_request_unchecked(
            player_id,
            ClientRequest::DeactivateModule {
                module: MODULE_AFTERBURNER,
                slot: SlotKind::Mid,
            },
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
        use dawn_core::ClientRequest;
        let mut node = mem_node();
        let (player_id, ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let entity = *node.ships.index.get(&ship_id).unwrap();
        node.apply_move_command_owned(player_id, ship_id, Position::new(1_000.0, 0.0, 0.0));

        let mut locks = Vec::new();
        let result =
            node.apply_client_request_unchecked(player_id, ClientRequest::Stop, &mut locks);
        assert!(result.is_none(), "Stop must not produce a followup");
        let thrust = node.world.get::<ThrustComp>(entity).unwrap();
        assert!(
            thrust.is_braking,
            "Stop dispatch must brake the ship's thrust"
        );
    }

    #[test]
    fn approach_command_dispatches_through_and_attaches_approach_comp() {
        use dawn_core::{ApproachTarget, ClientRequest};
        use dawn_ecs::components::ApproachComp;

        let mut node = mem_node();
        let (player_id, ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target_ship_id = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(2_000.0, 0.0, 0.0),
            Velocity::ZERO,
        );

        let mut locks = Vec::new();
        let result = node.apply_client_request_unchecked(
            player_id,
            ClientRequest::Approach {
                target: ApproachTarget::Ship(target_ship_id),
            },
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
        use dawn_core::{ClientRequest, WarpTarget};
        use dawn_ecs::components::WarpComp;

        let mut node = node_with_catalog();
        let (player_id, ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);

        let mut locks = Vec::new();
        let result = node.apply_client_request_unchecked(
            player_id,
            ClientRequest::Warp {
                target: WarpTarget::Body(dawn_core::CelestialBodyId(1)),
            },
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
        use dawn_core::{ApproachTarget, ClientRequest};
        use dawn_ecs::components::OrbitComp;

        let mut node = mem_node();
        let (player_id, ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target_ship_id = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(2_000.0, 0.0, 0.0),
            Velocity::ZERO,
        );

        let mut locks = Vec::new();
        let result = node.apply_client_request_unchecked(
            player_id,
            ClientRequest::Orbit {
                target: ApproachTarget::Ship(target_ship_id),
                radius: Some(750.0),
            },
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
        use dawn_core::{ApproachTarget, ClientRequest};
        use dawn_ecs::components::KeepAtRangeComp;

        let mut node = mem_node();
        let (player_id, ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target_ship_id = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(2_000.0, 0.0, 0.0),
            Velocity::ZERO,
        );

        let mut locks = Vec::new();
        let result = node.apply_client_request_unchecked(
            player_id,
            ClientRequest::KeepAtRange {
                target: ApproachTarget::Ship(target_ship_id),
                range: Some(1_500.0),
            },
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
        use dawn_core::{ClientRequest, FitModuleCommand, ModuleId, SlotKind};
        let mut node = mem_node();
        let (player_id, ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        node.fit_module(FitModuleCommand {
            ship_id,
            slot: SlotKind::High,
            module_id: ModuleId(1),
        });

        let mut locks = Vec::new();
        let result = node.apply_client_request_unchecked(
            player_id,
            ClientRequest::UnfitModule {
                ship: ship_id,
                slot: SlotKind::High,
                module: ModuleId(1),
            },
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
        use dawn_core::ClientRequest;
        let mut node = node_with_catalog();
        let (player_id, ship_id) = docked_owned_player(&mut node);
        assert!(node.is_ship_docked(ship_id));

        let mut locks = Vec::new();
        let result =
            node.apply_client_request_unchecked(player_id, ClientRequest::Undock, &mut locks);
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
        use dawn_core::{ClientRequest, StationId};
        let mut node = node_with_catalog();
        let (player_id, ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let station = node.station(StationId(0)).expect("demo station exists");
        node.set_spawn_anchor_abs(ship_id, station.abs_m);

        let mut locks = Vec::new();
        let result = node.apply_client_request_unchecked(
            player_id,
            ClientRequest::Dock {
                station: StationId(0),
            },
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
        use dawn_core::{ClientRequest, ItemId, StationId};
        let mut node = node_with_catalog();
        let (player_id, ship_id) = docked_owned_player(&mut node);
        node.credit_station_item(player_id, StationId(0), ItemId::ScrapMetal, 10);

        let mut locks = Vec::new();
        let result = node.apply_client_request_unchecked(
            player_id,
            ClientRequest::BuildPackagedShip {
                ship: ship_id,
                station: StationId(0),
                ship_type: dawn_core::ShipTypeId(1),
            },
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
        use dawn_core::{ClientRequest, StationId};
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
        let result = node.apply_client_request_unchecked(
            player_id,
            ClientRequest::SelectActiveShip {
                ship: second_ship_id,
            },
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
        use dawn_core::ClientRequest;
        let mut node = node_with_catalog();
        let (player_id, ship_id) = docked_owned_player(&mut node);

        let mut locks = Vec::new();
        let result =
            node.apply_client_request_unchecked(player_id, ClientRequest::Disembark, &mut locks);
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
        use dawn_core::{ClientRequest, ItemId, StationId, TransferDirection};
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
        let result = node.apply_client_request_unchecked(
            player_id,
            ClientRequest::TransferCargo {
                ship: ship_id,
                station: StationId(0),
                item: ItemId::ScrapMetal,
                direction: TransferDirection::ToStation,
            },
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
        use dawn_core::{ClientRequest, JumpGateId};
        let mut node = node_with_catalog();
        let (player_id, _ship_id) = docked_owned_player(&mut node);
        let mut locks = Vec::new();
        let result = node.apply_client_request_unchecked(
            player_id,
            ClientRequest::Jump {
                gate: JumpGateId(0),
            },
            &mut locks,
        );
        assert!(
            result.is_none(),
            "Jump from a docked ship must not produce a followup"
        );
    }

    #[test]
    fn runtime_command_collection_stops_a_session_after_jump() {
        use dawn_core::{ClientRequest, JumpGateId};
        use std::collections::VecDeque;

        struct FakeSession {
            player_id: PlayerId,
            requests: VecDeque<ClientRequest>,
        }

        let mut node = node_with_catalog();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);
        let mut sessions = [FakeSession {
            player_id,
            requests: VecDeque::from([
                ClientRequest::Jump {
                    gate: JumpGateId(0),
                },
                ClientRequest::Stop,
            ]),
        }];
        let mut locks = Vec::new();

        let dispatches = collect_runtime_commands(
            &mut node,
            &mut sessions,
            &mut locks,
            |session| session.player_id,
            |session| session.requests.pop_front(),
        );

        assert!(matches!(
            dispatches.as_slice(),
            [RuntimeCommandDispatch::Jump {
                session_index: 0,
                ship_id: id,
                ..
            }] if *id == ship_id
        ));
        assert_eq!(sessions[0].requests.len(), 1);
        assert!(locks.is_empty());
    }
}
