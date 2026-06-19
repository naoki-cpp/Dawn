//! Wire protocol translation between DomainEvents / ClientCommands and JSON.
//!
//! This module is the only place that knows both the Rust domain types and the
//! JSON keys the Godot client expects. Keeping it separate from ws_server.rs
//! means WebSocket transport logic never needs to change when the JSON schema
//! evolves, and vice versa.
//!
//! # Responsibilities
//! - [`domain_event_to_json`]: DomainEvent → newline-delimited JSON string (server → client).
//! - [`parse_client_command`]: JSON line → ClientCommand (client → server).

use dawn_actor::ClientCommand;
use dawn_core::{
    ActivateModuleCommand, ApproachCommand, ApproachTarget, AttackCommand,
    DeactivateModuleCommand, DomainEvent, EntityId, LockOnCommand, ModuleId,
    MoveCommand, Position, ShipId, SlotKind, StopCommand,
};
use serde::Serialize;

// ── Output types (server → client) ───────────────────────────────────────────

#[derive(Serialize)]
#[serde(tag = "type")]
enum EventJson {
    ShipSpawned      { ship_id: u64, position: PosJson, tick: u64 },
    VelocityChanged  { ship_id: u64, velocity: VelJson, tick: u64 },
    ShipDespawned    { ship_id: u64, tick: u64 },
    DamageTaken      { ship_id: u64, damage: f32, current_shield: f32, current_armor: f32, current_hull: f32, tick: u64 },
    ShipDestroyed    { ship_id: u64, killer_id: u64, tick: u64 },
    TargetLocked     { locker_id: u64, target_id: u64, tick: u64 },
    LockLost         { locker_id: u64, target_id: u64, tick: u64 },
    ModuleActivated  { ship_id: u64, module_id: u32, slot: String, tick: u64 },
    ModuleDeactivated{ ship_id: u64, module_id: u32, slot: String, tick: u64 },
    JumpGateUsed     { ship_id: u64, gate_id: u32, from_sector: u8, to_sector: u8, entry_pos: PosJson, tick: u64 },
    StarSystemChanged{ ship_id: u64, from_system: u32, to_system: u32, tick: u64 },
}

#[derive(Serialize, Clone, Copy)]
pub(crate) struct PosJson { pub x: f32, pub y: f32, pub z: f32 }

#[derive(Serialize, Clone, Copy)]
struct VelJson { dx: f32, dy: f32, dz: f32 }

impl From<Position> for PosJson {
    fn from(p: Position) -> Self { Self { x: p.x, y: p.y, z: p.z } }
}

impl From<dawn_core::Velocity> for VelJson {
    fn from(v: dawn_core::Velocity) -> Self { Self { dx: v.dx, dy: v.dy, dz: v.dz } }
}

/// Serialize a [`DomainEvent`] to the JSON string the Godot client expects.
/// Returns `None` for events that are not sent to clients (e.g. transit internals).
pub(crate) fn domain_event_to_json(event: &DomainEvent) -> Option<String> {
    let j = match event {
        DomainEvent::ShipSpawned(e) => EventJson::ShipSpawned {
            ship_id : e.ship_id.raw(),
            position: e.initial_position.into(),
            tick    : e.tick.value(),
        },
        DomainEvent::VelocityChanged(e) => EventJson::VelocityChanged {
            ship_id : e.ship_id.raw(),
            velocity: e.velocity.into(),
            tick    : e.tick.value(),
        },
        DomainEvent::ShipDespawned(e) => EventJson::ShipDespawned {
            ship_id: e.ship_id.raw(),
            tick   : e.tick.value(),
        },
        DomainEvent::DamageTaken(e) => EventJson::DamageTaken {
            ship_id       : e.ship_id.raw(),
            damage        : e.damage,
            current_shield: e.current_shield,
            current_armor : e.current_armor,
            current_hull  : e.current_hull,
            tick          : e.tick.value(),
        },
        DomainEvent::ShipDestroyed(e) => EventJson::ShipDestroyed {
            ship_id  : e.ship_id.raw(),
            killer_id: e.killer_id.raw(),
            tick     : e.tick.value(),
        },
        DomainEvent::TargetLocked(e) => EventJson::TargetLocked {
            locker_id: e.locker_id.raw(),
            target_id: e.target_id.raw(),
            tick     : e.tick.value(),
        },
        DomainEvent::LockLost(e) => EventJson::LockLost {
            locker_id: e.locker_id.raw(),
            target_id: e.target_id.raw(),
            tick     : e.tick.value(),
        },
        DomainEvent::ModuleActivated(e) => EventJson::ModuleActivated {
            ship_id  : e.ship_id.raw(),
            module_id: e.module_id.0,
            slot     : format!("{:?}", e.slot),
            tick     : e.tick.value(),
        },
        DomainEvent::ModuleDeactivated(e) => EventJson::ModuleDeactivated {
            ship_id  : e.ship_id.raw(),
            module_id: e.module_id.0,
            slot     : format!("{:?}", e.slot),
            tick     : e.tick.value(),
        },
        // Not sent to clients — internal node-ownership events (ADR-0014).
        DomainEvent::ShipFitted(_)             => return None,
        DomainEvent::WeaponFired(_)            => return None,
        DomainEvent::TackleApplied(_)          => return None,
        DomainEvent::TackleReleased(_)         => return None,
        DomainEvent::SectorTransitRequested(_) => return None,
        DomainEvent::SectorTransitCompleted(_) => return None,
        DomainEvent::SectorTransitAborted(_)   => return None,
        // Jump Gate Navigation (ADR-0009): Godot uses these to teleport the
        // ship to entry_pos and switch the star-system backdrop.
        DomainEvent::JumpGateUsed(e) => EventJson::JumpGateUsed {
            ship_id    : e.ship_id.raw(),
            gate_id    : e.gate_id.0,
            from_sector: e.from_sector.0,
            to_sector  : e.to_sector.0,
            entry_pos  : e.entry_pos.into(),
            tick       : e.tick.value(),
        },
        DomainEvent::StarSystemChanged(e) => EventJson::StarSystemChanged {
            ship_id    : e.ship_id.raw(),
            from_system: e.from_system.0,
            to_system  : e.to_system.0,
            tick       : e.tick.value(),
        },
    };
    serde_json::to_string(&j).ok()
}

