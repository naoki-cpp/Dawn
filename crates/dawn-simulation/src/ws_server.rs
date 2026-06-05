//! # WebSocket Server — Godot クライアントへのイベント配信
//!
//! ## 設計 (ADR-0005)
//!
//! `ClientConnection` trait の WebSocket 実装。
//! Godot 側の `connection.gd` と対になる。
//!
//! ## プロトコル
//!
//! ```text
//! サーバー → クライアント : DomainEvent を JSON（改行区切り）で送信
//! クライアント → サーバー : MoveCommand を JSON（改行区切り）で受信
//!
//! JSON フォーマット例:
//!   {"type":"ShipSpawned","ship_id":1,"position":{"x":0.0,"y":0.0,"z":0.0},...}
//!   {"type":"ShipMoved","ship_id":1,"from":{...},"to":{...},"tick":5}
//!   {"type":"ShipDespawned","ship_id":1,"tick":5}
//! ```
//!
//! ## Phase 4 の制限
//!
//! - クライアントは1接続のみ想定（Phase 4 ではマルチクライアント不要）
//! - MoveCommand の受信は今後の Cycle 2 以降で使用
//! - Phase 5 で GrpcConnection に差し替える（Godot 側は変更しない）

use dawn_actor::{ClientCommand, ClientConnection};
use dawn_core::{EntityId, LockOnCommand, MoveCommand, Position, ShipId};
use dawn_core::DomainEvent;
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use std::net::SocketAddr;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::mpsc,
};
use tokio_tungstenite::{accept_async, tungstenite::Message};

// ── JSON 表現 ─────────────────────────────────────────────────────────────────

/// DomainEvent を Godot が解釈できる JSON 形式にシリアライズする。
#[derive(Serialize)]
#[serde(tag = "type")]
enum EventJson {
    ShipSpawned {
        ship_id : u64,
        position: PosJson,
        tick    : u64,
    },
    ShipMoved {
        ship_id: u64,
        from   : PosJson,
        to     : PosJson,
        tick   : u64,
    },
    ShipDespawned {
        ship_id: u64,
        tick   : u64,
    },
    DamageTaken {
        ship_id   : u64,
        amount    : f32,
        current_hp: f32,
        tick      : u64,
    },
    ShipDestroyed {
        ship_id  : u64,
        killer_id: u64,
        tick     : u64,
    },
    TargetLocked {
        locker_id : u64,
        target_id : u64,
        tick      : u64,
    },
    LockLost {
        locker_id : u64,
        target_id : u64,
        tick      : u64,
    },
}

#[derive(Serialize, Clone, Copy)]
struct PosJson {
    x: f32,
    y: f32,
    z: f32,
}

/// JSON テキストから `ClientCommand` をパースする。
///
/// 対応フォーマット:
///   `{"type":"MoveCommand","ship_id":1,"target":{"x":1.0,"y":0.0,"z":2.0}}`
///   `{"type":"LockOnCommand","ship_id":1,"target_id":5}`
fn parse_client_command(line: &str) -> Option<ClientCommand> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    match v.get("type")?.as_str()? {
        "MoveCommand" => {
            let ship_id_raw: u64 = v.get("ship_id")?.as_u64()?;
            let target = v.get("target")?;
            let x = target.get("x")?.as_f64()? as f32;
            let y = target.get("y")?.as_f64()? as f32;
            let z = target.get("z")?.as_f64()? as f32;
            Some(ClientCommand::Move(MoveCommand {
                ship_id         : ShipId(EntityId::from_raw(ship_id_raw)),
                target_position : Position { x, y, z },
            }))
        }
        "LockOnCommand" => {
            let ship_id_raw    = v.get("ship_id")?.as_u64()?;
            let target_id_raw  = v.get("target_id")?.as_u64()?;
            Some(ClientCommand::LockOn(LockOnCommand {
                ship_id   : ShipId(EntityId::from_raw(ship_id_raw)),
                target_id : ShipId(EntityId::from_raw(target_id_raw)),
            }))
        }
        _ => None,
    }
}

