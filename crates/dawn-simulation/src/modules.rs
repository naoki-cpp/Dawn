//! 標準モジュール定義カタログ。
//!
//! Phase 4 デモ用の組み込みモジュール定義。
//! 将来はデータファイル（TOML/JSON）から読み込む形に移行する。
//!
//! # 設計原則
//! - モジュールの stat はすべて `StatDelta` で表現する。
//! - `ShipStatsComp` のベース値は武器ゼロ。武器能力はここで定義した
//!   モジュールを Fitting することでのみ得られる。

use dawn_core::{
    fitting::{ModuleDefinition, ModuleId, ModuleKind, SlotKind, StatDelta},
};

// ── Module ID 定数 ────────────────────────────────────────────────────────────

pub const MODULE_RAILGUN_SMALL  : ModuleId = ModuleId(1);
pub const MODULE_RAILGUN_MEDIUM : ModuleId = ModuleId(2);
pub const MODULE_SHIELD_BASIC   : ModuleId = ModuleId(3);
pub const MODULE_ARMOR_BASIC    : ModuleId = ModuleId(4);
pub const MODULE_AFTERBURNER    : ModuleId = ModuleId(5);
pub const MODULE_SENSOR_BOOSTER : ModuleId = ModuleId(6);

// ── 定義 ─────────────────────────────────────────────────────────────────────

/// 全標準モジュールの定義を返す。
pub fn all_modules() -> Vec<ModuleDefinition> {
    vec![
        // ── 武器（High slot）────────────────────────────────────────────────
        ModuleDefinition {
            id   : MODULE_RAILGUN_SMALL,
            name : "Small Railgun I".to_string(),
            kind : ModuleKind::Weapon,
            slot : SlotKind::High,
            stat_delta: StatDelta {
                weapon_damage_add   : 25.0,
                weapon_range_add    : 1_500.0,
                weapon_cooldown_add : 0,  // base cooldown = 1 → 5 ticks合計は NPC 側で設定
                ..StatDelta::ZERO
            },
        },
        ModuleDefinition {
            id   : MODULE_RAILGUN_MEDIUM,
            name : "Medium Railgun I".to_string(),
            kind : ModuleKind::Weapon,
            slot : SlotKind::High,
            stat_delta: StatDelta {
                weapon_damage_add   : 50.0,
                weapon_range_add    : 2_500.0,
                weapon_cooldown_add : 4,  // 発射間隔が長い
                ..StatDelta::ZERO
            },
        },

        // ── シールド（Mid slot）─────────────────────────────────────────────
        ModuleDefinition {
            id   : MODULE_SHIELD_BASIC,
            name : "Basic Shield Extender".to_string(),
            kind : ModuleKind::ShieldBooster,
            slot : SlotKind::Mid,
            stat_delta: StatDelta {
                max_hp_add : 300.0,
                ..StatDelta::ZERO
            },
        },

        // ── アーマー（Low slot）─────────────────────────────────────────────
        ModuleDefinition {
            id   : MODULE_ARMOR_BASIC,
            name : "Basic Armor Plate".to_string(),
            kind : ModuleKind::ArmorRepairer,
            slot : SlotKind::Low,
            stat_delta: StatDelta {
                max_hp_add : 200.0,
                ..StatDelta::ZERO
            },
        },

        // ── 推進（Mid slot）─────────────────────────────────────────────────
        ModuleDefinition {
            id   : MODULE_AFTERBURNER,
            name : "1MN Afterburner".to_string(),
            kind : ModuleKind::Propulsion,
            slot : SlotKind::Mid,
            stat_delta: StatDelta {
                max_speed_add : 150.0,
                thrust_add    : 10.0,
                ..StatDelta::ZERO
            },
        },

        // ── センサー（Mid slot）─────────────────────────────────────────────
        ModuleDefinition {
            id   : MODULE_SENSOR_BOOSTER,
            name : "Sensor Booster I".to_string(),
            kind : ModuleKind::Sensor,
            slot : SlotKind::Mid,
            stat_delta: StatDelta {
                lock_time_add  : -2,  // 2 Tick 短縮
                max_locks_add  : 1,   // 同時ロック +1
                ..StatDelta::ZERO
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_modules_have_unique_ids() {
        use std::collections::HashSet;
        let modules = all_modules();
        let ids: HashSet<u32> = modules.iter().map(|m| m.id.0).collect();
        assert_eq!(ids.len(), modules.len(), "各モジュールは一意な ID を持つ");
    }

    #[test]
    fn weapon_modules_are_in_high_slot() {
        for m in all_modules() {
            if m.kind == ModuleKind::Weapon {
                assert_eq!(m.slot, SlotKind::High,
                    "武器モジュール '{}' は High スロットでなければならない", m.name);
            }
        }
    }

    #[test]
    fn weapon_modules_provide_positive_damage_and_range() {
        for m in all_modules() {
            if m.kind == ModuleKind::Weapon {
                assert!(m.stat_delta.weapon_damage_add > 0.0,
                    "武器 '{}' はダメージを提供しなければならない", m.name);
                assert!(m.stat_delta.weapon_range_add > 0.0,
                    "武器 '{}' は射程を提供しなければならない", m.name);
            }
        }
    }

    #[test]
    fn base_npc_stats_have_no_weapon_capability() {
        use dawn_ecs::components::ShipStatsComp;
        assert_eq!(ShipStatsComp::NPC.weapon_damage, 0.0,
            "NPC ベーススタットに武器能力があってはならない（Fitting で付与する）");
        assert_eq!(ShipStatsComp::PLAYER.weapon_damage, 0.0,
            "PLAYER ベーススタットに武器能力があってはならない（Fitting で付与する）");
    }
}
