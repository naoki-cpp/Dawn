//! Inventory ECS component (ADR-0032).

use dawn_core::fitting::ModuleId;

/// Module instances a ship's pilot owns but has not fitted to a slot.
/// `FitModuleCommand`/`UnfitModuleCommand` move items between here and
/// `FittingComp`. Multiple entries of the same `ModuleId` represent separate
/// spare copies (modules have no individual identity yet -- ADR-0032 §"却下
/// した代替案").
///
/// Only attached to player ships (NPCs have no Fit/Unfit UI and never need
/// one).
#[derive(Debug, Clone, Default)]
pub struct InventoryComp {
    pub items: Vec<ModuleId>,
}

impl InventoryComp {
    pub fn empty() -> Self {
        Self { items: Vec::new() }
    }

    /// Remove one instance of `module_id`. Returns `true` if one was present.
    pub fn take(&mut self, module_id: ModuleId) -> bool {
        match self.items.iter().position(|&id| id == module_id) {
            Some(pos) => {
                self.items.remove(pos);
                true
            }
            None => false,
        }
    }

    /// Add one instance of `module_id` back (e.g. after Unfit).
    pub fn add(&mut self, module_id: ModuleId) {
        self.items.push(module_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_removes_one_matching_instance_and_returns_true() {
        let mut inv = InventoryComp {
            items: vec![ModuleId(1), ModuleId(2), ModuleId(1)],
        };
        assert!(inv.take(ModuleId(1)));
        assert_eq!(inv.items, vec![ModuleId(2), ModuleId(1)]);
    }

    #[test]
    fn take_returns_false_when_module_is_absent() {
        let mut inv = InventoryComp {
            items: vec![ModuleId(2)],
        };
        assert!(!inv.take(ModuleId(1)));
        assert_eq!(inv.items, vec![ModuleId(2)]);
    }

    #[test]
    fn add_appends_one_instance() {
        let mut inv = InventoryComp::empty();
        inv.add(ModuleId(5));
        inv.add(ModuleId(5));
        assert_eq!(inv.items, vec![ModuleId(5), ModuleId(5)]);
    }
}
