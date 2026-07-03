//! Load game data from TOML files at server startup.
//! Falls back to built-in defaults when a file is absent or fails to parse.
//! Adapted from dawn-simulation/src/data_loader/*.

use dawn_core::fitting::{
    ActivationMode, ModuleDefinition, ModuleId, ModuleKind, SlotKind, StatDelta,
};
use dawn_core::ship_type::{ShipBaseStats, ShipClass, ShipTypeDefinition, ShipTypeId, SlotLayout};
use dawn_sector::{modules, ship_types};
use serde::Deserialize;

// -- Modules -----------------------------------------------------------------

#[derive(Deserialize)]
struct ModulesFile {
    modules: Vec<ModuleEntry>,
}

#[derive(Deserialize)]
struct ModuleEntry {
    id: u32,
    name: String,
    kind: String,
    slot: String,
    activation_mode: String,
    #[serde(default)]
    cap_cost_per_cycle: f32,
    #[serde(default)]
    cycle_time_ticks: u64,
    #[serde(default)]
    stat_delta: StatDeltaEntry,
}

#[derive(Deserialize, Default)]
struct StatDeltaEntry {
    #[serde(default = "one")]
    speed_multiplier: f32,
    #[serde(default)]
    mass_add: f32,
    #[serde(default)]
    max_shield_add: f32,
    #[serde(default)]
    max_armor_add: f32,
    #[serde(default)]
    max_hull_add: f32,
    #[serde(default)]
    weapon_damage_add: f32,
    #[serde(default)]
    weapon_range_add: f32,
    #[serde(default)]
    tracking_speed_add: f32,
    #[serde(default)]
    falloff_range_add: f32,
    #[serde(default)]
    weapon_cooldown_add: i32,
    #[serde(default)]
    lock_time_add: i32,
    #[serde(default)]
    max_locks_add: i32,
    #[serde(default)]
    cap_max_add: f32,
    #[serde(default)]
    cap_recharge_add: f32,
    #[serde(default)]
    tackle_range_add: f32,
    #[serde(default)]
    repair_amount: f32,
    #[serde(default)]
    repair_range_add: f32,
}

fn one() -> f32 {
    1.0
}

fn parse_slot_kind(s: &str) -> SlotKind {
    match s {
        "Mid" => SlotKind::Mid,
        "Low" => SlotKind::Low,
        "Rig" => SlotKind::Rig,
        _ => SlotKind::High,
    }
}
fn parse_module_kind(s: &str) -> ModuleKind {
    match s {
        "ShieldBooster" => ModuleKind::ShieldBooster,
        "ArmorRepairer" => ModuleKind::ArmorRepairer,
        "Propulsion" => ModuleKind::Propulsion,
        "Sensor" => ModuleKind::Sensor,
        "Rig" => ModuleKind::Rig,
        "Tackle" => ModuleKind::Tackle,
        "RemoteShieldBooster" => ModuleKind::RemoteShieldBooster,
        "RemoteArmorRepairer" => ModuleKind::RemoteArmorRepairer,
        _ => ModuleKind::Weapon,
    }
}
fn parse_activation_mode(s: &str) -> ActivationMode {
    match s {
        "Passive" => ActivationMode::Passive,
        _ => ActivationMode::Active,
    }
}