fn domain_event_to_json(event: &DomainEvent) -> Option<String> {
    let j = match event {
        DomainEvent::ShipSpawned(e) => EventJson::ShipSpawned {
            ship_id : e.ship_id.raw(),
            position: PosJson { x: e.initial_position.x, y: e.initial_position.y, z: e.initial_position.z },
            tick    : e.tick.value(),
        },
        DomainEvent::ShipMoved(e) => EventJson::ShipMoved {
            ship_id: e.ship_id.raw(),
            from   : PosJson { x: e.from.x, y: e.from.y, z: e.from.z },
            to     : PosJson { x: e.to.x,   y: e.to.y,   z: e.to.z   },
            tick   : e.tick.value(),
        },
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
        // Fitting / WeaponFired はクライアント側の状態管理に使わないためスキップ
        DomainEvent::ShipFitted(_)  => return None,
        DomainEvent::WeaponFired(_) => return None,
    };
    serde_json::to_string(&j).ok()
}

// ── WsClientConnection ────────────────────────────────────────────────────────

/// `ClientConnection` trait の WebSocket 実装。
///
/// `WsServer::accept()` から取得する。
/// `send_events()` は WebSocket のテキストフレームとして送信する。
pub struct WsClientConnection {
    event_tx  : mpsc::UnboundedSender<String>,
    command_rx: mpsc::UnboundedReceiver<ClientCommand>,
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

// ── WsServer ─────────────────────────────────────────────────────────────────

/// WebSocket サーバー。クライアント（Godot）の接続を待ち受ける。
pub struct WsServer {
    listener: TcpListener,
}

impl WsServer {
    /// 指定アドレスで WebSocket サーバーを起動する。
    /// 例: `WsServer::bind("127.0.0.1:7878").await`
    pub async fn bind(addr: &str) -> anyhow::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        println!("[WsServer] listening on ws://{addr}");
        Ok(Self { listener })
    }

    /// クライアント接続を1件受け付け、`WsClientConnection` を返す。
    ///
    /// Phase 4 では1クライアントのみ想定。
    /// 接続が来るまでブロックする。
    pub async fn accept(&self) -> anyhow::Result<WsClientConnection> {
        loop {
            let (stream, peer_addr) = self.listener.accept().await?;
            match Self::handle_connection(stream, peer_addr).await {
                Ok(conn) => return Ok(conn),
                Err(e)   => eprintln!("[WsServer] handshake failed ({peer_addr}): {e}"),
            }
        }
    }

    async fn handle_connection(
        stream   : TcpStream,
        peer_addr: SocketAddr,
    ) -> anyhow::Result<WsClientConnection> {
        let ws_stream = accept_async(stream).await?;
        println!("[WsServer] client connected: {peer_addr}");

        let (event_tx,   event_rx)   = mpsc::unbounded_channel::<String>();
        let (command_tx, command_rx) = mpsc::unbounded_channel::<ClientCommand>();

        let (mut ws_sink, mut ws_source) = ws_stream.split();

        // イベント送信タスク: event_rx → WebSocket
        tokio::spawn(async move {
            let mut rx = event_rx;
            while let Some(msg) = rx.recv().await {
                if ws_sink.send(Message::Text(msg.into())).await.is_err() {
                    break;
                }
            }
            let _ = ws_sink.close().await;
        });

        // コマンド受信タスク: WebSocket → command_tx
        tokio::spawn(async move {
            while let Some(Ok(msg)) = ws_source.next().await {
                if let Message::Text(text) = msg {
                    for line in text.lines() {
                        if let Some(cmd) = parse_client_command(line) {
                            if command_tx.send(cmd).is_err() {
                                return;
                            }
                        }
                    }
                }
            }
            println!("[WsServer] client {peer_addr} disconnected");
        });

        Ok(WsClientConnection { event_tx, command_rx })
    }
}
