//! Fitting ECS components.

use dawn_core::fitting::{
    ActivationMode, FittingSnapshot, ModuleDefinition, ModuleId, SlotEntry, SlotKind, StatDelta,
};
use dawn_core::ShipId;
use std::collections::HashMap;

/// One fitted slot (module definition + runtime activation state).
#[derive(Debug, Clone)]
pub struct FittedSlot {
    pub def: ModuleDefinition,
    /// Active modules only.  Passive modules are always effective.
    pub is_active: bool,
    /// Ticks remaining in the current activation cycle.
    ///
    /// `0`  — the cycle is over; the capacitor system will attempt to start a
    ///         new cycle at the next tick (consuming `cap_cost_per_cycle`).
    /// `>0` — counting down; no cap is consumed until this reaches 0.
    ///
    /// Always `0` for Passive modules (cycle concept does not apply).
    pub cycle_remaining: u64,
    /// Per-slot target for kinds where `ModuleKind::requires_target()` is true
    /// (Weapon, Tackle), per ADR-0035. Set on activation, cleared on
    /// deactivation. `None` for self-only modules.
    pub target_ship_id: Option<ShipId>,
}

impl FittedSlot {
    /// Whether this slot's effect is currently applied.
    /// Passive modules are always effective; Active modules depend on `is_active`.
    pub fn is_effective(&self) -> bool {
        match self.def.activation_mode {
            ActivationMode::Passive => true,
            ActivationMode::Active => self.is_active,
        }
    }
}

/// Ship の装備スロット全体を保持するコンポーネント。
///
/// 装備変更または活性化状態変更後は必ず `apply_fitting()` を呼び出し
/// `ShipStatsComp` を再集計すること。
#[derive(Debug, Clone)]
pub struct FittingComp {
    pub high: Vec<FittedSlot>,
    pub mid: Vec<FittedSlot>,
    pub low: Vec<FittedSlot>,
    pub rig: Vec<FittedSlot>,
}

impl FittingComp {
    /// スロットが全て空の初期状態。
    pub fn empty() -> Self {
        Self {
            high: Vec::new(),
            mid: Vec::new(),
            low: Vec::new(),
            rig: Vec::new(),
        }
    }

    /// Iterates every slot in High → Mid → Low → Rig order. `flat_idx` (the
    /// flat, all-slots index used by the Capacitor System etc.) matches this
    /// order — the counterpart is `slot_at_flat`/`slot_at_flat_mut`.
    pub fn iter_slots(&self) -> impl Iterator<Item = &FittedSlot> {
        self.high
            .iter()
            .chain(self.mid.iter())
            .chain(self.low.iter())
            .chain(self.rig.iter())
    }

    /// Mutable version of `iter_slots`.
    pub fn iter_slots_mut(&mut self) -> impl Iterator<Item = &mut FittedSlot> {
        self.high
            .iter_mut()
            .chain(self.mid.iter_mut())
            .chain(self.low.iter_mut())
            .chain(self.rig.iter_mut())
    }

    /// Returns true if any effective slot has the given module kind.
    /// Used by the Tackle System to check for active Fold Disruptors (ADR-0024).
    pub fn has_active_module_of_kind(&self, kind: dawn_core::fitting::ModuleKind) -> bool {
        self.iter_slots()
            .any(|s| s.def.kind == kind && s.is_effective())
    }

    /// 有効なスロット（Passive または Active ON）の `StatDelta` を合計して返す。
    pub fn total_delta(&self) -> StatDelta {
        self.iter_slots()
            .filter(|slot| slot.is_effective())
            .fold(StatDelta::ZERO, |acc, slot| acc.add(&slot.def.stat_delta))
    }

    /// スロットに対応する `Vec` への可変参照を返す。
    pub fn slot_mut(&mut self, kind: SlotKind) -> &mut Vec<FittedSlot> {
        match kind {
            SlotKind::High => &mut self.high,
            SlotKind::Mid => &mut self.mid,
            SlotKind::Low => &mut self.low,
            SlotKind::Rig => &mut self.rig,
        }
    }

    /// 読み取り専用版の `slot_mut`（ADR-0032）: 容量チェックなど、変更せずに
    /// 件数や内容を見るだけの呼び出し元向け。
    pub fn slot(&self, kind: SlotKind) -> &[FittedSlot] {
        match kind {
            SlotKind::High => &self.high,
            SlotKind::Mid => &self.mid,
            SlotKind::Low => &self.low,
            SlotKind::Rig => &self.rig,
        }
    }

