//! Inventory ECS component (ADR-0032, generalized by ADR-0034).

use std::collections::BTreeMap;

use dawn_core::{fitting::ModuleId, ItemId};

/// Item stacks a ship's pilot owns but has not fitted to a slot or assembled
/// into an active hull (ADR-0034).
///
/// Only attached to player ships (NPCs have no Fit/Unfit UI and never need
/// one).
#[derive(Debug, Clone, Default)]
pub struct InventoryComp {
    pub items: BTreeMap<ItemId, u64>,
}

impl InventoryComp {
    pub fn empty() -> Self {
        Self {
            items: BTreeMap::new(),
        }
    }

    /// Remove one instance of `module_id`. Returns `true` if one was present.
    pub fn take(&mut self, module_id: ModuleId) -> bool {
        match self.items.get_mut(&ItemId::Module(module_id)) {
            Some(count) if *count > 1 => {
                *count -= 1;
                true
            }
            Some(_) => {
                self.items.remove(&ItemId::Module(module_id));
                true
            }
            None => false,
        }
    }

    /// Add one instance of `module_id` back (e.g. after Unfit).
    pub fn add(&mut self, module_id: ModuleId) {
        self.add_item(ItemId::Module(module_id), 1);
    }

    pub fn module_count(&self, module_id: ModuleId) -> u64 {
        self.items
            .get(&ItemId::Module(module_id))
            .copied()
            .unwrap_or(0)
    }

    pub fn add_item(&mut self, item_id: ItemId, count: u64) {
        if count == 0 {
            return;
        }
        *self.items.entry(item_id).or_default() += count;
    }

    pub fn item_count(&self, item_id: ItemId) -> u64 {
        self.items.get(&item_id).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_removes_one_matching_instance_and_returns_true() {
        let mut inv = InventoryComp {
            items: BTreeMap::from([
                (ItemId::Module(ModuleId(1)), 2),
                (ItemId::Module(ModuleId(2)), 1),
            ]),
        };
        assert!(inv.take(ModuleId(1)));
        assert_eq!(inv.module_count(ModuleId(1)), 1);
        assert_eq!(inv.module_count(ModuleId(2)), 1);
    }

    #[test]
    fn take_returns_false_when_module_is_absent() {
        let mut inv = InventoryComp {
            items: BTreeMap::from([(ItemId::Module(ModuleId(2)), 1)]),
        };
        assert!(!inv.take(ModuleId(1)));
        assert_eq!(inv.module_count(ModuleId(2)), 1);
    }

    #[test]
    fn add_appends_one_instance() {
        let mut inv = InventoryComp::empty();
        inv.add(ModuleId(5));
        inv.add(ModuleId(5));
        assert_eq!(inv.module_count(ModuleId(5)), 2);
    }

    #[test]
    fn add_item_accumulates_stack_counts_by_item_id() {
        let mut inv = InventoryComp::empty();
        inv.add_item(ItemId::ScrapMetal, 3);
        inv.add_item(ItemId::ScrapMetal, 2);
        assert_eq!(inv.item_count(ItemId::ScrapMetal), 5);
    }
}
