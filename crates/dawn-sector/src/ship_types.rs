//! Stable ship-type identifiers.
//!
//! Balance values are authoritative in `data/ship_types.toml` and are loaded by
//! [`crate::game_data::GameDataCatalog`].

use dawn_core::ship_type::{ShipTypeDefinition, ShipTypeId};

pub const SHIP_TYPE_NPC_FRIGATE: ShipTypeId = ShipTypeId(1);
pub const SHIP_TYPE_NPC_DESTROYER: ShipTypeId = ShipTypeId(3);
pub const SHIP_TYPE_NPC_CRUISER: ShipTypeId = ShipTypeId(4);
pub const SHIP_TYPE_PLAYER_DESTROYER: ShipTypeId = ShipTypeId(5);
pub const SHIP_TYPE_PLAYER_CRUISER: ShipTypeId = ShipTypeId(6);
pub const SHIP_TYPE_MAGPIE: ShipTypeId = ShipTypeId(7);

pub(crate) const REQUIRED_SHIP_TYPE_IDS: &[ShipTypeId] = &[
    SHIP_TYPE_NPC_FRIGATE,
    SHIP_TYPE_NPC_DESTROYER,
    SHIP_TYPE_NPC_CRUISER,
    SHIP_TYPE_PLAYER_DESTROYER,
    SHIP_TYPE_PLAYER_CRUISER,
    SHIP_TYPE_MAGPIE,
];

/// Compatibility accessor for callers that need the repository's production
/// definitions in tests or tooling. Runtime startup should load one
/// [`crate::game_data::GameDataCatalog`] and register it as a unit.
pub fn all_ship_types() -> Vec<ShipTypeDefinition> {
    crate::game_data::repository_catalog()
        .unwrap_or_else(|error| panic!("failed to load repository game-data catalog: {error}"))
        .ship_types()
        .to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_required_ship_type_ids_are_unique() {
        use std::collections::HashSet;
        let ids: HashSet<_> = REQUIRED_SHIP_TYPE_IDS.iter().copied().collect();
        assert_eq!(ids.len(), REQUIRED_SHIP_TYPE_IDS.len());
    }

    #[test]
    fn npc_frigate_has_positive_mass_and_inertia() {
        let npc = all_ship_types()
            .into_iter()
            .find(|definition| definition.id == SHIP_TYPE_NPC_FRIGATE)
            .expect("NPC Frigate");
        assert!(npc.base_stats.mass > 0.0);
        assert!(npc.base_stats.inertia_modifier > 0.0);
    }

    #[test]
    fn only_the_magpie_is_buildable() {
        let ship_types = all_ship_types();
        let buildable: Vec<_> = ship_types
            .iter()
            .filter(|definition| definition.buildable)
            .map(|definition| definition.id)
            .collect();
        assert_eq!(buildable, vec![SHIP_TYPE_MAGPIE]);
    }

    #[test]
    fn magpie_has_three_layer_hp() {
        let magpie = all_ship_types()
            .into_iter()
            .find(|definition| definition.id == SHIP_TYPE_MAGPIE)
            .expect("Magpie");
        assert!(magpie.base_stats.max_shield > 0.0);
        assert!(magpie.base_stats.max_armor > 0.0);
        assert!(magpie.base_stats.max_hull > 0.0);
    }
}
