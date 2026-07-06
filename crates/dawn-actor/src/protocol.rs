//! Wire protocol translation between DomainEvents / ClientCommands and JSON.
//!
//! This module is the single place that knows both the Rust domain types and
//! the JSON keys the Godot client expects. Keeping it separate from
//! `ws_server.rs` means WebSocket transport logic never changes when the JSON
//! schema evolves, and vice versa. Both binaries (`dawn-simulation`,
//! `dawn-sector-node`) share this one definition (previously duplicated).
//!
//! # Responsibilities
//! - [`domain_event_to_json`]: DomainEvent → newline-delimited JSON (server → client).
//! - [`redirect_json`]: tell a client to reconnect to another node's WS (multi-node jump).
//! - [`parse_client_command`]: JSON line → ClientCommand (client → server).

use dawn_core::{
    ActivateModuleCommand, ApproachCommand, ApproachTarget, AttackCommand,
    BuildPackagedShipCommand, ClientCommand, DeactivateModuleCommand, DisassembleShipCommand,
    DockCommand, DomainEvent, EntityId, LockOnCommand, ModuleId, MoveCommand, PlayerId, Position,
    ShipId, SlotKind, StopCommand, UndockCommand,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ── Output types (server → client) ───────────────────────────────────────────

/// Every message the server sends to a client over the WebSocket connection.
/// Serialized as a single JSON line tagged by `"type"`.
///
/// This enum is the schema-of-record for the server -> client half of the
/// wire protocol: [`event_json_schema()`] renders it to a JSON Schema document that
/// `docs/architecture/wire-protocol.md` is generated from. Adding, removing,
/// or renaming a field here changes the wire format for every client
/// (Godot today; any future client written against
/// `docs/architecture/wire-protocol.md`).
#[derive(Debug, Serialize, JsonSchema)]
#[serde(tag = "type")]
pub enum EventJson {
    ShipSpawned {
        ship_id: u64,
        position: PosJson,
        tick: u64,
    },
    VelocityChanged {
        ship_id: u64,
        velocity: VelJson,
        tick: u64,
    },
    ShipDespawned {
        ship_id: u64,
        tick: u64,
    },
    ShipDocked {
        ship_id: u64,
        station_id: u32,
        tick: u64,
    },
    ShipUndocked {
        ship_id: u64,
        station_id: u32,
        tick: u64,
    },
    DamageTaken {
        ship_id: u64,
        damage: f32,
        current_shield: f32,
        current_armor: f32,
        current_hull: f32,
        tick: u64,
    },
    RepairApplied {
        ship_id: u64,
        amount: f32,
        layer: String,
        current_shield: f32,
        current_armor: f32,
        current_hull: f32,
        tick: u64,
    },
    ShipDestroyed {
        ship_id: u64,
        killer_id: u64,
        tick: u64,
    },
    TargetLocked {
        locker_id: u64,
        target_id: u64,
        tick: u64,
    },
    LockLost {
        locker_id: u64,
        target_id: u64,
        tick: u64,
    },
    ModuleActivated {
        ship_id: u64,
        module_id: u32,
        slot: String,
        /// Target of a targeted module (Weapon/Tackle), per ADR-0035.
        #[serde(skip_serializing_if = "Option::is_none")]
        target_ship_id: Option<u64>,
        tick: u64,
    },
    ModuleDeactivated {
        ship_id: u64,
        module_id: u32,
        slot: String,
        /// Why the system forced this off ("cap" | "range"); omitted for a
        /// player-issued deactivation (ADR-0035).
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        tick: u64,
    },
    JumpGateUsed {
        ship_id: u64,
        gate_id: u32,
        from_sector: u8,
        to_sector: u8,
        entry_pos: PosJson,
        tick: u64,
    },
    StarSystemChanged {
        ship_id: u64,
        from_system: u32,
        to_system: u32,
        tick: u64,
    },
    // Sent when the player's ship jumps to a sector owned by a different
    // physical node (dawn-sector-node multi-node clusters only).
    Redirect {
        ws_addr: String,
        player_id: u64,
        ship_id: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResumeIdentity {
    pub player_id: PlayerId,
    pub ship_id: ShipId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HelloMessage {
    pub resume: Option<ResumeIdentity>,
}

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

/// Render the server -> client wire schema (see [`EventJson`]) as a JSON
/// Schema document.
///
/// `examples/gen_wire_schema.rs` writes this to
/// `docs/architecture/wire-protocol.schema.json`, and the
/// `wire_schema_doc_is_up_to_date` test below fails the build if the checked
/// in file drifts from what this function currently produces -- regenerate
/// with `cargo run -p dawn-actor --example gen_wire_schema` when it does.
pub fn event_json_schema() -> schemars::schema::RootSchema {
    schemars::schema_for!(EventJson)
}

/// Serialize a [`DomainEvent`] to the JSON line the Godot client expects.
/// Returns `None` for internal events that are not forwarded to clients
/// (transit internals, combat bookkeeping).
pub fn domain_event_to_json(event: &DomainEvent) -> Option<String> {
    let j = match event {
        DomainEvent::ShipSpawned(e) => EventJson::ShipSpawned {
            ship_id: e.ship_id.raw(),
            position: e.initial_position.into(),
            tick: e.tick.value(),
        },
        DomainEvent::VelocityChanged(e) => EventJson::VelocityChanged {
            ship_id: e.ship_id.raw(),
            velocity: e.velocity.into(),
            tick: e.tick.value(),
        },
        DomainEvent::ShipDespawned(e) => EventJson::ShipDespawned {
            ship_id: e.ship_id.raw(),
            tick: e.tick.value(),
        },
        DomainEvent::ShipDocked(e) => EventJson::ShipDocked {
            ship_id: e.ship_id.raw(),
            station_id: e.station_id.0,
            tick: e.tick.value(),
        },
        DomainEvent::ShipUndocked(e) => EventJson::ShipUndocked {
            ship_id: e.ship_id.raw(),
            station_id: e.station_id.0,
            tick: e.tick.value(),
        },
        DomainEvent::DamageTaken(e) => EventJson::DamageTaken {
            ship_id: e.ship_id.raw(),
            damage: e.damage,
            current_shield: e.current_shield,
            current_armor: e.current_armor,
            current_hull: e.current_hull,
            tick: e.tick.value(),
        },
        DomainEvent::RepairApplied(e) => EventJson::RepairApplied {
            ship_id: e.ship_id.raw(),
            amount: e.amount,
            layer: format!("{:?}", e.layer),
            current_shield: e.current_shield,
            current_armor: e.current_armor,
            current_hull: e.current_hull,
            tick: e.tick.value(),
        },
        DomainEvent::ShipDestroyed(e) => EventJson::ShipDestroyed {
            ship_id: e.ship_id.raw(),
            killer_id: e.killer_id.raw(),
            tick: e.tick.value(),
        },
        DomainEvent::TargetLocked(e) => EventJson::TargetLocked {
            locker_id: e.locker_id.raw(),
            target_id: e.target_id.raw(),
            tick: e.tick.value(),
        },
        DomainEvent::LockLost(e) => EventJson::LockLost {
            locker_id: e.locker_id.raw(),
            target_id: e.target_id.raw(),
            tick: e.tick.value(),
        },
        DomainEvent::ModuleActivated(e) => EventJson::ModuleActivated {
            ship_id: e.ship_id.raw(),
            module_id: e.module_id.0,
            slot: format!("{:?}", e.slot),
            target_ship_id: e.target_ship_id.map(|t| t.raw()),
            tick: e.tick.value(),
        },
        DomainEvent::ModuleDeactivated(e) => EventJson::ModuleDeactivated {
            ship_id: e.ship_id.raw(),
            module_id: e.module_id.0,
            slot: format!("{:?}", e.slot),
            reason: e.forced_reason.map(|r| match r {
                dawn_core::events::ModuleDeactivationReason::CapacitorExhausted => {
                    "cap".to_string()
                }
                dawn_core::events::ModuleDeactivationReason::OutOfRange => "range".to_string(),
            }),
            tick: e.tick.value(),
        },
        // Jump Gate Navigation (ADR-0009): Godot uses these to teleport the
        // ship to entry_pos and switch the star-system backdrop.
        DomainEvent::JumpGateUsed(e) => EventJson::JumpGateUsed {
            ship_id: e.ship_id.raw(),
            gate_id: e.gate_id.0,
            from_sector: e.from_sector.0,
            to_sector: e.to_sector.0,
            entry_pos: e.entry_pos.into(),
            tick: e.tick.value(),
        },
        DomainEvent::StarSystemChanged(e) => EventJson::StarSystemChanged {
            ship_id: e.ship_id.raw(),
            from_system: e.from_system.0,
            to_system: e.to_system.0,
            tick: e.tick.value(),
        },
        // Internal node-ownership / combat events — not forwarded to clients.
        DomainEvent::ShipFitted(_) => return None,
        DomainEvent::WeaponFired(_) => return None,
        DomainEvent::TackleApplied(_) => return None,
        DomainEvent::TackleReleased(_) => return None,
        DomainEvent::SectorTransitRequested(_) => return None,
        DomainEvent::SectorTransitCompleted(_) => return None,
        DomainEvent::SectorTransitAborted(_) => return None,
        // ADR-0029: a coordinate rebase keeps the absolute position unchanged
        // and velocity is frame-invariant, so a client that integrates
        // VelocityChanged stays consistent without seeing the rebase. Client
        // anchor handling (floating origin, fresh InitialState) lands in step 6.
        DomainEvent::AnchorRebased(_) => return None,
        DomainEvent::PackagedShipBuilt(_) => return None,
        DomainEvent::ShipDisassembled(_) => return None,
    };
    serde_json::to_string(&j).ok()
}

/// Build a `{"type":"Redirect","ws_addr":"..."}` JSON line for a client whose
/// ship just jumped to a sector owned by a different physical node.
pub fn redirect_json(
    ws_addr: std::net::SocketAddr,
    player_id: PlayerId,
    ship_id: ShipId,
) -> String {
    let j = EventJson::Redirect {
        ws_addr: ws_addr.to_string(),
        player_id: player_id.raw(),
        ship_id: ship_id.raw(),
    };
    serde_json::to_string(&j).unwrap_or_default()
}

/// Parse the client Hello line. Fresh clients send only `{"type":"Hello"}`;
/// clients following a Redirect include the identity to resume.
pub fn parse_hello(line: &str) -> Option<HelloMessage> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if v.get("type")?.as_str()? != "Hello" {
        return None;
    }

    let resume = match (
        v.get("player_id").and_then(|id| id.as_u64()),
        v.get("ship_id").and_then(|id| id.as_u64()),
    ) {
        (Some(player_id), Some(ship_id)) => Some(ResumeIdentity {
            player_id: PlayerId(player_id),
            ship_id: ShipId(EntityId::from_raw(ship_id)),
        }),
        _ => None,
    };

    Some(HelloMessage { resume })
}

// ── Input parser (client → server) ───────────────────────────────────────────

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
/// wire protocol (see [`EventJson`] for the server -> client half). It
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
}

/// Render the client -> server wire schema (see [`ClientCommandJson`]) as a
/// JSON Schema document.
///
/// `examples/gen_wire_schema.rs` writes this to
/// `docs/architecture/wire-protocol-commands.schema.json`, and the
/// `wire_schema_doc_is_up_to_date` test below fails the build if the checked
/// in file drifts from what this function currently produces -- regenerate
/// with `cargo run -p dawn-actor --example gen_wire_schema` when it does.
pub fn client_command_json_schema() -> schemars::schema::RootSchema {
    schemars::schema_for!(ClientCommandJson)
}

/// Parse a newline-terminated JSON line from the Godot client into a
/// [`ClientCommand`]. Returns `None` for unknown or malformed messages.
pub fn parse_client_command(line: &str) -> Option<ClientCommand> {
    let json: ClientCommandJson = serde_json::from_str(line).ok()?;
    client_command_from_json(json)
}

/// Resolve an `Option<gate_id>` / `Option<target_id>` pair into an
/// [`ApproachTarget`], shared by `ApproachCommand` / `OrbitCommand` /
/// `KeepAtRangeCommand`. `gate_id` wins if both are present, matching the
/// wire format documented on [`ClientCommandJson`].
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
            // `ship_id` is resolved server-side from the caller's active ship
            // (ADR-0037) -- it is never read from `lo.ship_id` in
            // `apply_client_command`, so a placeholder here is safe.
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{JumpGateId, NodeId};

    fn ship_id(n: u64) -> ShipId {
        ShipId(EntityId::new(NodeId(0), n))
    }

    #[test]
    fn move_command_json_is_parsed_into_client_command_move() {
        let line = r#"{"type":"MoveCommand","target":{"x":10.0,"y":0.0,"z":-5.0}}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            ClientCommand::Move(c) => {
                assert!((c.target_position.x - 10.0).abs() < 1e-6);
            }
            other => panic!("expected Move, got {other:?}"),
        }
    }

    #[test]
    fn warp_command_json_is_parsed_into_client_command_warp() {
        // Legacy wire format (gate_id key)
        let line = r#"{"type":"WarpCommand","gate_id":2}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            ClientCommand::Warp(c) => {
                assert_eq!(c.target, dawn_core::WarpTarget::Gate(JumpGateId(2)));
            }
            other => panic!("expected Warp, got {other:?}"),
        }
        // New wire format (target key with Gate variant)
        let line2 = r#"{"type":"WarpCommand","target":{"Gate":2}}"#;
        let cmd2 = parse_client_command(line2).expect("must parse");
        match cmd2 {
            ClientCommand::Warp(c) => {
                assert_eq!(c.target, dawn_core::WarpTarget::Gate(JumpGateId(2)));
            }
            other => panic!("expected Warp, got {other:?}"),
        }
        // Body target
        let line3 = r#"{"type":"WarpCommand","target":{"Body":1}}"#;
        let cmd3 = parse_client_command(line3).expect("must parse");
        match cmd3 {
            ClientCommand::Warp(c) => {
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
        let line = r#"{"type":"DockCommand","station_id":2}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            ClientCommand::Dock(c) => {
                assert_eq!(c.station_id, dawn_core::StationId(2));
            }
            other => panic!("expected Dock, got {other:?}"),
        }
    }

    #[test]
    fn disassemble_ship_command_json_is_parsed_into_client_command_disassemble_ship() {
        let line = r#"{"type":"DisassembleShipCommand","ship_id":42,"station_id":2}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            ClientCommand::DisassembleShip(c) => {
                assert_eq!(c.ship_id, ship_id(42));
                assert_eq!(c.station_id, dawn_core::StationId(2));
            }
            other => panic!("expected DisassembleShip, got {other:?}"),
        }
    }

    #[test]
    fn orbit_command_json_with_target_id_is_parsed_into_client_command_orbit() {
        let line = r#"{"type":"OrbitCommand","target_id":2,"radius":3000.0}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            ClientCommand::Orbit(c) => {
                assert_eq!(c.target, ApproachTarget::Ship(ship_id(2)));
                assert_eq!(c.radius, Some(3000.0));
            }
            other => panic!("expected Orbit, got {other:?}"),
        }
    }

    #[test]
    fn orbit_command_json_with_gate_id_and_no_radius_is_parsed() {
        let line = r#"{"type":"OrbitCommand","gate_id":4}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            ClientCommand::Orbit(c) => {
                assert_eq!(c.target, ApproachTarget::Gate(JumpGateId(4)));
                assert_eq!(c.radius, None);
            }
            other => panic!("expected Orbit, got {other:?}"),
        }
    }

    #[test]
    fn keep_at_range_command_json_is_parsed_into_client_command_keep_at_range() {
        let line = r#"{"type":"KeepAtRangeCommand","target_id":2,"range":5000.0}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            ClientCommand::KeepAtRange(c) => {
                assert_eq!(c.target, ApproachTarget::Ship(ship_id(2)));
                assert_eq!(c.range, Some(5000.0));
            }
            other => panic!("expected KeepAtRange, got {other:?}"),
        }
    }

    #[test]
    fn fit_module_command_json_is_parsed_into_client_command_fit() {
        let line = r#"{"type":"FitModuleCommand","ship_id":1,"module_id":2,"slot":"High"}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            ClientCommand::Fit(c) => {
                assert_eq!(c.ship_id, ship_id(1));
                assert_eq!(c.module_id, ModuleId(2));
                assert_eq!(c.slot, SlotKind::High);
            }
            other => panic!("expected Fit, got {other:?}"),
        }
    }

    #[test]
    fn unfit_module_command_json_is_parsed_into_client_command_unfit() {
        let line = r#"{"type":"UnfitModuleCommand","ship_id":1,"module_id":2,"slot":"Mid"}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            ClientCommand::Unfit(c) => {
                assert_eq!(c.ship_id, ship_id(1));
                assert_eq!(c.module_id, ModuleId(2));
                assert_eq!(c.slot, SlotKind::Mid);
            }
            other => panic!("expected Unfit, got {other:?}"),
        }
    }

    #[test]
    fn unknown_command_type_returns_none() {
        let line = r#"{"type":"UnknownCommand","ship_id":1}"#;
        assert!(parse_client_command(line).is_none());
    }

    #[test]
    fn ship_docked_event_is_serialized_for_clients() {
        let json = domain_event_to_json(&DomainEvent::ShipDocked(dawn_core::events::ShipDocked {
            ship_id: ship_id(42),
            station_id: dawn_core::StationId(3),
            tick: dawn_core::Tick(9),
        }))
        .expect("ShipDocked should be forwarded");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "ShipDocked");
        assert_eq!(v["ship_id"], ship_id(42).raw());
        assert_eq!(v["station_id"], 3);
        assert_eq!(v["tick"], 9);
    }

    #[test]
    fn redirect_json_carries_resume_identity() {
        let addr: std::net::SocketAddr = "127.0.0.1:7880".parse().unwrap();
        let json = redirect_json(addr, dawn_core::PlayerId(7), ship_id(42));
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "Redirect");
        assert_eq!(v["ws_addr"], "127.0.0.1:7880");
        assert_eq!(v["player_id"], 7);
        assert_eq!(v["ship_id"], ship_id(42).raw());
    }

    #[test]
    fn hello_json_can_carry_resume_identity() {
        let line = format!(
            r#"{{"type":"Hello","player_id":7,"ship_id":{}}}"#,
            ship_id(42).raw()
        );
        let hello = parse_hello(&line).expect("must parse Hello");
        assert_eq!(hello.resume.unwrap().player_id, dawn_core::PlayerId(7));
        assert_eq!(hello.resume.unwrap().ship_id, ship_id(42));
    }

    #[test]
    fn hello_json_without_resume_stays_fresh() {
        let hello = parse_hello(r#"{"type":"Hello"}"#).expect("must parse Hello");
        assert!(hello.resume.is_none());
    }

    /// Guards `docs/architecture/wire-protocol.schema.json` and
    /// `wire-protocol-commands.schema.json` against drift. If this fails,
    /// `EventJson` / `ClientCommandJson` (or a type either references)
    /// changed -- regenerate with
    /// `cargo run -p dawn-actor --example gen_wire_schema` and commit the
    /// updated files alongside the code change.
    #[test]
    fn wire_schema_doc_is_up_to_date() {
        assert_schema_file_matches(
            &event_json_schema(),
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../docs/architecture/wire-protocol.schema.json"
            ),
        );
        assert_schema_file_matches(
            &client_command_json_schema(),
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../docs/architecture/wire-protocol-commands.schema.json"
            ),
        );
    }

    fn assert_schema_file_matches(schema: &schemars::schema::RootSchema, path: &str) {
        let current = serde_json::to_string_pretty(schema).unwrap() + "\n";
        let checked_in =
            std::fs::read_to_string(path).unwrap_or_else(|_| panic!("{path} must exist"));
        assert_eq!(
            current, checked_in,
            "{path} is stale -- regenerate with \
             `cargo run -p dawn-actor --example gen_wire_schema`"
        );
    }
}
