use dawn_core::{
    ActivateModuleCommand, ApproachCommand, ApproachTarget, AttackCommand,
    BuildPackagedShipCommand, ClientCommand, DeactivateModuleCommand, DisassembleShipCommand,
    DockCommand, EntityId, LockOnCommand, ModuleId, MoveCommand, Position, ShipId, SlotKind,
    StopCommand, UndockCommand,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, JsonSchema, Clone, Copy)]
pub struct PosJson {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Debug, Serialize, JsonSchema, Clone, Copy)]
pub struct VelJson {
    pub dx: f32,
    pub dy: f32,
    pub dz: f32,
}

impl From<Position> for PosJson {
    fn from(p: Position) -> Self {
        Self {
            x: p.x,
            y: p.y,
            z: p.z,
        }
    }
}

impl From<dawn_core::Velocity> for VelJson {
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
#[derive(Debug, Deserialize, JsonSchema, Clone, Copy)]
pub enum WarpTargetJson {
    Gate(u32),
    Body(u32),
}

/// Every message a client can send to the server over the WebSocket
/// connection. Serialized as a single JSON line tagged by `"type"`.
///
/// This enum is the schema-of-record for the client -> server half of the
/// wire protocol (see [`super::EventJson`] for the server -> client half). It
/// intentionally mirrors the wire format exactly, including the two
/// backward-compatible quirks below -- it does not enforce the "exactly one
/// of these two fields" business rules those quirks involve; that
/// validation still happens in [`parse_client_command`], same as before
/// this enum existed.
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
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum ClientCommandJson {
    MoveCommand {
        target: PosJson,
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
        target: Option<WarpTargetJson>,
        /// Legacy form: `{"gate_id": N}` instead of `{"target": {"Gate": N}}`.
        gate_id: Option<u32>,
    },
    OrbitCommand {
        gate_id: Option<u32>,
        target_id: Option<u64>,
        radius: Option<f32>,
    },
    KeepAtRangeCommand {
        gate_id: Option<u32>,
        target_id: Option<u64>,
        range: Option<f32>,
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
    /// Move the entire stack of an item out of a docked ship's own cargo
    /// into the caller's station inventory (ADR-0034 9B). `item_type` is
    /// one of `"Module"`, `"PackagedShip"`, `"ScrapMetal"` (matching
    /// `ItemRow`'s wire shape) with `module_id`/`ship_type_id` populated
    /// only for the variant that uses them (`0` otherwise).
    TransferToStationCommand {
        ship_id: u64,
        station_id: u32,
        item_type: String,
        module_id: u32,
        ship_type_id: u32,
    },
}

/// Render the client -> server wire schema (see [`ClientCommandJson`]) as a
/// JSON Schema document.
pub fn client_command_json_schema() -> schemars::schema::RootSchema {
    schemars::schema_for!(ClientCommandJson)
}

/// Parse a newline-terminated JSON line from the Godot client into a
/// [`ClientCommand`]. Returns `None` for unknown or malformed messages.
pub fn parse_client_command(line: &str) -> Option<ClientCommand> {
    let json: ClientCommandJson = serde_json::from_str(line).ok()?;
    client_command_from_json(json)
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

fn client_command_from_json(json: ClientCommandJson) -> Option<ClientCommand> {
    match json {
        ClientCommandJson::MoveCommand { target } => Some(ClientCommand::Move(MoveCommand {
            target_position: Position {
                x: target.x,
                y: target.y,
                z: target.z,
            },
        })),
        ClientCommandJson::LockOnCommand { target_id } => {
            Some(ClientCommand::LockOn(LockOnCommand {
                ship_id: ShipId(EntityId::from_raw(0)),
                target_id: ShipId(EntityId::from_raw(target_id)),
            }))
        }
        ClientCommandJson::ActivateModuleCommand {
            module_id,
            slot,
            target_ship_id,
        } => Some(ClientCommand::Activate(ActivateModuleCommand {
            module_id: ModuleId(module_id),
            slot: parse_slot_kind(&slot)?,
            target_ship_id: target_ship_id.map(|raw| ShipId(EntityId::from_raw(raw))),
        })),
        ClientCommandJson::DeactivateModuleCommand { module_id, slot } => {
            Some(ClientCommand::Deactivate(DeactivateModuleCommand {
                module_id: ModuleId(module_id),
                slot: parse_slot_kind(&slot)?,
            }))
        }
        ClientCommandJson::AttackCommand {
            attacker_id,
            target_id,
        } => Some(ClientCommand::Attack(AttackCommand {
            attacker_id: ShipId(EntityId::from_raw(attacker_id)),
            target_id: ShipId(EntityId::from_raw(target_id)),
        })),
        ClientCommandJson::StopCommand {} => Some(ClientCommand::Stop(StopCommand)),
        ClientCommandJson::JumpCommand { gate_id } => {
            Some(ClientCommand::Jump(dawn_core::JumpCommand {
                gate_id: dawn_core::JumpGateId(gate_id),
            }))
        }
        ClientCommandJson::ApproachCommand { gate_id, target_id } => {
            let target = approach_target_from_gate_or_ship(gate_id, target_id)?;
            Some(ClientCommand::Approach(ApproachCommand { target }))
        }
        ClientCommandJson::WarpCommand { target, gate_id } => {
            let warp_target = match target {
                Some(WarpTargetJson::Gate(gate)) => {
                    dawn_core::WarpTarget::Gate(dawn_core::JumpGateId(gate))
                }
                Some(WarpTargetJson::Body(body)) => {
                    dawn_core::WarpTarget::Body(dawn_core::CelestialBodyId(body))
                }
                None => dawn_core::WarpTarget::Gate(dawn_core::JumpGateId(gate_id?)),
            };
            Some(ClientCommand::Warp(dawn_core::WarpCommand {
                target: warp_target,
            }))
        }
        ClientCommandJson::OrbitCommand {
            gate_id,
            target_id,
            radius,
        } => {
            let target = approach_target_from_gate_or_ship(gate_id, target_id)?;
            Some(ClientCommand::Orbit(dawn_core::OrbitCommand {
                target,
                radius,
            }))
        }
        ClientCommandJson::KeepAtRangeCommand {
            gate_id,
            target_id,
            range,
        } => {
            let target = approach_target_from_gate_or_ship(gate_id, target_id)?;
            Some(ClientCommand::KeepAtRange(dawn_core::KeepAtRangeCommand {
                target,
                range,
            }))
        }
        ClientCommandJson::FitModuleCommand {
            ship_id,
            module_id,
            slot,
        } => Some(ClientCommand::Fit(dawn_core::FitModuleCommand {
            ship_id: ShipId(EntityId::from_raw(ship_id)),
            module_id: ModuleId(module_id),
            slot: parse_slot_kind(&slot)?,
        })),
        ClientCommandJson::UnfitModuleCommand {
            ship_id,
            module_id,
            slot,
        } => Some(ClientCommand::Unfit(dawn_core::UnfitModuleCommand {
            ship_id: ShipId(EntityId::from_raw(ship_id)),
            module_id: ModuleId(module_id),
            slot: parse_slot_kind(&slot)?,
        })),
        ClientCommandJson::DockCommand { station_id } => Some(ClientCommand::Dock(DockCommand {
            station_id: dawn_core::StationId(station_id),
        })),
        ClientCommandJson::UndockCommand {} => Some(ClientCommand::Undock(UndockCommand)),
        ClientCommandJson::BuildPackagedShipCommand {
            ship_id,
            station_id,
            ship_type_id,
        } => Some(ClientCommand::BuildPackagedShip(BuildPackagedShipCommand {
            ship_id: ShipId(EntityId::from_raw(ship_id)),
            station_id: dawn_core::StationId(station_id),
            ship_type_id: dawn_core::ShipTypeId(ship_type_id),
        })),
        ClientCommandJson::DisassembleShipCommand {
            ship_id,
            station_id,
        } => Some(ClientCommand::DisassembleShip(DisassembleShipCommand {
            ship_id: ShipId(EntityId::from_raw(ship_id)),
            station_id: dawn_core::StationId(station_id),
        })),
        ClientCommandJson::SelectActiveShipCommand { ship_id } => Some(
            ClientCommand::SelectActiveShip(dawn_core::SelectActiveShipCommand {
                ship_id: ShipId(EntityId::from_raw(ship_id)),
            }),
        ),
        ClientCommandJson::AssembleCommand {
            station_id,
            ship_type_id,
        } => Some(ClientCommand::Assemble(dawn_core::AssembleCommand {
            station_id: dawn_core::StationId(station_id),
            ship_type_id: dawn_core::ShipTypeId(ship_type_id),
        })),
        ClientCommandJson::DisembarkCommand {} => {
            Some(ClientCommand::Disembark(dawn_core::DisembarkCommand))
        }
        ClientCommandJson::TransferToStationCommand {
            ship_id,
            station_id,
            item_type,
            module_id,
            ship_type_id,
        } => {
            let item_id = match item_type.as_str() {
                "Module" => dawn_core::ItemId::Module(ModuleId(module_id)),
                "PackagedShip" => {
                    dawn_core::ItemId::PackagedShip(dawn_core::ShipTypeId(ship_type_id))
                }
                "ScrapMetal" => dawn_core::ItemId::ScrapMetal,
                _ => return None,
            };
            Some(ClientCommand::TransferToStation(
                dawn_core::TransferToStationCommand {
                    ship_id: ShipId(EntityId::from_raw(ship_id)),
                    station_id: dawn_core::StationId(station_id),
                    item_id,
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
