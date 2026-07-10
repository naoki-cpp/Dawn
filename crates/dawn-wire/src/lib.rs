//! Client -> server wire schema for Dawn (ADR-0041).
//!
//! [`ClientCommandJson`] is the schema-of-record for every message a client
//! can send over the WebSocket connection. It lives in its own leaf crate
//! (dawn-core + serde only, no transport/runtime dependency) so that both
//! sides of the wire can depend on the *same* type instead of maintaining
//! parallel copies that can drift:
//!
//! - `dawn-actor` (server) deserializes it from the raw JSON line a client
//!   sent, via [`parse_client_command`].
//! - `dawn-client-gdext` (Godot client) constructs it directly and
//!   serializes it back out, replacing the old pattern of hand-building a
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
//! assert!(json.contains("\"type\":\"MoveCommand\""));
//! ```

mod client_command;

pub use client_command::{
    client_command_json_schema, parse_client_command, ClientCommandJson, PosJson, VelJson,
    WarpTargetJson,
};
