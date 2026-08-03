use crate::item_identity_gd::ItemIdentity;
use dawn_wire::{
    ClientCommandWire, ClientMessage, HelloMessage, ItemWire, MarketCommandWire,
    NavigationTargetWire, PosWire, ResumeTicket, WarpTargetWire,
};
use godot::prelude::*;

type Dict = Dictionary<Variant, Variant>;

/// Postcard-encode a [`ClientMessage`] into the bytes `connection.gd` sends
/// as a binary WebSocket frame (ADR-0042).
fn to_wire_bytes(msg: &ClientMessage) -> PackedByteArray {
    PackedByteArray::from(msg.encode().as_slice())
}

fn command_wire_bytes(cmd: ClientCommandWire) -> PackedByteArray {
    to_wire_bytes(&ClientMessage::Command(cmd))
}

fn market_command_wire_bytes(cmd: MarketCommandWire) -> PackedByteArray {
    to_wire_bytes(&ClientMessage::Market(cmd))
}

/// Converts a flat GDScript `Dictionary` (scalar values only -- `int`/
/// `float`/`String`/`bool`) into a `serde_json::Value` object, for
/// [`ClientCommand::build`]. Nested `Dictionary`/`Array` values are rejected
/// (`None`) rather than silently dropped: none of today's schema-driven
/// commands need them, so support is added only when a real command does
/// (see the design discussion in ADR-0041's follow-up note).
fn scalar_dict_to_json_object(fields: &Dict) -> Option<serde_json::Map<String, serde_json::Value>> {
    let mut map = serde_json::Map::with_capacity(fields.len());
    for (key, value) in fields.iter_shared() {
        let key: String = key.to::<GString>().to_string();
        let value = match value.get_type() {
            VariantType::INT => serde_json::Value::from(value.to::<i64>()),
            VariantType::FLOAT => serde_json::Value::from(value.to::<f64>()),
            VariantType::STRING | VariantType::STRING_NAME => {
                serde_json::Value::from(value.to::<GString>().to_string())
            }
            VariantType::BOOL => serde_json::Value::from(value.to::<bool>()),
            _ => {
                godot_error!(
                    "ClientCommand.build: field '{key}' has unsupported type {:?} (scalars only)",
                    value.get_type()
                );
                return None;
            }
        };
        map.insert(key, value);
    }
    Some(map)
}

/// Range/radius sentinel used throughout `connection.gd`'s public API:
/// `<= 0.0` means "no override, let the server pick its default"
/// (ADR-0031). Kept as a free function so every `*_command` below applies
/// the same rule instead of repeating the comparison.
fn positive_or_none(value: f64) -> Option<f64> {
    if value > 0.0 {
        Some(value)
    } else {
        None
    }
}

/// Ship-id sentinel used by `ActivateModuleCommand`'s optional target:
/// `< 0` means "no target" (ADR-0035).
fn non_negative_or_none(value: i64) -> Option<u64> {
    if value >= 0 {
        Some(value as u64)
    } else {
        None
    }
}

fn item_wire(item_id: &Gd<ItemIdentity>) -> ItemWire {
    item_id.bind().get().into()
}

/// Builds the client -> server wire message for every command the Godot
/// client can send (ADR-0041), postcard-encoded as a `ClientMessage::Command`
/// envelope (ADR-0042). Each method returns the exact bytes `connection.gd`
/// should hand to `WebSocketPeer.send` with `WRITE_MODE_BINARY`.
///
/// Commands whose wire shape carries sentinel values (e.g. `<= 0.0` meaning
/// "server default", ADR-0031) or a tagged navigation target get a dedicated
/// method, since that logic is domain semantics, not just field copying.
/// Everything else -- a flat struct of scalar fields with no such semantics --
/// goes through the schema-driven `build` method instead.
#[derive(GodotClass)]
#[class(init, base=RefCounted)]
pub struct ClientCommand {}

