//! Client <-> server wire schema for Dawn (ADR-0041, ADR-0042).
//!
//! [`ClientCommandWire`]/[`EventWire`] are the schema-of-record for every
//! message a client can send/receive over the WebSocket connection. They
//! live in their own leaf crate (dawn-core + serde + postcard only, no
//! transport/runtime dependency) so that both sides of the wire can depend
//! on the *same* types instead of maintaining parallel copies that can
//! drift:
//!
//! - `dawn-actor` (server) decodes [`ClientMessage`] from the bytes a
//!   client sent via `postcard::from_bytes` (the binary envelope) and
//!   serializes [`ServerMessage`] back out.
//! - `dawn-client-gdext` (Godot client) constructs [`ClientMessage`] directly
//!   and serializes it out, and decodes a received [`ServerMessage`] back
//!   into Godot-facing data -- replacing the old pattern of hand-building a
//!   GDScript `Dictionary` that had to be kept in sync with this schema by
//!   eye.
//!
//! ```
//! use dawn_wire::{ClientCommandWire, PosWire};
//!
//! let cmd = ClientCommandWire::MoveCommand {
//!     target: PosWire { x: 10.0, y: 0.0, z: -5.0 },
//! };
//! let json = serde_json::to_string(&cmd).unwrap();
//! assert!(json.contains("\"MoveCommand\""));
//! ```
//!
//! # Binary envelope (ADR-0042)
//!
//! postcard has no self-describing type tag -- it can't deserialize an
//! internally tagged enum at all (no `deserialize_any`), so
//! `ClientCommandWire`/`EventWire` are externally tagged
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
//! The server -> client direction round-trips the same way through
//! [`ServerMessage::Event`]:
//!
//! ```
//! use dawn_wire::{EventWire, ServerMessage};
//!
//! let msg = ServerMessage::Event(EventWire::ShipDespawned { ship_id: 7, tick: 1 });
//! let bytes = msg.encode();
//! let decoded = ServerMessage::decode(&bytes).unwrap();
//! assert!(matches!(
//!     decoded,
//!     ServerMessage::Event(EventWire::ShipDespawned { ship_id: 7, tick: 1 })
//! ));
//! ```
//!
//! Stage 1 covered the messages that already had a fixed Rust type:
//! `Welcome`/`Redirect`/`Event` (server -> client) and `Hello`/`Command`
//! (client -> server). Stage 2 folds in the remaining ad-hoc
//! `serde_json::Value` messages one at a time; 2a ([`PlayerLoadoutWire`]),
//! 2b ([`InitialStateWire`]), and 2c (`AoiEnter`/`AoiLeave`/`PositionSnap`)
//! are done. Every server -> client message now travels through the binary
//! envelope; there is no more ad-hoc JSON text path.
//!
//! # Naming convention
//!
//! Wire schema types use a `*Wire` suffix (`EventWire`, `ClientCommandWire`,
//! `PosWire`, ...), not `*Json` -- ADR-0042 moved every message onto the
//! postcard binary envelope, so a `Json` suffix would misdescribe what these
//! types actually carry on the wire today. Real JSON-producing helpers (the
//! `*_wire_json_schema()` doc-generation functions, which really do render a
//! JSON Schema document) keep `json` in their names since that's accurate.

mod client_command;
mod hello_resume;
mod initial_state;
mod market;
mod player_loadout;
mod server_event;

pub use client_command::{
    client_command_from_wire, client_command_wire_json_schema, ClientCommandWire, PosWire, VelWire,
    WarpTargetWire,
};
pub use hello_resume::{HelloMessage, ResumeIdentity};
pub use initial_state::{
    AbsPosWire, BuildableShipTypeWire, CelestialBodyWire, InitialStateWire, JumpGateWire,
    ShipStateWire, StationWire, SystemWire,
};
pub use market::{
    market_command_wire_json_schema, MarketCommandWire, MarketOrderWire, MarketSnapshotWire,
};
pub use player_loadout::{
    ItemRowWire, ModuleRowWire, OwnedShipRowWire, PlayerLoadoutWire, SlotCapacityWire,
};
pub use server_event::{domain_event_to_event_wire, event_wire_json_schema, EventWire};

use serde::{Deserialize, Serialize};

/// Every message the server sends over the binary WebSocket envelope
/// (ADR-0042).
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
    Event(EventWire),
    PlayerLoadout(PlayerLoadoutWire),
    InitialState(InitialStateWire),
    /// A ship just entered an observer's Area-of-Interest neighborhood
    /// (ADR-0019/ADR-0042 stage 2c).
    AoiEnter(ShipStateWire),
    /// A ship left an observer's Area-of-Interest neighborhood (ADR-0019/
    /// ADR-0042 stage 2c). Carries only the id -- the client already knows
    /// everything else about a ship it previously saw.
    AoiLeave {
        ship_id: u64,
    },
    /// Authoritative absolute position for a ship, sent on warp arrival
    /// (ADR-0029) to correct the client's capped warp-visual dead-reckoning.
    PositionSnap {
        ship_id: u64,
        position: AbsPosWire,
    },
    /// Owner-only normal-flight correction for client-side prediction
    /// (Phase 10 / ADR-0043). Warp and docking discontinuities use
    /// `PositionSnap` or domain events instead.
    MotionCorrection {
        ship_id: u64,
        position: AbsPosWire,
        velocity: VelWire,
        tick: u64,
    },
    MarketSnapshot(MarketSnapshotWire),
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
    Command(ClientCommandWire),
    Market(MarketCommandWire),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motion_correction_round_trips_through_the_binary_envelope() {
        let message = ServerMessage::MotionCorrection {
            ship_id: 7,
            position: AbsPosWire {
                x: 1.0e12,
                y: -2.0,
                z: 3.5,
            },
            velocity: VelWire {
                dx: 4.0,
                dy: 5.0,
                dz: -6.0,
            },
            tick: 42,
        };

        let decoded = ServerMessage::decode(&message.encode()).expect("valid postcard message");
        assert!(matches!(
            decoded,
            ServerMessage::MotionCorrection {
                ship_id: 7,
                tick: 42,
                ..
            }
        ));
    }
}
