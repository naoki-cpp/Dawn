use dawn_core::ItemId;
use serde::{de::Error as _, Deserialize, Deserializer};

/// One row shared by `PlayerLoadout`'s ship and station inventory arrays.
///
/// `item_id` is the same canonical variant used by the server domain. Display
/// metadata remains separate and cannot alter the Item identity.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemRow {
    pub item_id: ItemId,
    pub name: String,
    pub kind: String,
    pub slot: String,
    pub count: u64,
}

impl ItemRow {
    /// Compatibility projections for the existing Godot UI. Rust callers do
    /// not need string tags or unused-ID sentinels.
    pub const fn item_type(&self) -> &'static str {
        self.item_id.storage_columns().item_type()
    }

    pub const fn module_id(&self) -> u32 {
        self.item_id.storage_columns().module_id()
    }

    pub const fn ship_type_id(&self) -> u32 {
        self.item_id.storage_columns().ship_type_id()
    }
}

#[derive(Deserialize)]
struct ItemRowRepr {
    #[serde(default)]
    item_id: Option<ItemId>,
    #[serde(default)]
    item_type: Option<String>,
    #[serde(default)]
    module_id: u32,
    #[serde(default)]
    ship_type_id: u32,
    name: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    slot: String,
    count: u64,
}

impl<'de> Deserialize<'de> for ItemRow {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let row = ItemRowRepr::deserialize(deserializer)?;
        let item_id = match (row.item_id, row.item_type.as_deref()) {
            (Some(item_id), None) if row.module_id == 0 && row.ship_type_id == 0 => item_id,
            (None, Some(item_type)) => {
                ItemId::from_storage_columns(item_type, row.module_id, row.ship_type_id)
                    .map_err(D::Error::custom)?
            }
            _ => {
                return Err(D::Error::custom(
                    "item row must contain exactly one canonical or legacy identity",
                ));
            }
        };

        Ok(Self {
            item_id,
            name: row.name,
            kind: row.kind,
            slot: row.slot,
            count: row.count,
        })
    }
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

    #[test]
    fn legacy_scrap_metal_row_still_parses() {
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
        assert_eq!(row.item_id, ItemId::ScrapMetal);
        assert_eq!(row.count, 3);
    }

    #[test]
    fn invalid_legacy_identity_is_rejected() {
        let json = r#"{
            "item_type": "Module",
            "module_id": 3,
            "ship_type_id": 7,
            "name": "Impossible",
            "count": 1
        }"#;
        assert!(serde_json::from_str::<ItemRow>(json).is_err());
    }
}
