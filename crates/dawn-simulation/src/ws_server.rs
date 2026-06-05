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
use dawn_core::{ActivateModuleCommand, DeactivateModuleCommand, EntityId, LockOnCommand, ModuleId, MoveCommand, PlayerId, Position, ShipId, SlotKind};
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
    DamageTaken   { ship_id: u64, amount: f32, current_hp: f32, tick: u64 },
    ShipDestroyed { ship_id: u64, killer_id: u64, tick: u64 },
    TargetLocked  { locker_id: u64, target_id: u64, tick: u64 },
    LockLost      { locker_id: u64, target_id: u64, tick: u64 },
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
        // ShipMoved は deprecated（ADR-0008）。既存ログの Replay 用に受け取るが Godot には送らない。
        #[allow(deprecated)]
        DomainEvent::ShipMoved(_) => return None,
        DomainEvent::ShipDespawned(e) => EventJson::ShipDespawned {
            ship_id: e.ship_id.raw(),
            tick   : e.tick.value(),
        },
        DomainEvent::DamageTaken(e) => EventJson::DamageTaken {
            ship_id   : e.ship_id.raw(),
            amount    : e.amount,
            current_hp: e.current_hp,
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
        // 以下はクライアント側の状態管理に使わないためスキップ
        DomainEvent::ShipFitted(_)         => return None,
        DomainEvent::WeaponFired(_)        => return None,
        DomainEvent::ModuleActivated(_)    => return None,
        DomainEvent::ModuleDeactivated(_)  => return None,
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
        stream       : TcpStream,
        peer_addr    : SocketAddr,
        player_id    : PlayerId,
        ship_id      : ShipId,
        initial_state: &str,
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
