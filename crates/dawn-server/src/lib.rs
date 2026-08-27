//! Shared server-side client transport.
//!
//! The `simulate` and `sector-node` binaries use the same WebSocket framing,
//! handshake, and in-process connection seams from this library. Protocol
//! types remain owned by `dawn-protocol`; this crate owns only server runtime
//! transport and session mechanics.
//!
//! # Example
//!
//! ```
//! use dawn_server::client_connection::InProcessConnection;
//!
//! let (_server, _client) = InProcessConnection::pair();
//! ```

#![warn(missing_debug_implementations)]

pub mod client_connection;
pub mod runtime_frame;
pub mod ws_server;