#[godot_api]
impl ClientCommand {
    #[func]
    fn move_command(&self, x: f64, y: f64, z: f64) -> PackedByteArray {
        command_wire_bytes(ClientCommandWire::MoveCommand {
            target: PosWire { x, y, z },
        })
    }

    /// `target_ship_id < 0` means no target (ADR-0035) -- required only for
    /// targeted module kinds (Weapon/Tackle), validated server-side.
    #[func]
    fn activate_module_command(
        &self,
        module_id: i64,
        slot: GString,
        target_ship_id: i64,
    ) -> PackedByteArray {
        command_wire_bytes(ClientCommandWire::ActivateModuleCommand {
            module_id: module_id as u32,
            slot: slot.to_string(),
            target_ship_id: non_negative_or_none(target_ship_id),
        })
    }

    #[func]
    fn approach_command(&self, target_id: i64) -> PackedByteArray {
        command_wire_bytes(ClientCommandWire::ApproachCommand {
            target: NavigationTargetWire::Ship(target_id as u64),
        })
    }

    #[func]
    fn approach_gate_command(&self, gate_id: i64) -> PackedByteArray {
        command_wire_bytes(ClientCommandWire::ApproachCommand {
            target: NavigationTargetWire::Gate(gate_id as u32),
        })
    }

    #[func]
    fn warp_command(&self, gate_id: i64) -> PackedByteArray {
        command_wire_bytes(ClientCommandWire::WarpCommand {
            target: WarpTargetWire::Gate(gate_id as u32),
        })
    }

    #[func]
    fn warp_to_body_command(&self, body_id: i64) -> PackedByteArray {
        command_wire_bytes(ClientCommandWire::WarpCommand {
            target: WarpTargetWire::Body(body_id as u32),
        })
    }

    /// `range_m <= 0.0` falls back to the server-side default (weapon
    /// range, ADR-0031).
    #[func]
    fn orbit_command(&self, target_id: i64, range_m: f64) -> PackedByteArray {
        command_wire_bytes(ClientCommandWire::OrbitCommand {
            target: NavigationTargetWire::Ship(target_id as u64),
            radius: positive_or_none(range_m),
        })
    }

    #[func]
    fn orbit_gate_command(&self, gate_id: i64, range_m: f64) -> PackedByteArray {
        command_wire_bytes(ClientCommandWire::OrbitCommand {
            target: NavigationTargetWire::Gate(gate_id as u32),
            radius: positive_or_none(range_m),
        })
    }

    #[func]
    fn keep_at_range_command(&self, target_id: i64, range_m: f64) -> PackedByteArray {
        command_wire_bytes(ClientCommandWire::KeepAtRangeCommand {
            target: NavigationTargetWire::Ship(target_id as u64),
            range: positive_or_none(range_m),
        })
    }

    #[func]
    fn keep_at_range_gate_command(&self, gate_id: i64, range_m: f64) -> PackedByteArray {
        command_wire_bytes(ClientCommandWire::KeepAtRangeCommand {
            target: NavigationTargetWire::Gate(gate_id as u32),
            range: positive_or_none(range_m),
        })
    }

    /// Schema-driven builder for commands whose wire shape is a flat
    /// scalar-fields-only struct (no sentinel/tagged-target semantics --
    /// see the dedicated methods above and below for those). `kind` is the
    /// `ClientCommandWire` variant name (e.g. `"DockCommand"`); `fields`
    /// supplies that variant's fields by name.
    #[func]
    fn build(&self, kind: GString, fields: Dict) -> PackedByteArray {
        let Some(fields) = scalar_dict_to_json_object(&fields) else {
            return PackedByteArray::new();
        };
        let mut wrapper = serde_json::Map::with_capacity(1);
        wrapper.insert(kind.to_string(), serde_json::Value::Object(fields));
        match serde_json::from_value::<ClientCommandWire>(serde_json::Value::Object(wrapper)) {
            Ok(cmd) => command_wire_bytes(cmd),
            Err(err) => {
                godot_error!("ClientCommand.build({kind}): {err}");
                PackedByteArray::new()
            }
        }
    }

