//! 標準モジュール定義カタログ。

use dawn_core::fitting::{ActivationMode, ModuleDefinition, ModuleId, ModuleKind, SlotKind, StatDelta};

pub const MODULE_RAILGUN_SMALL  : ModuleId = ModuleId(1);
pub const MODULE_RAILGUN_MEDIUM : ModuleId = ModuleId(2);
pub const MODULE_SHIELD_BASIC   : ModuleId = ModuleId(3);
pub const MODULE_ARMOR_BASIC    : ModuleId = ModuleId(4);
pub const MODULE_AFTERBURNER    : ModuleId = ModuleId(5);
pub const MODULE_SENSOR_BOOSTER : ModuleId = ModuleId(6);

pub fn all_modules() -> Vec<ModuleDefinition> {
    vec![
        // ── Weapons (High / Active) ──────────────────────────────────────────
        // Frigate recharge 10 GJ/tick → 100 GJ per 10-tick cycle.
        // Gun alone: 100 - 60 = +40/cycle (sustainable).
        // Gun + AB:  100 - 60 - 80 = -40/cycle → drains in ~12 cycles (12 s).
        ModuleDefinition {
            id: MODULE_RAILGUN_SMALL, name: "Small Railgun I".to_string(),
            kind: ModuleKind::Weapon, slot: SlotKind::High,
            activation_mode: ActivationMode::Active,
            cap_cost_per_cycle: 60.0,
            cycle_time_ticks  : 10,
            stat_delta: StatDelta { weapon_damage_add: 25.0, weapon_range_add: 3_000.0, falloff_range_add: 2_000.0, tracking_speed_add: 0.035, ..StatDelta::ZERO },
        },
        ModuleDefinition {
            id: MODULE_RAILGUN_MEDIUM, name: "Medium Railgun I".to_string(),
            kind: ModuleKind::Weapon, slot: SlotKind::High,
            activation_mode: ActivationMode::Active,
            cap_cost_per_cycle: 100.0,
            cycle_time_ticks  : 14,
            stat_delta: StatDelta { weapon_damage_add: 50.0, weapon_range_add: 2_500.0, weapon_cooldown_add: 4, ..StatDelta::ZERO },
        },

        // ── Shield (Mid / Passive) ───────────────────────────────────────────
        ModuleDefinition {
            id: MODULE_SHIELD_BASIC, name: "Basic Shield Extender".to_string(),
            kind: ModuleKind::ShieldBooster, slot: SlotKind::Mid,
            activation_mode: ActivationMode::Passive,
            cap_cost_per_cycle: 0.0,
            cycle_time_ticks  : 0,
            stat_delta: StatDelta { max_shield_add: 300.0, ..StatDelta::ZERO },
        },

        // ── Armor (Low / Passive) ────────────────────────────────────────────
        ModuleDefinition {
            id: MODULE_ARMOR_BASIC, name: "Basic Armor Plate".to_string(),
            kind: ModuleKind::ArmorRepairer, slot: SlotKind::Low,
            activation_mode: ActivationMode::Passive,
            cap_cost_per_cycle: 0.0,
            cycle_time_ticks  : 0,
            stat_delta: StatDelta { max_armor_add: 200.0, ..StatDelta::ZERO },
        },

        // ── Propulsion (Mid / Active) ────────────────────────────────────────
        ModuleDefinition {
            id: MODULE_AFTERBURNER, name: "1MN Afterburner".to_string(),
            kind: ModuleKind::Propulsion, slot: SlotKind::Mid,
            activation_mode: ActivationMode::Active,
            cap_cost_per_cycle: 100.0, // 10 GJ/tick — must exceed Magpie's 8 GJ/tick recharge
            cycle_time_ticks  : 10,
            stat_delta: StatDelta { max_speed_add: 150.0, thrust_add: 10.0, ..StatDelta::ZERO },
        },

        // ── Sensor (Mid / Passive) ───────────────────────────────────────────
        ModuleDefinition {
            id: MODULE_SENSOR_BOOSTER, name: "Sensor Booster I".to_string(),
            kind: ModuleKind::Sensor, slot: SlotKind::Mid,
            activation_mode: ActivationMode::Passive,
            cap_cost_per_cycle: 0.0,
            cycle_time_ticks  : 0,
            stat_delta: StatDelta { lock_time_add: -2, max_locks_add: 1, ..StatDelta::ZERO },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_ecs::components::ShipStatsComp;
    use dawn_core::fitting::ActivationMode;

    #[test]
    fn all_modules_have_unique_ids() {
        use std::collections::HashSet;
        let ids: HashSet<u32> = all_modules().iter().map(|m| m.id.0).collect();
        assert_eq!(ids.len(), all_modules().len());
    }

    #[test]
    fn weapon_modules_are_active_and_in_high_slot() {
        for m in all_modules() {
            if m.kind == ModuleKind::Weapon {
                assert_eq!(m.slot, SlotKind::High);
                assert_eq!(m.activation_mode, ActivationMode::Active,
                    "weapon '{}' must be Active", m.name);
            }
        }
    }

    #[test]
    fn passive_modules_are_not_weapons() {
        for m in all_modules() {
            if m.activation_mode == ActivationMode::Passive {
                assert_ne!(m.kind, ModuleKind::Weapon,
                    "passive module '{}' should not be a weapon", m.name);
            }
        }
    }

    #[test]
    fn base_npc_stats_have_no_weapon_capability() {
        assert_eq!(ShipStatsComp::NPC.weapon_damage, 0.0);
        assert_eq!(ShipStatsComp::PLAYER.weapon_damage, 0.0);
    }

    #[test]
    fn weapon_modules_provide_positive_damage_and_range() {
        for m in all_modules() {
            if m.kind == ModuleKind::Weapon {
                assert!(m.stat_delta.weapon_damage_add > 0.0);
                assert!(m.stat_delta.weapon_range_add > 0.0);
            }
        }
    }
}
