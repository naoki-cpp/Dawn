//! Commands — requests to change the world that may be rejected.
//!
//! A Command is *not* an Event. It expresses intent, not fact. The system
//! validates a Command before producing committed authoritative state and, when
//! applicable, public `DomainEvent`s (INV-006 / ADR-0049). A successful command
//! may have a `RecoveryDelta` even when it emits no public event.
//!
//! # Owned ship vs. active ship (ADR-0037 / ADR-0049)
//!
//! Flight/steering/module/Undock commands do not carry a `ship_id` — the
//! server always resolves them against the caller's *active* ship
//! (`SimulationNode::apply_client_request`), so there is no wire-representable
//! way for a client to name a ship it does not currently control. Station
//! inventory-management commands (Fit/Unfit/Dock/BuildPackagedShip/
//! DisassembleShip) still carry an explicit `ship_id`, because they operate
//! on any *owned* docked ship, not just the active one (docs/architecture/
//! ownership.md §7).
//!
//! `active_ship` is authoritative Player routing state under ADR-0049 because it
//! changes which Ship later commands mutate. It must be recovered even though
//! `SelectActiveShip`/`Disembark` do not require public `DomainEvent`s.
//!
//! # Retry identity
//!
//! `ClientRequest` currently has no generic request/idempotency ID. A newly
//! submitted request after an ambiguous disconnect is therefore a new request;
//! the wire/runtime must not claim transparent exactly-once retry based only on
//! payload equality. Protocols that need such retry use their own stable IDs
//! (for example admission, Transit Saga, or Market settlement identities).

use crate::fitting::{ModuleId, SlotKind};
use crate::navigation::{JumpGateId, StationId, WarpTarget};
use crate::{ItemId, PlayerId, Position, SectorId, ShipId, ShipTypeId};
#[cfg(feature = "schema")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Request to move the caller's active ship to `target_position` within its
/// current Sector.
///
/// May be rejected if:
/// - The Ship does not exist.
/// - The Ship is currently in transit between Sectors.
/// - `target_position` is outside the Sector boundary.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MoveCommand {
    pub target_position: Position,
}

impl MoveCommand {
    pub fn new(target_position: Position) -> Self {
        Self { target_position }
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

/// Request to reorder two fitted modules within the same slot kind
/// (drag-and-drop reorder in the FITTED column). Persisted server-side
/// (not merely a client display order) because iteration order over a
/// slot kind's modules is what assigns weapon hotkey F-numbers.
///
/// May be rejected if:
/// - The Ship does not exist, or the caller does not own it.
/// - The caller's ship is not currently docked (same restriction as
///   Fit/Unfit).
/// - `from_index`/`to_index` is out of bounds for `slot`'s current module
///   count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReorderFittedModuleCommand {
    pub ship_id: ShipId,
    pub slot: SlotKind,
    pub from_index: u32,
    pub to_index: u32,
}

/// Request to dock the caller's active ship at an NPC station (ADR-0034 9B
/// foundation).
///
/// May be rejected if:
/// - The active Ship does not exist.
/// - The Ship is not within the station's docking radius.
/// - The Ship is already docked somewhere.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DockCommand {
    pub station_id: StationId,
}

/// Request to undock the caller's active ship from its currently-docked NPC
/// station (ADR-0037: only the active ship may leave dock — switch active
/// ship first via `SelectActiveShipCommand`).
///
/// May be rejected if:
/// - The active Ship does not exist.
/// - The Ship is not currently docked.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UndockCommand;

/// Request to disembark: clear the caller's `active_ship` while docked,
/// without disassembling or transferring ownership of it (ADR-0037). No
/// `ship_id` -- always targets the caller's own active ship, like
/// `UndockCommand`.
///
/// This command emits no required public `DomainEvent`, but ADR-0049 classifies
/// the successful `active_ship` change as authoritative Player routing state:
/// it changes which Ship subsequent commands target and whether Undock is legal.
/// The routing mutation therefore belongs to checkpoint/`RecoveryDelta` recovery.
/// A later `SelectActiveShipCommand` re-activates a ship (this one or another
/// owned ship docked at the same station).
///
/// May be rejected if:
/// - The caller has no active ship (already disembarked, or never had one).
/// - The active ship is not currently docked.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DisembarkCommand;

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

