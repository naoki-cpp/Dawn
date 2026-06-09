//! 標準船種定義カタログ。
//!
//! modules.rs の ModuleDefinition に対応する。
//! サーバー起動時に register_ship_type() で登録する。

use dawn_core::ship_type::{ShipBaseStats, ShipClass, ShipTypeDefinition, ShipTypeId, SlotLayout};

pub const SHIP_TYPE_NPC_FRIGATE : ShipTypeId = ShipTypeId(1);
pub const SHIP_TYPE_MAGPIE      : ShipTypeId = ShipTypeId(7);

pub fn all_ship_types() -> Vec<ShipTypeDefinition> {
    vec![
        ShipTypeDefinition {
            id         : SHIP_TYPE_NPC_FRIGATE,
            name       : "NPC Frigate".to_string(),
            class      : ShipClass::Frigate,
            slot_layout: SlotLayout::FRIGATE,
            base_stats : ShipBaseStats {
                max_speed            : 400.0,
                thrust_magnitude     : 0.0,    // NPC: constant velocity
                max_shield           : 200.0,
                max_armor            : 150.0,
                max_hull             : 150.0,
                lock_time            : 5,
                max_locks            : 1,
                cap_max              : 300.0,
                cap_recharge_per_tick: 6.0,    // 2 %/tick
                sig_radius           : 40.0,
            },
        },
        // Voral Drift Tackler/Interceptor frigate
        ShipTypeDefinition {
            id         : SHIP_TYPE_MAGPIE,
            name       : "Magpie".to_string(),
            class      : ShipClass::Frigate,
            slot_layout: SlotLayout { high: 2, mid: 4, low: 2, rig: 3 },
            base_stats : ShipBaseStats {
                max_speed            : 700.0,
                thrust_magnitude     : 65.0,
                max_shield           : 200.0,
                max_armor            : 120.0,
                max_hull             : 100.0,
                lock_time            : 2,
                max_locks            : 2,
                cap_max              : 350.0,
                cap_recharge_per_tick: 8.0,
                sig_radius           : 40.0,  // small fast frigate
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_ship_types_have_unique_ids() {
        use std::collections::HashSet;
        let ids: HashSet<u32> = all_ship_types().iter().map(|t| t.id.0).collect();
        assert_eq!(ids.len(), all_ship_types().len());
    }

    #[test]
    fn npc_frigate_has_zero_thrust() {
        let npc = all_ship_types().into_iter()
            .find(|t| t.id == SHIP_TYPE_NPC_FRIGATE).unwrap();
        assert_eq!(npc.base_stats.thrust_magnitude, 0.0);
    }

    #[test]
    fn magpie_has_three_layer_hp() {
        let magpie = all_ship_types().into_iter()
            .find(|t| t.id == SHIP_TYPE_MAGPIE).unwrap();
        assert!(magpie.base_stats.max_shield > 0.0);
        assert!(magpie.base_stats.max_armor  > 0.0);
        assert!(magpie.base_stats.max_hull   > 0.0);
    }
}
