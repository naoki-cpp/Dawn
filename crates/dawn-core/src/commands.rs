//! Commands — requests to change the world that may be rejected.
//!
//! A Command is *not* an Event.  It expresses intent, not fact.
//! The system validates a Command before producing an Event.  (INV-006)

use crate::fitting::{ModuleId, SlotKind};
use crate::star_system::JumpGateId;
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
    pub ship_id         : ShipId,
    pub target_position : Position,
}

impl MoveCommand {
    pub fn new(ship_id: ShipId, target_position: Position) -> Self {
        Self { ship_id, target_position }
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
    pub ship_id   : ShipId,
    pub slot      : SlotKind,
    pub module_id : ModuleId,
}

/// Request to begin locking onto a target.
///
/// May be rejected if:
/// - Either Ship does not exist.
/// - The locker is already at max_locks capacity.
/// - The target is already being locked or is locked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LockOnCommand {
    pub ship_id   : ShipId,
    pub target_id : ShipId,
}

/// Request to activate an Active module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActivateModuleCommand {
    pub ship_id   : ShipId,
    pub module_id : ModuleId,
    pub slot      : SlotKind,
}

/// Request to deactivate an Active module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeactivateModuleCommand {
    pub ship_id   : ShipId,
    pub module_id : ModuleId,
    pub slot      : SlotKind,
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
    pub attacker_id : ShipId,
    pub target_id   : ShipId,
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
    pub to     : SectorId,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NodeId, Position};

    fn ship_id(n: u64) -> ShipId { ShipId::new(NodeId(0), n) }

    #[test]
    fn move_command_stores_ship_id_and_target() {
        let cmd = MoveCommand::new(ship_id(1), Position::new(10.0, 0.0, 0.0));
        assert_eq!(cmd.ship_id, ship_id(1));
        assert_eq!(cmd.target_position, Position::new(10.0, 0.0, 0.0));
    }

    #[test]
    fn fit_module_command_carries_slot_and_module_id() {
        let cmd = FitModuleCommand {
            ship_id   : ship_id(2),
            slot      : SlotKind::High,
            module_id : ModuleId(42),
        };
        assert_eq!(cmd.slot, SlotKind::High);
        assert_eq!(cmd.module_id, ModuleId(42));
    }

    #[test]
    fn attack_command_identifies_attacker_and_target() {
        let cmd = AttackCommand { attacker_id: ship_id(1), target_id: ship_id(2) };
        assert_ne!(cmd.attacker_id, cmd.target_id);
    }

    #[test]
    fn transit_command_carries_ship_id_and_destination_sector() {
        let cmd = TransitCommand { ship_id: ship_id(1), to: SectorId(2) };
        assert_eq!(cmd.ship_id, ship_id(1));
        assert_eq!(cmd.to, SectorId(2));
    }

    #[test]
    fn jump_command_carries_ship_id_and_gate_id() {
        let cmd = JumpCommand { ship_id: ship_id(1), gate_id: crate::star_system::JumpGateId(0) };
        assert_eq!(cmd.ship_id, ship_id(1));
        assert_eq!(cmd.gate_id, crate::star_system::JumpGateId(0));
    }
}