/// Request to move an item from a docked ship's own cargo (`InventoryComp`)
/// into the caller's station inventory (ADR-0034 9B), all of it in one go --
/// no partial-count transfer. Ship cargo can currently only ever hold
/// `ItemId::Module` (starter loadout) or `ItemId::ScrapMetal` (combat loot,
/// `tick.rs`'s kill credit); `ItemId::PackagedShip` never enters ship cargo,
/// so it's never a meaningful `item_id` here, but nothing stops a client from
/// naming one -- the server rejects it the same way as naming an item the
/// ship doesn't have any of.
///
/// May be rejected if:
/// - The Ship does not exist or the caller does not own it.
/// - The caller is not currently docked at the target station.
/// - The source side (ship cargo for `ToStation`, station inventory for
///   `ToShip`) has none of `item_id`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TransferToStationCommand {
    pub ship_id: ShipId,
    pub station_id: StationId,
    pub item_id: ItemId,
    pub direction: TransferDirection,
}

/// Request to remove an Item from a player's ship cargo for a Market listing
/// (ADR-0034 §4, roadmap 9D-4).
///
/// This is an internal bridge command, not a client-facing `ClientRequest`.
/// The caller routes it to the Sector that owns `ship_id` and applies the
/// normal ownership and inventory validation there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoveItemCommand {
    pub player_id: PlayerId,
    pub ship_id: ShipId,
    pub item_id: ItemId,
    pub quantity: u64,
}

/// Request to return the remaining Item quantity from a cancelled Market Ask
/// to the seller's ship cargo (ADR-0034 §4, roadmap 9D-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReturnItemCommand {
    pub player_id: PlayerId,
    pub ship_id: ShipId,
    pub item_id: ItemId,
    pub quantity: u64,
}

/// Request to credit purchased Items to the buyer's ship cargo after Market
/// settlement (ADR-0034 §4, roadmap 9D-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreditItemCommand {
    pub player_id: PlayerId,
    pub ship_id: ShipId,
    pub item_id: ItemId,
    pub quantity: u64,
}

/// Which way `TransferToStationCommand` moves the stack.
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferDirection {
    /// Ship cargo -> station inventory (the original, one-way behavior).
    ToStation,
    /// Station inventory -> ship cargo.
    ToShip,
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

/// Request to convert a station-inventory `PackagedShip` item into a new
/// live docked ship, owned by the caller (ADR-0034 9B, ADR-0037). There is
/// no `ship_id` field -- the ship doesn't exist yet; the resulting ship's ID
/// is allocated on success and reported via the followup.
///
/// Does not change the caller's `active_ship`; a later `SelectActiveShipCommand`
/// makes the newly-assembled ship active.
///
/// May be rejected if:
/// - The caller is not currently docked at `station_id`.
/// - `ship_type_id` is unknown to the current node.
/// - The station inventory does not contain a `PackagedShip` of that type.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AssembleCommand {
    pub station_id: StationId,
    pub ship_type_id: ShipTypeId,
}

/// Request to make an owned, docked ship the caller's active ship
/// (ADR-0037, recovery classification amended by ADR-0049).
///
/// Unlike Assemble (which will later add a new owned ship without switching),
/// this is the only way an already-owned ship becomes active. Scoped to
/// station-local switches for now: `ship_id` must be docked at the same
/// station the caller is currently docked at.
///
/// The successful routing change is authoritative Player state even though no
/// public `DomainEvent` is required; it belongs to ADR-0049 RecoveryDelta/
/// checkpoint recovery.
///
/// May be rejected if:
/// - The caller does not own `ship_id`.
/// - `ship_id` is already the caller's active ship.
/// - `ship_id` is not docked at the caller's current docked station.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SelectActiveShipCommand {
    pub ship_id: ShipId,
}

/// What an approaching Ship is steering toward (ADR-0015).
///
/// A `Ship` target is dynamic (its position is read from the ECS each tick);
/// a `Gate` target is a static Jump Gate position, letting players fly back
/// into a gate's `activation_radius` to jump.
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ApproachTarget {
    Ship(ShipId),
    Gate(JumpGateId),
}