pub fn load_modules(path: &str) -> Vec<ModuleDefinition> {
    let fallback = modules::all_modules();
    let content = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "[DataLoader] '{}' not found ({}), using built-in defaults.",
                path, e
            );
            return fallback;
        }
    };
    match toml::from_str::<ModulesFile>(&content) {
        Ok(f) => {
            let result: Vec<_> = f
                .modules
                .into_iter()
                .map(|e| ModuleDefinition {
                    id: ModuleId(e.id),
                    name: e.name,
                    kind: parse_module_kind(&e.kind),
                    slot: parse_slot_kind(&e.slot),
                    activation_mode: parse_activation_mode(&e.activation_mode),
                    cap_cost_per_cycle: e.cap_cost_per_cycle,
                    cycle_time_ticks: e.cycle_time_ticks,
                    stat_delta: StatDelta {
                        speed_multiplier: e.stat_delta.speed_multiplier,
                        mass_add: e.stat_delta.mass_add,
                        max_shield_add: e.stat_delta.max_shield_add,
                        max_armor_add: e.stat_delta.max_armor_add,
                        max_hull_add: e.stat_delta.max_hull_add,
                        weapon_damage_add: e.stat_delta.weapon_damage_add,
                        weapon_range_add: e.stat_delta.weapon_range_add,
                        tracking_speed_add: e.stat_delta.tracking_speed_add,
                        falloff_range_add: e.stat_delta.falloff_range_add,
                        weapon_cooldown_add: e.stat_delta.weapon_cooldown_add,
                        lock_time_add: e.stat_delta.lock_time_add,
                        max_locks_add: e.stat_delta.max_locks_add,
                        cap_max_add: e.stat_delta.cap_max_add,
                        cap_recharge_add: e.stat_delta.cap_recharge_add,
                        tackle_range_add: e.stat_delta.tackle_range_add,
                        repair_amount: e.stat_delta.repair_amount,
                        repair_range_add: e.stat_delta.repair_range_add,
                    },
                })
                .collect();
            println!(
                "[DataLoader] loaded {} modules from '{}'.",
                result.len(),
                path
            );
            result
        }
        Err(e) => {
            eprintln!(
                "[DataLoader] parse error in '{}': {}, using built-in defaults.",
                path, e
            );
            fallback
        }
    }
}
// -- Ship types --------------------------------------------------------------

#[derive(Deserialize)]
struct ShipTypesFile {
    ship_types: Vec<ShipTypeEntry>,
}

#[derive(Deserialize)]
struct ShipTypeEntry {
    id: u32,
    name: String,
    class: String,
    slot_layout: SlotLayoutEntry,
    base_stats: BaseStatsEntry,
}

#[derive(Deserialize)]
struct SlotLayoutEntry {
    high: u8,
    mid: u8,
    low: u8,
    rig: u8,
}

#[derive(Deserialize)]
struct BaseStatsEntry {
    max_speed: f32,
    mass: f32,
    inertia_modifier: f32,
    max_shield: f32,
    max_armor: f32,
    max_hull: f32,
    lock_time: u64,
    max_locks: u32,
    #[serde(default = "default_cap_max")]
    cap_max: f32,
    #[serde(default = "default_cap_recharge")]
    cap_recharge_per_tick: f32,
    #[serde(default = "default_sig_radius")]
    sig_radius: f32,
}

fn default_cap_max() -> f32 {
    400.0
}
fn default_cap_recharge() -> f32 {
    8.0
}
fn default_sig_radius() -> f32 {
    40.0
}

fn parse_ship_class(s: &str) -> ShipClass {
    match s {
        "Cruiser" => ShipClass::Cruiser,
        "Battleship" => ShipClass::Battleship,
        _ => ShipClass::Frigate,
    }
}

pub fn load_ship_types(path: &str) -> Vec<ShipTypeDefinition> {
    let fallback = ship_types::all_ship_types();
    let content = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "[DataLoader] '{}' not found ({}), using built-in defaults.",
                path, e
            );
            return fallback;
        }
    };
    match toml::from_str::<ShipTypesFile>(&content) {
        Ok(f) => {
            let result: Vec<_> = f
                .ship_types
                .into_iter()
                .map(|e| ShipTypeDefinition {
                    id: ShipTypeId(e.id),
                    name: e.name,
                    class: parse_ship_class(&e.class),
                    slot_layout: SlotLayout {
                        high: e.slot_layout.high,
                        mid: e.slot_layout.mid,
                        low: e.slot_layout.low,
                        rig: e.slot_layout.rig,
                    },
                    base_stats: ShipBaseStats {
                        max_speed: e.base_stats.max_speed,
                        mass: e.base_stats.mass,
                        inertia_modifier: e.base_stats.inertia_modifier,
                        max_shield: e.base_stats.max_shield,
                        max_armor: e.base_stats.max_armor,
                        max_hull: e.base_stats.max_hull,
                        lock_time: e.base_stats.lock_time,
                        max_locks: e.base_stats.max_locks,
                        cap_max: e.base_stats.cap_max,
                        cap_recharge_per_tick: e.base_stats.cap_recharge_per_tick,
                        sig_radius: e.base_stats.sig_radius,
                    },
                })
                .collect();
            println!(
                "[DataLoader] loaded {} ship types from '{}'.",
                result.len(),
                path
            );
            result
        }
        Err(e) => {
            eprintln!(
                "[DataLoader] parse error in '{}': {}, using built-in defaults.",
                path, e
            );
            fallback
        }
    }
}
