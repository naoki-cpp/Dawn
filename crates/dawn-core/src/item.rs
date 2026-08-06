use crate::fitting::ModuleId;
use crate::ship_type::ShipTypeId;
#[cfg(feature = "schema")]
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

/// Canonical identity for every inventory and Market item (ADR-0034).
///
/// The variant owns the only identifier that is valid for that item kind, so
/// impossible combinations such as a Module carrying a ShipTypeId cannot be
/// represented in domain code.
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum ItemId {
    Module(ModuleId),
    PackagedShip(ShipTypeId),
    ScrapMetal,
}

impl<'de> Deserialize<'de> for ItemId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        enum ItemIdRepr {
            Module(ModuleId),
            PackagedShip(ShipTypeId),
            ScrapMetal,
        }

        match ItemIdRepr::deserialize(deserializer)? {
            ItemIdRepr::Module(ModuleId(0)) => Err(serde::de::Error::custom(
                "Module Item identity must be non-zero",
            )),
            ItemIdRepr::PackagedShip(ShipTypeId(0)) => Err(serde::de::Error::custom(
                "PackagedShip Item identity must be non-zero",
            )),
            ItemIdRepr::Module(module_id) => Ok(Self::Module(module_id)),
            ItemIdRepr::PackagedShip(ship_type_id) => Ok(Self::PackagedShip(ship_type_id)),
            ItemIdRepr::ScrapMetal => Ok(Self::ScrapMetal),
        }
    }
}

/// The legacy SQLite column representation used by both Station inventory and
/// the Market order book.
///
/// Fields stay private so callers cannot manufacture an invalid combination.
/// Use [`ItemId::storage_columns`] to encode and
/// [`ItemId::from_storage_columns`] to decode existing rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemStorageColumns {
    item_type: &'static str,
    module_id: u32,
    ship_type_id: u32,
}

impl ItemStorageColumns {
    pub const fn item_type(self) -> &'static str {
        self.item_type
    }

    pub const fn module_id(self) -> u32 {
        self.module_id
    }

    pub const fn ship_type_id(self) -> u32 {
        self.ship_type_id
    }

    pub const fn into_tuple(self) -> (&'static str, u32, u32) {
        (self.item_type, self.module_id, self.ship_type_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ItemIdentityError {
    #[error("unknown item type {0:?}")]
    UnknownItemType(String),
    #[error(
        "invalid item identity for {item_type:?}: module_id={module_id}, ship_type_id={ship_type_id}"
    )]
    InvalidStorageColumns {
        item_type: String,
        module_id: u32,
        ship_type_id: u32,
    },
}

impl ItemId {
    /// Encode the canonical identity into the existing persistent column
    /// layout. This preserves all currently deployed SQLite data without
    /// making the flattened layout part of the domain interface.
    pub const fn storage_columns(self) -> ItemStorageColumns {
        match self {
            Self::Module(module_id) => ItemStorageColumns {
                item_type: "Module",
                module_id: module_id.0,
                ship_type_id: 0,
            },
            Self::PackagedShip(ship_type_id) => ItemStorageColumns {
                item_type: "PackagedShip",
                module_id: 0,
                ship_type_id: ship_type_id.0,
            },
            Self::ScrapMetal => ItemStorageColumns {
                item_type: "ScrapMetal",
                module_id: 0,
                ship_type_id: 0,
            },
        }
    }

    /// Decode the existing persistent column layout into the canonical
    /// identity, rejecting unknown kinds, zero IDs for ID-bearing variants,
    /// and unrelated non-zero sentinel columns.
    pub fn from_storage_columns(
        item_type: &str,
        module_id: u32,
        ship_type_id: u32,
    ) -> Result<Self, ItemIdentityError> {
        let invalid = || ItemIdentityError::InvalidStorageColumns {
            item_type: item_type.to_owned(),
            module_id,
            ship_type_id,
        };

        match item_type {
            "Module" if module_id != 0 && ship_type_id == 0 => {
                Ok(Self::Module(ModuleId(module_id)))
            }
            "PackagedShip" if module_id == 0 && ship_type_id != 0 => {
                Ok(Self::PackagedShip(ShipTypeId(ship_type_id)))
            }
            "ScrapMetal" if module_id == 0 && ship_type_id == 0 => Ok(Self::ScrapMetal),
            "Module" | "PackagedShip" | "ScrapMetal" => Err(invalid()),
            other => Err(ItemIdentityError::UnknownItemType(other.to_owned())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_item_variant() -> [ItemId; 3] {
        [
            ItemId::Module(ModuleId(2)),
            ItemId::PackagedShip(ShipTypeId(7)),
            ItemId::ScrapMetal,
        ]
    }

    #[test]
    fn item_ids_have_a_stable_total_order_for_deterministic_maps() {
        let mut items = vec![
            ItemId::ScrapMetal,
            ItemId::Module(ModuleId(2)),
            ItemId::PackagedShip(ShipTypeId(1)),
            ItemId::Module(ModuleId(1)),
        ];
        items.sort();
        assert_eq!(
            items,
            vec![
                ItemId::Module(ModuleId(1)),
                ItemId::Module(ModuleId(2)),
                ItemId::PackagedShip(ShipTypeId(1)),
                ItemId::ScrapMetal,
            ]
        );
    }

    #[test]
    fn every_item_variant_round_trips_through_storage_columns() {
        for item_id in every_item_variant() {
            let columns = item_id.storage_columns();
            let decoded = ItemId::from_storage_columns(
                columns.item_type(),
                columns.module_id(),
                columns.ship_type_id(),
            )
            .expect("canonical columns must decode");
            assert_eq!(decoded, item_id);
        }
    }

    #[test]
    fn invalid_storage_combinations_are_rejected() {
        for columns in [
            ("Module", 0, 0),
            ("Module", 3, 7),
            ("PackagedShip", 1, 7),
            ("PackagedShip", 0, 0),
            ("ScrapMetal", 1, 0),
            ("Unknown", 0, 0),
        ] {
            assert!(ItemId::from_storage_columns(columns.0, columns.1, columns.2).is_err());
        }
    }
}
