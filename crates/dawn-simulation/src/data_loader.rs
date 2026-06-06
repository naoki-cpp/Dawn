//! # data_loader — TOML からゲームデータを読み込む
//!
//! `data/ship_types.toml` と `data/modules.toml` をサーバー起動時に読み込む。
//! ファイルが見つからない場合は `ship_types.rs` / `modules.rs` のデフォルトに
//! フォールバックし、警告を出力する。
//!
//! ## 調整サイクル
//! ```text
//! data/ship_types.toml を編集
//!   → サーバーを再起動（cargo run --release -- --serve）
//!   → Godot クライアントで動作確認
//!   → 繰り返す（リビルド不要）
//! ```

use dawn_core::fitting::{ActivationMode, ModuleDefinition, ModuleId, ModuleKind, SlotKind, StatDelta};
use dawn_core::ship_type::{ShipBaseStats, ShipClass, ShipTypeDefinition, ShipTypeId, SlotLayout};
use serde::Deserialize;

// ── TOML 中間型（ship_types.toml）────────────────────────────────────────────

#[derive(Deserialize)]
struct ShipTypesFile {
    ship_types: Vec<ShipTypeEntry>,
}

#[derive(Deserialize)]
struct ShipTypeEntry {
    id          : u32,
    name        : String,
    class       : String,
    slot_layout : SlotLayoutEntry,
    base_stats  : BaseStatsEntry,
}

#[derive(Deserialize)]
struct SlotLayoutEntry {
    high: u8,
    mid : u8,
    low : u8,
    rig : u8,
}

#[derive(Deserialize)]
struct BaseStatsEntry {
    max_speed        : f32,
    thrust_magnitude : f32,
    max_shield       : f32,
    max_armor        : f32,
    max_hull         : f32,
    lock_time        : u64,
    max_locks        : u32,
}

// ── TOML 中間型（modules.toml）───────────────────────────────────────────────

#[derive(Deserialize)]
struct ModulesFile {
    modules: Vec<ModuleEntry>,
}

#[derive(Deserialize)]
struct ModuleEntry {
    id              : u32,
    name            : String,
    kind            : String,
    slot            : String,
    activation_mode : String,
    #[serde(default)]
    stat_delta      : StatDeltaEntry,
}

#[derive(Deserialize, Default)]
struct StatDeltaEntry {
    #[serde(default)] max_speed_add       : f32,
    #[serde(default)] thrust_add          : f32,
    #[serde(default)] max_shield_add      : f32,
    #[serde(default)] max_armor_add       : f32,
    #[serde(default)] max_hull_add        : f32,
    #[serde(default)] weapon_damage_add   : f32,
    #[serde(default)] weapon_range_add    : f32,
    #[serde(default)] weapon_cooldown_add : i32,
    #[serde(default)] lock_time_add       : i32,
    #[serde(default)] max_locks_add       : i32,
}

// ── 変換 ─────────────────────────────────────────────────────────────────────

fn parse_ship_class(s: &str) -> ShipClass {
    match s {
        "Cruiser"     => ShipClass::Cruiser,
        "Battleship"  => ShipClass::Battleship,
        _             => ShipClass::Frigate,
    }
}

fn parse_slot_kind(s: &str) -> SlotKind {
    match s {
        "Mid" => SlotKind::Mid,
        "Low" => SlotKind::Low,
        "Rig" => SlotKind::Rig,
        _     => SlotKind::High,
    }
}

fn parse_module_kind(s: &str) -> ModuleKind {
    match s {
        "ShieldBooster"  => ModuleKind::ShieldBooster,
        "ArmorRepairer"  => ModuleKind::ArmorRepairer,
        "Propulsion"     => ModuleKind::Propulsion,
        "Sensor"         => ModuleKind::Sensor,
        "Rig"            => ModuleKind::Rig,
        _                => ModuleKind::Weapon,
    }
}

fn parse_activation_mode(s: &str) -> ActivationMode {
    match s {
        "Passive" => ActivationMode::Passive,
        _         => ActivationMode::Active,
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
            max_speed        : e.base_stats.max_speed,
            thrust_magnitude : e.base_stats.thrust_magnitude,
            max_shield       : e.base_stats.max_shield,
            max_armor        : e.base_stats.max_armor,
            max_hull         : e.base_stats.max_hull,
            lock_time        : e.base_stats.lock_time,
            max_locks        : e.base_stats.max_locks,
        },
    }
}

