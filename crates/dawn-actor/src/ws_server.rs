//! # WebSocket Server — production client transport (ADR-0005, ADR-0007)
//!
//! The single WebSocket server / session implementation shared by both
//! binaries (`dawn-simulation`, `dawn-sector-node`). It owns:
//!   - Hello/Welcome handshake (assigns and announces a PlayerId)
//!   - InitialState + PlayerLoadout on connect
//!   - `PlayerSession` mapping a connection to its PlayerId / ShipId
//!   - `WsClientConnection`, the [`ClientConnection`] impl over a socket
//!
//! ## Protocol (ADR-0042)
//!
//! Every server -> client message (Hello/Welcome/Redirect/DomainEvent/
//! ClientCommand/Market/InitialState/PlayerLoadout/AoiEnter/AoiLeave/
//! PositionSnap) travels as a binary WebSocket frame, postcard-encoded via the
//! [`ClientMessage`]/[`ServerMessage`] envelope in `dawn-wire` (ADR-0042
//! stages 1-2c). There is no more ad-hoc JSON text path. One WebSocket frame
//! always carries exactly one message (no length-prefix framing needed;
//! WebSocket already delimits frames).
//!
//! ```text
//! Client → Server:  ClientMessage::Hello           (binary, postcard)
//! Server → Client:  ServerMessage::Welcome         (binary, postcard)
//! Server → Client:  ServerMessage::InitialState(..) (binary, postcard)
//! Server → Client:  ServerMessage::Event(..)       (binary, postcard stream)
//! Client → Server:  ClientMessage::Command(..)     (binary, postcard)
//! Client → Server:  ClientMessage::Market(..)      (binary, postcard)
//! ```

use crate::protocol::{
    domain_event_to_event_wire, ClientMessage, InitialStateWire, MarketCommandWire,
    PlayerLoadoutWire, ResumeIdentity, ServerMessage,
};
use crate::{ClientCommand, ClientConnection};
use dawn_core::{DomainEvent, PlayerId, ShipId};
use futures_util::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use std::fmt::Display;
use std::net::SocketAddr;
use tokio::{
    net::{TcpListener, TcpStream, ToSocketAddrs},
    sync::mpsc,
    time::{timeout, Duration},
};
use tokio_tungstenite::{accept_async, tungstenite::Message, WebSocketStream};

/// Postcard-encode a [`ServerMessage`] and wrap it as a binary WS frame.
fn server_message_frame(msg: &ServerMessage) -> Message {
    Message::Binary(msg.encode())
}

// ── WsClientConnection ────────────────────────────────────────────────────────

/// Cap on how many parsed commands a single connection may have queued
/// waiting for the tick loop to drain them. At `TICK_MS` = 100ms this is
/// several seconds of buffer even under total starvation -- generous for a
/// normal player, but bounded, so a client sending commands faster than the
/// server drains them applies TCP backpressure (the read task's `.await` on
/// `send` stalls) instead of growing server memory without limit
/// (security-review.md SEC-4).
const COMMAND_QUEUE_CAP: usize = 256;

#[derive(Debug)]
pub struct WsClientConnection {
    event_tx: mpsc::UnboundedSender<Message>,
    command_rx: mpsc::Receiver<ClientCommand>,
    market_command_rx: mpsc::Receiver<MarketCommandWire>,
}

impl WsClientConnection {
    /// Send a [`ServerMessage`] as a postcard-encoded binary frame
    /// (ADR-0042: every server -> client message, now that stage 2c folded
    /// in the last ad-hoc JSON messages).
    pub fn send_message(&self, msg: &ServerMessage) -> bool {
        self.event_tx.send(server_message_frame(msg)).is_ok()
    }
}

impl ClientConnection for WsClientConnection {
    fn send_events(&self, events: &[DomainEvent]) -> Result<(), crate::ConnectionError> {
        for event in events {
            if let Some(event_wire) = domain_event_to_event_wire(event) {
                self.event_tx
                    .send(server_message_frame(&ServerMessage::Event(event_wire)))
                    .map_err(|_| crate::ConnectionError::Disconnected)?;
            }
        }
        Ok(())
    }

    fn try_recv_command(&mut self) -> Option<ClientCommand> {
        self.command_rx.try_recv().ok()
    }

    fn try_recv_market_command(&mut self) -> Option<MarketCommandWire> {
        self.market_command_rx.try_recv().ok()
    }
}

// ── PlayerSession ─────────────────────────────────────────────────────────────

/// One player connection: holds its PlayerId, ShipId, and connection.
#[derive(Debug)]
pub struct PlayerSession {
    pub player_id: PlayerId,
    pub ship_id: ShipId,
    pub conn: WsClientConnection,
}

// ── HandshakeRequest ─────────────────────────────────────────────────────────

type WsSink = SplitSink<WebSocketStream<TcpStream>, Message>;
type WsSource = SplitStream<WebSocketStream<TcpStream>>;

pub struct HandshakeRequest {
    pub peer_addr: SocketAddr,
    pub resume: Option<ResumeIdentity>,
    ws_sink: WsSink,
    ws_source: WsSource,
}

impl std::fmt::Debug for HandshakeRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HandshakeRequest")
            .field("peer_addr", &self.peer_addr)
            .field("resume", &self.resume)
            .finish_non_exhaustive()
    }
}

