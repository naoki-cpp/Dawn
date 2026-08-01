//! `Dict` -> `NavigationInput` conversion for `WorldSession::ingest_navigation`.
//!
//! `InitialState`'s navigation payload (system names, jump gates, stations,
//! celestial bodies, buildable ship types) is a nested structure, not flat
//! scalars. The inbound typed outcome projects it directly from
//! `InitialStateWire` into this Godot-facing `Dictionary`; this module then
//! converts that boundary object into the pure Rust client model. No JSON
//! value or text round-trip exists on the runtime path.
//! Missing fields default the same way the old `#[serde(default)]`
//! `Deserialize` impls did. Wrong-typed fields are *not* equivalent to the
//! old behavior: the old JSON parse failed outright on a type mismatch and
//! `ingest_navigation` returned `false` without touching state, whereas
//! here a wrong-typed value quietly falls back to its default and the
//! ingest still proceeds. Harmless today since production values always
//! come from the typed `InitialStateWire`, not hand-built JSON, but a
//! caller relying on the old fail-closed behavior would be surprised.

use dawn_client_core::{
    BuildableShipTypeInput, CelestialBodyInput, GateInput, NavigationInput, PositionInput,
    StationInput, SystemNameInput,
};
use godot::prelude::*;

use crate::json_variant::Dict;

pub(crate) fn dict_f64(d: &Dict, key: &str, default: f64) -> f64 {
    d.get(key)
        .and_then(|v| v.try_to::<f64>().ok())
        .unwrap_or(default)
}

pub(crate) fn dict_i64(d: &Dict, key: &str, default: i64) -> i64 {
    d.get(key)
        .and_then(|v| v.try_to::<i64>().ok())
        .unwrap_or(default)
}

pub(crate) fn dict_string(d: &Dict, key: &str, default: &str) -> String {
    d.get(key)
        .and_then(|v| v.try_to::<GString>().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| default.to_string())
}

/// Like `dict_f64`, but distinguishes "key absent" (`None`) from "key
/// present" (`Some`) instead of collapsing both to a default value --
/// mirrors the old `#[serde(default)] Option<f64>` semantics for fields
/// where a caller-visible "unset" is meaningful (e.g. `ShipInput`'s
/// `current_shield`/`current_armor`/`current_hull`).
pub(crate) fn dict_f64_opt(d: &Dict, key: &str) -> Option<f64> {
    d.get(key).and_then(|v| v.try_to::<f64>().ok())
}

pub(crate) fn dict_sub(d: &Dict, key: &str) -> Dict {
    d.get(key)
        .and_then(|v| v.try_to::<Dict>().ok())
        .unwrap_or_default()
}

pub(crate) fn dict_array(d: &Dict, key: &str) -> Array<Variant> {
    d.get(key)
        .and_then(|v| v.try_to::<Array<Variant>>().ok())
        .unwrap_or_default()
}

pub(crate) fn vec_from_array<T>(arr: Array<Variant>, from_dict: impl Fn(&Dict) -> T) -> Vec<T> {
    arr.iter_shared()
        .filter_map(|v| v.try_to::<Dict>().ok())
        .map(|d| from_dict(&d))
        .collect()
}

fn position_from_dict(d: &Dict) -> PositionInput {
    PositionInput {
        x: dict_f64(d, "x", 0.0),
        y: dict_f64(d, "y", 0.0),
        z: dict_f64(d, "z", 0.0),
    }
}

fn system_name_from_dict(d: &Dict) -> SystemNameInput {
    SystemNameInput {
        id: dict_i64(d, "id", 0),
        name: dict_string(d, "name", ""),
    }
}

fn gate_from_dict(d: &Dict) -> GateInput {
    GateInput {
        gate_id: dict_i64(d, "gate_id", 0),
        position: position_from_dict(&dict_sub(d, "position")),
        activation_radius: dict_f64(d, "activation_radius", 0.0),
        to_system_name: dict_string(d, "to_system_name", ""),
    }
}

fn station_from_dict(d: &Dict) -> StationInput {
    StationInput {
        station_id: dict_i64(d, "station_id", 0),
        name: dict_string(d, "name", ""),
        position: position_from_dict(&dict_sub(d, "position")),
        docking_radius: dict_f64(d, "docking_radius", 0.0),
    }
}

fn celestial_body_from_dict(d: &Dict) -> CelestialBodyInput {
    CelestialBodyInput {
        id: dict_i64(d, "id", 0),
        kind: dict_string(d, "kind", ""),
        name: dict_string(d, "name", ""),
        position: position_from_dict(&dict_sub(d, "position")),
        radius: dict_f64(d, "radius", 1.0),
        spectral_type: dict_f64(d, "spectral_type", 0.0),
    }
}

fn buildable_ship_type_from_dict(d: &Dict) -> BuildableShipTypeInput {
    BuildableShipTypeInput {
        ship_type_id: dict_i64(d, "ship_type_id", 0),
        name: dict_string(d, "name", ""),
    }
}

pub(crate) fn navigation_input_from_dict(d: &Dict) -> NavigationInput {
    NavigationInput {
        system_name: dict_string(d, "system_name", "Unknown"),
        systems: vec_from_array(dict_array(d, "systems"), system_name_from_dict),
        jump_gates: vec_from_array(dict_array(d, "jump_gates"), gate_from_dict),
        stations: vec_from_array(dict_array(d, "stations"), station_from_dict),
        celestial_bodies: vec_from_array(
            dict_array(d, "celestial_bodies"),
            celestial_body_from_dict,
        ),
        buildable_ship_types: vec_from_array(
            dict_array(d, "buildable_ship_types"),
            buildable_ship_type_from_dict,
        ),
    }
}

// No #[cfg(test)] module here: constructing a real Godot `Dictionary`/
// `Variant` panics without a live Godot engine ("Godot engine not
// available"), which plain `cargo test` never provides -- the same reason
// this crate's other Rust tests (`loadout_gd.rs`) carefully avoid touching
// Godot types. `world_session_test.gd`'s
// `test_ingest_navigation_preserves_absolute_f64_positions` (gdUnit4, runs
// inside the real Godot runtime) exercises this conversion instead.
