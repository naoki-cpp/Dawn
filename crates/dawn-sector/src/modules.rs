//! Stable module identifiers.
//!
//! Balance values are authoritative in `data/modules.toml` and are loaded by
//! [`crate::game_data::GameDataCatalog`].

use dawn_core::fitting::ModuleId;

pub const MODULE_RAILGUN_SMALL: ModuleId = ModuleId(1);
pub const MODULE_RAILGUN_MEDIUM: ModuleId = ModuleId(2);
pub const MODULE_SHIELD_BASIC: ModuleId = ModuleId(3);
pub const MODULE_ARMOR_BASIC: ModuleId = ModuleId(4);
pub const MODULE_AFTERBURNER: ModuleId = ModuleId(5);
pub const MODULE_SENSOR_BOOSTER: ModuleId = ModuleId(6);
pub const MODULE_RAILGUN_HEAVY: ModuleId = ModuleId(7);
pub const MODULE_SHIELD_LARGE: ModuleId = ModuleId(8);
pub const MODULE_ARMOR_REINFORCED: ModuleId = ModuleId(9);
pub const MODULE_AFTERBURNER_10MN: ModuleId = ModuleId(10);
pub const MODULE_SIGNAL_AMPLIFIER: ModuleId = ModuleId(11);
pub const MODULE_FOLD_DISRUPTOR: ModuleId = ModuleId(12);
pub const MODULE_SMALL_SHIELD_BOOSTER: ModuleId = ModuleId(13);
pub const MODULE_SMALL_ARMOR_REPAIRER: ModuleId = ModuleId(14);
pub const MODULE_SMALL_REMOTE_SHIELD_BOOSTER: ModuleId = ModuleId(15);
pub const MODULE_SMALL_REMOTE_ARMOR_REPAIRER: ModuleId = ModuleId(16);

pub(crate) const REQUIRED_MODULE_IDS: &[ModuleId] = &[
    MODULE_RAILGUN_SMALL,
    MODULE_RAILGUN_MEDIUM,
    MODULE_SHIELD_BASIC,
    MODULE_ARMOR_BASIC,
    MODULE_AFTERBURNER,
    MODULE_SENSOR_BOOSTER,
    MODULE_RAILGUN_HEAVY,
    MODULE_SHIELD_LARGE,
    MODULE_ARMOR_REINFORCED,
    MODULE_AFTERBURNER_10MN,
    MODULE_SIGNAL_AMPLIFIER,
    MODULE_FOLD_DISRUPTOR,
    MODULE_SMALL_SHIELD_BOOSTER,
    MODULE_SMALL_ARMOR_REPAIRER,
    MODULE_SMALL_REMOTE_SHIELD_BOOSTER,
    MODULE_SMALL_REMOTE_ARMOR_REPAIRER,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_data::test_catalog;
    use dawn_core::fitting::{ActivationMode, ModuleKind, SlotKind};
    use dawn_ecs::components::ShipStatsComp;

    #[test]
    fn all_required_module_ids_are_unique() {
        use std::collections::HashSet;
        let ids: HashSet<_> = REQUIRED_MODULE_IDS.iter().copied().collect();
        assert_eq!(ids.len(), REQUIRED_MODULE_IDS.len());
    }

    #[test]
    fn weapon_modules_are_active_and_in_high_slot() {
        for module in test_catalog().modules() {
            if module.kind == ModuleKind::Weapon {
                assert_eq!(module.slot, SlotKind::High);
                assert_eq!(
                    module.activation_mode,
                    ActivationMode::Active,
                    "weapon '{}' must be Active",
                    module.name
                );
            }
        }
    }

    #[test]
    fn passive_modules_are_not_weapons() {
        for module in test_catalog().modules() {
            if module.activation_mode == ActivationMode::Passive {
                assert_ne!(
                    module.kind,
                    ModuleKind::Weapon,
                    "passive module '{}' should not be a weapon",
                    module.name
                );
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
        for module in test_catalog().modules() {
            if module.kind == ModuleKind::Weapon {
                assert!(module.stat_delta.weapon_damage_add > 0.0);
                assert!(module.stat_delta.weapon_range_add > 0.0);
            }
        }
    }
}