// ── Input parser (client → server) ───────────────────────────────────────────

/// Parse a newline-terminated JSON line from the Godot client into a
/// [`ClientCommand`]. Returns `None` for unknown or malformed messages.
pub(crate) fn parse_client_command(line: &str) -> Option<ClientCommand> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    match v.get("type")?.as_str()? {
        "MoveCommand" => {
            let ship_id_raw = v.get("ship_id")?.as_u64()?;
            let target      = v.get("target")?;
            Some(ClientCommand::Move(MoveCommand {
                ship_id        : ShipId(EntityId::from_raw(ship_id_raw)),
                target_position: Position {
                    x: target.get("x")?.as_f64()? as f32,
                    y: target.get("y")?.as_f64()? as f32,
                    z: target.get("z")?.as_f64()? as f32,
                },
            }))
        }
        "LockOnCommand" => {
            let ship_id_raw   = v.get("ship_id")?.as_u64()?;
            let target_id_raw = v.get("target_id")?.as_u64()?;
            Some(ClientCommand::LockOn(LockOnCommand {
                ship_id  : ShipId(EntityId::from_raw(ship_id_raw)),
                target_id: ShipId(EntityId::from_raw(target_id_raw)),
            }))
        }
        "ActivateModuleCommand" => {
            let ship_id_raw   = v.get("ship_id")?.as_u64()?;
            let module_id_raw = v.get("module_id")?.as_u64()? as u32;
            let slot_str      = v.get("slot")?.as_str()?;
            Some(ClientCommand::Activate(ActivateModuleCommand {
                ship_id  : ShipId(EntityId::from_raw(ship_id_raw)),
                module_id: ModuleId(module_id_raw),
                slot     : parse_slot_kind(slot_str)?,
            }))
        }
        "DeactivateModuleCommand" => {
            let ship_id_raw   = v.get("ship_id")?.as_u64()?;
            let module_id_raw = v.get("module_id")?.as_u64()? as u32;
            let slot_str      = v.get("slot")?.as_str()?;
            Some(ClientCommand::Deactivate(DeactivateModuleCommand {
                ship_id  : ShipId(EntityId::from_raw(ship_id_raw)),
                module_id: ModuleId(module_id_raw),
                slot     : parse_slot_kind(slot_str)?,
            }))
        }
        "AttackCommand" => {
            let attacker_id_raw = v.get("attacker_id")?.as_u64()?;
            let target_id_raw   = v.get("target_id")?.as_u64()?;
            Some(ClientCommand::Attack(AttackCommand {
                attacker_id: ShipId(EntityId::from_raw(attacker_id_raw)),
                target_id  : ShipId(EntityId::from_raw(target_id_raw)),
            }))
        }
        "StopCommand" => {
            let ship_id_raw = v.get("ship_id")?.as_u64()?;
            Some(ClientCommand::Stop(StopCommand {
                ship_id: ShipId(EntityId::from_raw(ship_id_raw)),
            }))
        }
        "JumpCommand" => {
            let ship_id_raw = v.get("ship_id")?.as_u64()?;
            let gate_id_raw = v.get("gate_id")?.as_u64()? as u32;
            Some(ClientCommand::Jump(dawn_core::JumpCommand {
                ship_id: ShipId(EntityId::from_raw(ship_id_raw)),
                gate_id: dawn_core::JumpGateId(gate_id_raw),
            }))
        }
        "ApproachCommand" => {
            let ship_id_raw = v.get("ship_id")?.as_u64()?;
            // gate_id selects a Jump Gate target; otherwise target_id is a Ship.
            let target = if let Some(gate) = v.get("gate_id").and_then(|g| g.as_u64()) {
                ApproachTarget::Gate(dawn_core::JumpGateId(gate as u32))
            } else {
                let target_id_raw = v.get("target_id")?.as_u64()?;
                ApproachTarget::Ship(ShipId(EntityId::from_raw(target_id_raw)))
            };
            Some(ClientCommand::Approach(ApproachCommand {
                ship_id: ShipId(EntityId::from_raw(ship_id_raw)),
                target,
            }))
        }
        "WarpCommand" => {
            let ship_id_raw = v.get("ship_id")?.as_u64()?;
            // Accept {"target":{"Gate":2}} or legacy {"gate_id":2}.
            let target = if let Some(t) = v.get("target") {
                if let Some(gate_val) = t.get("Gate").and_then(|g| g.as_u64()) {
                    dawn_core::WarpTarget::Gate(dawn_core::JumpGateId(gate_val as u32))
                } else if let Some(body_val) = t.get("Body").and_then(|b| b.as_u64()) {
                    dawn_core::WarpTarget::Body(dawn_core::CelestialBodyId(body_val as u32))
                } else {
                    return None;
                }
            } else {
                let gate_id_raw = v.get("gate_id")?.as_u64()? as u32;
                dawn_core::WarpTarget::Gate(dawn_core::JumpGateId(gate_id_raw))
            };
            Some(ClientCommand::Warp(dawn_core::WarpCommand {
                ship_id: ShipId(EntityId::from_raw(ship_id_raw)),
                target,
            }))
        }
        _ => None,
    }
}