impl HandshakeRequest {
    pub async fn complete(
        self,
        player_id: PlayerId,
        ship_id: ShipId,
        initial_state: InitialStateWire,
        player_loadout: Option<PlayerLoadoutWire>,
    ) -> anyhow::Result<PlayerSession> {
        let Self {
            peer_addr,
            mut ws_sink,
            mut ws_source,
            ..
        } = self;

        let (event_tx, event_rx) = mpsc::unbounded_channel::<Message>();
        let (command_tx, command_rx) = mpsc::channel::<ClientCommand>(COMMAND_QUEUE_CAP);
        let (market_command_tx, market_command_rx) =
            mpsc::channel::<MarketCommandWire>(COMMAND_QUEUE_CAP);

        // Send Welcome + InitialState + (optional) PlayerLoadout, all binary
        // (ADR-0042 stage 2b).
        ws_sink
            .send(server_message_frame(&ServerMessage::Welcome {
                player_id: player_id.raw(),
                ship_id: ship_id.raw(),
            }))
            .await?;
        ws_sink
            .send(server_message_frame(&ServerMessage::InitialState(
                initial_state,
            )))
            .await?;
        if let Some(loadout) = player_loadout {
            ws_sink
                .send(server_message_frame(&ServerMessage::PlayerLoadout(loadout)))
                .await?;
        }

        // Event-send task.
        tokio::spawn(async move {
            let mut rx = event_rx;
            while let Some(msg) = rx.recv().await {
                if ws_sink.send(msg).await.is_err() {
                    break;
                }
            }
            let _ = ws_sink.close().await;
        });

        // Command-receive task.
        tokio::spawn(async move {
            while let Some(Ok(msg)) = ws_source.next().await {
                if let Message::Binary(bytes) = msg {
                    if let Ok(message) = ClientMessage::decode(&bytes) {
                        match message {
                            ClientMessage::Command(cmd_wire) => {
                                if let Some(cmd) =
                                    crate::protocol::client_command_from_wire(cmd_wire)
                                {
                                    // Bounded send: blocks (backpressures the socket
                                    // read) once COMMAND_QUEUE_CAP is reached instead
                                    // of growing memory without limit.
                                    if command_tx.send(cmd).await.is_err() {
                                        return;
                                    }
                                }
                            }
                            ClientMessage::Market(market_command) => {
                                if market_command_tx.send(market_command).await.is_err() {
                                    return;
                                }
                            }
                            ClientMessage::Hello(_) => {}
                        }
                    }
                }
            }
            println!("[WsServer] {peer_addr} disconnected");
        });

        let conn = WsClientConnection {
            event_tx,
            command_rx,
            market_command_rx,
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

impl PlayerSession {
    /// Send events to this client. Returns false on send failure (disconnect).
    pub fn send_events(&self, events: &[DomainEvent]) -> bool {
        self.conn.send_events(events).is_ok()
    }

    /// Pull one pending command, if any.
    pub fn try_recv_command(&mut self) -> Option<ClientCommand> {
        self.conn.try_recv_command()
    }

    /// Pull one pending Market request, if any.
    pub fn try_recv_market_command(&mut self) -> Option<MarketCommandWire> {
        self.conn.try_recv_market_command()
    }

    /// Send a [`ServerMessage`] as a postcard-encoded binary frame
    /// (ADR-0042 -- e.g. `Redirect` on cross-node Sector Transit,
    /// `PlayerLoadout` after Fit/Unfit ADR-0032, `AoiEnter`/`AoiLeave`/
    /// `PositionSnap`).
    pub fn send_message(&self, msg: &ServerMessage) -> bool {
        self.conn.send_message(msg)
    }
}

// ── WsServer ─────────────────────────────────────────────────────────────────

#[derive(Debug)]
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
    /// 3. Send Welcome + InitialState (+ PlayerLoadout)
    /// 4. Return the `PlayerSession`
    pub async fn handshake(
        stream: TcpStream,
        peer_addr: SocketAddr,
        player_id: PlayerId,
        ship_id: ShipId,
        initial_state: InitialStateWire,
        player_loadout: Option<PlayerLoadoutWire>,
    ) -> anyhow::Result<PlayerSession> {
        let request = Self::accept_handshake_request(stream, peer_addr).await?;
        request
            .complete(player_id, ship_id, initial_state, player_loadout)
            .await
    }

    /// Upgrade a socket and read the client Hello without committing to a
    /// player identity yet. Callers can inspect `resume` before completing the
    /// handshake with Welcome + InitialState.
    pub async fn accept_handshake_request(
        stream: TcpStream,
        peer_addr: SocketAddr,
    ) -> anyhow::Result<HandshakeRequest> {
        let ws_stream = accept_async(stream).await?;
        let (ws_sink, mut ws_source) = ws_stream.split();

        // Wait for Hello (3s timeout, binary ClientMessage envelope -- ADR-0042).
        let hello_result = timeout(Duration::from_secs(3), async {
            while let Some(Ok(msg)) = ws_source.next().await {
                if let Message::Binary(bytes) = msg {
                    if let Ok(ClientMessage::Hello(hello)) = ClientMessage::decode(&bytes) {
                        return Some(hello);
                    }
                }
            }
            None
        })
        .await;

        let hello = match hello_result {
            Ok(Some(hello)) => hello,
            _ => anyhow::bail!("Hello timeout or not received from {peer_addr}"),
        };

        Ok(HandshakeRequest {
            peer_addr,
            resume: hello.resume,
            ws_sink,
            ws_source,
        })
    }
}
