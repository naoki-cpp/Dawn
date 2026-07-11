use crate::json_variant::{externally_tagged_to_dict, struct_to_dict, Dict};
use dawn_wire::ServerMessage;
use godot::prelude::*;

/// Decodes the binary WebSocket envelope the server sends (ADR-0042) into a
/// plain Godot `Dictionary` shaped exactly like the pre-ADR-0042 JSON
/// messages (`{"type": "...", ...fields}`), so `connection.gd::_handle_message`'s
/// existing `"type"`-keyed dispatch needed no changes as each message kind
/// migrated off ad-hoc JSON text onto this binary envelope.
///
/// GDScript itself cannot decode postcard -- it has no self-describing type
/// tag, unlike JSON, so the byte layout only makes sense once decoded
/// against the known `ServerMessage` Rust type. That decode step has to
/// happen here, in the GDExtension binding.
#[derive(GodotClass)]
#[class(init, base=RefCounted)]
pub struct ServerMessageDecoder {}

#[godot_api]
impl ServerMessageDecoder {
    /// Decode one binary WebSocket frame. Returns an empty `Dictionary` and
    /// logs a Godot error if `bytes` is not a valid `ServerMessage` (e.g. a
    /// version mismatch or corrupted frame) -- callers should treat an empty
    /// result the same as a failed `JSON.parse_string`.
    #[func]
    fn decode(&self, bytes: PackedByteArray) -> Dict {
        match ServerMessage::decode(bytes.as_slice()) {
            Ok(msg) => server_message_to_dict(&msg),
            Err(err) => {
                godot_error!("ServerMessageDecoder.decode: {err}");
                Dict::new()
            }
        }
    }
}

fn server_message_to_dict(msg: &ServerMessage) -> Dict {
    match msg {
        ServerMessage::Welcome { player_id, ship_id } => {
            let mut d = Dict::new();
            d.set("type", "Welcome");
            d.set("player_id", *player_id as f64);
            d.set("ship_id", *ship_id as f64);
            d
        }
        ServerMessage::Redirect {
            ws_addr,
            player_id,
            ship_id,
        } => {
            let mut d = Dict::new();
            d.set("type", "Redirect");
            d.set("ws_addr", ws_addr.as_str());
            d.set("player_id", *player_id as f64);
            d.set("ship_id", *ship_id as f64);
            d
        }
        // EventWire serializes externally tagged (ADR-0042, postcard can't
        // deserialize internal tagging), so converting to serde_json::Value
        // first and re-tagging it as {"type": ..., ...} reuses the existing
        // derive instead of hand-writing a match arm per DomainEvent variant
        // here too.
        ServerMessage::Event(event) => match serde_json::to_value(event) {
            Ok(value) => externally_tagged_to_dict(&value),
            Err(err) => {
                godot_error!("ServerMessageDecoder.decode: EventWire -> JSON failed: {err}");
                Dict::new()
            }
        },
        // PlayerLoadout is deliberately not materialized into a Dictionary
        // here: `PlayerLoadout.apply_wire_bytes` (loadout_gd.rs) decodes the
        // same raw bytes directly into typed Rust state for precision and
        // richer behavior (capacitor simulation, etc). This is just the
        // dispatch tag connection.gd's `_handle_message` needs to route the
        // raw bytes there.
        ServerMessage::PlayerLoadout(_) => {
            let mut d = Dict::new();
            d.set("type", "PlayerLoadout");
            d
        }
        // InitialState is a plain struct (not an enum like EventWire), so it
        // serializes to a flat JSON object with no wrapping variant name --
        // convert it directly and stamp "type" on, rather than going through
        // externally_tagged_to_dict (which expects a single-key wrapper).
        ServerMessage::InitialState(state) => {
            let mut d = struct_to_dict(state, "InitialStateWire");
            d.set("type", "InitialState");
            d
        }
        // Nested under "ship" to match the shape main.gd's _handle_aoi_enter
        // already expects (unchanged since the ADR-0042 stage 2c migration).
        ServerMessage::AoiEnter(ship) => {
            let mut d = Dict::new();
            d.set("type", "AoiEnter");
            d.set("ship", &struct_to_dict(ship, "ShipStateWire"));
            d
        }
        ServerMessage::AoiLeave { ship_id } => {
            let mut d = Dict::new();
            d.set("type", "AoiLeave");
            d.set("ship_id", *ship_id as f64);
            d
        }
        ServerMessage::PositionSnap { ship_id, position } => {
            let mut d = Dict::new();
            d.set("type", "PositionSnap");
            d.set("ship_id", *ship_id as f64);
            let mut pos = Dict::new();
            pos.set("x", position.x);
            pos.set("y", position.y);
            pos.set("z", position.z);
            d.set("position", &pos);
            d
        }
    }
}