fn parse_slot_kind(s: &str) -> Option<SlotKind> {
    match s {
        "High" => Some(SlotKind::High),
        "Mid"  => Some(SlotKind::Mid),
        "Low"  => Some(SlotKind::Low),
        "Rig"  => Some(SlotKind::Rig),
        _      => None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{JumpGateId, NodeId};

    fn ship_id(n: u64) -> ShipId { ShipId(EntityId::new(NodeId(0), n)) }

    #[test]
    fn move_command_json_is_parsed_into_client_command_move() {
        let line = r#"{"type":"MoveCommand","ship_id":1,"target":{"x":10.0,"y":0.0,"z":-5.0}}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            ClientCommand::Move(c) => {
                assert_eq!(c.ship_id, ship_id(1));
                assert!((c.target_position.x - 10.0).abs() < 1e-6);
            }
            other => panic!("expected Move, got {other:?}"),
        }
    }

    #[test]
    fn warp_command_json_is_parsed_into_client_command_warp() {
        // Legacy wire format (gate_id key)
        let line = r#"{"type":"WarpCommand","ship_id":42,"gate_id":2}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            ClientCommand::Warp(c) => {
                assert_eq!(c.ship_id.raw(), 42);
                assert_eq!(c.target, dawn_core::WarpTarget::Gate(JumpGateId(2)));
            }
            other => panic!("expected Warp, got {other:?}"),
        }
        // New wire format (target key with Gate variant)
        let line2 = r#"{"type":"WarpCommand","ship_id":42,"target":{"Gate":2}}"#;
        let cmd2 = parse_client_command(line2).expect("must parse");
        match cmd2 {
            ClientCommand::Warp(c) => {
                assert_eq!(c.target, dawn_core::WarpTarget::Gate(JumpGateId(2)));
            }
            other => panic!("expected Warp, got {other:?}"),
        }
        // Body target
        let line3 = r#"{"type":"WarpCommand","ship_id":42,"target":{"Body":1}}"#;
        let cmd3 = parse_client_command(line3).expect("must parse");
        match cmd3 {
            ClientCommand::Warp(c) => {
                assert_eq!(c.target, dawn_core::WarpTarget::Body(dawn_core::CelestialBodyId(1)));
            }
            other => panic!("expected Warp, got {other:?}"),
        }
    }

    #[test]
    fn unknown_command_type_returns_none() {
        let line = r#"{"type":"UnknownCommand","ship_id":1}"#;
        assert!(parse_client_command(line).is_none());
    }
}
