use dawn_core::{
    ActivateModuleCommand, ApproachCommand, ApproachTarget, AttackCommand,
    BuildPackagedShipCommand, ClientCommand, DeactivateModuleCommand, DisassembleShipCommand,
    DockCommand, EntityId, LockOnCommand, ModuleId, MoveCommand, Position, ShipId, SlotKind,
    StopCommand, UndockCommand,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Serialize, Deserialize, JsonSchema, Clone, Copy)]
pub struct PosWire {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl PosWire {
    /// `false` if any component is NaN/Infinity. A client-supplied non-finite
    /// coordinate would otherwise flow straight into position/velocity math
    /// (`SimulationNode::apply_move_command`) and poison shared simulation
    /// state -- NaN propagates through arithmetic silently and makes range
    /// comparisons (`dist < range`) always false, so it's cheaper to reject
    /// at the wire boundary than to guard every downstream consumer
    /// (security-review.md SEC-5).
    fn is_finite(&self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}

#[derive(Debug, PartialEq, Serialize, Deserialize, JsonSchema, Clone, Copy)]
pub struct VelWire {
    pub dx: f64,
    pub dy: f64,
    pub dz: f64,
}

impl From<Position> for PosWire {
    fn from(p: Position) -> Self {
        Self {
            x: p.x,
            y: p.y,
            z: p.z,
        }
    }
}

impl From<dawn_core::Velocity> for VelWire {
    fn from(v: dawn_core::Velocity) -> Self {
        Self {
            dx: v.dx,
            dy: v.dy,
            dz: v.dz,
        }
    }
}

/// A `{"Gate": N}` or `{"Body": N}` warp destination, as sent by
/// `WarpCommand`'s current wire format (externally tagged: the variant name
/// is the JSON object's only key).
#[derive(Debug, PartialEq, Serialize, Deserialize, JsonSchema, Clone, Copy)]
pub enum WarpTargetWire {
    Gate(u32),
    Body(u32),
}

/// Every message a client can send to the server, as the postcard-encoded
/// binary `ClientMessage::Command` envelope (ADR-0042).
///
/// This enum is the schema-of-record for the client -> server half of the
/// wire protocol (see [`crate::EventWire`] for the server -> client half). It
/// intentionally mirrors the wire format exactly, including the two
/// backward-compatible quirks below -- it does not enforce the "exactly one
/// of these two fields" business rules those quirks involve; that
/// validation still happens in [`client_command_from_wire`].
///
/// - `WarpCommand` accepts either `target` (current) or `gate_id` (legacy);
///   `target` wins if both are present.
/// - `ApproachCommand` / `OrbitCommand` / `KeepAtRangeCommand` select their
///   target with either `gate_id` (a Jump Gate) or `target_id` (a Ship);
///   `gate_id` wins if both are present.
///
/// Flight/steering/module/Undock variants carry no `ship_id` (ADR-0037): the
/// server always resolves them against the caller's active ship, so there is
/// no wire-representable way to name a ship the player isn't currently
/// flying. Station inventory-management variants (Fit/Unfit/Dock/
/// BuildPackagedShip/DisassembleShip) still carry an explicit `ship_id`,
/// since they may target any owned docked ship, not just the active one.
///
/// Derives both `Serialize` and `Deserialize` (ADR-0041): the server
/// decodes a postcard-received `ClientMessage::Command` into this enum
/// ([`client_command_from_wire`]); the Godot client (`dawn-client-gdext`)
/// constructs a variant directly from typed arguments and encodes it back
/// out, replacing the old GDScript pattern of hand-building a `Dictionary`
/// that had to match this schema by eye.
///
/// Externally tagged (serde's default enum representation, `{"VariantName":
/// {...fields}}`), not `#[serde(tag = "type")]` -- `postcard` (the binary
/// wire format since ADR-0042) cannot deserialize an internally tagged enum
/// at all (it has no `deserialize_any`, which internal tagging requires).
/// JSON-text serialization of this type is not the runtime wire format (see
/// [`postcard::to_stdvec`]/[`postcard::from_bytes`] via the `ClientMessage`
/// envelope); `serde_json` is used only by [`client_command_wire_json_schema`]
/// (doc generation) and this crate's own unit tests.
#[derive(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum ClientCommandWire {
    MoveCommand {
        target: PosWire,
    },
    LockOnCommand {
        target_id: u64,
    },
    ActivateModuleCommand {
        module_id: u32,
        slot: String,
        /// Target of a targeted module (Weapon/Tackle), per ADR-0035.
        /// Required only for targeted module kinds; validated server-side.
        target_ship_id: Option<u64>,
    },
    DeactivateModuleCommand {
        module_id: u32,
        slot: String,
    },
    AttackCommand {
        attacker_id: u64,
        target_id: u64,
    },
    StopCommand {},
    JumpCommand {
        gate_id: u32,
    },
    ApproachCommand {
        gate_id: Option<u32>,
        target_id: Option<u64>,
    },
    WarpCommand {
        target: Option<WarpTargetWire>,
        /// Legacy form: `{"gate_id": N}` instead of `{"target": {"Gate": N}}`.
        gate_id: Option<u32>,
    },
    OrbitCommand {
        gate_id: Option<u32>,
        target_id: Option<u64>,
        radius: Option<f64>,
    },
    KeepAtRangeCommand {
        gate_id: Option<u32>,
        target_id: Option<u64>,
        range: Option<f64>,
    },
    FitModuleCommand {
        ship_id: u64,
        module_id: u32,
        slot: String,
    },
    UnfitModuleCommand {
        ship_id: u64,
        module_id: u32,
        slot: String,
    },
    /// Reorder two fitted modules within the same slot kind (drag-and-drop
    /// reorder in the FITTED column).
    ReorderFittedModuleCommand {
        ship_id: u64,
        slot: String,
        from_index: u32,
        to_index: u32,
    },
    DockCommand {
        station_id: u32,
    },
    UndockCommand {},
    BuildPackagedShipCommand {
        ship_id: u64,
        station_id: u32,
        ship_type_id: u32,
    },
    DisassembleShipCommand {
        ship_id: u64,
        station_id: u32,
    },
    SelectActiveShipCommand {
        ship_id: u64,
    },
    /// Convert a station-inventory Packaged Ship item into a new live docked
    /// ship (ADR-0034 9B, ADR-0037). No `ship_id` -- the ship doesn't exist
    /// yet; its ID is reported via the resulting `ShipAssembled` event.
    AssembleCommand {
        station_id: u32,
        ship_type_id: u32,
    },
    /// Clear the caller's active ship while docked, without disassembling it
    /// (ADR-0037). No `ship_id` -- always targets the caller's own active
    /// ship, like `UndockCommand`.
    DisembarkCommand {},
    /// Move the entire stack of an item between a docked ship's own cargo
    /// and the caller's station inventory (ADR-0034 9B), in the direction
    /// `direction` says (`"ToStation"` or `"ToShip"`). `item_type` is one of
    /// `"Module"`, `"PackagedShip"`, `"ScrapMetal"` (matching `ItemRow`'s
    /// wire shape) with `module_id`/`ship_type_id` populated only for the
    /// variant that uses them (`0` otherwise).
    TransferToStationCommand {
        ship_id: u64,
        station_id: u32,
        item_type: String,
        module_id: u32,
        ship_type_id: u32,
        direction: String,
    },
}

/// Render the client -> server wire schema (see [`ClientCommandWire`]) as a
/// JSON Schema document.
pub fn client_command_wire_json_schema() -> schemars::Schema {
    schemars::schema_for!(ClientCommandWire)
}

fn approach_target_from_gate_or_ship(
    gate_id: Option<u32>,
    target_id: Option<u64>,
) -> Option<ApproachTarget> {
    if let Some(gate) = gate_id {
        Some(ApproachTarget::Gate(dawn_core::JumpGateId(gate)))
    } else {
        Some(ApproachTarget::Ship(ShipId(EntityId::from_raw(target_id?))))
    }
}

/// Convert an already-decoded [`ClientCommandWire`] (from the binary
/// `ClientMessage::Command` envelope, ADR-0042) into a [`ClientCommand`].
/// Returns `None` for a value that fails domain validation (see each match
/// arm below, e.g. non-finite coordinates).
pub fn client_command_from_wire(wire: ClientCommandWire) -> Option<ClientCommand> {
    match wire {
        ClientCommandWire::MoveCommand { target } => {
            if !target.is_finite() {
                return None;
            }
            Some(ClientCommand::Move(MoveCommand {
                target_position: Position {
                    x: target.x,
                    y: target.y,
                    z: target.z,
                },
            }))
        }
        ClientCommandWire::LockOnCommand { target_id } => {
            Some(ClientCommand::LockOn(LockOnCommand {
                ship_id: ShipId(EntityId::from_raw(0)),
                target_id: ShipId(EntityId::from_raw(target_id)),
            }))
        }
        ClientCommandWire::ActivateModuleCommand {
            module_id,
            slot,
            target_ship_id,
        } => Some(ClientCommand::Activate(ActivateModuleCommand {
            module_id: ModuleId(module_id),
            slot: parse_slot_kind(&slot)?,
            target_ship_id: target_ship_id.map(|raw| ShipId(EntityId::from_raw(raw))),
        })),
        ClientCommandWire::DeactivateModuleCommand { module_id, slot } => {
            Some(ClientCommand::Deactivate(DeactivateModuleCommand {
                module_id: ModuleId(module_id),
                slot: parse_slot_kind(&slot)?,
            }))
        }
        ClientCommandWire::AttackCommand {
            attacker_id,
            target_id,
        } => Some(ClientCommand::Attack(AttackCommand {
            attacker_id: ShipId(EntityId::from_raw(attacker_id)),
            target_id: ShipId(EntityId::from_raw(target_id)),
        })),
        ClientCommandWire::StopCommand {} => Some(ClientCommand::Stop(StopCommand)),
        ClientCommandWire::JumpCommand { gate_id } => {
            Some(ClientCommand::Jump(dawn_core::JumpCommand {
                gate_id: dawn_core::JumpGateId(gate_id),
            }))
        }
        ClientCommandWire::ApproachCommand { gate_id, target_id } => {
            let target = approach_target_from_gate_or_ship(gate_id, target_id)?;
            Some(ClientCommand::Approach(ApproachCommand { target }))
        }
        ClientCommandWire::WarpCommand { target, gate_id } => {
            let warp_target = match target {
                Some(WarpTargetWire::Gate(gate)) => {
                    dawn_core::WarpTarget::Gate(dawn_core::JumpGateId(gate))
                }
                Some(WarpTargetWire::Body(body)) => {
                    dawn_core::WarpTarget::Body(dawn_core::CelestialBodyId(body))
                }
                None => dawn_core::WarpTarget::Gate(dawn_core::JumpGateId(gate_id?)),
            };
            Some(ClientCommand::Warp(dawn_core::WarpCommand {
                target: warp_target,
            }))
        }
        ClientCommandWire::OrbitCommand {
            gate_id,
            target_id,
            radius,
        } => {
            if radius.is_some_and(|r| !r.is_finite()) {
                return None;
            }
            let target = approach_target_from_gate_or_ship(gate_id, target_id)?;
            Some(ClientCommand::Orbit(dawn_core::OrbitCommand {
                target,
                radius,
            }))
        }
        ClientCommandWire::KeepAtRangeCommand {
            gate_id,
            target_id,
            range,
        } => {
            if range.is_some_and(|r| !r.is_finite()) {
                return None;
            }
            let target = approach_target_from_gate_or_ship(gate_id, target_id)?;
            Some(ClientCommand::KeepAtRange(dawn_core::KeepAtRangeCommand {
                target,
                range,
            }))
        }
        ClientCommandWire::FitModuleCommand {
            ship_id,
            module_id,
            slot,
        } => Some(ClientCommand::Fit(dawn_core::FitModuleCommand {
            ship_id: ShipId(EntityId::from_raw(ship_id)),
            module_id: ModuleId(module_id),
            slot: parse_slot_kind(&slot)?,
        })),
        ClientCommandWire::UnfitModuleCommand {
            ship_id,
            module_id,
            slot,
        } => Some(ClientCommand::Unfit(dawn_core::UnfitModuleCommand {
            ship_id: ShipId(EntityId::from_raw(ship_id)),
            module_id: ModuleId(module_id),
            slot: parse_slot_kind(&slot)?,
        })),
        ClientCommandWire::ReorderFittedModuleCommand {
            ship_id,
            slot,
            from_index,
            to_index,
        } => Some(ClientCommand::ReorderFittedModule(
            dawn_core::ReorderFittedModuleCommand {
                ship_id: ShipId(EntityId::from_raw(ship_id)),
                slot: parse_slot_kind(&slot)?,
                from_index,
                to_index,
            },
        )),
        ClientCommandWire::DockCommand { station_id } => Some(ClientCommand::Dock(DockCommand {
            station_id: dawn_core::StationId(station_id),
        })),
        ClientCommandWire::UndockCommand {} => Some(ClientCommand::Undock(UndockCommand)),
        ClientCommandWire::BuildPackagedShipCommand {
            ship_id,
            station_id,
            ship_type_id,
        } => Some(ClientCommand::BuildPackagedShip(BuildPackagedShipCommand {
            ship_id: ShipId(EntityId::from_raw(ship_id)),
            station_id: dawn_core::StationId(station_id),
            ship_type_id: dawn_core::ShipTypeId(ship_type_id),
        })),
        ClientCommandWire::DisassembleShipCommand {
            ship_id,
            station_id,
        } => Some(ClientCommand::DisassembleShip(DisassembleShipCommand {
            ship_id: ShipId(EntityId::from_raw(ship_id)),
            station_id: dawn_core::StationId(station_id),
        })),
        ClientCommandWire::SelectActiveShipCommand { ship_id } => Some(
            ClientCommand::SelectActiveShip(dawn_core::SelectActiveShipCommand {
                ship_id: ShipId(EntityId::from_raw(ship_id)),
            }),
        ),
        ClientCommandWire::AssembleCommand {
            station_id,
            ship_type_id,
        } => Some(ClientCommand::Assemble(dawn_core::AssembleCommand {
            station_id: dawn_core::StationId(station_id),
            ship_type_id: dawn_core::ShipTypeId(ship_type_id),
        })),
        ClientCommandWire::DisembarkCommand {} => {
            Some(ClientCommand::Disembark(dawn_core::DisembarkCommand))
        }
        ClientCommandWire::TransferToStationCommand {
            ship_id,
            station_id,
            item_type,
            module_id,
            ship_type_id,
            direction,
        } => {
            let item_id = match item_type.as_str() {
                "Module" => dawn_core::ItemId::Module(ModuleId(module_id)),
                "PackagedShip" => {
                    dawn_core::ItemId::PackagedShip(dawn_core::ShipTypeId(ship_type_id))
                }
                "ScrapMetal" => dawn_core::ItemId::ScrapMetal,
                _ => return None,
            };
            let direction = match direction.as_str() {
                "ToStation" => dawn_core::TransferDirection::ToStation,
                "ToShip" => dawn_core::TransferDirection::ToShip,
                _ => return None,
            };
            Some(ClientCommand::TransferToStation(
                dawn_core::TransferToStationCommand {
                    ship_id: ShipId(EntityId::from_raw(ship_id)),
                    station_id: dawn_core::StationId(station_id),
                    item_id,
                    direction,
                },
            ))
        }
    }
}

fn parse_slot_kind(s: &str) -> Option<SlotKind> {
    match s {
        "High" => Some(SlotKind::High),
        "Mid" => Some(SlotKind::Mid),
        "Low" => Some(SlotKind::Low),
        "Rig" => Some(SlotKind::Rig),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{JumpGateId, NodeId};

    fn ship_id(n: u64) -> ShipId {
        ShipId(EntityId::new(NodeId(0), n))
    }

    /// Test-only convenience: `parse_client_command` (the old JSON-text
    /// parser combining this deserialize + convert step) was deleted since
    /// nothing at runtime called it -- production decodes `ClientCommandWire`
    /// straight off the binary `ClientMessage::Command` envelope (ADR-0042).
    /// These tests keep exercising literal JSON text (matching
    /// `docs/architecture/wire-protocol-commands.schema.json`, the
    /// documented shape for a hypothetical non-Godot client) rather than
    /// constructing `ClientCommandWire` values directly, so this helper
    /// stays local to the test module instead of becoming production API.
    fn command_from_json(line: &str) -> Option<dawn_core::ClientCommand> {
        let wire: ClientCommandWire = serde_json::from_str(line).ok()?;
        client_command_from_wire(wire)
    }

    #[test]
    fn lock_on_command_json_is_parsed_into_client_command_lock_on() {
        let line = r#"{"LockOnCommand":{"target_id":7}}"#;
        let cmd = command_from_json(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::LockOn(c) => {
                assert_eq!(c.target_id, ship_id(7));
            }
            other => panic!("expected LockOn, got {other:?}"),
        }
    }

    #[test]
    fn activate_module_command_json_is_parsed_with_and_without_a_target() {
        let line = r#"{"ActivateModuleCommand":{"module_id":3,"slot":"High","target_ship_id":9}}"#;
        let cmd = command_from_json(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::Activate(c) => {
                assert_eq!(c.module_id, ModuleId(3));
                assert_eq!(c.slot, SlotKind::High);
                assert_eq!(c.target_ship_id, Some(ship_id(9)));
            }
            other => panic!("expected Activate, got {other:?}"),
        }

        let line_no_target = r#"{"ActivateModuleCommand":{"module_id":3,"slot":"High"}}"#;
        let cmd_no_target = command_from_json(line_no_target).expect("must parse");
        match cmd_no_target {
            dawn_core::ClientCommand::Activate(c) => assert_eq!(c.target_ship_id, None),
            other => panic!("expected Activate, got {other:?}"),
        }
    }

    #[test]
    fn deactivate_module_command_json_is_parsed_into_client_command_deactivate() {
        let line = r#"{"DeactivateModuleCommand":{"module_id":3,"slot":"Mid"}}"#;
        let cmd = command_from_json(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::Deactivate(c) => {
                assert_eq!(c.module_id, ModuleId(3));
                assert_eq!(c.slot, SlotKind::Mid);
            }
            other => panic!("expected Deactivate, got {other:?}"),
        }
    }

    #[test]
    fn attack_command_json_is_parsed_into_client_command_attack() {
        let line = r#"{"AttackCommand":{"attacker_id":1,"target_id":2}}"#;
        let cmd = command_from_json(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::Attack(c) => {
                assert_eq!(c.attacker_id, ship_id(1));
                assert_eq!(c.target_id, ship_id(2));
            }
            other => panic!("expected Attack, got {other:?}"),
        }
    }

    #[test]
    fn stop_command_json_is_parsed_into_client_command_stop() {
        let line = r#"{"StopCommand":{}}"#;
        let cmd = command_from_json(line).expect("must parse");
        assert!(matches!(cmd, dawn_core::ClientCommand::Stop(_)));
    }

    #[test]
    fn undock_command_json_is_parsed_into_client_command_undock() {
        let line = r#"{"UndockCommand":{}}"#;
        let cmd = command_from_json(line).expect("must parse");
        assert!(matches!(cmd, dawn_core::ClientCommand::Undock(_)));
    }

    #[test]
    fn build_packaged_ship_command_json_is_parsed_into_client_command_build_packaged_ship() {
        let line = r#"{"BuildPackagedShipCommand":{"ship_id":1,"station_id":2,"ship_type_id":7}}"#;
        let cmd = command_from_json(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::BuildPackagedShip(c) => {
                assert_eq!(c.ship_id, ship_id(1));
                assert_eq!(c.station_id, dawn_core::StationId(2));
                assert_eq!(c.ship_type_id, dawn_core::ShipTypeId(7));
            }
            other => panic!("expected BuildPackagedShip, got {other:?}"),
        }
    }

    #[test]
    fn select_active_ship_command_json_is_parsed_into_client_command_select_active_ship() {
        let line = r#"{"SelectActiveShipCommand":{"ship_id":5}}"#;
        let cmd = command_from_json(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::SelectActiveShip(c) => {
                assert_eq!(c.ship_id, ship_id(5));
            }
            other => panic!("expected SelectActiveShip, got {other:?}"),
        }
    }

    #[test]
    fn move_command_json_is_parsed_into_client_command_move() {
        let line = r#"{"MoveCommand":{"target":{"x":10.0,"y":0.0,"z":-5.0}}}"#;
        let cmd = command_from_json(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::Move(c) => {
                assert!((c.target_position.x - 10.0).abs() < 1e-6);
            }
            other => panic!("expected Move, got {other:?}"),
        }
    }

    /// security-review.md SEC-5: a non-finite coordinate must be rejected at
    /// the wire boundary instead of flowing into position/velocity math.
    /// JSON has no `NaN`/`Infinity` literals, so the attack shape a real
    /// client can actually send is a magnitude that overflows `f64` on
    /// parse (`1e400` exceeds `f64::MAX`, so serde_json rejects it) -- literal
    /// `NaN`/`Infinity` tokens would
    /// just fail JSON parsing itself, which doesn't exercise `is_finite()`.
    #[test]
    fn move_command_json_with_an_overflowing_coordinate_fails_to_parse() {
        let line = r#"{"MoveCommand":{"target":{"x":1e+400,"y":0.0,"z":0.0}}}"#;
        assert!(command_from_json(line).is_none());
    }

    #[test]
    fn orbit_command_json_with_an_overflowing_radius_fails_to_parse() {
        let line = r#"{"OrbitCommand":{"gate_id":2,"radius":1e+400}}"#;
        assert!(command_from_json(line).is_none());
    }

    #[test]
    fn keep_at_range_command_json_with_an_overflowing_range_fails_to_parse() {
        let line = r#"{"KeepAtRangeCommand":{"gate_id":2,"range":1e+400}}"#;
        assert!(command_from_json(line).is_none());
    }

    #[test]
    fn warp_command_json_is_parsed_into_client_command_warp() {
        let line = r#"{"WarpCommand":{"gate_id":2}}"#;
        let cmd = command_from_json(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::Warp(c) => {
                assert_eq!(c.target, dawn_core::WarpTarget::Gate(JumpGateId(2)));
            }
            other => panic!("expected Warp, got {other:?}"),
        }

        let line2 = r#"{"WarpCommand":{"target":{"Gate":2}}}"#;
        let cmd2 = command_from_json(line2).expect("must parse");
        match cmd2 {
            dawn_core::ClientCommand::Warp(c) => {
                assert_eq!(c.target, dawn_core::WarpTarget::Gate(JumpGateId(2)));
            }
            other => panic!("expected Warp, got {other:?}"),
        }

        let line3 = r#"{"WarpCommand":{"target":{"Body":1}}}"#;
        let cmd3 = command_from_json(line3).expect("must parse");
        match cmd3 {
            dawn_core::ClientCommand::Warp(c) => {
                assert_eq!(
                    c.target,
                    dawn_core::WarpTarget::Body(dawn_core::CelestialBodyId(1))
                );
            }
            other => panic!("expected Warp, got {other:?}"),
        }
    }

    #[test]
    fn dock_command_json_is_parsed_into_client_command_dock() {
        let line = r#"{"DockCommand":{"station_id":2}}"#;
        let cmd = command_from_json(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::Dock(c) => {
                assert_eq!(c.station_id, dawn_core::StationId(2));
            }
            other => panic!("expected Dock, got {other:?}"),
        }
    }

    #[test]
    fn disassemble_ship_command_json_is_parsed_into_client_command_disassemble_ship() {
        let line = r#"{"DisassembleShipCommand":{"ship_id":42,"station_id":2}}"#;
        let cmd = command_from_json(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::DisassembleShip(c) => {
                assert_eq!(c.ship_id, ship_id(42));
                assert_eq!(c.station_id, dawn_core::StationId(2));
            }
            other => panic!("expected DisassembleShip, got {other:?}"),
        }
    }

    #[test]
    fn assemble_command_json_is_parsed_into_client_command_assemble() {
        let line = r#"{"AssembleCommand":{"station_id":2,"ship_type_id":1}}"#;
        let cmd = command_from_json(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::Assemble(c) => {
                assert_eq!(c.station_id, dawn_core::StationId(2));
                assert_eq!(c.ship_type_id, dawn_core::ShipTypeId(1));
            }
            other => panic!("expected Assemble, got {other:?}"),
        }
    }

    #[test]
    fn disembark_command_json_is_parsed_into_client_command_disembark() {
        let line = r#"{"DisembarkCommand":{}}"#;
        let cmd = command_from_json(line).expect("must parse");
        assert!(matches!(cmd, dawn_core::ClientCommand::Disembark(_)));
    }

    #[test]
    fn transfer_to_station_command_json_with_scrap_metal_is_parsed() {
        let line = r#"{"TransferToStationCommand":{"ship_id":42,"station_id":2,"item_type":"ScrapMetal","module_id":0,"ship_type_id":0,"direction":"ToStation"}}"#;
        let cmd = command_from_json(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::TransferToStation(c) => {
                assert_eq!(c.ship_id, ship_id(42));
                assert_eq!(c.station_id, dawn_core::StationId(2));
                assert_eq!(c.item_id, dawn_core::ItemId::ScrapMetal);
                assert_eq!(c.direction, dawn_core::TransferDirection::ToStation);
            }
            other => panic!("expected TransferToStation, got {other:?}"),
        }
    }

    #[test]
    fn transfer_to_station_command_json_with_module_is_parsed() {
        let line = r#"{"TransferToStationCommand":{"ship_id":42,"station_id":2,"item_type":"Module","module_id":7,"ship_type_id":0,"direction":"ToStation"}}"#;
        let cmd = command_from_json(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::TransferToStation(c) => {
                assert_eq!(c.item_id, dawn_core::ItemId::Module(ModuleId(7)));
            }
            other => panic!("expected TransferToStation, got {other:?}"),
        }
    }

    #[test]
    fn transfer_to_station_command_json_with_to_ship_direction_is_parsed() {
        let line = r#"{"TransferToStationCommand":{"ship_id":42,"station_id":2,"item_type":"ScrapMetal","module_id":0,"ship_type_id":0,"direction":"ToShip"}}"#;
        let cmd = command_from_json(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::TransferToStation(c) => {
                assert_eq!(c.direction, dawn_core::TransferDirection::ToShip);
            }
            other => panic!("expected TransferToStation, got {other:?}"),
        }
    }

    #[test]
    fn transfer_to_station_command_json_with_unknown_item_type_fails_to_parse() {
        let line = r#"{"TransferToStationCommand":{"ship_id":42,"station_id":2,"item_type":"Bogus","module_id":0,"ship_type_id":0,"direction":"ToStation"}}"#;
        assert!(command_from_json(line).is_none());
    }

    #[test]
    fn transfer_to_station_command_json_with_unknown_direction_fails_to_parse() {
        let line = r#"{"TransferToStationCommand":{"ship_id":42,"station_id":2,"item_type":"ScrapMetal","module_id":0,"ship_type_id":0,"direction":"Bogus"}}"#;
        assert!(command_from_json(line).is_none());
    }

    #[test]
    fn reorder_fitted_module_command_json_is_parsed() {
        let line = r#"{"ReorderFittedModuleCommand":{"ship_id":1,"slot":"Mid","from_index":0,"to_index":1}}"#;
        let cmd = command_from_json(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::ReorderFittedModule(c) => {
                assert_eq!(c.ship_id, ship_id(1));
                assert_eq!(c.slot, dawn_core::SlotKind::Mid);
                assert_eq!(c.from_index, 0);
                assert_eq!(c.to_index, 1);
            }
            other => panic!("expected ReorderFittedModule, got {other:?}"),
        }
    }

    #[test]
    fn orbit_command_json_with_target_id_is_parsed_into_client_command_orbit() {
        let line = r#"{"OrbitCommand":{"target_id":2,"radius":3000.0}}"#;
        let cmd = command_from_json(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::Orbit(c) => {
                assert_eq!(c.target, ApproachTarget::Ship(ship_id(2)));
                assert_eq!(c.radius, Some(3000.0));
            }
            other => panic!("expected Orbit, got {other:?}"),
        }
    }

    #[test]
    fn orbit_command_json_with_gate_id_and_no_radius_is_parsed() {
        let line = r#"{"OrbitCommand":{"gate_id":4}}"#;
        let cmd = command_from_json(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::Orbit(c) => {
                assert_eq!(c.target, ApproachTarget::Gate(JumpGateId(4)));
                assert_eq!(c.radius, None);
            }
            other => panic!("expected Orbit, got {other:?}"),
        }
    }

    #[test]
    fn keep_at_range_command_json_is_parsed_into_client_command_keep_at_range() {
        let line = r#"{"KeepAtRangeCommand":{"target_id":2,"range":5000.0}}"#;
        let cmd = command_from_json(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::KeepAtRange(c) => {
                assert_eq!(c.target, ApproachTarget::Ship(ship_id(2)));
                assert_eq!(c.range, Some(5000.0));
            }
            other => panic!("expected KeepAtRange, got {other:?}"),
        }
    }

    #[test]
    fn fit_module_command_json_is_parsed_into_client_command_fit() {
        let line = r#"{"FitModuleCommand":{"ship_id":1,"module_id":2,"slot":"High"}}"#;
        let cmd = command_from_json(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::Fit(c) => {
                assert_eq!(c.ship_id, ship_id(1));
                assert_eq!(c.module_id, ModuleId(2));
                assert_eq!(c.slot, SlotKind::High);
            }
            other => panic!("expected Fit, got {other:?}"),
        }
    }

    #[test]
    fn unfit_module_command_json_is_parsed_into_client_command_unfit() {
        let line = r#"{"UnfitModuleCommand":{"ship_id":1,"module_id":2,"slot":"Mid"}}"#;
        let cmd = command_from_json(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::Unfit(c) => {
                assert_eq!(c.ship_id, ship_id(1));
                assert_eq!(c.module_id, ModuleId(2));
                assert_eq!(c.slot, SlotKind::Mid);
            }
            other => panic!("expected Unfit, got {other:?}"),
        }
    }

    #[test]
    fn unknown_command_type_returns_none() {
        let line = r#"{"UnknownCommand":{"ship_id":1}}"#;
        assert!(command_from_json(line).is_none());
    }

    /// ADR-0041: `ClientCommandWire` gained `Serialize` so the Godot client
    /// can construct a variant directly and serialize it out, instead of
    /// hand-building a `Dictionary` that had to match this schema by eye.
    /// This proves the round trip agrees with `client_command_from_wire`'s
    /// deserialize path (both directions must describe the same wire shape).
    #[test]
    fn move_command_wire_round_trips_through_serialize_and_deserialize() {
        let cmd = ClientCommandWire::MoveCommand {
            target: PosWire {
                x: 10.0,
                y: 0.0,
                z: -5.0,
            },
        };
        let line = serde_json::to_string(&cmd).expect("serialize");
        let wire: ClientCommandWire = serde_json::from_str(&line).expect("deserialize");

        let parsed = client_command_from_wire(wire).expect("must convert");
        match parsed {
            ClientCommand::Move(c) => {
                assert!((c.target_position.x - 10.0).abs() < 1e-6);
                assert!((c.target_position.z - (-5.0)).abs() < 1e-6);
            }
            other => panic!("expected Move, got {other:?}"),
        }
    }

    /// A non-finite coordinate must still be rejected after the move to
    /// dawn-wire (security-review.md SEC-5) -- this crate now owns the
    /// check, not dawn-actor.
    #[test]
    fn move_command_wire_with_an_overflowing_coordinate_fails_to_convert() {
        let wire = ClientCommandWire::MoveCommand {
            target: PosWire {
                x: f64::INFINITY,
                y: 0.0,
                z: 0.0,
            },
        };
        assert!(client_command_from_wire(wire).is_none());
    }
}
