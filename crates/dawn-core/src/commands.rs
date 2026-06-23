//! Commands — requests to change the world that may be rejected.
//!
//! A Command is *not* an Event.  It expresses intent, not fact.
//! The system validates a Command before producing an Event.  (INV-006)

use crate::fitting::{ModuleId, SlotKind};
use crate::navigation::{JumpGateId, WarpTarget};
use crate::{Position, SectorId, ShipId};
use serde::{Deserialize, Serialize};

/// Request to move a Ship to `target_position` within its current Sector.
///
/// May be rejected if:
/// - The Ship does not exist.
/// - The Ship is currently in transit between Sectors.
/// - `target_position` is outside the Sector boundary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoveCommand {
    pub ship_id: ShipId,
    pub target_position: Position,
}

impl MoveCommand {
    pub fn new(ship_id: ShipId, target_position: Position) -> Self {
        Self {
            ship_id,
            target_position,
        }
    }
}

/// Request to fit a module into the specified slot.
///
/// May be rejected if:
/// - The Ship does not exist.
/// - The slot kind does not accept this module kind.
/// - The slot is already full (exceeds max slots for that kind).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FitModuleCommand {
    pub ship_id: ShipId,
    pub slot: SlotKind,
    pub module_id: ModuleId,
}

/// What an approaching Ship is steering toward (ADR-0015).
///
/// A `Ship` target is dynamic (its position is read from the ECS each tick);
/// a `Gate` target is a static Jump Gate position, letting players fly back
/// into a gate's `activation_radius` to jump.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ApproachTarget {
    Ship(ShipId),
    Gate(JumpGateId),
}

/// Request to begin approaching a Ship or a Jump Gate (semi-automatic piloting).
///
/// Unlike `MoveCommand` (a one-shot thrust direction), an accepted approach
/// is a persistent steering mode: each tick the movement pipeline re-aims
/// thrust at the target's latest position until the ship arrives, the target
/// disappears, or a `MoveCommand` / `StopCommand` cancels it (ADR-0015).
///
/// May be rejected if:
/// - The approaching Ship does not exist or is in transit between Sectors.
/// - A `Ship` target does not exist or is the approaching Ship itself.
/// - A `Gate` target does not originate in the Ship's current Sector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApproachCommand {
    pub ship_id: ShipId,
    pub target: ApproachTarget,
}

/// Request to begin locking onto a target.
///
/// May be rejected if:
/// - Either Ship does not exist.
/// - The locker is already at max_locks capacity.
/// - The target is already being locked or is locked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LockOnCommand {
    pub ship_id: ShipId,
    pub target_id: ShipId,
}

/// Request to activate an Active module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActivateModuleCommand {
    pub ship_id: ShipId,
    pub module_id: ModuleId,
    pub slot: SlotKind,
}

/// Request to deactivate an Active module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeactivateModuleCommand {
    pub ship_id: ShipId,
    pub module_id: ModuleId,
    pub slot: SlotKind,
}

/// Request to attack another Ship.
///
/// May be rejected if:
/// - Either Ship does not exist.
/// - The attacker has no weapon modules fitted.
/// - The target is out of range.
/// - The weapon is still on cooldown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttackCommand {
    pub attacker_id: ShipId,
    pub target_id: ShipId,
}

/// Decelerate a ship to zero using its own thrust.
///
/// The movement system applies thrust opposite to the current velocity each
/// tick until the ship reaches zero speed. Cancels any active thrust direction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StopCommand {
    pub ship_id: ShipId,
}

/// Request to transit a Ship from its current Sector to `to`.
///
/// Submitted to the Raft consensus layer as a `TransitProposal` (ADR-0014).
/// No event is appended until the proposal is committed.
///
/// May be rejected if:
/// - The Ship does not exist.
/// - The Ship is already in transit (`TransitState::InTransit`).
/// - `to` is not adjacent to the Ship's current Sector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransitCommand {
    pub ship_id: ShipId,
    pub to: SectorId,
}