/// Request to begin approaching a Ship or a Jump Gate with the caller's
/// active ship (semi-automatic piloting).
///
/// Unlike `MoveCommand` (a one-shot thrust direction), an accepted approach
/// is a persistent steering mode: each tick the movement pipeline re-aims
/// thrust at the target's latest position until the ship arrives, the target
/// disappears, or a `MoveCommand` / `StopCommand` cancels it (ADR-0015).
///
/// May be rejected if:
/// - The active Ship does not exist or is in transit between Sectors.
/// - A `Ship` target does not exist or is the approaching Ship itself.
/// - A `Gate` target does not originate in the Ship's current Sector.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ApproachCommand {
    pub target: ApproachTarget,
}

/// Request to begin locking onto a target with the caller's active ship.
///
/// May be rejected if:
/// - Either Ship does not exist.
/// - The locker is already at max_locks capacity.
/// - The target is already being locked or is locked.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LockOnCommand {
    pub ship_id: ShipId,
    pub target_id: ShipId,
}

/// Request to activate an Active module on the caller's active ship.
///
/// `target_ship_id` (ADR-0035): required for module kinds where
/// `ModuleKind::requires_target()` is true (Weapon, Tackle), forbidden
/// otherwise. When required, the target must already be a `Locked` entry in
/// the activating ship's `LockComp` — activation is rejected otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ActivateModuleCommand {
    pub module_id: ModuleId,
    pub slot: SlotKind,
    pub target_ship_id: Option<ShipId>,
}

/// Request to deactivate an Active module on the caller's active ship.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DeactivateModuleCommand {
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
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AttackCommand {
    pub attacker_id: ShipId,
    pub target_id: ShipId,
}

/// Decelerate the caller's active ship to zero using its own thrust.
///
/// The movement system applies thrust opposite to the current velocity each
/// tick until the ship reaches zero speed. Cancels any active thrust direction.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StopCommand;

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

/// Request to use a Jump Gate to move the caller's active ship to its
/// destination Sector (ADR-0009).
///
/// Like `TransitCommand`, the actual Sector change is committed via the
/// Raft consensus layer (ADR-0014 / INV-003); this command only carries
/// the player's intent.
///
/// May be rejected if:
/// - The active Ship does not exist.
/// - The Ship is not within the gate's `activation_radius`.
/// - The Ship is already in transit (`TransitState::InTransit`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct JumpCommand {
    pub gate_id: JumpGateId,
}

/// Request to warp the caller's active ship toward a Jump Gate or a celestial
/// body within its current Sector (intra-Sector short-range Fold,
/// ADR-0022/ADR-0025).
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
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WarpCommand {
    pub target: WarpTarget,
}

/// Request to begin orbiting a Ship or a Jump Gate at `radius` with the
/// caller's active ship (ADR-0031).
///
/// Like `ApproachCommand`, an accepted orbit is a persistent steering mode:
/// each tick the movement pipeline re-aims thrust at a point on the circle of
/// `radius` around the target's latest position, leading the orbit so the
/// ship sweeps around it rather than just closing distance. `radius` defaults
/// to the ship's fitted weapon range when omitted.
///
/// May be rejected if:
/// - The active Ship does not exist or is in transit between Sectors.
/// - A `Ship` target does not exist or is the orbiting Ship itself.
/// - A `Gate` target does not originate in the Ship's current Sector.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OrbitCommand {
    pub target: ApproachTarget,
    pub radius: Option<f64>,
}

/// Request to hold at least `range` away from a Ship or a Jump Gate with the
/// caller's active ship (ADR-0031).
///
/// Like `ApproachCommand`, an accepted keep-at-range is a persistent steering
/// mode: each tick the ship is steered directly away from the target while
/// closer than `range`, and braked once at or beyond it. Unlike `OrbitCommand`
/// this has no tangential component -- it is a pure stand-off, not a sweep.
/// `range` defaults to the ship's fitted weapon range when omitted.
///
/// May be rejected if:
/// - The active Ship does not exist or is in transit between Sectors.
/// - A `Ship` target does not exist or is the Ship itself.
/// - A `Gate` target does not originate in the Ship's current Sector.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct KeepAtRangeCommand {
    pub target: ApproachTarget,
    pub range: Option<f64>,
}

