use dawn_core::ItemId;
use serde::Deserialize;

/// One row shared by `PlayerLoadout`'s ship and station inventory arrays.
///
/// `item_id` is the same canonical variant used by the server domain. Display
/// metadata remains separate and cannot alter the Item identity.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemRow {
    pub item_id: ItemId,
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
    use dawn_core::{ModuleId, ShipTypeId};

    #[test]
    fn every_item_variant_parses_with_canonical_identity() {
        for (json, expected) in [
            (
                r#"{"item_id":{"Module":3},"name":"Railgun","kind":"Weapon","slot":"High","count":2}"#,
                ItemId::Module(ModuleId(3)),
            ),
            (
                r#"{"item_id":{"PackagedShip":7},"name":"Magpie","kind":"","slot":"","count":1}"#,
                ItemId::PackagedShip(ShipTypeId(7)),
            ),
            (
                r#"{"item_id":"ScrapMetal","name":"Scrap Metal","kind":"","slot":"","count":4}"#,
                ItemId::ScrapMetal,
            ),
        ] {
            let row: ItemRow = serde_json::from_str(json).unwrap();
            assert_eq!(row.item_id, expected);
        }
    }
}