/// Request to use a Jump Gate to move a Ship to its destination Sector
/// (ADR-0009).
///
/// Like `TransitCommand`, the actual Sector change is committed via the
/// Raft consensus layer (ADR-0014 / INV-003); this command only carries
/// the player's intent.
///
/// May be rejected if:
/// - The Ship does not exist.
/// - The Ship is not within the gate's `activation_radius`.
/// - The Ship is already in transit (`TransitState::InTransit`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JumpCommand {
    pub ship_id: ShipId,
    pub gate_id: JumpGateId,
}

/// Request to warp a Ship toward a Jump Gate or a celestial body within its
/// current Sector (intra-Sector short-range Fold, ADR-0022/ADR-0025).
///
/// An accepted warp is a persistent two-phase steering mode (`WarpComp`):
/// an interruptible alignment phase, then a committed warping phase.
/// For Gate targets, the ship stops inside the gate's `activation_radius`.
/// For Body targets, the ship stops at `body.radius * 1.5` from the centre.
///
/// May be rejected (`can_propose_warp`) if:
/// - The Ship does not exist, is in transit, or is already warping.
/// - The target does not belong to the Ship's current Sector.
/// - The target is closer than the minimum warp distance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WarpCommand {
    pub ship_id: ShipId,
    pub target: WarpTarget,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NodeId, Position};

    fn ship_id(n: u64) -> ShipId {
        ShipId::new(NodeId(0), n)
    }

    #[test]
    fn move_command_stores_ship_id_and_target() {
        let cmd = MoveCommand::new(ship_id(1), Position::new(10.0, 0.0, 0.0));
        assert_eq!(cmd.ship_id, ship_id(1));
        assert_eq!(cmd.target_position, Position::new(10.0, 0.0, 0.0));
    }

    #[test]
    fn fit_module_command_carries_slot_and_module_id() {
        let cmd = FitModuleCommand {
            ship_id: ship_id(2),
            slot: SlotKind::High,
            module_id: ModuleId(42),
        };
        assert_eq!(cmd.slot, SlotKind::High);
        assert_eq!(cmd.module_id, ModuleId(42));
    }

    #[test]
    fn approach_command_can_target_a_ship() {
        let cmd = ApproachCommand {
            ship_id: ship_id(1),
            target: ApproachTarget::Ship(ship_id(2)),
        };
        assert_eq!(cmd.ship_id, ship_id(1));
        assert_eq!(cmd.target, ApproachTarget::Ship(ship_id(2)));
    }

    #[test]
    fn approach_command_can_target_a_jump_gate() {
        let cmd = ApproachCommand {
            ship_id: ship_id(1),
            target: ApproachTarget::Gate(crate::navigation::JumpGateId(3)),
        };
        assert_eq!(
            cmd.target,
            ApproachTarget::Gate(crate::navigation::JumpGateId(3))
        );
    }

    #[test]
    fn attack_command_identifies_attacker_and_target() {
        let cmd = AttackCommand {
            attacker_id: ship_id(1),
            target_id: ship_id(2),
        };
        assert_ne!(cmd.attacker_id, cmd.target_id);
    }

    #[test]
    fn transit_command_carries_ship_id_and_destination_sector() {
        let cmd = TransitCommand {
            ship_id: ship_id(1),
            to: SectorId(2),
        };
        assert_eq!(cmd.ship_id, ship_id(1));
        assert_eq!(cmd.to, SectorId(2));
    }

    #[test]
    fn jump_command_carries_ship_id_and_gate_id() {
        let cmd = JumpCommand {
            ship_id: ship_id(1),
            gate_id: crate::navigation::JumpGateId(0),
        };
        assert_eq!(cmd.ship_id, ship_id(1));
        assert_eq!(cmd.gate_id, crate::navigation::JumpGateId(0));
    }

    #[test]
    fn warp_command_carries_ship_id_and_target() {
        use crate::navigation::{JumpGateId, WarpTarget};
        let cmd = WarpCommand {
            ship_id: ship_id(1),
            target: WarpTarget::Gate(JumpGateId(2)),
        };
        assert_eq!(cmd.ship_id, ship_id(1));
        assert_eq!(cmd.target, WarpTarget::Gate(JumpGateId(2)));
    }
}
