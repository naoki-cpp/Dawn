//! # dawn-actor
//!
//! Actor infrastructure for the dawn simulation platform.
//!
//! ## Invariants (CLAUDE.md §5, FBD-004)
//!
//! - Actors own their data exclusively. No `Arc<Mutex<T>>` across Actor boundaries.
//! - All inter-Actor communication is via Mailbox (`tokio::mpsc`) only.
//! - Callers interact only with `*Handle` types, never with Actor internals.
//!
//! ## Crates that may depend on dawn-actor
//!
//! dawn-simulation and dawn-sector-node.
//! dawn-actor must never depend on dawn-ecs or dawn-simulation.
//!
//! ## Client transport
//!
//! `protocol` (DomainEvent <-> JSON <-> ClientCommand) and `ws_server`
//! (`WsServer` / `WsClientConnection` / `PlayerSession`) are the production
//! WebSocket transport, shared by both binaries (previously duplicated).

pub mod client_connection;
pub mod protocol;
pub mod ws_server;

pub use client_connection::{
    ClientCommand, ClientConnection, ConnectionError, InProcessClientEndpoint, InProcessConnection,
};
