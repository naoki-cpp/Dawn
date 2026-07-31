use super::{parse_required, read_required, validation::*, CatalogError};
use dawn_core::fitting::{
    ActivationMode, ModuleDefinition, ModuleId, ModuleKind, SlotKind, StatDelta,
};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::Path;

pub(crate) fn load_modules_file(
    path: impl AsRef<Path>,
) -> Result<Vec<ModuleDefinition>, CatalogError> {
    let path = path.as_ref();
    let source = read_required("module", path)?;
    let parsed: ModulesFile = parse_required("module", path, &source)?;
    let modules = parsed
        .modules
        .into_iter()
        .map(Into::into)
        .collect::<Vec<_>>();
    validate_modules(&modules, path)?;
    Ok(modules)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModulesFile {
    modules: Vec<ModuleEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModuleEntry {
    id: u32,
    name: String,
    kind: ModuleKind,
    slot: SlotKind,
    activation_mode: ActivationMode,
    #[serde(default)]
    cap_cost_per_cycle: f32,
    #[serde(default)]
    cycle_time_ticks: u64,
    #[serde(default = "default_stat_delta")]
    stat_delta: StatDeltaEntry,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct StatDeltaEntry {
    #[serde(default = "default_speed_multiplier")]
    speed_multiplier: f64,
    #[serde(default)]
    mass_add: f64,
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

fn default_speed_multiplier() -> f64 {
    1.0
}

fn default_stat_delta() -> StatDeltaEntry {
    StatDeltaEntry {
        speed_multiplier: default_speed_multiplier(),
        ..StatDeltaEntry::default()
    }
}

impl From<ModuleEntry> for ModuleDefinition {
    fn from(entry: ModuleEntry) -> Self {
        Self {
            id: ModuleId(entry.id),
            name: entry.name,
            kind: entry.kind,
            slot: entry.slot,
            activation_mode: entry.activation_mode,
            cap_cost_per_cycle: entry.cap_cost_per_cycle,
            cycle_time_ticks: entry.cycle_time_ticks,
            stat_delta: StatDelta {
                speed_multiplier: entry.stat_delta.speed_multiplier,
                mass_add: entry.stat_delta.mass_add,
                max_shield_add: entry.stat_delta.max_shield_add,
                max_armor_add: entry.stat_delta.max_armor_add,
                max_hull_add: entry.stat_delta.max_hull_add,
                weapon_damage_add: entry.stat_delta.weapon_damage_add,
                weapon_range_add: entry.stat_delta.weapon_range_add,
                tracking_speed_add: entry.stat_delta.tracking_speed_add,
                falloff_range_add: entry.stat_delta.falloff_range_add,
                weapon_cooldown_add: entry.stat_delta.weapon_cooldown_add,
                lock_time_add: entry.stat_delta.lock_time_add,
                max_locks_add: entry.stat_delta.max_locks_add,
                cap_max_add: entry.stat_delta.cap_max_add,
                cap_recharge_add: entry.stat_delta.cap_recharge_add,
                tackle_range_add: entry.stat_delta.tackle_range_add,
                repair_amount: entry.stat_delta.repair_amount,
                repair_range_add: entry.stat_delta.repair_range_add,
            },
        }
    }
}

fn validate_modules(modules: &[ModuleDefinition], path: &Path) -> Result<(), CatalogError> {
    if modules.is_empty() {
        return validation_error("module", path, "catalog must contain at least one module");
    }

    let mut ids = HashSet::new();
    for module in modules {
        if module.id.0 == 0 {
            return validation_error("module", path, "module id 0 is reserved");
        }
        if !ids.insert(module.id) {
            return validation_error(
                "module",
                path,
                format!("duplicate module id {}", module.id.0),
            );
        }
        if module.name.trim().is_empty() {
            return validation_error(
                "module",
                path,
                format!("module id {} has an empty name", module.id.0),
            );
        }

        validate_non_negative_f32(
            "module",
            path,
            module.id.0,
            "cap_cost_per_cycle",
            module.cap_cost_per_cycle,
        )?;
        match module.activation_mode {
            ActivationMode::Active if module.cycle_time_ticks == 0 => {
                return validation_error(
                    "module",
                    path,
                    format!(
                        "module id {} is Active but cycle_time_ticks is 0",
                        module.id.0
                    ),
                );
            }
            ActivationMode::Passive
                if module.cycle_time_ticks != 0 || module.cap_cost_per_cycle != 0.0 =>
            {
                return validation_error(
                    "module",
                    path,
                    format!(
                        "module id {} is Passive but has a cycle time or capacitor cost",
                        module.id.0
                    ),
                );
            }
            _ => {}
        }

        let delta = &module.stat_delta;
        if !delta.speed_multiplier.is_finite() || delta.speed_multiplier <= 0.0 {
            return validation_error(
                "module",
                path,
                format!(
                    "module id {} has invalid speed_multiplier {}",
                    module.id.0, delta.speed_multiplier
                ),
            );
        }
        validate_non_negative_f64("module", path, module.id.0, "mass_add", delta.mass_add)?;
        for (field, value) in [
            ("max_shield_add", delta.max_shield_add),
            ("max_armor_add", delta.max_armor_add),
            ("max_hull_add", delta.max_hull_add),
            ("weapon_damage_add", delta.weapon_damage_add),
            ("weapon_range_add", delta.weapon_range_add),
            ("tracking_speed_add", delta.tracking_speed_add),
            ("falloff_range_add", delta.falloff_range_add),
            ("cap_max_add", delta.cap_max_add),
            ("cap_recharge_add", delta.cap_recharge_add),
            ("tackle_range_add", delta.tackle_range_add),
            ("repair_amount", delta.repair_amount),
            ("repair_range_add", delta.repair_range_add),
        ] {
            validate_non_negative_f32("module", path, module.id.0, field, value)?;
        }
    }
    Ok(())
}
