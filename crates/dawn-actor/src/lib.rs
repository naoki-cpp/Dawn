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
//! dawn-server.
//! dawn-actor must never depend on dawn-ecs or dawn-server.
//!
//! ## Client transport
//!
//! `ws_server` (`WsServer` / `WsClientConnection` / `PlayerSession`) is the
//! production WebSocket transport, shared by both binaries (previously
//! duplicated). Every message travels as a postcard-encoded binary frame
//! (ADR-0042), using the `dawn-protocol` schema (`ClientMessage`/`ServerMessage`,
//! `project_domain_event`, `ClientRequest`) directly --
//! this crate no longer re-exports it under its own `protocol` module
//! (deleted: it was 28 lines of `pub use` and 900 lines of tests that
//! belonged in `dawn-protocol`, where they now live).
//!
//! ## Example
//!
//! ```
//! use dawn_protocol::{ClientMessage, HelloMessage};
//!
//! let msg = ClientMessage::Hello(HelloMessage { resume: None });
//! let bytes = msg.encode().unwrap();
//! let decoded = ClientMessage::decode(&bytes).unwrap();
//! assert!(matches!(decoded, ClientMessage::Hello(HelloMessage { resume: None })));
//! ```

// Rust API Guidelines C-DEBUG: catch new pub types that forget to derive
// Debug at compile time instead of relying on periodic audits (see #83).
#![warn(missing_debug_implementations)]

pub mod client_connection;
pub mod ws_server;

pub use client_connection::{
    ClientConnection, ClientRequest, ConnectionError, InProcessClientEndpoint, InProcessConnection,
};
