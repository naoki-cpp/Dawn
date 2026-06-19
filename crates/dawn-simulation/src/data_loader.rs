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
use dawn_core::{CelestialBodyDef, CelestialBodyId, CelestialBodyKind, JumpGateDef, JumpGateId, Position, SectorId, StarSystemDef, StarSystemId};
use serde::Deserialize;

// ── TOML 中間型（star_map.toml）──────────────────────────────────────────────

#[derive(Deserialize)]
struct StarMapFile {
    #[serde(default)] star_systems    : Vec<StarSystemEntry>,
    #[serde(default)] jump_gates      : Vec<JumpGateEntry>,
    #[serde(default)] celestial_bodies: Vec<CelestialBodyEntry>,
}

#[derive(Deserialize)]
struct StarSystemEntry {
    id     : u32,
    name   : String,
    sectors: Vec<u8>,
}

#[derive(Deserialize)]
struct JumpGateEntry {
    id               : u32,
    from_sector      : u8,
    to_sector        : u8,
    position         : [f32; 3],
    activation_radius: f32,
}

#[derive(Deserialize)]
struct CelestialBodyEntry {
    id           : u32,
    kind         : String,
    name         : String,
    position     : [f32; 3],
    radius       : f32,
    #[serde(default)] spectral_type: f32,
}

fn parse_body_kind(s: &str) -> CelestialBodyKind {
    match s { "Star" => CelestialBodyKind::Star, _ => CelestialBodyKind::Planet }
}

fn entry_to_system(e: StarSystemEntry) -> StarSystemDef {
    StarSystemDef { id: StarSystemId(e.id), name: e.name, sectors: e.sectors.into_iter().map(SectorId).collect() }
}

fn entry_to_gate(e: JumpGateEntry) -> JumpGateDef {
    JumpGateDef {
        id               : JumpGateId(e.id),
        from_sector      : SectorId(e.from_sector),
        to_sector        : SectorId(e.to_sector),
        position         : Position::new(e.position[0], e.position[1], e.position[2]),
        activation_radius: e.activation_radius,
    }
}

fn entry_to_body(e: CelestialBodyEntry) -> CelestialBodyDef {
    CelestialBodyDef {
        id           : CelestialBodyId(e.id),
        kind         : parse_body_kind(&e.kind),
        name         : e.name,
        position     : Position::new(e.position[0], e.position[1], e.position[2]),
        radius       : e.radius,
        spectral_type: e.spectral_type,
    }
}

/// Load the star map (star systems, jump gates, celestial bodies) from a TOML
/// file.  Falls back to `fallback` if the file is absent or cannot be parsed.
pub fn load_star_map(path: &str, fallback: dawn_sector::star_map::StarMap) -> dawn_sector::star_map::StarMap {
    let content = match std::fs::read_to_string(path) {
        Ok(s)  => s,
        Err(e) => {
            eprintln!("[DataLoader] '{}' not found ({}), using built-in star map.", path, e);
            return fallback;
        }
    };

    match toml::from_str::<StarMapFile>(&content) {
        Ok(f) => {
            let systems = f.star_systems.into_iter().map(entry_to_system).collect::<Vec<_>>();
            let gates   = f.jump_gates.into_iter().map(entry_to_gate).collect::<Vec<_>>();
            let bodies  = f.celestial_bodies.into_iter().map(entry_to_body).collect::<Vec<_>>();
            println!("[DataLoader] loaded star map from '{}': {} systems, {} gates, {} bodies.",
                path, systems.len(), gates.len(), bodies.len());
            dawn_sector::star_map::StarMap::new(systems, gates, bodies)
        }
        Err(e) => {
            eprintln!("[DataLoader] parse error in '{}': {}, using built-in star map.", path, e);
            fallback
        }
    }
}

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
    max_speed            : f32,
    mass                 : f32,
    inertia_modifier     : f32,
    max_shield           : f32,
    max_armor            : f32,
    max_hull             : f32,
    lock_time            : u64,
    max_locks            : u32,
    #[serde(default = "default_cap_max")]
    cap_max              : f32,
    #[serde(default = "default_cap_recharge")]
    cap_recharge_per_tick: f32,
    #[serde(default = "default_sig_radius")]
    sig_radius           : f32,
}

fn default_cap_max() -> f32 { 400.0 }
fn default_cap_recharge() -> f32 { 8.0 }
fn default_sig_radius() -> f32 { 40.0 }
fn default_speed_multiplier() -> f32 { 1.0 }

// ── TOML 中間型（modules.toml）───────────────────────────────────────────────

#[derive(Deserialize)]
struct ModulesFile {
    modules: Vec<ModuleEntry>,
}

#[derive(Deserialize)]
struct ModuleEntry {
    id               : u32,
    name             : String,
    kind             : String,
    slot             : String,
    activation_mode  : String,
    #[serde(default)]
    cap_cost_per_cycle: f32,
    #[serde(default)]
    cycle_time_ticks : u64,
    #[serde(default)]
    stat_delta       : StatDeltaEntry,
}

#[derive(Deserialize, Default)]
struct StatDeltaEntry {
    #[serde(default = "default_speed_multiplier")] speed_multiplier : f32,
    #[serde(default)] mass_add            : f32,
    #[serde(default)] max_shield_add      : f32,
    #[serde(default)] max_armor_add       : f32,
    #[serde(default)] max_hull_add        : f32,
    #[serde(default)] weapon_damage_add   : f32,
    #[serde(default)] weapon_range_add    : f32,
    #[serde(default)] tracking_speed_add  : f32,
    #[serde(default)] falloff_range_add   : f32,
    #[serde(default)] weapon_cooldown_add : i32,
    #[serde(default)] lock_time_add       : i32,
    #[serde(default)] max_locks_add       : i32,
    #[serde(default)] cap_max_add         : f32,
    #[serde(default)] cap_recharge_add    : f32,
    #[serde(default)] tackle_range_add    : f32,
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
        "Tackle"         => ModuleKind::Tackle,
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

fn entry_to_module(e: ModuleEntry) -> ModuleDefinition {
    ModuleDefinition {
        id                : ModuleId(e.id),
        name              : e.name,
        kind              : parse_module_kind(&e.kind),
        slot              : parse_slot_kind(&e.slot),
        activation_mode   : parse_activation_mode(&e.activation_mode),
        cap_cost_per_cycle: e.cap_cost_per_cycle,
        cycle_time_ticks  : e.cycle_time_ticks,
        stat_delta        : StatDelta {
            speed_multiplier    : e.stat_delta.speed_multiplier,
            mass_add            : e.stat_delta.mass_add,
            max_shield_add      : e.stat_delta.max_shield_add,
            max_armor_add       : e.stat_delta.max_armor_add,
            max_hull_add        : e.stat_delta.max_hull_add,
            weapon_damage_add   : e.stat_delta.weapon_damage_add,
            weapon_range_add    : e.stat_delta.weapon_range_add,
            tracking_speed_add  : e.stat_delta.tracking_speed_add,
            falloff_range_add   : e.stat_delta.falloff_range_add,
            weapon_cooldown_add : e.stat_delta.weapon_cooldown_add,
            lock_time_add       : e.stat_delta.lock_time_add,
            max_locks_add       : e.stat_delta.max_locks_add,
            cap_max_add         : e.stat_delta.cap_max_add,
            cap_recharge_add    : e.stat_delta.cap_recharge_add,
            tackle_range_add    : e.stat_delta.tackle_range_add,
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
mass             = 1500000.0
inertia_modifier = 0.4
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
