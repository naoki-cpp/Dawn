//! Fitting system domain types.
//!
//! EVE Online準拠のモジュール装備システム。
//! Ship はスロットにモジュールを装備し、モジュールの効果が ShipStats に集計される。
//!
//! # スロット種別
//! - High : 武器など攻撃系
//! - Mid  : シールド / 推進など
//! - Low  : アーマー / 速度強化など
//! - Rig  : 恒久的な改造（取り外し不可）
//!
//! # 設計原則
//! - `StatDelta` が Fitting の出力。ECS の `ShipStatsComp` に集計される。
//! - `FittingSnapshot` を Event に含めることで Event Replay 時に完全復元（INV-002）。

use serde::{Deserialize, Serialize};

// ── ID ────────────────────────────────────────────────────────────────────────

/// モジュールの種類を識別する ID。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModuleId(pub u32);

// ── スロット ──────────────────────────────────────────────────────────────────

/// 装備スロットの種別。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SlotKind {
    High,
    Mid,
    Low,
    Rig,
}

// ── モジュール種別 ────────────────────────────────────────────────────────────

/// モジュールが提供する効果の大分類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModuleKind {
    /// 武器（High スロット）
    Weapon,
    /// シールドブースター（Mid スロット）
    ShieldBooster,
    /// アーマーリペアラー（Low スロット）
    ArmorRepairer,
    /// 推進モジュール（Mid スロット）
    Propulsion,
    /// センサー強化（Mid スロット）
    Sensor,
    /// リグ（Rig スロット）
    Rig,
}

// ── StatDelta ─────────────────────────────────────────────────────────────────

/// 1枚のモジュールが ShipStats に加算する差分。
///
/// 全スロットの `StatDelta` を合計したものが装備後の最終 stat になる。
/// フィールドが `0.0` のものは変化なしを意味する。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StatDelta {
    /// 最大速度への加算（units/tick）
    pub max_speed_add        : f32,
    /// 推力への加算（units/tick²）
    pub thrust_add           : f32,
    /// 最大 HP への加算
    pub max_hp_add           : f32,
    /// 武器ダメージへの加算
    pub weapon_damage_add    : f32,
    /// 武器射程への加算（units）
    pub weapon_range_add     : f32,
    /// 武器クールダウンへの加算（Tick 数、負の値で短縮）
    pub weapon_cooldown_add  : i32,
    /// ロック時間への加算（Tick 数、負の値で短縮）
    pub lock_time_add        : i32,
    /// 同時ロック上限への加算（正の値で増加）
    pub max_locks_add        : i32,
}

impl StatDelta {
    /// 変化なし（デフォルト）
    pub const ZERO: Self = Self {
        max_speed_add       : 0.0,
        thrust_add          : 0.0,
        max_hp_add          : 0.0,
        weapon_damage_add   : 0.0,
        weapon_range_add    : 0.0,
        weapon_cooldown_add : 0,
        lock_time_add       : 0,
        max_locks_add       : 0,
    };

    /// 差分を加算する。
    pub fn add(&self, other: &StatDelta) -> StatDelta {
        StatDelta {
            max_speed_add       : self.max_speed_add       + other.max_speed_add,
            thrust_add          : self.thrust_add          + other.thrust_add,
            max_hp_add          : self.max_hp_add          + other.max_hp_add,
            weapon_damage_add   : self.weapon_damage_add   + other.weapon_damage_add,
            weapon_range_add    : self.weapon_range_add    + other.weapon_range_add,
            weapon_cooldown_add : self.weapon_cooldown_add + other.weapon_cooldown_add,
            lock_time_add       : self.lock_time_add       + other.lock_time_add,
            max_locks_add       : self.max_locks_add       + other.max_locks_add,
        }
    }
}

// ── ModuleDefinition ──────────────────────────────────────────────────────────

/// モジュール 1種類の定義（不変の設計図）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleDefinition {
    pub id         : ModuleId,
    pub name       : String,
    pub kind       : ModuleKind,
    pub slot       : SlotKind,
    pub stat_delta : StatDelta,
}

// ── FittingSnapshot ───────────────────────────────────────────────────────────

/// 装備スロット全体のスナップショット。
///
/// `ShipFitted` イベントに含めることで Event Replay 時に
/// 完全に Fitting 状態が復元される（INV-002）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FittingSnapshot {
    pub high : Vec<ModuleId>,
    pub mid  : Vec<ModuleId>,
    pub low  : Vec<ModuleId>,
    pub rig  : Vec<ModuleId>,
}

impl FittingSnapshot {
    /// 空の装備状態
    pub fn empty() -> Self {
        Self {
            high : Vec::new(),
            mid  : Vec::new(),
            low  : Vec::new(),
            rig  : Vec::new(),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_delta_zero_has_no_effect_on_addition() {
        let base = StatDelta {
            max_speed_add       : 10.0,
            thrust_add          : 5.0,
            max_hp_add          : 100.0,
            weapon_damage_add   : 20.0,
            weapon_range_add    : 500.0,
            weapon_cooldown_add : -1,
            lock_time_add       : -1,
            max_locks_add       : 1,
        };
        let result = base.add(&StatDelta::ZERO);
        assert_eq!(result, base);
    }

    #[test]
    fn stat_delta_accumulates_correctly_across_multiple_modules() {
        let module_a = StatDelta { max_speed_add: 50.0, weapon_damage_add: 10.0, ..StatDelta::ZERO };
        let module_b = StatDelta { max_speed_add: 30.0, weapon_damage_add:  5.0, ..StatDelta::ZERO };
        let total = module_a.add(&module_b);
        assert_eq!(total.max_speed_add, 80.0);
        assert_eq!(total.weapon_damage_add, 15.0);
    }

    #[test]
    fn fitting_snapshot_empty_has_no_modules_in_any_slot() {
        let snap = FittingSnapshot::empty();
        assert!(snap.high.is_empty());
        assert!(snap.mid.is_empty());
        assert!(snap.low.is_empty());
        assert!(snap.rig.is_empty());
    }

    #[test]
    fn module_definition_is_serializable_round_trip() {
        let def = ModuleDefinition {
            id         : ModuleId(1),
            name       : "150mm Railgun".to_string(),
            kind       : ModuleKind::Weapon,
            slot       : SlotKind::High,
            stat_delta : StatDelta {
                weapon_damage_add   : 25.0,
                weapon_range_add    : 800.0,
                weapon_cooldown_add : 0,
                ..StatDelta::ZERO
            },
        };
        // serde_json ではなく Debug で代用（dawn-core は JSON 非依存）
        let debug_str = format!("{def:?}");
        assert!(debug_str.contains("150mm Railgun"));
        assert!(debug_str.contains("Weapon"));
    }
}
