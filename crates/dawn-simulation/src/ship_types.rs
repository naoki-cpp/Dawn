//! 標準船種定義カタログ。
//!
//! modules.rs の ModuleDefinition に対応する。
//! サーバー起動時に register_ship_type() で登録する。

use dawn_core::ship_type::{ShipBaseStats, ShipClass, ShipTypeDefinition, ShipTypeId, SlotLayout};

pub const SHIP_TYPE_NPC_FRIGATE    : ShipTypeId = ShipTypeId(1);
pub const SHIP_TYPE_PLAYER_FRIGATE : ShipTypeId = ShipTypeId(2);

pub fn all_ship_types() -> Vec<ShipTypeDefinition> {
    vec![
        ShipTypeDefinition {
            id         : SHIP_TYPE_NPC_FRIGATE,
            name       : "NPC Frigate".to_string(),
            class      : ShipClass::Frigate,
            slot_layout: SlotLayout::FRIGATE,
            base_stats : ShipBaseStats {
                max_speed        : 400.0,
                thrust_magnitude : 0.0,   // NPC: 等速直線運動
                max_shield       : 200.0,
                max_armor        : 150.0,
                max_hull         : 150.0,
                lock_time        : 5,
                max_locks        : 1,
            },
        },
        ShipTypeDefinition {
            id         : SHIP_TYPE_PLAYER_FRIGATE,
            name       : "Player Frigate".to_string(),
            class      : ShipClass::Frigate,
            slot_layout: SlotLayout::FRIGATE,
            base_stats : ShipBaseStats {
                max_speed        : 500.0,
                thrust_magnitude : 40.0,
                max_shield       : 500.0,
                max_armor        : 300.0,
                max_hull         : 200.0,
                lock_time        : 3,
                max_locks        : 2,
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
    fn player_frigate_has_three_layer_hp() {
        let player = all_ship_types().into_iter()
            .find(|t| t.id == SHIP_TYPE_PLAYER_FRIGATE).unwrap();
        assert!(player.base_stats.max_shield > 0.0);
        assert!(player.base_stats.max_armor  > 0.0);
        assert!(player.base_stats.max_hull   > 0.0);
    }
}
