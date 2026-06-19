//! # WebSocket Server — Phase 5 multi-client support
//!
//! ## Design (ADR-0005, ADR-0007)
//!
//! Phase 5 changes:
//!   - Hello/Welcome handshake assigns and announces a PlayerId
//!   - InitialState sends the visible Ship state on connect
//!   - PlayerSession maps a connection to its PlayerId
//!   - Multiple clients can connect concurrently
//!   - Ownership check: a player may only command its own ship
//!
//! ## Protocol
//!
//! ```text
//! Client → Server:  {"type":"Hello"}
//! Server → Client:  {"type":"Welcome","player_id":N,"ship_id":N}
//! Server → Client:  {"type":"InitialState","ships":[...]}
//! Server → Client:  DomainEvent JSON (newline-delimited stream)
//! Client → Server:  ClientCommand JSON (MoveCommand / LockOnCommand)
//! ```

use dawn_actor::{ClientCommand, ClientConnection};
use dawn_core::{ActivateModuleCommand, ApproachCommand, ApproachTarget, AttackCommand, DeactivateModuleCommand, EntityId, LockOnCommand, ModuleId, MoveCommand, PlayerId, Position, ShipId, SlotKind, StopCommand};
use dawn_core::DomainEvent;
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use std::net::SocketAddr;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::mpsc,
    time::{timeout, Duration},
};
use tokio_tungstenite::{accept_async, tungstenite::Message};

// ── JSON representation (server → client) ─────────────────────────────────────

#[derive(Serialize)]
#[serde(tag = "type")]
enum EventJson {
    ShipSpawned      { ship_id: u64, position: PosJson, tick: u64 },
    VelocityChanged  { ship_id: u64, velocity: VelJson, tick: u64 },
    ShipDespawned    { ship_id: u64, tick: u64 },
    DamageTaken   { ship_id: u64, damage: f32, current_shield: f32, current_armor: f32, current_hull: f32, tick: u64 },
    ShipDestroyed { ship_id: u64, killer_id: u64, tick: u64 },
    TargetLocked      { locker_id: u64, target_id: u64, tick: u64 },
    LockLost          { locker_id: u64, target_id: u64, tick: u64 },
    ModuleActivated   { ship_id: u64, module_id: u32, slot: String, tick: u64 },
    ModuleDeactivated { ship_id: u64, module_id: u32, slot: String, tick: u64 },
    JumpGateUsed      { ship_id: u64, gate_id: u32, from_sector: u8, to_sector: u8, entry_pos: PosJson, tick: u64 },
    StarSystemChanged { ship_id: u64, from_system: u32, to_system: u32, tick: u64 },
}

#[derive(Serialize, Clone, Copy)]
struct PosJson { x: f32, y: f32, z: f32 }

#[derive(Serialize, Clone, Copy)]
struct VelJson { dx: f32, dy: f32, dz: f32 }

impl From<Position> for PosJson {
    fn from(p: Position) -> Self { Self { x: p.x, y: p.y, z: p.z } }
}

impl From<dawn_core::Velocity> for VelJson {
    fn from(v: dawn_core::Velocity) -> Self { Self { dx: v.dx, dy: v.dy, dz: v.dz } }
}

