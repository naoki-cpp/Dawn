use dawn_core::fitting::{
    ActivationMode, ModuleDefinition, ModuleId, ModuleKind, SlotKind, StatDelta,
};
use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct ModulesFile {
    pub(super) modules: Vec<ModuleEntry>,
}

#[derive(Deserialize)]
pub(super) struct ModuleEntry {
    pub(super) id: u32,
    pub(super) name: String,
    pub(super) kind: String,
    pub(super) slot: String,
    pub(super) activation_mode: String,
    #[serde(default)]
    pub(super) cap_cost_per_cycle: f32,
    #[serde(default)]
    pub(super) cycle_time_ticks: u64,
    #[serde(default)]
    pub(super) stat_delta: StatDeltaEntry,
}

#[derive(Deserialize, Default)]
pub(super) struct StatDeltaEntry {
    #[serde(default = "default_speed_multiplier")]
    pub(super) speed_multiplier: f32,
    #[serde(default)]
    pub(super) mass_add: f32,
    #[serde(default)]
    pub(super) max_shield_add: f32,
    #[serde(default)]
    pub(super) max_armor_add: f32,
    #[serde(default)]
    pub(super) max_hull_add: f32,
    #[serde(default)]
    pub(super) weapon_damage_add: f32,
    #[serde(default)]
    pub(super) weapon_range_add: f32,
    #[serde(default)]
    pub(super) tracking_speed_add: f32,
    #[serde(default)]
    pub(super) falloff_range_add: f32,
    #[serde(default)]
    pub(super) weapon_cooldown_add: i32,
    #[serde(default)]
    pub(super) lock_time_add: i32,
    #[serde(default)]
    pub(super) max_locks_add: i32,
    #[serde(default)]
    pub(super) cap_max_add: f32,
    #[serde(default)]
    pub(super) cap_recharge_add: f32,
    #[serde(default)]
    pub(super) tackle_range_add: f32,
    #[serde(default)]
    pub(super) repair_amount: f32,
    #[serde(default)]
    pub(super) repair_range_add: f32,
}

fn default_speed_multiplier() -> f32 {
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

fn entry_to_module(e: ModuleEntry) -> ModuleDefinition {
    ModuleDefinition {
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
    }
}

/// Load module definitions from a TOML file. Falls back to `fallback` if the
/// file is absent or cannot be parsed.
pub fn load_modules(path: &str, fallback: Vec<ModuleDefinition>) -> Vec<ModuleDefinition> {
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
            let modules: Vec<ModuleDefinition> =
                f.modules.into_iter().map(entry_to_module).collect();
            println!(
                "[DataLoader] loaded {} modules from '{}'.",
                modules.len(),
                path
            );
            modules
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn module_is_parsed_from_valid_toml() {
        let result: ModulesFile = toml::from_str(SAMPLE_MODULES_TOML).unwrap();
        assert_eq!(result.modules.len(), 1);
        let m = &result.modules[0];
        assert_eq!(m.id, 1);
        assert_eq!(m.stat_delta.weapon_damage_add, 30.0);
    }

    #[test]
    fn load_modules_uses_fallback_on_missing_file() {
        let result = load_modules("nonexistent_file.toml", vec![]);
        assert!(result.is_empty());
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
        assert_eq!(delta.max_shield_add, 0.0);
        assert_eq!(delta.lock_time_add, 0);
    }
}
