//! `Dict` -> `ShipInput` conversion for `WorldSession::register_ship`.
//!
//! `InitialState`/`AoiEnter`'s per-ship payload is a `Dictionary` the decoder
//! already produced from the decoded `ShipStateWire` (server_message_gd.rs).
//! This walks it directly, the same way `navigation_gd::navigation_input_from_dict`
//! does for `InitialState`'s navigation portion, so `main.gd` never needs to
//! `JSON.stringify` it back into text only for `serde_json` to parse it again
//! on this side of the FFI boundary (issue #178). Missing fields default the
//! same way the old `#[serde(default = ...)]` `Deserialize` impl did.

use dawn_client_core::{
    default_cap_max, default_cap_recharge, default_max_armor, default_max_hull, default_max_shield,
    ShipInput,
};

use crate::json_variant::Dict;
use crate::navigation_gd::{dict_f64, dict_f64_opt, dict_string};

pub(crate) fn ship_input_from_dict(d: &Dict) -> ShipInput {
    ShipInput {
        is_player: d
            .get("is_player")
            .and_then(|v| v.try_to::<bool>().ok())
            .unwrap_or(false),
        ship_type_name: dict_string(d, "ship_type_name", ""),
        max_shield: dict_f64(d, "max_shield", default_max_shield()),
        max_armor: dict_f64(d, "max_armor", default_max_armor()),
        max_hull: dict_f64(d, "max_hull", default_max_hull()),
        current_shield: dict_f64_opt(d, "current_shield"),
        current_armor: dict_f64_opt(d, "current_armor"),
        current_hull: dict_f64_opt(d, "current_hull"),
        cap_max: dict_f64(d, "cap_max", default_cap_max()),
        cap_recharge_per_tick: dict_f64(d, "cap_recharge_per_tick", default_cap_recharge()),
    }
}
