use crate::fitting::ModuleId;
use crate::ship_type::ShipTypeId;
use serde::{Deserialize, Serialize};

/// Identifier for an inventory item (ADR-0034).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ItemId {
    Module(ModuleId),
    PackagedShip(ShipTypeId),
    ScrapMetal,
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
