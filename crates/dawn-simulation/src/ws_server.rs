//! # WebSocket Server — Phase 5 マルチクライアント対応
//!
//! ## 設計 (ADR-0005, ADR-0007)
//!
//! Phase 5 の変更点:
//!   - Hello/Welcome ハンドシェイクで PlayerId を採番・通知
//!   - InitialState で接続時の全 Ship 状態を送信
//!   - PlayerSession で接続 ↔ PlayerId を管理
//!   - 複数クライアントの同時接続に対応
//!   - 所有権チェック: 自分の船だけ操作できる
//!
//! ## プロトコル
//!
//! ```text
//! Client → Server:  {"type":"Hello"}
//! Server → Client:  {"type":"Welcome","player_id":N,"ship_id":N}
//! Server → Client:  {"type":"InitialState","ships":[...]}
//! Server → Client:  DomainEvent JSON（改行区切りストリーム）
//! Client → Server:  ClientCommand JSON（MoveCommand / LockOnCommand）
//! ```

use dawn_actor::{ClientCommand, ClientConnection};
use dawn_core::{ActivateModuleCommand, AttackCommand, DeactivateModuleCommand, EntityId, LockOnCommand, ModuleId, MoveCommand, PlayerId, Position, ShipId, SlotKind, StopCommand};
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

// ── JSON 表現（サーバー → クライアント）───────────────────────────────────────

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

fn domain_event_to_json(event: &DomainEvent) -> Option<String> {
    let j = match event {
        DomainEvent::ShipSpawned(e) => EventJson::ShipSpawned {
            ship_id : e.ship_id.raw(),
            position: PosJson { x: e.initial_position.x, y: e.initial_position.y, z: e.initial_position.z },
            tick    : e.tick.value(),
        },
        DomainEvent::VelocityChanged(e) => EventJson::VelocityChanged {
            ship_id : e.ship_id.raw(),
            velocity: VelJson { dx: e.velocity.dx, dy: e.velocity.dy, dz: e.velocity.dz },
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
        // 以下はクライアント側の状態管理に使わない
        DomainEvent::ShipFitted(_)  => return None,
        DomainEvent::WeaponFired(_) => return None,
        // Sector Transit はノード内部の所有権イベント。クライアントへは配信しない（ADR-0014）。
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
            entry_pos  : PosJson { x: e.entry_pos.x, y: e.entry_pos.y, z: e.entry_pos.z },
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

// ── コマンドパーサー（クライアント → サーバー）─────────────────────────────────

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
    /// 生文字列（Welcome / InitialState など）を直接送信する。
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

/// 1 プレイヤー接続を表す。PlayerId・ShipId・コネクションを保持する。
pub struct PlayerSession {
    pub player_id : PlayerId,
    pub ship_id   : ShipId,
    pub conn      : WsClientConnection,
}

impl PlayerSession {
    /// イベントをこのクライアントに送信する。
    /// 送信失敗（切断）の場合は false を返す。
    pub fn send_events(&self, events: &[DomainEvent]) -> bool {
        self.conn.send_events(events).is_ok()
    }

    /// コマンドを1件取り出す。
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

    /// ブロッキングで1クライアントを受け付け `WsClientConnection` を返す。
    ///
    /// Phase 4 後方互換用（Phase 5 では `try_accept_raw` を使う）。
    pub async fn accept(&self) -> anyhow::Result<WsClientConnection> {
        loop {
            let (stream, peer_addr) = self.listener.accept().await?;
            match Self::make_connection(stream, peer_addr).await {
                Ok(conn) => return Ok(conn),
                Err(e)   => eprintln!("[WsServer] handshake failed ({peer_addr}): {e}"),
            }
        }
    }

    /// ノンブロッキングで新しい接続を試みる。
    /// 接続がなければ `None` を即座に返す。
    pub async fn try_accept_raw(&self) -> Option<(TcpStream, SocketAddr)> {
        timeout(Duration::from_millis(0), self.listener.accept())
            .await
            .ok()
            .and_then(|r| r.ok())
    }

    /// Hello/Welcome ハンドシェイクを実行し `PlayerSession` を返す。
    ///
    /// # 流れ
    /// 1. WebSocket アップグレード
    /// 2. Hello メッセージ待ち（1秒でタイムアウト）
    /// 3. Welcome + InitialState を送信
    /// 4. `PlayerSession` を返す
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

        // Hello を待つ（タイムアウト 3 秒）
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

        // Welcome を送信
        let welcome = format!(
            "{{\"type\":\"Welcome\",\"player_id\":{},\"ship_id\":{}}}\n",
            player_id.raw(), ship_id.raw()
        );
        ws_sink.send(Message::Text(welcome.into())).await?;

        // InitialState を送信
        ws_sink.send(Message::Text((initial_state.to_string() + "\n").into())).await?;

        // PlayerFitting を送信（プレイヤー自身の装備情報）
        if let Some(fitting) = player_fitting {
            ws_sink.send(Message::Text((fitting + "\n").into())).await?;
        }

        // イベント送信タスク
        tokio::spawn(async move {
            let mut rx = event_rx;
            while let Some(msg) = rx.recv().await {
                if ws_sink.send(Message::Text(msg.into())).await.is_err() {
                    break;
                }
            }
            let _ = ws_sink.close().await;
        });

        // コマンド受信タスク
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

    async fn make_connection(
        stream   : TcpStream,
        peer_addr: SocketAddr,
    ) -> anyhow::Result<WsClientConnection> {
        let ws_stream = accept_async(stream).await?;
        println!("[WsServer] client connected: {peer_addr}");

        let (event_tx,   event_rx)   = mpsc::unbounded_channel::<String>();
        let (command_tx, command_rx) = mpsc::unbounded_channel::<ClientCommand>();

        let (mut ws_sink, mut ws_source) = ws_stream.split();

        tokio::spawn(async move {
            let mut rx = event_rx;
            while let Some(msg) = rx.recv().await {
                if ws_sink.send(Message::Text(msg.into())).await.is_err() { break; }
            }
            let _ = ws_sink.close().await;
        });

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
            println!("[WsServer] client {peer_addr} disconnected");
        });

        Ok(WsClientConnection { event_tx, command_rx })
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
