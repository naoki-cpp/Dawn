//! # ClientConnection — server/client communication boundary
//!
//! ## Design (ADR-0005)
//!
//! The trait defines the server event direction plus two independent client
//! input queues:
//!
//! ```text
//! Server side                     Client side
//! ─────────────────────────       ──────────────────
//! serve loop (WsServer)           Godot scene / tests
//!     │  send_facts()                 ↑  recv_fact()
//!     │                              │
//!     └─── ClientConnection ─────────┘
//!              │  try_recv_request() → ClientRequest (Sector)
//!              └── try_recv_market_command() → MarketCommandWire
//! ```
//!
//! Implementations:
//! - `WsClientConnection` (ws_server.rs) — the production WebSocket transport
//!   (ADR-0007; gRPC/protobuf was not adopted).
//! - `InProcessConnection` (below) — an in-memory `tokio::mpsc` pair used by
//!   tests to drive the serve pipeline without a socket.
//!
//! ## Extending client input
//!
//! Sector requests use the single `dawn_core::ClientRequest` protocol authority; Market
//! requests remain `dawn_protocol::MarketCommandWire` and never enter
//! `SimulationNode::apply_client_request`.

use dawn_core::ClientRequest;
use dawn_protocol::{MarketCommandWire, ServerFact};
use tokio::sync::mpsc;

// ── Error ─────────────────────────────────────────────────────────────────────

/// Errors that a `ClientConnection` operation can produce.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConnectionError {
    /// The peer (client) is already disconnected.
    #[error("client disconnected")]
    Disconnected,
    /// The typed server fact could not be serialized into a wire frame.
    #[error("server message encoding failed: {0}")]
    Encoding(String),
}

// ── Trait ─────────────────────────────────────────────────────────────────────

/// A client connection as seen from the server side.
///
/// # Implementation rules
///
/// - `send_facts` must complete without blocking (`Err` signals back-pressure
///   / disconnection).
/// - `try_recv_request` must be non-blocking (`None` when no command is ready).
/// - Implementations must be `Send + 'static` (they move across actor threads).
pub trait ClientConnection: Send + 'static {
    /// Send projected, client-visible facts from the server to the client.
    fn send_facts(&self, facts: &[ServerFact]) -> Result<(), ConnectionError>;

    /// Take one pending command from the client, non-blocking.
    fn try_recv_request(&mut self) -> Option<ClientRequest>;

    /// Take one pending Market request from the client, non-blocking.
    fn try_recv_market_command(&mut self) -> Option<MarketCommandWire>;
}

// ── InProcessConnection ───────────────────────────────────────────────────────

/// In-process implementation backed by `tokio` unbounded channels.
///
/// Used by tests to drive the serve pipeline without a socket; the production
/// transport is `WsClientConnection` (ws_server.rs, ADR-0007).
///
/// ## Usage
///
/// ```rust
/// use dawn_server::client_connection::{InProcessConnection, InProcessClientEndpoint};
///
/// let (server_side, client_side) = InProcessConnection::pair();
/// // server_side → drive from the serve loop (drain commands, send events)
/// // client_side → test code (send commands, observe events)
/// ```
#[derive(Debug)]
pub struct InProcessConnection {
    fact_tx: mpsc::UnboundedSender<ServerFact>,
    request_rx: mpsc::UnboundedReceiver<ClientRequest>,
    market_command_rx: mpsc::UnboundedReceiver<MarketCommandWire>,
}

/// The client-side endpoint of an [`InProcessConnection`].
#[derive(Debug)]
pub struct InProcessClientEndpoint {
    pub fact_rx: mpsc::UnboundedReceiver<ServerFact>,
    pub request_tx: mpsc::UnboundedSender<ClientRequest>,
    pub market_command_tx: mpsc::UnboundedSender<MarketCommandWire>,
}

impl InProcessConnection {
    pub fn pair() -> (Self, InProcessClientEndpoint) {
        let (fact_tx, fact_rx) = mpsc::unbounded_channel();
        let (request_tx, request_rx) = mpsc::unbounded_channel();
        let (market_command_tx, market_command_rx) = mpsc::unbounded_channel();
        (
            InProcessConnection {
                fact_tx,
                request_rx,
                market_command_rx,
            },
            InProcessClientEndpoint {
                fact_rx,
                request_tx,
                market_command_tx,
            },
        )
    }
}