/// The single authoritative catalog of requests an external Sector client may send.
///
/// This serialization-ready enum is shared by the client encoder, WebSocket decoder,
/// and the application admission seam. Family-local command structs above remain
/// internal policy inputs; there is no second mirrored full request enum. Acting
/// identities for active-ship operations are intentionally absent and are supplied
/// by the admitted server session.
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClientRequest {
    Move {
        target: Position,
    },
    LockOn {
        target: ShipId,
    },
    ActivateModule {
        module: ModuleId,
        slot: SlotKind,
        target: Option<ShipId>,
    },
    DeactivateModule {
        module: ModuleId,
        slot: SlotKind,
    },
    Attack {
        target: ShipId,
    },
    Stop,
    Jump {
        gate: JumpGateId,
    },
    Approach {
        target: ApproachTarget,
    },
    Warp {
        target: WarpTarget,
    },
    Orbit {
        target: ApproachTarget,
        radius: Option<f64>,
    },
    KeepAtRange {
        target: ApproachTarget,
        range: Option<f64>,
    },
    FitModule {
        ship: ShipId,
        module: ModuleId,
        slot: SlotKind,
    },
    UnfitModule {
        ship: ShipId,
        module: ModuleId,
        slot: SlotKind,
    },
    ReorderFittedModule {
        ship: ShipId,
        slot: SlotKind,
        from_index: u32,
        to_index: u32,
    },
    Dock {
        station: StationId,
    },
    Undock,
    BuildPackagedShip {
        ship: ShipId,
        station: StationId,
        ship_type: ShipTypeId,
    },
    DisassembleShip {
        ship: ShipId,
        station: StationId,
    },
    SelectActiveShip {
        ship: ShipId,
    },
    Assemble {
        station: StationId,
        ship_type: ShipTypeId,
    },
    Disembark,
    TransferCargo {
        ship: ShipId,
        station: StationId,
        item: ItemId,
        direction: TransferDirection,
    },
}

