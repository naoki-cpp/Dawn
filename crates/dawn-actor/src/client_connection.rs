//! # ClientConnection — server/client communication boundary
//!
//! ## Design (ADR-0005)
//!
//! The trait defines exactly **two directions**:
//!
//! ```text
//! Server side                     Client side
//! ─────────────────────────       ──────────────────
//! serve loop (WsServer)           Godot scene / tests
//!     │  send_events()                ↑  recv_event()
//!     │                              │
//!     └─── ClientConnection ─────────┘
//!              │  try_recv_command() → ClientCommand
//!              └──────── ←  command_tx.send()
//! ```
//!
//! Implementations:
//! - `WsClientConnection` (ws_server.rs) — the production WebSocket transport
//!   (ADR-0007; gRPC/protobuf was not adopted).
//! - `InProcessConnection` (below) — an in-memory `tokio::mpsc` pair used by
//!   tests to drive the serve pipeline without a socket.
//!
//! ## Extending ClientCommand
//!
//! Commands the client may send are variants of the `ClientCommand` enum
//! (defined in `dawn-core`); the `ClientConnection` trait itself does not
//! change. Adding a command = add a variant to `dawn_core::ClientCommand`
//! (then update the `dawn-actor/protocol.rs` JSON parser and add a branch to
//! `SimulationNode::apply_client_command` in `dawn-sector`).

pub use dawn_core::ClientCommand;
use dawn_core::DomainEvent;
use tokio::sync::mpsc;

// ── Error ─────────────────────────────────────────────────────────────────────

/// Errors that a `ClientConnection` operation can produce.
#[derive(Debug, thiserror::Error)]
pub enum ConnectionError {
    /// The peer (client) is already disconnected.
    #[error("client disconnected")]
    Disconnected,
}

// ── Trait ─────────────────────────────────────────────────────────────────────

/// A client connection as seen from the server side.
///
/// # Implementation rules
///
/// - `send_events` must complete without blocking (`Err` signals back-pressure
///   / disconnection).
/// - `try_recv_command` must be non-blocking (`None` when no command is ready).
/// - Implementations must be `Send + 'static` (they move across actor threads).
pub trait ClientConnection: Send + 'static {
    /// Send events from the server to the client.
    fn send_events(&self, events: &[DomainEvent]) -> Result<(), ConnectionError>;

    /// Take one pending command from the client, non-blocking.
    fn try_recv_command(&mut self) -> Option<ClientCommand>;
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
/// use dawn_actor::client_connection::{InProcessConnection, InProcessClientEndpoint};
///
/// let (server_side, client_side) = InProcessConnection::pair();
/// // server_side → drive from the serve loop (drain commands, send events)
/// // client_side → test code (send commands, observe events)
/// ```
#[derive(Debug)]
pub struct InProcessConnection {
    event_tx: mpsc::UnboundedSender<DomainEvent>,
    command_rx: mpsc::UnboundedReceiver<ClientCommand>,
}

/// The client-side endpoint of an [`InProcessConnection`].
#[derive(Debug)]
pub struct InProcessClientEndpoint {
    pub event_rx: mpsc::UnboundedReceiver<DomainEvent>,
    pub command_tx: mpsc::UnboundedSender<ClientCommand>,
}

impl InProcessConnection {
    pub fn pair() -> (Self, InProcessClientEndpoint) {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        (
            InProcessConnection {
                event_tx,
                command_rx,
            },
            InProcessClientEndpoint {
                event_rx,
                command_tx,
            },
        )
    }
}

impl ClientConnection for InProcessConnection {
    fn send_events(&self, events: &[DomainEvent]) -> Result<(), ConnectionError> {
        for event in events {
            self.event_tx
                .send(event.clone())
                .map_err(|_| ConnectionError::Disconnected)?;
        }
        Ok(())
    }

