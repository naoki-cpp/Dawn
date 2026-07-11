use godot::prelude::*;

/// Matches the `Dict` alias used throughout this crate (gdext's `Dictionary`
/// is generic over key/value element type).
pub type Dict = Dictionary<Variant, Variant>;

/// Mirrors the Variant shape `JSON.parse_string()` already produces on the
/// GDScript side (numbers as `float`, per Godot's own JSON parser) so
/// existing consumers (`main.gd`, `hud_manager.gd`, GdUnit4 tests) that were
/// written against JSON-parsed Dictionaries need no changes. Shared by
/// [`crate::ServerMessageDecoder`] (server -> client) and
/// [`crate::client_command_gd::ClientMessageDecoder`] (test-only,
/// client -> server) since both decode a serde type into the same shape.
pub fn json_value_to_variant(value: &serde_json::Value) -> Variant {
    match value {
        serde_json::Value::Null => Variant::nil(),
        serde_json::Value::Bool(b) => Variant::from(*b),
        serde_json::Value::Number(n) => Variant::from(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => Variant::from(s.as_str()),
        serde_json::Value::Array(items) => {
            let mut arr = Array::<Variant>::new();
            for item in items {
                arr.push(&json_value_to_variant(item));
            }
            Variant::from(arr)
        }
        serde_json::Value::Object(fields) => {
            let mut dict = Dict::new();
            for (key, val) in fields {
                dict.set(key.as_str(), &json_value_to_variant(val));
            }
            Variant::from(dict)
        }
    }
}

/// Converts any `Serialize` struct into a `Dict`, logging a Godot error and
/// returning an empty `Dict` if serialization fails (mirrors the empty-Dict-
/// on-failure contract `ServerMessageDecoder::decode` already exposes to
/// callers). `context` names the type being converted, for the error message
/// -- callers still own tagging (`d.set("type", ...)`) and nesting (e.g.
/// `AoiEnter`'s `"ship"` key) themselves, since those differ per caller and
/// aren't part of what this function does.
pub fn struct_to_dict(value: &impl serde::Serialize, context: &str) -> Dict {
    match serde_json::to_value(value) {
        Ok(v) => json_value_to_variant(&v).to::<Dict>(),
        Err(err) => {
            godot_error!("ServerMessageDecoder.decode: {context} -> JSON failed: {err}");
            Dict::new()
        }
    }
}

/// Converts an externally tagged serde value (`{"<VariantName>": {...fields}}`,
/// the shape `ClientCommandWire`/`EventWire` serialize to since ADR-0042
/// dropped `#[serde(tag = "type")]` for postcard compatibility) into the
/// `{"type": "<VariantName>", ...fields}` Dictionary shape GDScript
/// consumers were already written against (back when these types were
/// internally tagged JSON). `value` must be a single-key object -- true for
/// every variant of both enums, since all of them are struct-like (`Foo {
/// .. }`), never unit or tuple variants.
pub fn externally_tagged_to_dict(value: &serde_json::Value) -> Dict {
    let serde_json::Value::Object(outer) = value else {
        return Dict::new();
    };
    let Some((variant_name, inner)) = outer.iter().next() else {
        return Dict::new();
    };
    let mut dict = json_value_to_variant(inner).to::<Dict>();
    dict.set("type", variant_name.as_str());
    dict
}
