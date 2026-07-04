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
    ActivateModuleCommand, ApproachCommand, ApproachTarget, AttackCommand, ClientCommand,
    DeactivateModuleCommand, DomainEvent, EntityId, LockOnCommand, ModuleId, MoveCommand, PlayerId,
    Position, ShipId, SlotKind, StopCommand,
};
use serde::Serialize;

// ── Output types (server → client) ───────────────────────────────────────────

#[derive(Serialize)]
#[serde(tag = "type")]
enum EventJson {
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

#[derive(Debug, Serialize, Clone, Copy)]
pub struct PosJson {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Debug, Serialize, Clone, Copy)]
struct VelJson {
    dx: f32,
    dy: f32,
    dz: f32,
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

/// Parse a newline-terminated JSON line from the Godot client into a
/// [`ClientCommand`]. Returns `None` for unknown or malformed messages.
pub fn parse_client_command(line: &str) -> Option<ClientCommand> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    match v.get("type")?.as_str()? {
        "MoveCommand" => {
            let ship_id_raw = v.get("ship_id")?.as_u64()?;
            let target = v.get("target")?;
            Some(ClientCommand::Move(MoveCommand {
                ship_id: ShipId(EntityId::from_raw(ship_id_raw)),
                target_position: Position {
                    x: target.get("x")?.as_f64()? as f32,
                    y: target.get("y")?.as_f64()? as f32,
                    z: target.get("z")?.as_f64()? as f32,
                },
            }))
        }
        "LockOnCommand" => {
            let ship_id_raw = v.get("ship_id")?.as_u64()?;
            let target_id_raw = v.get("target_id")?.as_u64()?;
            Some(ClientCommand::LockOn(LockOnCommand {
                ship_id: ShipId(EntityId::from_raw(ship_id_raw)),
                target_id: ShipId(EntityId::from_raw(target_id_raw)),
            }))
        }
        "ActivateModuleCommand" => {
            let ship_id_raw = v.get("ship_id")?.as_u64()?;
            let module_id_raw = v.get("module_id")?.as_u64()? as u32;
            let slot_str = v.get("slot")?.as_str()?;
            // target_ship_id (ADR-0035): optional — required only for
            // targeted module kinds (Weapon/Tackle), validated server-side.
            let target_ship_id = v
                .get("target_ship_id")
                .and_then(|t| t.as_u64())
                .map(|raw| ShipId(EntityId::from_raw(raw)));
            Some(ClientCommand::Activate(ActivateModuleCommand {
                ship_id: ShipId(EntityId::from_raw(ship_id_raw)),
                module_id: ModuleId(module_id_raw),
                slot: parse_slot_kind(slot_str)?,
                target_ship_id,
            }))
        }
        "DeactivateModuleCommand" => {
            let ship_id_raw = v.get("ship_id")?.as_u64()?;
            let module_id_raw = v.get("module_id")?.as_u64()? as u32;
            let slot_str = v.get("slot")?.as_str()?;
            Some(ClientCommand::Deactivate(DeactivateModuleCommand {
                ship_id: ShipId(EntityId::from_raw(ship_id_raw)),
                module_id: ModuleId(module_id_raw),
                slot: parse_slot_kind(slot_str)?,
            }))
        }
        "AttackCommand" => {
            let attacker_id_raw = v.get("attacker_id")?.as_u64()?;
            let target_id_raw = v.get("target_id")?.as_u64()?;
            Some(ClientCommand::Attack(AttackCommand {
                attacker_id: ShipId(EntityId::from_raw(attacker_id_raw)),
                target_id: ShipId(EntityId::from_raw(target_id_raw)),
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
            // Accept {"target":{"Gate":2}} / {"target":{"Body":1}} or legacy {"gate_id":2}.
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
        "OrbitCommand" => {
            let ship_id_raw = v.get("ship_id")?.as_u64()?;
            let target = if let Some(gate) = v.get("gate_id").and_then(|g| g.as_u64()) {
                ApproachTarget::Gate(dawn_core::JumpGateId(gate as u32))
            } else {
                let target_id_raw = v.get("target_id")?.as_u64()?;
                ApproachTarget::Ship(ShipId(EntityId::from_raw(target_id_raw)))
            };
            let radius = v.get("radius").and_then(|r| r.as_f64()).map(|r| r as f32);
            Some(ClientCommand::Orbit(dawn_core::OrbitCommand {
                ship_id: ShipId(EntityId::from_raw(ship_id_raw)),
                target,
                radius,
            }))
        }
        "KeepAtRangeCommand" => {
            let ship_id_raw = v.get("ship_id")?.as_u64()?;
            let target = if let Some(gate) = v.get("gate_id").and_then(|g| g.as_u64()) {
                ApproachTarget::Gate(dawn_core::JumpGateId(gate as u32))
            } else {
                let target_id_raw = v.get("target_id")?.as_u64()?;
                ApproachTarget::Ship(ShipId(EntityId::from_raw(target_id_raw)))
            };
            let range = v.get("range").and_then(|r| r.as_f64()).map(|r| r as f32);
            Some(ClientCommand::KeepAtRange(dawn_core::KeepAtRangeCommand {
                ship_id: ShipId(EntityId::from_raw(ship_id_raw)),
                target,
                range,
            }))
        }
        "FitModuleCommand" => {
            let ship_id_raw = v.get("ship_id")?.as_u64()?;
            let module_id_raw = v.get("module_id")?.as_u64()? as u32;
            let slot_str = v.get("slot")?.as_str()?;
            Some(ClientCommand::Fit(dawn_core::FitModuleCommand {
                ship_id: ShipId(EntityId::from_raw(ship_id_raw)),
                module_id: ModuleId(module_id_raw),
                slot: parse_slot_kind(slot_str)?,
            }))
        }
        "UnfitModuleCommand" => {
            let ship_id_raw = v.get("ship_id")?.as_u64()?;
            let module_id_raw = v.get("module_id")?.as_u64()? as u32;
            let slot_str = v.get("slot")?.as_str()?;
            Some(ClientCommand::Unfit(dawn_core::UnfitModuleCommand {
                ship_id: ShipId(EntityId::from_raw(ship_id_raw)),
                module_id: ModuleId(module_id_raw),
                slot: parse_slot_kind(slot_str)?,
            }))
        }
        _ => None,
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
                assert_eq!(
                    c.target,
                    dawn_core::WarpTarget::Body(dawn_core::CelestialBodyId(1))
                );
            }
            other => panic!("expected Warp, got {other:?}"),
        }
    }

    #[test]
    fn orbit_command_json_with_target_id_is_parsed_into_client_command_orbit() {
        let line = r#"{"type":"OrbitCommand","ship_id":1,"target_id":2,"radius":3000.0}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            ClientCommand::Orbit(c) => {
                assert_eq!(c.ship_id, ship_id(1));
                assert_eq!(c.target, ApproachTarget::Ship(ship_id(2)));
                assert_eq!(c.radius, Some(3000.0));
            }
            other => panic!("expected Orbit, got {other:?}"),
        }
    }

    #[test]
    fn orbit_command_json_with_gate_id_and_no_radius_is_parsed() {
        let line = r#"{"type":"OrbitCommand","ship_id":1,"gate_id":4}"#;
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
        let line = r#"{"type":"KeepAtRangeCommand","ship_id":1,"target_id":2,"range":5000.0}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            ClientCommand::KeepAtRange(c) => {
                assert_eq!(c.ship_id, ship_id(1));
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
}