    fn try_recv_command(&mut self) -> Option<ClientCommand> {
        self.command_rx.try_recv().ok()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::events::ShipSpawned;
    use dawn_core::{
        DomainEvent, EntityId, LockOnCommand, MoveCommand, NodeId, Position, SectorId, ShipId, Tick,
    };

    fn make_ship_spawned() -> DomainEvent {
        DomainEvent::ShipSpawned(ShipSpawned {
            ship_id: ShipId(EntityId::new(NodeId(0), 1)),
            initial_position: Position {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
            sector_id: SectorId(0),
            ship_type_id: dawn_core::ShipTypeId(1),
            tick: Tick::ZERO,
        })
    }

    fn make_move_command() -> ClientCommand {
        ClientCommand::Move(MoveCommand {
            ship_id: ShipId(EntityId::new(NodeId(0), 1)),
            target_position: Position {
                x: 10.0,
                y: 0.0,
                z: 0.0,
            },
        })
    }

    fn make_lock_on_command() -> ClientCommand {
        ClientCommand::LockOn(LockOnCommand {
            ship_id: ShipId(EntityId::new(NodeId(0), 1)),
            target_id: ShipId(EntityId::new(NodeId(0), 2)),
        })
    }

    #[test]
    fn server_events_are_received_by_client_endpoint() {
        let (server, mut client) = InProcessConnection::pair();
        let event = make_ship_spawned();
        server.send_events(std::slice::from_ref(&event)).unwrap();
        let received = client
            .event_rx
            .try_recv()
            .expect("event should be available");
        assert_eq!(format!("{:?}", received), format!("{:?}", event));
    }

    #[test]
    fn move_command_is_received_by_server_connection() {
        let (mut server, client) = InProcessConnection::pair();
        client.command_tx.send(make_move_command()).unwrap();
        let cmd = server
            .try_recv_command()
            .expect("command should be available");
        assert!(matches!(cmd, ClientCommand::Move(_)));
    }

    #[test]
    fn lock_on_command_is_received_by_server_connection() {
        let (mut server, client) = InProcessConnection::pair();
        client.command_tx.send(make_lock_on_command()).unwrap();
        let cmd = server
            .try_recv_command()
            .expect("command should be available");
        assert!(matches!(cmd, ClientCommand::LockOn(_)));
    }

    #[test]
    fn commands_are_delivered_in_order() {
        let (mut server, client) = InProcessConnection::pair();
        client.command_tx.send(make_move_command()).unwrap();
        client.command_tx.send(make_lock_on_command()).unwrap();
        assert!(matches!(
            server.try_recv_command().unwrap(),
            ClientCommand::Move(_)
        ));
        assert!(matches!(
            server.try_recv_command().unwrap(),
            ClientCommand::LockOn(_)
        ));
    }

    #[test]
    fn jump_command_is_received_by_server_connection() {
        let (mut server, client) = InProcessConnection::pair();
        client
            .command_tx
            .send(ClientCommand::Jump(dawn_core::JumpCommand {
                ship_id: ShipId(EntityId::new(NodeId(0), 1)),
                gate_id: dawn_core::JumpGateId(0),
            }))
            .unwrap();
        let cmd = server
            .try_recv_command()
            .expect("command should be available");
        assert!(matches!(cmd, ClientCommand::Jump(_)));
    }

    #[test]
    fn try_recv_command_returns_none_when_no_command_pending() {
        let (mut server, _client) = InProcessConnection::pair();
        assert!(server.try_recv_command().is_none());
    }

    #[test]
    fn send_events_returns_disconnected_when_client_dropped() {
        let (server, client) = InProcessConnection::pair();
        drop(client);
        let result = server.send_events(&[make_ship_spawned()]);
        assert!(matches!(result, Err(ConnectionError::Disconnected)));
    }

    #[test]
    fn send_events_with_empty_slice_is_always_ok() {
        let (server, _client) = InProcessConnection::pair();
        assert!(server.send_events(&[]).is_ok());
    }

    #[test]
    fn multiple_events_are_delivered_in_order() {
        let (server, mut client) = InProcessConnection::pair();
        let events: Vec<DomainEvent> = (0..3).map(|_| make_ship_spawned()).collect();
        server.send_events(&events).unwrap();
        let mut count = 0;
        while client.event_rx.try_recv().is_ok() {
            count += 1;
        }
        assert_eq!(count, 3);
    }
}