impl ClientConnection for InProcessConnection {
    fn send_facts(&self, facts: &[ServerFact]) -> Result<(), ConnectionError> {
        for fact in facts {
            self.fact_tx
                .send(fact.clone())
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::events::ShipSpawned;
    use dawn_core::{
        AbsolutePosition, DomainEvent, EntityId, NodeId, Position, SectorId, ShipId, Tick,
    };
    use dawn_protocol::project_domain_event;

    fn make_ship_spawned() -> DomainEvent {
        DomainEvent::ShipSpawned(ShipSpawned {
            ship_id: ShipId(EntityId::new(NodeId(0), 1)),
            initial_position: AbsolutePosition::new(1.0, 2.0, 3.0),
            sector_id: SectorId(0),
            ship_type_id: dawn_core::ShipTypeId(1),
            tick: Tick::ZERO,
        })
    }

    fn make_ship_spawned_fact() -> dawn_protocol::ServerFact {
        project_domain_event(&make_ship_spawned()).expect("ShipSpawned is client-visible")
    }

    fn make_move_command() -> ClientRequest {
        ClientRequest::Move {
            target: Position {
                x: 10.0,
                y: 0.0,
                z: 0.0,
            },
        }
    }

    fn make_lock_on_command() -> ClientRequest {
        ClientRequest::LockOn {
            target: ShipId(EntityId::new(NodeId(0), 2)),
        }
    }

    #[test]
    fn server_facts_are_received_by_client_endpoint() {
        let (server, mut client) = InProcessConnection::pair();
        let fact = make_ship_spawned_fact();
        server.send_facts(std::slice::from_ref(&fact)).unwrap();
        let received = client.fact_rx.try_recv().expect("fact should be available");
        assert_eq!(received, fact);
    }

    #[test]
    fn move_command_is_received_by_server_connection() {
        let (mut server, client) = InProcessConnection::pair();
        client.request_tx.send(make_move_command()).unwrap();
        let cmd = server
            .try_recv_request()
            .expect("command should be available");
        assert!(matches!(cmd, ClientRequest::Move { .. }));
    }

    #[test]
    fn lock_on_command_is_received_by_server_connection() {
        let (mut server, client) = InProcessConnection::pair();
        client.request_tx.send(make_lock_on_command()).unwrap();
        let cmd = server
            .try_recv_request()
            .expect("command should be available");
        assert!(matches!(cmd, ClientRequest::LockOn { .. }));
    }

    #[test]
    fn commands_are_delivered_in_order() {
        let (mut server, client) = InProcessConnection::pair();
        client.request_tx.send(make_move_command()).unwrap();
        client.request_tx.send(make_lock_on_command()).unwrap();
        assert!(matches!(
            server.try_recv_request().unwrap(),
            ClientRequest::Move { .. }
        ));
        assert!(matches!(
            server.try_recv_request().unwrap(),
            ClientRequest::LockOn { .. }
        ));
    }

    #[test]
    fn jump_command_is_received_by_server_connection() {
        let (mut server, client) = InProcessConnection::pair();
        client
            .request_tx
            .send(ClientRequest::Jump {
                gate: dawn_core::JumpGateId(0),
            })
            .unwrap();
        let cmd = server
            .try_recv_request()
            .expect("command should be available");
        assert!(matches!(cmd, ClientRequest::Jump { .. }));
    }

    #[test]
    fn try_recv_request_returns_none_when_no_command_pending() {
        let (mut server, _client) = InProcessConnection::pair();
        assert!(server.try_recv_request().is_none());
    }

    #[test]
    fn market_command_is_received_by_server_connection() {
        let (mut server, client) = InProcessConnection::pair();
        client
            .market_command_tx
            .send(MarketCommandWire::RefreshMarketCommand {})
            .unwrap();
        assert!(matches!(
            server.try_recv_market_command(),
            Some(MarketCommandWire::RefreshMarketCommand {})
        ));
    }

    #[test]
    fn send_facts_returns_disconnected_when_client_dropped() {
        let (server, client) = InProcessConnection::pair();
        drop(client);
        let result = server.send_facts(&[make_ship_spawned_fact()]);
        assert!(matches!(result, Err(ConnectionError::Disconnected)));
    }

    #[test]
    fn send_facts_with_empty_slice_is_always_ok() {
        let (server, _client) = InProcessConnection::pair();
        assert!(server.send_facts(&[]).is_ok());
    }

    #[test]
    fn multiple_facts_are_delivered_in_order() {
        let (server, mut client) = InProcessConnection::pair();
        let facts: Vec<dawn_protocol::ServerFact> =
            (0..3).map(|_| make_ship_spawned_fact()).collect();
        server.send_facts(&facts).unwrap();
        let mut count = 0;
        while client.fact_rx.try_recv().is_ok() {
            count += 1;
        }
        assert_eq!(count, 3);
    }
}
