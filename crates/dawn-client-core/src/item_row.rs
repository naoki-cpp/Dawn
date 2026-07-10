use serde::Deserialize;

/// Wire-format mirror of `dawn_core::ItemId`'s three variants, as serialized
/// by `player_loadout_projection.rs::item_id_to_row_json`'s `"item_type"`
/// field. `Unknown` absorbs any variant added server-side before this enum
/// catches up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum ItemType {
    Module,
    PackagedShip,
    ScrapMetal,
    #[serde(other)]
    Unknown,
}

/// One row shared by `PlayerLoadout`'s `inventory` and `station_inventory`
/// arrays (ADR-0034 generalized `InventoryComp` to Module / PackagedShip /
/// ScrapMetal, so this shape stays generic enough for all three). Mirrors
/// `player_loadout_projection.rs::item_id_to_row_json`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ItemRow {
    pub item_type: ItemType,
    #[serde(default)]
    pub module_id: u32,
    #[serde(default)]
    pub ship_type_id: u32,
    pub name: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub slot: String,
    pub count: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_type_falls_back_to_unknown_for_an_unrecognized_variant_name() {
        let item_type: ItemType = serde_json::from_str("\"SomeFutureItemType\"").unwrap();
        assert_eq!(item_type, ItemType::Unknown);
    }

    #[test]
    fn item_row_parses_a_scrap_metal_row_with_empty_kind_and_slot() {
        let json = r#"{
            "item_type": "ScrapMetal",
            "module_id": 0,
            "ship_type_id": 0,
            "name": "Scrap Metal",
            "kind": "",
            "slot": "",
            "count": 3
        }"#;
        let row: ItemRow = serde_json::from_str(json).unwrap();
        assert_eq!(row.item_type, ItemType::ScrapMetal);
        assert_eq!(row.count, 3);
        assert_eq!(row.kind, "");
    }
}
