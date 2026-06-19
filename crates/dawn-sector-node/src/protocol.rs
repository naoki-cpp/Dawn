//! Wire protocol: DomainEvent → JSON (server→client) and JSON → ClientCommand (client→server).
//! Adapted from dawn-simulation/src/protocol.rs — kept in sync manually.

use dawn_actor::ClientCommand;
use dawn_core::{
    ActivateModuleCommand, ApproachCommand, ApproachTarget, AttackCommand,
    DeactivateModuleCommand, DomainEvent, EntityId, LockOnCommand, ModuleId,
    MoveCommand, Position, ShipId, SlotKind, StopCommand,
};
use serde::Serialize;

// ── Server → Client ───────────────────────────────────────────────────────────

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
    // Sent when the player's ship jumps to a sector on a different physical node.
    Redirect         { ws_addr: String },
}

#[derive(Serialize, Clone, Copy)]
pub struct PosJson { pub x: f32, pub y: f32, pub z: f32 }

#[derive(Serialize, Clone, Copy)]
struct VelJson { dx: f32, dy: f32, dz: f32 }

impl From<Position> for PosJson {
    fn from(p: Position) -> Self { Self { x: p.x, y: p.y, z: p.z } }
}
impl From<dawn_core::Velocity> for VelJson {
    fn from(v: dawn_core::Velocity) -> Self { Self { dx: v.dx, dy: v.dy, dz: v.dz } }
}

/// Serialize a [`DomainEvent`] to the JSON line the Godot client expects.
/// Returns `None` for internal events that are not forwarded to clients.
pub fn domain_event_to_json(event: &DomainEvent) -> Option<String> {
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
        // Internal transit / combat events — not forwarded.
        DomainEvent::ShipFitted(_)             => return None,
        DomainEvent::WeaponFired(_)            => return None,
        DomainEvent::TackleApplied(_)          => return None,
        DomainEvent::TackleReleased(_)         => return None,
        DomainEvent::SectorTransitRequested(_) => return None,
        DomainEvent::SectorTransitCompleted(_) => return None,
        DomainEvent::SectorTransitAborted(_)   => return None,
    };
    serde_json::to_string(&j).ok()
}

/// Build a `{"type":"Redirect","ws_addr":"..."}` JSON line to send to a client
/// whose ship just jumped to a different physical node.
pub fn redirect_json(ws_addr: std::net::SocketAddr) -> String {
    let j = EventJson::Redirect { ws_addr: ws_addr.to_string() };
    serde_json::to_string(&j).unwrap_or_default()
}

// ── Client → Server ───────────────────────────────────────────────────────────

/// Parse a JSON line from the Godot client into a [`ClientCommand`].
pub fn parse_client_command(line: &str) -> Option<ClientCommand> {
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