    /// Resolves the slot at `flat_idx`, the index into the `iter_slots` order
    /// (High → Mid → Low → Rig). The single place the Capacitor System and
    /// Range Gate System resolve this index back to a slot — centralizes the
    /// High/Mid/Low boundary arithmetic instead of each caller re-deriving
    /// `high.len()`/`mid.len()`/`low.len()`.
    pub fn slot_at_flat(&self, flat_idx: usize) -> Option<&FittedSlot> {
        let high_len = self.high.len();
        let mid_len = self.mid.len();
        let low_len = self.low.len();
        if flat_idx < high_len {
            self.high.get(flat_idx)
        } else if flat_idx < high_len + mid_len {
            self.mid.get(flat_idx - high_len)
        } else if flat_idx < high_len + mid_len + low_len {
            self.low.get(flat_idx - high_len - mid_len)
        } else {
            self.rig.get(flat_idx - high_len - mid_len - low_len)
        }
    }

    /// Mutable version of `slot_at_flat`.
    pub fn slot_at_flat_mut(&mut self, flat_idx: usize) -> Option<&mut FittedSlot> {
        let high_len = self.high.len();
        let mid_len = self.mid.len();
        let low_len = self.low.len();
        if flat_idx < high_len {
            self.high.get_mut(flat_idx)
        } else if flat_idx < high_len + mid_len {
            self.mid.get_mut(flat_idx - high_len)
        } else if flat_idx < high_len + mid_len + low_len {
            self.low.get_mut(flat_idx - high_len - mid_len)
        } else {
            self.rig.get_mut(flat_idx - high_len - mid_len - low_len)
        }
    }

    /// 指定 module_id を持つスロットの可変参照を返す。
    pub fn find_slot_mut(
        &mut self,
        module_id: ModuleId,
        slot_kind: SlotKind,
    ) -> Option<&mut FittedSlot> {
        self.slot_mut(slot_kind)
            .iter_mut()
            .find(|s| s.def.id == module_id)
    }

    /// 現在の装備状態を `FittingSnapshot` に変換する。
    /// `ShipFitted` イベントへの埋め込みに使用する。
    pub fn to_snapshot(&self) -> FittingSnapshot {
        let to_entries = |slots: &[FittedSlot]| -> Vec<SlotEntry> {
            slots
                .iter()
                .map(|s| SlotEntry {
                    module_id: s.def.id,
                    is_active: s.is_active,
                })
                .collect()
        };
        FittingSnapshot {
            high: to_entries(&self.high),
            mid: to_entries(&self.mid),
            low: to_entries(&self.low),
            rig: to_entries(&self.rig),
        }
    }

