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

use crate::protocol::{domain_event_to_json, parse_client_command};
use dawn_actor::{ClientCommand, ClientConnection};
use dawn_core::{DomainEvent, PlayerId, ShipId};
use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::mpsc,
    time::{timeout, Duration},
};
use tokio_tungstenite::{accept_async, tungstenite::Message};

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

// Protocol-level tests (command parsing, event serialization) live in protocol.rs.
