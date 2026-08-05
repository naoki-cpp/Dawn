//! Flight-family client command dispatch.
//!
//! This module owns the policy that is shared by commands directed at the
//! caller's active ship: direct movement, steering modes, lock-on admission,
//! and Jump follow-up creation. The outer command boundary resolves the active
//! ship once and passes it here; this module never re-reads player routing.

use dawn_core::{
    ApproachCommand, JumpCommand, KeepAtRangeCommand, LockOnCommand, MoveCommand, OrbitCommand,
    PlayerId, ShipId, StopCommand, WarpCommand,
};
use dawn_event_store::store::EventStore;

use super::SimulationNode;

pub(super) enum FlightDispatchCommand {
    Move(MoveCommand),
    LockOn(LockOnCommand),
    Attack,
    Stop(StopCommand),
    Jump(JumpCommand),
    Approach(ApproachCommand),
    Warp(WarpCommand),
    Orbit(OrbitCommand),
    KeepAtRange(KeepAtRangeCommand),
}

pub(super) enum FlightDispatchEffect {
    NoFollowup,
    Jump {
        ship_id: ShipId,
        command: JumpCommand,
    },
}

impl<S: EventStore> SimulationNode<S> {
    pub(super) fn dispatch_flight_command(
        &mut self,
        player_id: PlayerId,
        active_ship: Option<ShipId>,
        cmd: FlightDispatchCommand,
        lock_commands: &mut Vec<dawn_core::LockOnCommand>,
    ) -> FlightDispatchEffect {
        match cmd {
            FlightDispatchCommand::Move(cmd) => {
                if let Some(ship_id) = active_ship {
                    self.apply_move_command_owned(player_id, ship_id, cmd.target_position);
                }
            }
            FlightDispatchCommand::LockOn(cmd) => {
                // The wire-provided ship_id is untrusted. Lock routing is
                // always rebuilt from the caller's resolved active ship.
                if let Some(ship_id) = active_ship {
                    if !self.is_ship_docked(ship_id) && !self.is_ship_in_transit(ship_id) {
                        lock_commands.push(dawn_core::LockOnCommand {
                            ship_id,
                            target_id: cmd.target_id,
                        });
                    }
                }
            }
            // Combat is automatic (CombatSystem each tick); AttackCommand is
            // reserved for a future manual-fire mode.
            FlightDispatchCommand::Attack => {}
            FlightDispatchCommand::Stop(_) => {
                if let Some(ship_id) = active_ship {
                    self.apply_stop_command_owned(player_id, ship_id);
                }
            }
            FlightDispatchCommand::Jump(command) => {
                if let Some(ship_id) = active_ship {
                    if !self.is_ship_docked(ship_id) {
                        return FlightDispatchEffect::Jump { ship_id, command };
                    }
                }
            }
            FlightDispatchCommand::Approach(cmd) => {
                if let Some(ship_id) = active_ship {
                    self.apply_approach_command_owned(player_id, ship_id, cmd);
                }
            }
            FlightDispatchCommand::Warp(cmd) => {
                if let Some(ship_id) = active_ship {
                    self.apply_warp_command_owned(player_id, ship_id, cmd);
                }
            }
            FlightDispatchCommand::Orbit(cmd) => {
                if let Some(ship_id) = active_ship {
                    self.apply_orbit_command_owned(player_id, ship_id, cmd);
                }
            }
            FlightDispatchCommand::KeepAtRange(cmd) => {
                if let Some(ship_id) = active_ship {
                    self.apply_keep_at_range_command_owned(player_id, ship_id, cmd);
                }
            }
        }
        FlightDispatchEffect::NoFollowup
    }
}

#[cfg(test)]
mod tests {
    use dawn_core::{
        EntityId, LockOnCommand, NodeId, PlayerId, Position, SectorBounds, SectorId, ShipId,
        ShipTypeId, Velocity,
    };

    use super::*;

    fn node() -> SimulationNode {
        SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        )
    }

    #[test]
    fn lock_on_dispatch_uses_resolved_active_ship_instead_of_wire_ship_id() {
        let mut node = node();
        let player_id = node.next_player_id();
        let active_ship = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);
        let target_id = node.spawn_ship(
            ShipTypeId(1),
            Position::new(100.0, 0.0, 0.0),
            Velocity::ZERO,
        );
        let forged_ship = ShipId(EntityId::from_raw(999));
        let mut locks = Vec::new();

        let effect = node.dispatch_flight_command(
            player_id,
            Some(active_ship),
            FlightDispatchCommand::LockOn(LockOnCommand {
                ship_id: forged_ship,
                target_id,
            }),
            &mut locks,
        );

        assert!(matches!(effect, FlightDispatchEffect::NoFollowup));
        assert_eq!(locks.len(), 1);
        assert_eq!(locks[0].ship_id, active_ship);
        assert_eq!(locks[0].target_id, target_id);
    }

    #[test]
    fn flight_dispatch_without_an_active_ship_is_a_noop() {
        let mut node = node();
        let mut locks = Vec::new();
        let effect = node.dispatch_flight_command(
            PlayerId(7),
            None,
            FlightDispatchCommand::LockOn(LockOnCommand {
                ship_id: ShipId(EntityId::from_raw(1)),
                target_id: ShipId(EntityId::from_raw(2)),
            }),
            &mut locks,
        );

        assert!(matches!(effect, FlightDispatchEffect::NoFollowup));
        assert!(locks.is_empty());
    }
}
