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
//! `protocol` (DomainEvent <-> `EventWire` <-> ClientCommand) and `ws_server`
//! (`WsServer` / `WsClientConnection` / `PlayerSession`) are the production
//! WebSocket transport, shared by both binaries (previously duplicated).
//! Every message travels as a postcard-encoded binary frame (ADR-0042).
//!
//! ## Example
//!
//! ```
//! use dawn_actor::protocol::{ClientMessage, HelloMessage};
//!
//! let msg = ClientMessage::Hello(HelloMessage { resume: None });
//! let bytes = msg.encode();
//! let decoded = ClientMessage::decode(&bytes).unwrap();
//! assert!(matches!(decoded, ClientMessage::Hello(HelloMessage { resume: None })));
//! ```

// Rust API Guidelines C-DEBUG: catch new pub types that forget to derive
// Debug at compile time instead of relying on periodic audits (see #83).
#![warn(missing_debug_implementations)]

pub mod client_connection;
pub mod protocol;
pub mod ws_server;

pub use client_connection::{
    ClientCommand, ClientConnection, ConnectionError, InProcessClientEndpoint, InProcessConnection,
};
