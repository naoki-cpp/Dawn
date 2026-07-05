//! Commands — requests to change the world that may be rejected.
//!
//! A Command is *not* an Event.  It expresses intent, not fact.
//! The system validates a Command before producing an Event.  (INV-006)

use crate::fitting::{ModuleId, SlotKind};
use crate::navigation::{JumpGateId, StationId, WarpTarget};
use crate::{Position, SectorId, ShipId, ShipTypeId};
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

/// Request to move a fitted module back into the owning player's inventory
/// (ADR-0032). Symmetric with `FitModuleCommand`; `module_id` + `slot`
/// together identify which fitted instance to remove (a slot can hold
/// several modules of different ids).
///
/// May be rejected if:
/// - The Ship does not exist, or the caller does not own it.
/// - No module with `module_id` is fitted in `slot`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnfitModuleCommand {
    pub ship_id: ShipId,
    pub slot: SlotKind,
    pub module_id: ModuleId,
}

/// Request to dock at an NPC station (ADR-0034 9B foundation).
///
/// May be rejected if:
/// - The Ship does not exist or the caller does not own it.
/// - The Ship is not within the station's docking radius.
/// - The Ship is already docked somewhere.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DockCommand {
    pub ship_id: ShipId,
    pub station_id: StationId,
}

/// Request to undock from the currently-docked NPC station.
///
/// May be rejected if:
/// - The Ship does not exist or the caller does not own it.
/// - The Ship is not currently docked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UndockCommand {
    pub ship_id: ShipId,
}

/// Request to build a packaged ship inside the currently-docked station.
///
/// May be rejected if:
/// - The Ship does not exist or the caller does not own it.
/// - The caller is not currently docked at the target station.
/// - The station inventory does not contain enough Scrap Metal.
/// - `ship_type_id` is unknown to the current node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuildPackagedShipCommand {
    pub ship_id: ShipId,
    pub station_id: StationId,
    pub ship_type_id: ShipTypeId,
}

/// Request to disassemble a docked ship into a packaged ship item.
///
/// May be rejected if:
/// - The Ship does not exist or the caller does not own it.
/// - The caller is not currently docked at the target station.
/// - The Ship is damaged or has any fitted modules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisassembleShipCommand {
    pub ship_id: ShipId,
    pub station_id: StationId,
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
///
/// `target_ship_id` (ADR-0035): required for module kinds where
/// `ModuleKind::requires_target()` is true (Weapon, Tackle), forbidden
/// otherwise. When required, the target must already be a `Locked` entry in
/// the activating ship's `LockComp` — activation is rejected otherwise.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActivateModuleCommand {
    pub ship_id: ShipId,
    pub module_id: ModuleId,
    pub slot: SlotKind,
    pub target_ship_id: Option<ShipId>,
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

/// Request to begin orbiting a Ship or a Jump Gate at `radius` (ADR-0031).
///
/// Like `ApproachCommand`, an accepted orbit is a persistent steering mode:
/// each tick the movement pipeline re-aims thrust at a point on the circle of
/// `radius` around the target's latest position, leading the orbit so the
/// ship sweeps around it rather than just closing distance. `radius` defaults
/// to the ship's fitted weapon range when omitted.
///
/// May be rejected if:
/// - The orbiting Ship does not exist or is in transit between Sectors.
/// - A `Ship` target does not exist or is the orbiting Ship itself.
/// - A `Gate` target does not originate in the Ship's current Sector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrbitCommand {
    pub ship_id: ShipId,
    pub target: ApproachTarget,
    pub radius: Option<f32>,
}

