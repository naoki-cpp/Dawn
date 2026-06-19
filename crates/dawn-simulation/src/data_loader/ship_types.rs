use dawn_core::ship_type::{ShipBaseStats, ShipClass, ShipTypeDefinition, ShipTypeId, SlotLayout};
use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct ShipTypesFile {
    pub(super) ship_types: Vec<ShipTypeEntry>,
}

#[derive(Deserialize)]
pub(super) struct ShipTypeEntry {
    pub(super) id         : u32,
    pub(super) name       : String,
    pub(super) class      : String,
    pub(super) slot_layout: SlotLayoutEntry,
    pub(super) base_stats : BaseStatsEntry,
}

#[derive(Deserialize)]
pub(super) struct SlotLayoutEntry {
    pub(super) high: u8,
    pub(super) mid : u8,
    pub(super) low : u8,
    pub(super) rig : u8,
}

#[derive(Deserialize)]
pub(super) struct BaseStatsEntry {
    pub(super) max_speed            : f32,
    pub(super) mass                 : f32,
    pub(super) inertia_modifier     : f32,
    pub(super) max_shield           : f32,
    pub(super) max_armor            : f32,
    pub(super) max_hull             : f32,
    pub(super) lock_time            : u64,
    pub(super) max_locks            : u32,
    #[serde(default = "default_cap_max")]
    pub(super) cap_max              : f32,
    #[serde(default = "default_cap_recharge")]
    pub(super) cap_recharge_per_tick: f32,
    #[serde(default = "default_sig_radius")]
    pub(super) sig_radius           : f32,
}

fn default_cap_max() -> f32 { 400.0 }
fn default_cap_recharge() -> f32 { 8.0 }
fn default_sig_radius() -> f32 { 40.0 }

fn parse_ship_class(s: &str) -> ShipClass {
    match s {
        "Cruiser"     => ShipClass::Cruiser,
        "Battleship"  => ShipClass::Battleship,
        _             => ShipClass::Frigate,
    }
}

fn entry_to_ship_type(e: ShipTypeEntry) -> ShipTypeDefinition {
    ShipTypeDefinition {
        id         : ShipTypeId(e.id),
        name       : e.name,
        class      : parse_ship_class(&e.class),
        slot_layout: SlotLayout {
            high: e.slot_layout.high,
            mid : e.slot_layout.mid,
            low : e.slot_layout.low,
            rig : e.slot_layout.rig,
        },
        base_stats : ShipBaseStats {
            max_speed            : e.base_stats.max_speed,
            mass                 : e.base_stats.mass,
            inertia_modifier     : e.base_stats.inertia_modifier,
            max_shield           : e.base_stats.max_shield,
            max_armor            : e.base_stats.max_armor,
            max_hull             : e.base_stats.max_hull,
            lock_time            : e.base_stats.lock_time,
            max_locks            : e.base_stats.max_locks,
            cap_max              : e.base_stats.cap_max,
            cap_recharge_per_tick: e.base_stats.cap_recharge_per_tick,
            sig_radius           : e.base_stats.sig_radius,
        },
    }
}

/// Load ship type definitions from a TOML file. Falls back to `fallback` if
/// the file is absent or cannot be parsed.
pub fn load_ship_types(path: &str, fallback: Vec<ShipTypeDefinition>) -> Vec<ShipTypeDefinition> {
    let content = match std::fs::read_to_string(path) {
        Ok(s)  => s,
        Err(e) => {
            eprintln!("[DataLoader] '{}' not found ({}), using built-in defaults.", path, e);
            return fallback;
        }
    };

    match toml::from_str::<ShipTypesFile>(&content) {
        Ok(f) => {
            let types: Vec<ShipTypeDefinition> = f.ship_types.into_iter()
                .map(entry_to_ship_type)
                .collect();
            println!("[DataLoader] loaded {} ship types from '{}'.", types.len(), path);
            types
        }
        Err(e) => {
            eprintln!("[DataLoader] parse error in '{}': {}, using built-in defaults.", path, e);
            fallback
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_SHIP_TYPES_TOML: &str = r#"
[[ship_types]]
id    = 1
name  = "Test Frigate"
class = "Frigate"

[ship_types.slot_layout]
high = 3
mid  = 3
low  = 2
rig  = 3

[ship_types.base_stats]
max_speed        = 400.0
mass             = 1500000.0
inertia_modifier = 0.4
max_shield       = 200.0
max_armor        = 150.0
max_hull         = 150.0
lock_time        = 5
max_locks        = 1
"#;

    const INVALID_TOML: &str = "this is [not valid toml";

    #[test]
    fn ship_type_is_parsed_from_valid_toml() {
        let result: ShipTypesFile = toml::from_str(SAMPLE_SHIP_TYPES_TOML).unwrap();
        assert_eq!(result.ship_types.len(), 1);
        let st = &result.ship_types[0];
        assert_eq!(st.id, 1);
        assert_eq!(st.name, "Test Frigate");
        assert_eq!(st.base_stats.max_speed, 400.0);
    }

    #[test]
    fn load_ship_types_uses_fallback_on_missing_file() {
        let result = load_ship_types("nonexistent_file.toml", vec![]);
        assert!(result.is_empty());
    }

    #[test]
    fn load_ship_types_uses_fallback_on_invalid_toml() {
        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "{}", INVALID_TOML).unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        let result = load_ship_types(&path, vec![]);
        assert!(result.is_empty());
    }

    #[test]
    fn load_ship_types_succeeds_on_valid_file() {
        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "{}", SAMPLE_SHIP_TYPES_TOML).unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        let result = load_ship_types(&path, vec![]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, ShipTypeId(1));
    }
}