fn entry_to_module(e: ModuleEntry) -> ModuleDefinition {
    ModuleDefinition {
        id             : ModuleId(e.id),
        name           : e.name,
        kind           : parse_module_kind(&e.kind),
        slot           : parse_slot_kind(&e.slot),
        activation_mode: parse_activation_mode(&e.activation_mode),
        stat_delta     : StatDelta {
            max_speed_add       : e.stat_delta.max_speed_add,
            thrust_add          : e.stat_delta.thrust_add,
            max_shield_add      : e.stat_delta.max_shield_add,
            max_armor_add       : e.stat_delta.max_armor_add,
            max_hull_add        : e.stat_delta.max_hull_add,
            weapon_damage_add   : e.stat_delta.weapon_damage_add,
            weapon_range_add    : e.stat_delta.weapon_range_add,
            weapon_cooldown_add : e.stat_delta.weapon_cooldown_add,
            lock_time_add       : e.stat_delta.lock_time_add,
            max_locks_add       : e.stat_delta.max_locks_add,
        },
    }
}

// ── 公開 API ──────────────────────────────────────────────────────────────────

/// `data/ship_types.toml` を読み込む。
///
/// ファイルが存在しない・パース失敗の場合は `fallback` を使用する。
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

/// `data/modules.toml` を読み込む。
///
/// ファイルが存在しない・パース失敗の場合は `fallback` を使用する。
pub fn load_modules(path: &str, fallback: Vec<ModuleDefinition>) -> Vec<ModuleDefinition> {
    let content = match std::fs::read_to_string(path) {
        Ok(s)  => s,
        Err(e) => {
            eprintln!("[DataLoader] '{}' not found ({}), using built-in defaults.", path, e);
            return fallback;
        }
    };

    match toml::from_str::<ModulesFile>(&content) {
        Ok(f) => {
            let modules: Vec<ModuleDefinition> = f.modules.into_iter()
                .map(entry_to_module)
                .collect();
            println!("[DataLoader] loaded {} modules from '{}'.", modules.len(), path);
            modules
        }
        Err(e) => {
            eprintln!("[DataLoader] parse error in '{}': {}, using built-in defaults.", path, e);
            fallback
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

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
thrust_magnitude = 0.0
max_shield       = 200.0
max_armor        = 150.0
max_hull         = 150.0
lock_time        = 5
max_locks        = 1
"#;

    const SAMPLE_MODULES_TOML: &str = r#"
[[modules]]
id              = 1
name            = "Test Railgun"
kind            = "Weapon"
slot            = "High"
activation_mode = "Active"

[modules.stat_delta]
weapon_damage_add = 30.0
weapon_range_add  = 2000.0
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
    fn module_is_parsed_from_valid_toml() {
        let result: ModulesFile = toml::from_str(SAMPLE_MODULES_TOML).unwrap();
        assert_eq!(result.modules.len(), 1);
        let m = &result.modules[0];
        assert_eq!(m.id, 1);
        assert_eq!(m.stat_delta.weapon_damage_add, 30.0);
    }

    #[test]
    fn load_ship_types_uses_fallback_on_missing_file() {
        let fallback = vec![];
        let result = load_ship_types("nonexistent_file.toml", fallback);
        assert!(result.is_empty());
    }

    #[test]
    fn load_modules_uses_fallback_on_missing_file() {
        let fallback = vec![];
        let result = load_modules("nonexistent_file.toml", fallback);
        assert!(result.is_empty());
    }

    #[test]
    fn load_ship_types_uses_fallback_on_invalid_toml() {
        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "{}", INVALID_TOML).unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let fallback = vec![];
        let result = load_ship_types(&path, fallback);
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

    #[test]
    fn load_modules_succeeds_on_valid_file() {
        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "{}", SAMPLE_MODULES_TOML).unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let result = load_modules(&path, vec![]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, ModuleId(1));
        assert_eq!(result[0].stat_delta.weapon_damage_add, 30.0);
    }

    #[test]
    fn stat_delta_fields_default_to_zero_when_omitted() {
        let toml_str = r#"
[[modules]]
id              = 99
name            = "Minimal Module"
kind            = "Sensor"
slot            = "Mid"
activation_mode = "Passive"
"#;
        let result: ModulesFile = toml::from_str(toml_str).unwrap();
        let delta = &result.modules[0].stat_delta;
        assert_eq!(delta.weapon_damage_add, 0.0);
        assert_eq!(delta.max_shield_add,    0.0);
        assert_eq!(delta.lock_time_add,     0);
    }
}
