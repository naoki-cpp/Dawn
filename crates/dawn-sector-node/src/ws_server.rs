//! WebSocket server for Godot clients.
//! Adapted from dawn-simulation/src/ws_server.rs.

use crate::protocol::parse_client_command;
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
    /// Send a raw JSON string directly (Welcome, InitialState, Redirect, etc.).
    pub fn send_raw(&self, msg: &str) -> bool {
        self.event_tx.send(msg.to_string() + "\n").is_ok()
    }
}

impl ClientConnection for WsClientConnection {
    fn send_events(&self, events: &[DomainEvent]) -> Result<(), dawn_actor::ConnectionError> {
        for event in events {
            if let Some(json) = crate::protocol::domain_event_to_json(event) {
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

pub struct PlayerSession {
    pub player_id: PlayerId,
    pub ship_id  : ShipId,
    pub conn     : WsClientConnection,
}

impl PlayerSession {
    pub fn send_events(&self, events: &[DomainEvent]) -> bool {
        self.conn.send_events(events).is_ok()
    }

    pub fn try_recv_command(&mut self) -> Option<ClientCommand> {
        self.conn.try_recv_command()
    }
}

// ── WsServer ─────────────────────────────────────────────────────────────────

pub struct WsServer {
    listener: TcpListener,
}

impl WsServer {
    pub async fn bind(addr: SocketAddr) -> anyhow::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        println!("[WsServer] listening on ws://{addr}");
        Ok(Self { listener })
    }

    pub async fn try_accept_raw(&self) -> Option<(TcpStream, SocketAddr)> {
        timeout(Duration::from_millis(0), self.listener.accept())
            .await
            .ok()
            .and_then(|r| r.ok())
    }

    pub async fn handshake(
        stream        : TcpStream,
        peer_addr     : SocketAddr,
        player_id     : PlayerId,
        ship_id       : ShipId,
        initial_state : &str,
        player_fitting: Option<String>,
    ) -> anyhow::Result<PlayerSession> {
        let ws_stream = accept_async(stream).await?;

        let (event_tx,   event_rx)   = mpsc::unbounded_channel::<String>();
        let (command_tx, command_rx) = mpsc::unbounded_channel::<ClientCommand>();
        let (mut ws_sink, mut ws_source) = ws_stream.split();

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

        let welcome = format!(
            "{{\"type\":\"Welcome\",\"player_id\":{},\"ship_id\":{}}}\n",
            player_id.raw(), ship_id.raw()
        );
        ws_sink.send(Message::Text(welcome.into())).await?;
        ws_sink.send(Message::Text((initial_state.to_string() + "\n").into())).await?;
        if let Some(fitting) = player_fitting {
            ws_sink.send(Message::Text((fitting + "\n").into())).await?;
        }

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
            println!("[WsServer] {peer_addr} disconnected");
        });

        let conn = WsClientConnection { event_tx, command_rx };
        println!("[WsServer] {peer_addr} handshake complete: {player_id:?} ship={}", ship_id.raw());
        Ok(PlayerSession { player_id, ship_id, conn })
    }
}