/// Request to hold at least `range` away from a Ship or a Jump Gate (ADR-0031).
///
/// Like `ApproachCommand`, an accepted keep-at-range is a persistent steering
/// mode: each tick the ship is steered directly away from the target while
/// closer than `range`, and braked once at or beyond it. Unlike `OrbitCommand`
/// this has no tangential component -- it is a pure stand-off, not a sweep.
/// `range` defaults to the ship's fitted weapon range when omitted.
///
/// May be rejected if:
/// - The Ship does not exist or is in transit between Sectors.
/// - A `Ship` target does not exist or is the Ship itself.
/// - A `Gate` target does not originate in the Ship's current Sector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeepAtRangeCommand {
    pub ship_id: ShipId,
    pub target: ApproachTarget,
    pub range: Option<f32>,
}

/// All commands a client may send to the server.
///
/// Variants map directly to the concrete command structs above. To add a
/// command: add a variant here, update `dawn-actor/protocol.rs` (wire parse),
/// and add a branch to `SimulationNode::apply_client_command` (dawn-sector).
#[derive(Debug, Clone)]
pub enum ClientCommand {
    /// Set thrust direction.
    Move(MoveCommand),
    /// Begin locking a target.
    LockOn(LockOnCommand),
    /// Turn an active module on.
    Activate(ActivateModuleCommand),
    /// Turn an active module off.
    Deactivate(DeactivateModuleCommand),
    /// Attack a target (reserved for future manual-fire mode; combat is
    /// currently automatic each tick).
    Attack(AttackCommand),
    /// Decelerate to a stop (applies reverse thrust until velocity is zero).
    Stop(StopCommand),
    /// Cross a Sector boundary via a Jump Gate (ADR-0009).
    Jump(JumpCommand),
    /// Semi-automatic piloting: approach a selected ship/gate (ADR-0015).
    Approach(ApproachCommand),
    /// Intra-Sector warp toward a Jump Gate (short-range Fold, ADR-0022).
    Warp(WarpCommand),
    /// Sweep around a selected ship/gate at a chosen radius (ADR-0031).
    Orbit(OrbitCommand),
    /// Hold at least a chosen range from a selected ship/gate (ADR-0031).
    KeepAtRange(KeepAtRangeCommand),
    /// Move a module from inventory into a fitting slot (ADR-0032).
    Fit(FitModuleCommand),
    /// Move a fitted module back into inventory (ADR-0032).
    Unfit(UnfitModuleCommand),
    /// Dock at an NPC station.
    Dock(DockCommand),
    /// Leave a previously-docked NPC station.
    Undock(UndockCommand),
    /// Consume Scrap Metal in the docked station and create a Packaged Ship.
    BuildPackagedShip(BuildPackagedShipCommand),
    /// Convert a docked, unfitted, undamaged ship into a Packaged Ship.
    DisassembleShip(DisassembleShipCommand),
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
    fn unfit_module_command_carries_slot_and_module_id() {
        let cmd = UnfitModuleCommand {
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

    #[test]
    fn orbit_command_carries_ship_id_target_and_optional_radius() {
        let cmd = OrbitCommand {
            ship_id: ship_id(1),
            target: ApproachTarget::Ship(ship_id(2)),
            radius: Some(5000.0),
        };
        assert_eq!(cmd.ship_id, ship_id(1));
        assert_eq!(cmd.target, ApproachTarget::Ship(ship_id(2)));
        assert_eq!(cmd.radius, Some(5000.0));
    }

    #[test]
    fn orbit_command_radius_defaults_to_none_when_omitted() {
        let cmd = OrbitCommand {
            ship_id: ship_id(1),
            target: ApproachTarget::Gate(crate::navigation::JumpGateId(0)),
            radius: None,
        };
        assert_eq!(cmd.radius, None);
    }

    #[test]
    fn keep_at_range_command_carries_ship_id_target_and_optional_range() {
        let cmd = KeepAtRangeCommand {
            ship_id: ship_id(1),
            target: ApproachTarget::Ship(ship_id(2)),
            range: Some(8000.0),
        };
        assert_eq!(cmd.ship_id, ship_id(1));
        assert_eq!(cmd.target, ApproachTarget::Ship(ship_id(2)));
        assert_eq!(cmd.range, Some(8000.0));
    }
}
