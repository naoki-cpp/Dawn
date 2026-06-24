//! # WebSocket Server — production client transport (ADR-0005, ADR-0007)
//!
//! The single WebSocket server / session implementation shared by both
//! binaries (`dawn-simulation`, `dawn-sector-node`). It owns:
//!   - Hello/Welcome handshake (assigns and announces a PlayerId)
//!   - InitialState + PlayerFitting on connect
//!   - `PlayerSession` mapping a connection to its PlayerId / ShipId
//!   - `WsClientConnection`, the [`ClientConnection`] impl over a socket
//!
//! ## Protocol
//!
//! ```text
//! Client → Server:  {"type":"Hello"}
//! Server → Client:  {"type":"Welcome","player_id":N,"ship_id":N}
//! Server → Client:  {"type":"InitialState","ships":[...]}
//! Server → Client:  DomainEvent JSON (newline-delimited stream)
//! Client → Server:  ClientCommand JSON
//! ```

use crate::protocol::{domain_event_to_json, parse_client_command};
use crate::{ClientCommand, ClientConnection};
use dawn_core::{DomainEvent, PlayerId, ShipId};
use futures_util::{SinkExt, StreamExt};
use std::fmt::Display;
use std::net::SocketAddr;
use tokio::{
    net::{TcpListener, TcpStream, ToSocketAddrs},
    sync::mpsc,
    time::{timeout, Duration},
};
use tokio_tungstenite::{accept_async, tungstenite::Message};

// ── WsClientConnection ────────────────────────────────────────────────────────

pub struct WsClientConnection {
    event_tx: mpsc::UnboundedSender<String>,
    command_rx: mpsc::UnboundedReceiver<ClientCommand>,
}

impl WsClientConnection {
    /// Send a raw JSON string directly (Welcome, InitialState, Redirect, etc.).
    pub fn send_raw(&self, msg: &str) -> bool {
        self.event_tx.send(msg.to_string() + "\n").is_ok()
    }
}

impl ClientConnection for WsClientConnection {
    fn send_events(&self, events: &[DomainEvent]) -> Result<(), crate::ConnectionError> {
        for event in events {
            if let Some(json) = domain_event_to_json(event) {
                self.event_tx
                    .send(json + "\n")
                    .map_err(|_| crate::ConnectionError::Disconnected)?;
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
    pub player_id: PlayerId,
    pub ship_id: ShipId,
    pub conn: WsClientConnection,
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

    /// Send a raw JSON string directly (e.g. a refreshed PlayerFitting after
    /// Fit/Unfit, ADR-0032 -- mirrors the one sent once at connect).
    pub fn send_raw(&self, msg: &str) -> bool {
        self.conn.send_raw(msg)
    }
}

// ── WsServer ─────────────────────────────────────────────────────────────────

pub struct WsServer {
    listener: TcpListener,
}

impl WsServer {
    /// Bind the listener. `addr` accepts either a `&str` ("127.0.0.1:7878") or a
    /// `SocketAddr` — both binaries pass one or the other.
    pub async fn bind<A: ToSocketAddrs + Display>(addr: A) -> anyhow::Result<Self> {
        let listener = TcpListener::bind(&addr).await?;
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
    /// 3. Send Welcome + InitialState (+ PlayerFitting)
    /// 4. Return the `PlayerSession`
    pub async fn handshake(
        stream: TcpStream,
        peer_addr: SocketAddr,
        player_id: PlayerId,
        ship_id: ShipId,
        initial_state: &str,
        player_fitting: Option<String>,
    ) -> anyhow::Result<PlayerSession> {
        let ws_stream = accept_async(stream).await?;

        let (event_tx, event_rx) = mpsc::unbounded_channel::<String>();
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
        })
        .await;

        match hello_result {
            Ok(true) => {}
            _ => anyhow::bail!("Hello timeout or not received from {peer_addr}"),
        }

        // Send Welcome + InitialState + (optional) PlayerFitting.
        let welcome = format!(
            "{{\"type\":\"Welcome\",\"player_id\":{},\"ship_id\":{}}}\n",
            player_id.raw(),
            ship_id.raw()
        );
        ws_sink.send(Message::Text(welcome)).await?;
        ws_sink
            .send(Message::Text(initial_state.to_string() + "\n"))
            .await?;
        if let Some(fitting) = player_fitting {
            ws_sink.send(Message::Text(fitting + "\n")).await?;
        }

        // Event-send task.
        tokio::spawn(async move {
            let mut rx = event_rx;
            while let Some(msg) = rx.recv().await {
                if ws_sink.send(Message::Text(msg)).await.is_err() {
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
                            if command_tx.send(cmd).is_err() {
                                return;
                            }
                        }
                    }
                }
            }
            println!("[WsServer] {peer_addr} disconnected");
        });

        let conn = WsClientConnection {
            event_tx,
            command_rx,
        };
        println!(
            "[WsServer] {peer_addr} handshake complete: {player_id} ship={}",
            ship_id.raw()
        );
        Ok(PlayerSession {
            player_id,
            ship_id,
            conn,
        })
    }
}
