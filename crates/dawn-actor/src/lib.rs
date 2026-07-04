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
//!
//! ## Example
//!
//! ```
//! use dawn_actor::protocol::parse_hello;
//!
//! let hello = parse_hello(r#"{"type":"Hello"}"#).expect("valid Hello line");
//! assert!(hello.resume.is_none());
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
