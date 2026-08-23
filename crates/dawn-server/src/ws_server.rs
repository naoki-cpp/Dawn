//! # WebSocket Server — production client transport (ADR-0005, ADR-0007)
//!
//! The single WebSocket server / session implementation shared by both
//! binaries (`dawn-server --bin simulate`, `dawn-server --bin sector-node`).
//! It owns:
//!   - Hello/Welcome handshake (assigns and announces a PlayerId)
//!   - InitialState + PlayerLoadout on connect
//!   - `PlayerSession` mapping a connection to its PlayerId / ShipId
//!   - `WsClientConnection`, the [`ClientConnection`] impl over a socket
//!
//! ## Protocol (ADR-0042)
//!
//! Every server -> client message (Hello/Welcome/Redirect/ServerFact/
//! ClientRequest/Market/InitialState/PlayerLoadout/AoiEnter/AoiLeave/
//! PositionSnap) travels as a binary WebSocket frame, postcard-encoded via the
//! [`ClientMessage`]/[`ServerMessage`] envelope in `dawn-protocol` (ADR-0042
//! stages 1-2c). There is no more ad-hoc JSON text path. One WebSocket frame
//! always carries exactly one message (no length-prefix framing needed;
//! WebSocket already delimits frames).
//!
//! ```text
//! Client → Server:  ClientMessage::Hello           (binary, postcard)
//! Server → Client:  ServerMessage::Welcome         (binary, postcard)
//! Server → Client:  ServerMessage::InitialState(..) (binary, postcard)
//! Server → Client:  ServerMessage::Fact(..)       (binary, postcard stream)
//! Client → Server:  ClientMessage::Command(..)     (binary, postcard)
//! Client → Server:  ClientMessage::Market(..)      (binary, postcard)
//! ```

use crate::client_connection::{ClientConnection, ConnectionError};
use dawn_core::{ClientRequest, PlayerId, ShipId};
use dawn_protocol::{
    ClientMessage, InitialStateWire, MarketCommandWire, PlayerLoadoutWire, ResumeTicket,
    ServerFact, ServerMessage,
};
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
fn server_message_frame(msg: &ServerMessage) -> Result<Message, ConnectionError> {
    msg.encode()
        .map(|bytes| Message::Binary(bytes.into()))
        .map_err(|error| ConnectionError::Encoding(error.to_string()))
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
    request_rx: mpsc::Receiver<ClientRequest>,
    market_command_rx: mpsc::Receiver<MarketCommandWire>,
}

impl WsClientConnection {
    /// Send a [`ServerMessage`] as a postcard-encoded binary frame
    /// (ADR-0042: every server -> client message, now that stage 2c folded
    /// in the last ad-hoc JSON messages).
    pub fn send_message(&self, msg: &ServerMessage) -> bool {
        match server_message_frame(msg) {
            Ok(frame) => self.event_tx.send(frame).is_ok(),
            Err(_) => false,
        }
    }
}

impl ClientConnection for WsClientConnection {
    fn send_facts(&self, facts: &[ServerFact]) -> Result<(), ConnectionError> {
        for fact in facts {
            self.event_tx
                .send(server_message_frame(&ServerMessage::Fact(fact.clone()))?)
                .map_err(|_| ConnectionError::Disconnected)?;
        }
        Ok(())
    }

    fn try_recv_request(&mut self) -> Option<ClientRequest> {
        self.request_rx.try_recv().ok()
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
    pub resume_ticket: ResumeTicket,
    pub conn: WsClientConnection,
}

// ── HandshakeRequest ─────────────────────────────────────────────────────────

type WsSink = SplitSink<WebSocketStream<TcpStream>, Message>;
type WsSource = SplitStream<WebSocketStream<TcpStream>>;

pub struct HandshakeRequest {
    pub peer_addr: SocketAddr,
    pub resume: Option<ResumeTicket>,
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
        resume_ticket: ResumeTicket,
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
        let (request_tx, request_rx) = mpsc::channel::<ClientRequest>(COMMAND_QUEUE_CAP);
        let (market_command_tx, market_command_rx) =
            mpsc::channel::<MarketCommandWire>(COMMAND_QUEUE_CAP);

        // Send Welcome + InitialState + (optional) PlayerLoadout, all binary
        // (ADR-0042 stage 2b).
        ws_sink
            .send(server_message_frame(&ServerMessage::Welcome {
                player_id: player_id.raw(),
                ship_id: ship_id.raw(),
                resume_ticket,
            })?)
            .await?;
        ws_sink
            .send(server_message_frame(&ServerMessage::InitialState(
                initial_state,
            ))?)
            .await?;
        if let Some(loadout) = player_loadout {
            ws_sink
                .send(server_message_frame(&ServerMessage::PlayerLoadout(
                    loadout,
                ))?)
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

        // Request-receive task.
        let rejection_tx = event_tx.clone();
        tokio::spawn(async move {
            while let Some(Ok(msg)) = ws_source.next().await {
                if let Message::Binary(bytes) = msg {
                    match ClientMessage::decode(&bytes) {
                        Ok(ClientMessage::Command(request)) => {
                            // Bounded send backpressures the socket reader once the
                            // application queue reaches COMMAND_QUEUE_CAP.
                            if request_tx.send(request).await.is_err() {
                                return;
                            }
                        }
                        Ok(ClientMessage::Market(market_command)) => {
                            if market_command_tx.send(market_command).await.is_err() {
                                return;
                            }
                        }
                        Ok(ClientMessage::Hello(_)) => {}
                        Err(error) => {
                            // A malformed peer receives at most one structured
                            // rejection before the connection is closed. Returning
                            // here prevents untrusted input from growing the
                            // unbounded outbound queue without limit.
                            let rejection = ServerMessage::ClientRequestRejected(error.rejection());
                            if let Ok(frame) = server_message_frame(&rejection) {
                                let _ = rejection_tx.send(frame);
                            }
                            let _ = rejection_tx.send(Message::Close(None));
                            return;
                        }
                    }
                }
            }
            println!("[WsServer] {peer_addr} disconnected");
        });

        let conn = WsClientConnection {
            event_tx,
            request_rx,
            market_command_rx,
        };
        println!(
            "[WsServer] {peer_addr} handshake complete: {player_id} ship={}",
            ship_id.raw()
        );
        Ok(PlayerSession {
            player_id,
            ship_id,
            resume_ticket,
            conn,
        })
    }
}

impl PlayerSession {
    /// Send projected facts to this client. Returns false on send failure.
    pub fn send_facts(&self, facts: &[ServerFact]) -> bool {
        self.conn.send_facts(facts).is_ok()
    }

    /// Pull one pending request, if any.
    pub fn try_recv_request(&mut self) -> Option<ClientRequest> {
        self.conn.try_recv_request()
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
        resume_ticket: ResumeTicket,
        initial_state: InitialStateWire,
        player_loadout: Option<PlayerLoadoutWire>,
    ) -> anyhow::Result<PlayerSession> {
        let request = Self::accept_handshake_request(stream, peer_addr).await?;
        request
            .complete(
                player_id,
                ship_id,
                resume_ticket,
                initial_state,
                player_loadout,
            )
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
