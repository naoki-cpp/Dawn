//! Client <-> server wire schema for Dawn (ADR-0041, ADR-0042).
//!
//! [`ClientCommandJson`]/[`EventJson`] are the schema-of-record for every
//! message a client can send/receive over the WebSocket connection. They
//! live in their own leaf crate (dawn-core + serde + postcard only, no
//! transport/runtime dependency) so that both sides of the wire can depend
//! on the *same* types instead of maintaining parallel copies that can
//! drift:
//!
//! - `dawn-actor` (server) deserializes [`ClientMessage`] from the bytes a
//!   client sent, via [`parse_client_command`] (legacy JSON path) or
//!   `postcard::from_bytes` (binary envelope), and serializes
//!   [`ServerMessage`] back out.
//! - `dawn-client-gdext` (Godot client) constructs [`ClientMessage`] directly
//!   and serializes it out, and decodes a received [`ServerMessage`] back
//!   into Godot-facing data -- replacing the old pattern of hand-building a
//!   GDScript `Dictionary` that had to be kept in sync with this schema by
//!   eye.
//!
//! ```
//! use dawn_wire::{ClientCommandJson, PosJson};
//!
//! let cmd = ClientCommandJson::MoveCommand {
//!     target: PosJson { x: 10.0, y: 0.0, z: -5.0 },
//! };
//! let json = serde_json::to_string(&cmd).unwrap();
//! assert!(json.contains("\"MoveCommand\""));
//! ```
//!
//! # Binary envelope (ADR-0042)
//!
//! postcard has no self-describing type tag -- it can't deserialize an
//! internally tagged enum at all (no `deserialize_any`), so
//! `ClientCommandJson`/`EventJson` are externally tagged
//! (`{"VariantName": {...}}`, serde's default) rather than
//! `#[serde(tag = "type")]`. The wire also needs one outer enum per
//! direction that the receiver can decode without knowing the message kind
//! up front:
//!
//! ```
//! use dawn_wire::{ClientMessage, HelloMessage};
//!
//! let msg = ClientMessage::Hello(HelloMessage { resume: None });
//! let bytes = msg.encode();
//! let decoded = ClientMessage::decode(&bytes).unwrap();
//! assert!(matches!(decoded, ClientMessage::Hello(HelloMessage { resume: None })));
//! ```
//!
//! Stage 1 (this ADR) covers only the messages that already had a fixed Rust
//! type: `Welcome`/`Redirect`/`Event` (server -> client) and `Hello`/
//! `Command` (client -> server). `InitialState`/`AoiEnter`/`PlayerLoadout`
//! are still built as ad-hoc `serde_json::Value` in `dawn-sector`/
//! `dawn-simulation` and remain JSON text frames for now (stage 2, a
//! follow-up task, would give them fixed types and fold them into
//! [`ServerMessage`] too). WebSocket carries text and binary frames on the
//! same connection without conflict, so this split is not a compatibility
//! problem.

mod client_command;
mod hello_resume;
mod server_event;

pub use client_command::{
    client_command_from_json, client_command_json_schema, parse_client_command, ClientCommandJson,
    PosJson, VelJson, WarpTargetJson,
};
pub use hello_resume::{parse_hello, HelloMessage, ResumeIdentity};
pub use server_event::{
    domain_event_to_event_json, domain_event_to_json, event_json_schema, redirect_json, EventJson,
};

use serde::{Deserialize, Serialize};

/// Every message the server sends over the binary WebSocket envelope
/// (ADR-0042 stage 1). `InitialState`/`PlayerLoadout`/`AoiEnter` are not
/// members yet -- see the module docs.
#[derive(Debug, Serialize, Deserialize)]
pub enum ServerMessage {
    Welcome {
        player_id: u64,
        ship_id: u64,
    },
    Redirect {
        ws_addr: String,
        player_id: u64,
        ship_id: u64,
    },
    Event(EventJson),
}

impl ServerMessage {
    /// Postcard-encode this message into the bytes a binary WebSocket frame
    /// carries (ADR-0042). The single call site for this crate's `postcard`
    /// dependency on the server -> client side, so callers (`dawn-actor`,
    /// `dawn-client-gdext`) never invoke `postcard` directly.
    pub fn encode(&self) -> Vec<u8> {
        postcard::to_stdvec(self).unwrap_or_default()
    }

    /// Decode a binary WebSocket frame back into a [`ServerMessage`].
    pub fn decode(bytes: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(bytes)
    }
}

/// Every message a client sends over the binary WebSocket envelope
/// (ADR-0042 stage 1).
#[derive(Debug, Serialize, Deserialize)]
pub enum ClientMessage {
    Hello(HelloMessage),
    Command(ClientCommandJson),
}

impl ClientMessage {
    /// Postcard-encode this message into the bytes a binary WebSocket frame
    /// carries (ADR-0042). The single call site for this crate's `postcard`
    /// dependency on the client -> server side.
    pub fn encode(&self) -> Vec<u8> {
        postcard::to_stdvec(self).unwrap_or_default()
    }

    /// Decode a binary WebSocket frame back into a [`ClientMessage`].
    pub fn decode(bytes: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(bytes)
    }
}
