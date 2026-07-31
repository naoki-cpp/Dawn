use dawn_core::{ItemId, ModuleId, ShipTypeId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Variant-preserving Item identity used by every client/server wire surface.
///
/// The externally tagged representation is postcard-compatible and carries
/// only the identifier owned by the selected variant:
/// `{"Module":{"module_id":3}}`,
/// `{"PackagedShip":{"ship_type_id":7}}`, or `"ScrapMetal"` in JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ItemWire {
    Module { module_id: u32 },
    PackagedShip { ship_type_id: u32 },
    ScrapMetal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemWireError {
    ZeroModuleId,
    ZeroShipTypeId,
}

impl From<ItemId> for ItemWire {
    fn from(item_id: ItemId) -> Self {
        match item_id {
            ItemId::Module(module_id) => Self::Module {
                module_id: module_id.0,
            },
            ItemId::PackagedShip(ship_type_id) => Self::PackagedShip {
                ship_type_id: ship_type_id.0,
            },
            ItemId::ScrapMetal => Self::ScrapMetal,
        }
    }
}

impl TryFrom<ItemWire> for ItemId {
    type Error = ItemWireError;

    fn try_from(item: ItemWire) -> Result<Self, Self::Error> {
        match item {
            ItemWire::Module { module_id: 0 } => Err(ItemWireError::ZeroModuleId),
            ItemWire::Module { module_id } => Ok(Self::Module(ModuleId(module_id))),
            ItemWire::PackagedShip { ship_type_id: 0 } => {
                Err(ItemWireError::ZeroShipTypeId)
            }
            ItemWire::PackagedShip { ship_type_id } => {
                Ok(Self::PackagedShip(ShipTypeId(ship_type_id)))
            }
            ItemWire::ScrapMetal => Ok(Self::ScrapMetal),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_item_variant_round_trips_through_wire_identity() {
        for item_id in [
            ItemId::Module(ModuleId(3)),
            ItemId::PackagedShip(ShipTypeId(7)),
            ItemId::ScrapMetal,
        ] {
            assert_eq!(ItemId::try_from(ItemWire::from(item_id)), Ok(item_id));
        }
    }

    #[test]
    fn wire_identity_rejects_zero_ids() {
        assert_eq!(
            ItemId::try_from(ItemWire::Module { module_id: 0 }),
            Err(ItemWireError::ZeroModuleId)
        );
        assert_eq!(
            ItemId::try_from(ItemWire::PackagedShip { ship_type_id: 0 }),
            Err(ItemWireError::ZeroShipTypeId)
        );
    }

    #[test]
    fn wire_identity_round_trips_through_postcard() {
        for item in [
            ItemWire::Module { module_id: 3 },
            ItemWire::PackagedShip { ship_type_id: 7 },
            ItemWire::ScrapMetal,
        ] {
            let bytes = postcard::to_stdvec(&item).expect("encode ItemWire");
            let decoded: ItemWire = postcard::from_bytes(&bytes).expect("decode ItemWire");
            assert_eq!(decoded, item);
        }
    }
}
