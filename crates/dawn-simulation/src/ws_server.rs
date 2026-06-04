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

use dawn_actor::ClientConnection;
use dawn_core::{DomainEvent, MoveCommand};
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
}

#[derive(Serialize, Clone, Copy)]
struct PosJson {
    x: f32,
    y: f32,
    z: f32,
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
    command_rx: mpsc::UnboundedReceiver<MoveCommand>,
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

    fn try_recv_command(&mut self) -> Option<MoveCommand> {
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
        let (command_tx, command_rx) = mpsc::unbounded_channel::<MoveCommand>();

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
                    // 改行区切りで複数コマンドが来る可能性を考慮
                    for line in text.lines() {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                            if v.get("type").and_then(|t| t.as_str()) == Some("MoveCommand") {
                                // MoveCommand のパース（Cycle 2 以降で本格利用）
                                let _ = command_tx; // 将来使用
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