    /// Schema-driven builder for the Market-only request envelope. Market
    /// requests intentionally do not enter `ClientCommandWire` or the Sector
    /// command stream (ADR-0034).
    #[func]
    fn market_build(&self, kind: GString, fields: Dict) -> PackedByteArray {
        let Some(fields) = scalar_dict_to_json_object(&fields) else {
            return PackedByteArray::new();
        };
        let mut wrapper = serde_json::Map::with_capacity(1);
        wrapper.insert(kind.to_string(), serde_json::Value::Object(fields));
        match serde_json::from_value::<MarketCommandWire>(serde_json::Value::Object(wrapper)) {
            Ok(command) => market_command_wire_bytes(command),
            Err(err) => {
                godot_error!("ClientCommand.market_build({kind}): {err}");
                PackedByteArray::new()
            }
        }
    }

    #[func]
    fn transfer_to_station_command(
        &self,
        ship_id: i64,
        station_id: i64,
        item_id: Gd<ItemIdentity>,
    ) -> PackedByteArray {
        self.transfer_command(ship_id, station_id, item_id, "ToStation")
    }

    /// The reverse of `transfer_to_station_command`: move the entire stack
    /// back into the docked ship's own cargo.
    #[func]
    fn transfer_from_station_command(
        &self,
        ship_id: i64,
        station_id: i64,
        item_id: Gd<ItemIdentity>,
    ) -> PackedByteArray {
        self.transfer_command(ship_id, station_id, item_id, "ToShip")
    }

    #[func]
    fn market_place_order_command(
        &self,
        ship_id: i64,
        item_id: Gd<ItemIdentity>,
        side: GString,
        price: i64,
        quantity: i64,
    ) -> PackedByteArray {
        let (Ok(ship_id), Ok(price), Ok(quantity)) = (
            u64::try_from(ship_id),
            u64::try_from(price),
            u64::try_from(quantity),
        ) else {
            godot_error!(
                "ClientCommand.market_place_order_command: ship, price, and quantity must be non-negative"
            );
            return PackedByteArray::new();
        };
        market_command_wire_bytes(MarketCommandWire::PlaceMarketOrderCommand {
            ship_id,
            item_id: item_wire(&item_id),
            side: side.to_string(),
            price,
            quantity,
        })
    }

    /// Build the `ClientMessage::Hello` binary frame `connection.gd` sends
    /// on connect/reconnect (ADR-0007 §2, ADR-0042). `player_id < 0` means a
    /// fresh connection (no resume identity); both `player_id`/`ship_id`
    /// must be non-negative to resume (following a `Redirect`).
    #[func]
    fn hello_command(&self, resume_ticket: PackedByteArray) -> PackedByteArray {
        let resume = match resume_ticket.to_vec().try_into() {
            Ok(bytes) => Some(ResumeTicket::from_bytes(bytes)),
            Err(bytes) if bytes.is_empty() => None,
            Err(_) => {
                godot_error!(
                    "ClientCommand.hello_command: resume ticket must be empty or 32 bytes"
                );
                return PackedByteArray::new();
            }
        };
        to_wire_bytes(&ClientMessage::Hello(HelloMessage { resume }))
    }
}

impl ClientCommand {
    fn transfer_command(
        &self,
        ship_id: i64,
        station_id: i64,
        item_id: Gd<ItemIdentity>,
        direction: &str,
    ) -> PackedByteArray {
        let (Ok(ship_id), Ok(station_id)) = (u64::try_from(ship_id), u32::try_from(station_id))
        else {
            godot_error!("ClientCommand.transfer_command: invalid ship or station ID");
            return PackedByteArray::new();
        };
        command_wire_bytes(ClientCommandWire::TransferToStationCommand {
            ship_id,
            station_id,
            item_id: item_wire(&item_id),
            direction: direction.to_string(),
        })
    }
}
