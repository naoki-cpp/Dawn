//! Backward-compatible game-data loading entry points.
//!
//! Production paths reach the process-wide strict
//! [`crate::game_data::GameDataCatalog`]. Custom paths retain the historical
//! category-only loading behavior, but validation is strict and fallback values
//! are deliberately ignored.

use crate::game_data::{
    load_modules_file, load_ship_types_file, runtime_catalog, PRODUCTION_MODULES_PATH,
    PRODUCTION_SHIP_TYPES_PATH,
};
use dawn_core::fitting::ModuleDefinition;
use dawn_core::ship_type::ShipTypeDefinition;

/// Load authoritative module definitions.
///
/// The production path returns the module half of the process-wide catalog.
/// A custom path strictly parses and validates that module file by itself.
/// Missing, malformed, or invalid data aborts instead of selecting `fallback`.
pub fn load_modules(path: &str, _fallback: Vec<ModuleDefinition>) -> Vec<ModuleDefinition> {
    if path == PRODUCTION_MODULES_PATH {
        return runtime_catalog()
            .unwrap_or_else(|error| panic!("failed to load required game-data catalog: {error}"))
            .modules()
            .to_vec();
    }

    load_modules_file(path)
        .unwrap_or_else(|error| panic!("failed to load required module game data: {error}"))
}

/// Load authoritative ship-type definitions.
///
/// The production path returns the ship-type half of the process-wide catalog.
/// A custom path strictly parses and validates that ship-type file by itself.
/// Missing, malformed, or invalid data aborts instead of selecting `fallback`.
pub fn load_ship_types(path: &str, _fallback: Vec<ShipTypeDefinition>) -> Vec<ShipTypeDefinition> {
    if path == PRODUCTION_SHIP_TYPES_PATH {
        return runtime_catalog()
            .unwrap_or_else(|error| panic!("failed to load required game-data catalog: {error}"))
            .ship_types()
            .to_vec();
    }

    load_ship_types_file(path)
        .unwrap_or_else(|error| panic!("failed to load required ship-type game data: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn custom_module_path_loads_one_strict_category() {
        let mut file = tempfile::NamedTempFile::new().expect("temp module file");
        write!(
            file,
            r#"
[[modules]]
id = 999
name = "Custom Sensor"
kind = "Sensor"
slot = "Mid"
activation_mode = "Passive"
"#
        )
        .expect("write module TOML");

        let modules = load_modules(file.path().to_str().expect("UTF-8 path"), Vec::new());
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].id.0, 999);
    }

    #[test]
    fn custom_ship_type_path_loads_one_strict_category() {
        let mut file = tempfile::NamedTempFile::new().expect("temp ship-type file");
        write!(
            file,
            r#"
[[ship_types]]
id = 999
name = "Custom Frigate"
class = "Frigate"
buildable = false

[ship_types.slot_layout]
high = 1
mid = 1
low = 1
rig = 1

[ship_types.base_stats]
max_speed = 100.0
mass = 1000.0
inertia_modifier = 0.5
max_shield = 10.0
max_armor = 10.0
max_hull = 10.0
lock_time = 1
max_locks = 1
cap_max = 10.0
cap_recharge_per_tick = 1.0
sig_radius = 10.0
"#
        )
        .expect("write ship-type TOML");

        let ship_types = load_ship_types(file.path().to_str().expect("UTF-8 path"), Vec::new());
        assert_eq!(ship_types.len(), 1);
        assert_eq!(ship_types[0].id.0, 999);
    }
}