    /// `FittingSnapshot` から `FittingComp` を復元する（Event Replay 用）。
    pub fn from_snapshot(
        snapshot: &FittingSnapshot,
        registry: &HashMap<ModuleId, ModuleDefinition>,
    ) -> Self {
        let resolve = |entries: &[SlotEntry]| -> Vec<FittedSlot> {
            entries
                .iter()
                .filter_map(|e| {
                    registry.get(&e.module_id).map(|def| FittedSlot {
                        def: def.clone(),
                        is_active: e.is_active,
                        cycle_remaining: 0, // Cycle state is not persisted; reset to 0.
                        target_ship_id: None, // Target is not persisted; re-selected on next activation.
                    })
                })
                .collect()
        };
        Self {
            high: resolve(&snapshot.high),
            mid: resolve(&snapshot.mid),
            low: resolve(&snapshot.low),
            rig: resolve(&snapshot.rig),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::fitting::{ActivationMode, ModuleKind, StatDelta};

    fn weapon_slot(active: bool) -> FittedSlot {
        FittedSlot {
            def: ModuleDefinition {
                id: ModuleId(1),
                name: "Railgun".to_string(),
                kind: ModuleKind::Weapon,
                slot: SlotKind::High,
                stat_delta: StatDelta {
                    weapon_damage_add: 25.0,
                    ..StatDelta::ZERO
                },
                activation_mode: ActivationMode::Active,
                cap_cost_per_cycle: 60.0,
                cycle_time_ticks: 10,
            },
            is_active: active,
            cycle_remaining: 0,
            target_ship_id: None,
        }
    }

    fn shield_slot() -> FittedSlot {
        FittedSlot {
            def: ModuleDefinition {
                id: ModuleId(2),
                name: "Shield".to_string(),
                kind: ModuleKind::ShieldBooster,
                slot: SlotKind::Mid,
                stat_delta: StatDelta {
                    max_shield_add: 300.0,
                    ..StatDelta::ZERO
                },
                activation_mode: ActivationMode::Passive,
                cap_cost_per_cycle: 0.0,
                cycle_time_ticks: 0,
            },
            is_active: false, // Passive: is_active is ignored
            cycle_remaining: 0,
            target_ship_id: None,
        }
    }

    #[test]
    fn empty_fitting_produces_zero_delta() {
        assert_eq!(FittingComp::empty().total_delta(), StatDelta::ZERO);
    }

    #[test]
    fn slot_at_flat_resolves_across_high_mid_low_rig_boundaries() {
        let mut fitting = FittingComp::empty();
        fitting.high.push(weapon_slot(true)); // flat_idx 0
        fitting.mid.push(shield_slot()); // flat_idx 1
        fitting.low.push(weapon_slot(false)); // flat_idx 2 (id reused for brevity)
        fitting.rig.push(shield_slot()); // flat_idx 3

        assert_eq!(fitting.slot_at_flat(0).unwrap().def.id, ModuleId(1));
        assert_eq!(fitting.slot_at_flat(1).unwrap().def.id, ModuleId(2));
        assert_eq!(fitting.slot_at_flat(2).unwrap().def.id, ModuleId(1));
        assert_eq!(fitting.slot_at_flat(3).unwrap().def.id, ModuleId(2));
        assert!(
            fitting.slot_at_flat(4).is_none(),
            "out of range must be None"
        );
    }

    #[test]
    fn slot_at_flat_mut_allows_in_place_mutation() {
        let mut fitting = FittingComp::empty();
        fitting.high.push(weapon_slot(false));
        fitting.mid.push(shield_slot());

        fitting.slot_at_flat_mut(1).unwrap().is_active = true;

        assert!(!fitting.high[0].is_active);
        assert!(fitting.mid[0].is_active);
    }

    #[test]
    fn slot_at_flat_handles_empty_leading_vecs() {
        // High/Mid empty, so flat_idx 0 must resolve into Low, not panic.
        let mut fitting = FittingComp::empty();
        fitting.low.push(weapon_slot(true));
        assert_eq!(fitting.slot_at_flat(0).unwrap().def.id, ModuleId(1));
    }

    #[test]
    fn iter_slots_visits_high_mid_low_rig_in_order() {
        let mut fitting = FittingComp::empty();
        fitting.high.push(weapon_slot(true));
        fitting.mid.push(shield_slot());
        let ids: Vec<ModuleId> = fitting.iter_slots().map(|s| s.def.id).collect();
        assert_eq!(ids, vec![ModuleId(1), ModuleId(2)]);
    }

    #[test]
    fn passive_module_always_applies_delta_regardless_of_is_active() {
        let mut fitting = FittingComp::empty();
        fitting.mid.push(shield_slot()); // is_active=false だが Passive なので有効
        let delta = fitting.total_delta();
        assert_eq!(
            delta.max_shield_add, 300.0,
            "Passive module is always effective"
        );
    }

    #[test]
    fn active_module_on_applies_delta() {
        let mut fitting = FittingComp::empty();
        fitting.high.push(weapon_slot(true));
        assert_eq!(fitting.total_delta().weapon_damage_add, 25.0);
    }

    #[test]
    fn active_module_off_does_not_apply_delta() {
        let mut fitting = FittingComp::empty();
        fitting.high.push(weapon_slot(false));
        assert_eq!(
            fitting.total_delta().weapon_damage_add,
            0.0,
            "Active module OFF does not contribute to stats"
        );
    }

    #[test]
    fn snapshot_round_trip_preserves_module_id_and_activation_state() {
        let mut fitting = FittingComp::empty();
        fitting.high.push(weapon_slot(true));
        fitting.mid.push(shield_slot());
        let snap = fitting.to_snapshot();
        assert_eq!(snap.high[0].module_id, ModuleId(1));
        assert!(snap.high[0].is_active);
        assert_eq!(snap.mid[0].module_id, ModuleId(2));
    }

    #[test]
    fn from_snapshot_restores_activation_state() {
        let mut registry = HashMap::new();
        let weapon_def = weapon_slot(false).def;
        registry.insert(ModuleId(1), weapon_def);

        let snap = FittingSnapshot {
            high: vec![SlotEntry {
                module_id: ModuleId(1),
                is_active: true,
            }],
            mid: vec![],
            low: vec![],
            rig: vec![],
        };
        let fitting = FittingComp::from_snapshot(&snap, &registry);
        assert!(fitting.high[0].is_active);
        assert_eq!(fitting.total_delta().weapon_damage_add, 25.0);
    }
}
