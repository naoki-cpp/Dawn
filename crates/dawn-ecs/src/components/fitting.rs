//! Fitting ECS components.

use dawn_core::fitting::{FittingSnapshot, ModuleDefinition, ModuleId, SlotKind, StatDelta};
use std::collections::HashMap;

/// Ship の装備スロット全体を保持するコンポーネント。
///
/// 装備変更後は必ず `apply_fitting()` システムを呼び出し
/// `ShipStatsComp` を再集計すること。
///
/// # 設計メモ
/// モジュール定義の解決（ID → 定義）は `SimulationNode.module_registry` が担う。
/// `FittingComp` は装備内容（定義オブジェクト）のみを保持し、registry を持たない。
#[derive(Debug, Clone)]
pub struct FittingComp {
    pub high : Vec<ModuleDefinition>,
    pub mid  : Vec<ModuleDefinition>,
    pub low  : Vec<ModuleDefinition>,
    pub rig  : Vec<ModuleDefinition>,
}

impl FittingComp {
    /// スロットが全て空の初期状態。
    pub fn empty() -> Self {
        Self {
            high: Vec::new(),
            mid : Vec::new(),
            low : Vec::new(),
            rig : Vec::new(),
        }
    }

    /// 全スロットの `StatDelta` を合計して返す。
    pub fn total_delta(&self) -> StatDelta {
        self.high.iter()
            .chain(self.mid.iter())
            .chain(self.low.iter())
            .chain(self.rig.iter())
            .fold(StatDelta::ZERO, |acc, m| acc.add(&m.stat_delta))
    }

    /// スロットに対応する `Vec` への可変参照を返す。
    pub fn slot_mut(&mut self, kind: SlotKind) -> &mut Vec<ModuleDefinition> {
        match kind {
            SlotKind::High => &mut self.high,
            SlotKind::Mid  => &mut self.mid,
            SlotKind::Low  => &mut self.low,
            SlotKind::Rig  => &mut self.rig,
        }
    }

    /// 現在の装備状態を `FittingSnapshot`（ID のみ）に変換する。
    /// `ShipFitted` イベントへの埋め込みに使用する。
    pub fn to_snapshot(&self) -> FittingSnapshot {
        FittingSnapshot {
            high: self.high.iter().map(|m| m.id).collect(),
            mid : self.mid.iter().map(|m| m.id).collect(),
            low : self.low.iter().map(|m| m.id).collect(),
            rig : self.rig.iter().map(|m| m.id).collect(),
        }
    }

    /// `FittingSnapshot` から `FittingComp` を復元する（Event Replay 用）。
    ///
    /// `registry` にモジュール定義が存在しない ID はスキップされる。
    pub fn from_snapshot(
        snapshot: &FittingSnapshot,
        registry: &HashMap<ModuleId, ModuleDefinition>,
    ) -> Self {
        let resolve = |ids: &[ModuleId]| -> Vec<ModuleDefinition> {
            ids.iter().filter_map(|id| registry.get(id).cloned()).collect()
        };
        Self {
            high: resolve(&snapshot.high),
            mid : resolve(&snapshot.mid),
            low : resolve(&snapshot.low),
            rig : resolve(&snapshot.rig),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::fitting::{ModuleKind, StatDelta};

    fn weapon_module() -> ModuleDefinition {
        ModuleDefinition {
            id        : ModuleId(1),
            name      : "Small Railgun".to_string(),
            kind      : ModuleKind::Weapon,
            slot      : SlotKind::High,
            stat_delta: StatDelta { weapon_damage_add: 20.0, weapon_range_add: 500.0, ..StatDelta::ZERO },
        }
    }

    fn shield_module() -> ModuleDefinition {
        ModuleDefinition {
            id        : ModuleId(2),
            name      : "Shield Booster".to_string(),
            kind      : ModuleKind::ShieldBooster,
            slot      : SlotKind::Mid,
            stat_delta: StatDelta { max_hp_add: 200.0, ..StatDelta::ZERO },
        }
    }

    #[test]
    fn empty_fitting_produces_zero_delta() {
        assert_eq!(FittingComp::empty().total_delta(), StatDelta::ZERO);
    }

    #[test]
    fn total_delta_sums_all_slots_correctly() {
        let mut fitting = FittingComp::empty();
        fitting.high.push(weapon_module());
        fitting.mid.push(shield_module());
        let delta = fitting.total_delta();
        assert_eq!(delta.weapon_damage_add, 20.0);
        assert_eq!(delta.weapon_range_add,  500.0);
        assert_eq!(delta.max_hp_add,        200.0);
    }

    #[test]
    fn snapshot_round_trip_preserves_module_ids() {
        let mut fitting = FittingComp::empty();
        fitting.high.push(weapon_module());
        fitting.mid.push(shield_module());
        let snap = fitting.to_snapshot();
        assert_eq!(snap.high, vec![ModuleId(1)]);
        assert_eq!(snap.mid,  vec![ModuleId(2)]);
        assert!(snap.low.is_empty());
    }

    #[test]
    fn from_snapshot_restores_fitting_from_registry() {
        let mut registry = HashMap::new();
        registry.insert(ModuleId(1), weapon_module());
        registry.insert(ModuleId(2), shield_module());
        let snap = FittingSnapshot {
            high: vec![ModuleId(1)],
            mid : vec![ModuleId(2)],
            low : vec![],
            rig : vec![],
        };
        let fitting = FittingComp::from_snapshot(&snap, &registry);
        assert_eq!(fitting.high.len(), 1);
        assert_eq!(fitting.mid.len(),  1);
        assert_eq!(fitting.total_delta().weapon_damage_add, 20.0);
    }
}
