use dawn_wire::{ClientCommandJson, PosJson, WarpTargetJson};
use godot::prelude::*;

/// Matches the `Dict` alias used by `ItemRow`/`ModuleRow`/`PlayerLoadout`'s
/// `from_json` (gdext's `Dictionary` is generic over key/value element type).
type Dict = Dictionary<Variant, Variant>;

fn to_json_line(cmd: ClientCommandJson) -> GString {
    (&serde_json::to_string(&cmd).unwrap_or_default()).into()
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
fn positive_or_none(value: f32) -> Option<f32> {
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

/// Builds the client -> server wire JSON line for every command the Godot
/// client can send (ADR-0041). Each method returns the exact line
/// `connection.gd` should hand to `WebSocketPeer.send_text` (already
/// `"type"`-tagged, no further assembly needed).
///
/// Commands whose wire shape carries sentinel values (e.g. `<= 0.0` meaning
/// "server default", ADR-0031) or an exclusive-selection field pair (e.g.
/// `gate_id` xor `target_id`) get a dedicated method, since that logic is
/// domain semantics, not just field copying. Everything else -- a flat
/// struct of scalar fields with no such semantics -- goes through the
/// schema-driven [`Self::build`] instead, so adding one of those needs no
/// new method here (only a new `dawn-wire` variant and dispatch arm).
#[derive(GodotClass)]
#[class(init, base=RefCounted)]
pub struct ClientCommand {}

#[godot_api]
impl ClientCommand {
    #[func]
    fn move_command(&self, x: f32, y: f32, z: f32) -> GString {
        to_json_line(ClientCommandJson::MoveCommand {
            target: PosJson { x, y, z },
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
    ) -> GString {
        to_json_line(ClientCommandJson::ActivateModuleCommand {
            module_id: module_id as u32,
            slot: slot.to_string(),
            target_ship_id: non_negative_or_none(target_ship_id),
        })
    }

    #[func]
    fn approach_command(&self, target_id: i64) -> GString {
        to_json_line(ClientCommandJson::ApproachCommand {
            gate_id: None,
            target_id: Some(target_id as u64),
        })
    }

    #[func]
    fn approach_gate_command(&self, gate_id: i64) -> GString {
        to_json_line(ClientCommandJson::ApproachCommand {
            gate_id: Some(gate_id as u32),
            target_id: None,
        })
    }

    #[func]
    fn warp_command(&self, gate_id: i64) -> GString {
        to_json_line(ClientCommandJson::WarpCommand {
            target: Some(WarpTargetJson::Gate(gate_id as u32)),
            gate_id: None,
        })
    }

    #[func]
    fn warp_to_body_command(&self, body_id: i64) -> GString {
        to_json_line(ClientCommandJson::WarpCommand {
            target: Some(WarpTargetJson::Body(body_id as u32)),
            gate_id: None,
        })
    }

    /// `range_m <= 0.0` falls back to the server-side default (weapon
    /// range, ADR-0031).
    #[func]
    fn orbit_command(&self, target_id: i64, range_m: f32) -> GString {
        to_json_line(ClientCommandJson::OrbitCommand {
            gate_id: None,
            target_id: Some(target_id as u64),
            radius: positive_or_none(range_m),
        })
    }

    #[func]
    fn orbit_gate_command(&self, gate_id: i64, range_m: f32) -> GString {
        to_json_line(ClientCommandJson::OrbitCommand {
            gate_id: Some(gate_id as u32),
            target_id: None,
            radius: positive_or_none(range_m),
        })
    }

    #[func]
    fn keep_at_range_command(&self, target_id: i64, range_m: f32) -> GString {
        to_json_line(ClientCommandJson::KeepAtRangeCommand {
            gate_id: None,
            target_id: Some(target_id as u64),
            range: positive_or_none(range_m),
        })
    }

    #[func]
    fn keep_at_range_gate_command(&self, gate_id: i64, range_m: f32) -> GString {
        to_json_line(ClientCommandJson::KeepAtRangeCommand {
            gate_id: Some(gate_id as u32),
            target_id: None,
            range: positive_or_none(range_m),
        })
    }

    /// Schema-driven builder for commands whose wire shape is a flat
    /// scalar-fields-only struct (no sentinel/exclusive-selection semantics --
    /// see the dedicated methods above and below for those). `kind` is the
    /// `ClientCommandJson` variant name (its serde `"type"` tag, e.g.
    /// `"DockCommand"`); `fields` supplies that variant's other fields by
    /// name. Validates by deserializing into `ClientCommandJson` itself, so
    /// an unknown `kind` or a wrong/missing field is caught here rather than
    /// producing a silently-malformed wire line.
    #[func]
    fn build(&self, kind: GString, fields: Dict) -> GString {
        let Some(mut object) = scalar_dict_to_json_object(&fields) else {
            return GString::new();
        };
        object.insert(
            "type".to_string(),
            serde_json::Value::from(kind.to_string()),
        );
        match serde_json::from_value::<ClientCommandJson>(serde_json::Value::Object(object)) {
            Ok(cmd) => to_json_line(cmd),
            Err(err) => {
                godot_error!("ClientCommand.build({kind}): {err}");
                GString::new()
            }
        }
    }

    /// Move the entire stack of an item out of a docked ship's own cargo
    /// into the caller's station inventory (ADR-0034 9B). `item_type` is
    /// one of `"Module"`/`"PackagedShip"`/`"ScrapMetal"` (matches
    /// `ItemRow.item_type`); `module_id`/`ship_type_id` are only meaningful
    /// for the matching variant (`0` otherwise).
    #[func]
    fn transfer_to_station_command(
        &self,
        ship_id: i64,
        station_id: i64,
        item_type: GString,
        module_id: i64,
        ship_type_id: i64,
    ) -> GString {
        self.transfer_command(
            ship_id,
            station_id,
            item_type,
            module_id,
            ship_type_id,
            "ToStation",
        )
    }

    /// The reverse of `transfer_to_station_command`: move the entire stack
    /// back into the docked ship's own cargo.
    #[func]
    fn transfer_from_station_command(
        &self,
        ship_id: i64,
        station_id: i64,
        item_type: GString,
        module_id: i64,
        ship_type_id: i64,
    ) -> GString {
        self.transfer_command(
            ship_id,
            station_id,
            item_type,
            module_id,
            ship_type_id,
            "ToShip",
        )
    }
}

impl ClientCommand {
    fn transfer_command(
        &self,
        ship_id: i64,
        station_id: i64,
        item_type: GString,
        module_id: i64,
        ship_type_id: i64,
        direction: &str,
    ) -> GString {
        to_json_line(ClientCommandJson::TransferToStationCommand {
            ship_id: ship_id as u64,
            station_id: station_id as u32,
            item_type: item_type.to_string(),
            module_id: module_id as u32,
            ship_type_id: ship_type_id as u32,
            direction: direction.to_string(),
        })
    }
}