fn domain_event_to_json(event: &DomainEvent) -> Option<String> {
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
            ship_id   : e.ship_id.raw(),
            damage         : e.damage,
            current_shield : e.current_shield,
            current_armor  : e.current_armor,
            current_hull   : e.current_hull,
            tick      : e.tick.value(),
        },
        DomainEvent::ShipDestroyed(e) => EventJson::ShipDestroyed {
            ship_id  : e.ship_id.raw(),
            killer_id: e.killer_id.raw(),
            tick     : e.tick.value(),
        },
        DomainEvent::TargetLocked(e) => EventJson::TargetLocked {
            locker_id : e.locker_id.raw(),
            target_id : e.target_id.raw(),
            tick      : e.tick.value(),
        },
        DomainEvent::LockLost(e) => EventJson::LockLost {
            locker_id : e.locker_id.raw(),
            target_id : e.target_id.raw(),
            tick      : e.tick.value(),
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
        // The following are not used by client-side state management.
        DomainEvent::ShipFitted(_)       => return None,
        DomainEvent::WeaponFired(_)      => return None,
        DomainEvent::TackleApplied(_)    => return None,
        DomainEvent::TackleReleased(_)   => return None,
        // Sector Transit is an internal node-ownership event; not sent to clients (ADR-0014).
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

// ── Command parser (client → server) ──────────────────────────────────────────

fn parse_client_command(line: &str) -> Option<ClientCommand> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    match v.get("type")?.as_str()? {
        "MoveCommand" => {
            let ship_id_raw = v.get("ship_id")?.as_u64()?;
            let target      = v.get("target")?;
            Some(ClientCommand::Move(MoveCommand {
                ship_id         : ShipId(EntityId::from_raw(ship_id_raw)),
                target_position : Position {
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
                ship_id   : ShipId(EntityId::from_raw(ship_id_raw)),
                target_id : ShipId(EntityId::from_raw(target_id_raw)),
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
                // Legacy wire format: {"gate_id": N}
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

// ── WsClientConnection ────────────────────────────────────────────────────────

pub struct WsClientConnection {
    event_tx  : mpsc::UnboundedSender<String>,
    command_rx: mpsc::UnboundedReceiver<ClientCommand>,
}

impl WsClientConnection {
    /// Send a raw string directly (Welcome / InitialState etc.).
    pub fn send_raw(&self, msg: &str) -> bool {
        self.event_tx.send(msg.to_string() + "\n").is_ok()
    }
}

impl ClientConnection for WsClientConnection {
    fn send_events(&self, events: &[DomainEvent]) -> Result<(), dawn_actor::ConnectionError> {
        for event in events {
            if let Some(json) = domain_event_to_json(event) {
                self.event_tx
                    .send(json + "\n")
                    .map_err(|_| dawn_actor::ConnectionError::Disconnected)?;
            }
        }
        Ok(())
    }

    fn try_recv_command(&mut self) -> Option<ClientCommand> {
        self.command_rx.try_recv().ok()
    }
}

// ── PlayerSession ─────────────────────────────────────────────────────────────

/// One player connection: holds its PlayerId, ShipId, and connection.
pub struct PlayerSession {
    pub player_id : PlayerId,
    pub ship_id   : ShipId,
    pub conn      : WsClientConnection,
}

impl PlayerSession {
    /// Send events to this client. Returns false on send failure (disconnect).
    pub fn send_events(&self, events: &[DomainEvent]) -> bool {
        self.conn.send_events(events).is_ok()
    }

    /// Pull one pending command, if any.
    pub fn try_recv_command(&mut self) -> Option<ClientCommand> {
        self.conn.try_recv_command()
    }
}

// ── WsServer ─────────────────────────────────────────────────────────────────

pub struct WsServer {
    listener: TcpListener,
}

impl WsServer {
    pub async fn bind(addr: &str) -> anyhow::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        println!("[WsServer] listening on ws://{addr}");
        Ok(Self { listener })
    }

    /// Try to accept a new connection without blocking; returns `None` at once
    /// if none is pending.
    pub async fn try_accept_raw(&self) -> Option<(TcpStream, SocketAddr)> {
        timeout(Duration::from_millis(0), self.listener.accept())
            .await
            .ok()
            .and_then(|r| r.ok())
    }

    /// Run the Hello/Welcome handshake and return a `PlayerSession`.
    ///
    /// # Flow
    /// 1. WebSocket upgrade
    /// 2. Wait for the Hello message (3s timeout)
    /// 3. Send Welcome + InitialState
    /// 4. Return the `PlayerSession`
    pub async fn handshake(
        stream        : TcpStream,
        peer_addr     : SocketAddr,
        player_id     : PlayerId,
        ship_id       : ShipId,
        initial_state : &str,
        player_fitting: Option<String>,
    ) -> anyhow::Result<PlayerSession> {
        let ws_stream = accept_async(stream).await?;
        println!("[WsServer] client connected: {peer_addr}");

        let (event_tx,   event_rx)   = mpsc::unbounded_channel::<String>();
        let (command_tx, command_rx) = mpsc::unbounded_channel::<ClientCommand>();

        let (mut ws_sink, mut ws_source) = ws_stream.split();

        // Wait for Hello (3s timeout).
        let hello_result = timeout(Duration::from_secs(3), async {
            while let Some(Ok(msg)) = ws_source.next().await {
                if let Message::Text(text) = msg {
                    for line in text.lines() {
                        let v: serde_json::Value = serde_json::from_str(line).unwrap_or_default();
                        if v.get("type").and_then(|t| t.as_str()) == Some("Hello") {
                            return true;
                        }
                    }
                }
            }
            false
        }).await;

        match hello_result {
            Ok(true) => {}
            _ => anyhow::bail!("Hello timeout or not received from {peer_addr}"),
        }

        // Send Welcome.
        let welcome = format!(
            "{{\"type\":\"Welcome\",\"player_id\":{},\"ship_id\":{}}}\n",
            player_id.raw(), ship_id.raw()
        );
        ws_sink.send(Message::Text(welcome.into())).await?;

        // Send InitialState.
        ws_sink.send(Message::Text((initial_state.to_string() + "\n").into())).await?;

        // Send PlayerFitting (the player's own loadout).
        if let Some(fitting) = player_fitting {
            ws_sink.send(Message::Text((fitting + "\n").into())).await?;
        }

        // Event-send task.
        tokio::spawn(async move {
            let mut rx = event_rx;
            while let Some(msg) = rx.recv().await {
                if ws_sink.send(Message::Text(msg.into())).await.is_err() {
                    break;
                }
            }
            let _ = ws_sink.close().await;
        });

        // Command-receive task.
        tokio::spawn(async move {
            while let Some(Ok(msg)) = ws_source.next().await {
                if let Message::Text(text) = msg {
                    for line in text.lines() {
                        if let Some(cmd) = parse_client_command(line) {
                            if command_tx.send(cmd).is_err() { return; }
                        }
                    }
                }
            }
            println!("[WsServer] {peer_addr} disconnected");
        });

        let conn = WsClientConnection { event_tx, command_rx };
        println!("[WsServer] {peer_addr} handshake complete: {player_id} ship={}", ship_id.raw());
        Ok(PlayerSession { player_id, ship_id, conn })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{JumpGateId, NodeId, SectorId, StarSystemId, Tick};

    #[test]
    fn jump_command_json_is_parsed_into_client_command_jump() {
        let line = r#"{"type":"JumpCommand","ship_id":42,"gate_id":2}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            ClientCommand::Jump(c) => {
                assert_eq!(c.ship_id.raw(), 42);
                assert_eq!(c.gate_id, JumpGateId(2));
            }
            other => panic!("expected Jump, got {other:?}"),
        }
    }

    #[test]
    fn jump_command_without_gate_id_is_rejected() {
        let line = r#"{"type":"JumpCommand","ship_id":42}"#;
        assert!(parse_client_command(line).is_none());
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
    fn warp_command_without_gate_id_is_rejected() {
        let line = r#"{"type":"WarpCommand","ship_id":42}"#;
        assert!(parse_client_command(line).is_none());
    }

    #[test]
    fn approach_command_with_target_id_is_parsed_as_a_ship_target() {
        let line = r#"{"type":"ApproachCommand","ship_id":7,"target_id":13}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            ClientCommand::Approach(c) => {
                assert_eq!(c.ship_id.raw(), 7);
                assert_eq!(c.target, ApproachTarget::Ship(ShipId(EntityId::from_raw(13))));
            }
            other => panic!("expected Approach, got {other:?}"),
        }
    }

    #[test]
    fn approach_command_with_gate_id_is_parsed_as_a_gate_target() {
        let line = r#"{"type":"ApproachCommand","ship_id":7,"gate_id":2}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            ClientCommand::Approach(c) => {
                assert_eq!(c.ship_id.raw(), 7);
                assert_eq!(c.target, ApproachTarget::Gate(JumpGateId(2)));
            }
            other => panic!("expected Approach, got {other:?}"),
        }
    }

    #[test]
    fn approach_command_without_a_target_is_rejected() {
        let line = r#"{"type":"ApproachCommand","ship_id":7}"#;
        assert!(parse_client_command(line).is_none());
    }

    #[test]
    fn jump_gate_used_event_is_serialized_with_entry_pos_for_godot() {
        let event = DomainEvent::JumpGateUsed(dawn_core::events::JumpGateUsed {
            ship_id    : ShipId(EntityId::new(NodeId(0), 1)),
            gate_id    : JumpGateId(0),
            from_sector: SectorId(0),
            to_sector  : SectorId(1),
            entry_pos  : Position::new(1.0, 2.0, 3.0),
            tick       : Tick(5),
        });
        let json = domain_event_to_json(&event).expect("must serialize");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "JumpGateUsed");
        assert_eq!(v["to_sector"], 1);
        assert_eq!(v["entry_pos"]["x"].as_f64().unwrap() as f32, 1.0);
    }

    #[test]
    fn star_system_changed_event_is_serialized_with_from_and_to_systems() {
        let event = DomainEvent::StarSystemChanged(dawn_core::events::StarSystemChanged {
            ship_id    : ShipId(EntityId::new(NodeId(0), 1)),
            from_system: StarSystemId(0),
            to_system  : StarSystemId(1),
            tick       : Tick(5),
        });
        let json = domain_event_to_json(&event).expect("must serialize");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "StarSystemChanged");
        assert_eq!(v["from_system"], 0);
        assert_eq!(v["to_system"], 1);
    }
}