/// Structured validation failures at the protocol-to-application boundary.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum ClientRequestValidationError {
    #[error("Move.target contains a non-finite coordinate")]
    NonFinitePosition,
    #[error("Orbit.radius must be finite and positive, got {value}")]
    InvalidOrbitRadius { value: f64 },
    #[error("KeepAtRange.range must be finite and positive, got {value}")]
    InvalidKeepAtRange { value: f64 },
    #[error("{field} must be a non-zero module ID")]
    ZeroModuleId { field: &'static str },
    #[error("{field} must be a non-zero ship-type ID")]
    ZeroShipTypeId { field: &'static str },
}

impl ClientRequest {
    /// Validate untrusted numeric values before request admission or domain policy.
    pub fn validate(&self) -> Result<(), ClientRequestValidationError> {
        match self {
            Self::Move { target } if !target.is_finite() => {
                Err(ClientRequestValidationError::NonFinitePosition)
            }
            Self::Orbit {
                radius: Some(value),
                ..
            } if !value.is_finite() || *value <= 0.0 => {
                Err(ClientRequestValidationError::InvalidOrbitRadius { value: *value })
            }
            Self::KeepAtRange {
                range: Some(value), ..
            } if !value.is_finite() || *value <= 0.0 => {
                Err(ClientRequestValidationError::InvalidKeepAtRange { value: *value })
            }
            Self::ActivateModule {
                module: ModuleId(0),
                ..
            }
            | Self::DeactivateModule {
                module: ModuleId(0),
                ..
            }
            | Self::FitModule {
                module: ModuleId(0),
                ..
            }
            | Self::UnfitModule {
                module: ModuleId(0),
                ..
            } => Err(ClientRequestValidationError::ZeroModuleId { field: "module" }),
            Self::BuildPackagedShip {
                ship_type: ShipTypeId(0),
                ..
            }
            | Self::Assemble {
                ship_type: ShipTypeId(0),
                ..
            } => Err(ClientRequestValidationError::ZeroShipTypeId { field: "ship_type" }),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NodeId, Position};

    fn ship_id(n: u64) -> ShipId {
        ShipId::new(NodeId(0), n)
    }

    #[test]
    fn move_command_stores_target() {
        let cmd = MoveCommand::new(Position::new(10.0, 0.0, 0.0));
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
            target: ApproachTarget::Ship(ship_id(2)),
        };
        assert_eq!(cmd.target, ApproachTarget::Ship(ship_id(2)));
    }

    #[test]
    fn approach_command_can_target_a_jump_gate() {
        let cmd = ApproachCommand {
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
    fn jump_command_carries_gate_id() {
        let cmd = JumpCommand {
            gate_id: crate::navigation::JumpGateId(0),
        };
        assert_eq!(cmd.gate_id, crate::navigation::JumpGateId(0));
    }

    #[test]
    fn warp_command_carries_target() {
        use crate::navigation::{JumpGateId, WarpTarget};
        let cmd = WarpCommand {
            target: WarpTarget::Gate(JumpGateId(2)),
        };
        assert_eq!(cmd.target, WarpTarget::Gate(JumpGateId(2)));
    }

    #[test]
    fn orbit_command_carries_target_and_optional_radius() {
        let cmd = OrbitCommand {
            target: ApproachTarget::Ship(ship_id(2)),
            radius: Some(5000.0),
        };
        assert_eq!(cmd.target, ApproachTarget::Ship(ship_id(2)));
        assert_eq!(cmd.radius, Some(5000.0));
    }

    #[test]
    fn orbit_command_radius_defaults_to_none_when_omitted() {
        let cmd = OrbitCommand {
            target: ApproachTarget::Gate(crate::navigation::JumpGateId(0)),
            radius: None,
        };
        assert_eq!(cmd.radius, None);
    }

    #[test]
    fn keep_at_range_command_carries_target_and_optional_range() {
        let cmd = KeepAtRangeCommand {
            target: ApproachTarget::Ship(ship_id(2)),
            range: Some(8000.0),
        };
        assert_eq!(cmd.target, ApproachTarget::Ship(ship_id(2)));
        assert_eq!(cmd.range, Some(8000.0));
    }

    #[test]
    fn select_active_ship_command_carries_ship_id() {
        let cmd = SelectActiveShipCommand {
            ship_id: ship_id(1),
        };
        assert_eq!(cmd.ship_id, ship_id(1));
    }

    #[test]
    fn assemble_command_carries_no_ship_id() {
        let cmd = AssembleCommand {
            station_id: StationId(0),
            ship_type_id: ShipTypeId(1),
        };
        assert_eq!(cmd.station_id, StationId(0));
        assert_eq!(cmd.ship_type_id, ShipTypeId(1));
    }

    #[test]
    fn market_bridge_commands_carry_the_player_ship_item_and_quantity() {
        let remove = RemoveItemCommand {
            player_id: PlayerId(7),
            ship_id: ship_id(3),
            item_id: ItemId::ScrapMetal,
            quantity: 2,
        };
        let returned = ReturnItemCommand {
            player_id: remove.player_id,
            ship_id: remove.ship_id,
            item_id: remove.item_id,
            quantity: remove.quantity,
        };
        let credited = CreditItemCommand {
            player_id: PlayerId(8),
            ship_id: ship_id(4),
            item_id: remove.item_id,
            quantity: remove.quantity,
        };

        assert_eq!(returned.player_id, remove.player_id);
        assert_eq!(credited.ship_id, ship_id(4));
        assert_eq!(credited.quantity, 2);
    }

    #[test]
    fn disembark_is_a_unit_client_request() {
        let request = ClientRequest::Disembark;
        assert!(matches!(request, ClientRequest::Disembark));
    }

    #[test]
    fn client_request_validation_rejects_non_finite_and_non_positive_values() {
        assert_eq!(
            ClientRequest::Move {
                target: Position::new(f64::INFINITY, 0.0, 0.0),
            }
            .validate(),
            Err(ClientRequestValidationError::NonFinitePosition)
        );
        assert!(matches!(
            ClientRequest::Orbit {
                target: ApproachTarget::Ship(ship_id(1)),
                radius: Some(0.0),
            }
            .validate(),
            Err(ClientRequestValidationError::InvalidOrbitRadius { value: 0.0 })
        ));
    }
}
