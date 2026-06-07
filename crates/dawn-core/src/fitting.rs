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

// ── 活性化モード ──────────────────────────────────────────────────────────────

/// モジュールの活性化モード。
///
/// Passive: 装備するだけで StatDelta が常時適用される（Shield Extender など）。
/// Active : プレイヤーがオン/オフを切り替える（Weapon, Afterburner など）。
///          オフ時は StatDelta が適用されない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivationMode {
    /// 常時効果。装備するだけで有効。
    Passive,
    /// オン/オフ切り替え可能。オフ時は効果なし。
    Active,
}

// ── StatDelta ─────────────────────────────────────────────────────────────────

/// Per-module stat additions applied to the base ship stats after fitting.
///
/// All fields default to zero (no change).  Positive values increase the stat;
/// negative values decrease it (where the field is signed).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StatDelta {
    /// Bonus to max speed (units/tick).
    pub max_speed_add        : f32,
    /// Bonus to thrust acceleration (units/tick²).
    pub thrust_add           : f32,
    /// Bonus to max Shield HP.
    pub max_shield_add       : f32,
    /// Bonus to max Armor HP.
    pub max_armor_add        : f32,
    /// Bonus to max Hull HP.
    pub max_hull_add         : f32,
    /// Bonus to weapon damage per shot.
    pub weapon_damage_add    : f32,
    /// Bonus to weapon range (units).
    pub weapon_range_add     : f32,
    /// Weapon cooldown adjustment (ticks; negative = shorter cooldown).
    pub weapon_cooldown_add  : i32,
    /// Lock-on time adjustment (ticks; negative = faster lock).
    pub lock_time_add        : i32,
    /// Simultaneous lock limit adjustment.
    pub max_locks_add        : i32,
    /// Bonus to capacitor capacity (GJ).
    pub cap_max_add          : f32,
    /// Bonus to capacitor recharge rate (GJ/tick).
    pub cap_recharge_add     : f32,
}

impl StatDelta {
    /// No change (all fields zero).
    pub const ZERO: Self = Self {
        max_speed_add       : 0.0,
        thrust_add          : 0.0,
        max_shield_add      : 0.0,
        max_armor_add       : 0.0,
        max_hull_add        : 0.0,
        weapon_damage_add   : 0.0,
        weapon_range_add    : 0.0,
        weapon_cooldown_add : 0,
        lock_time_add       : 0,
        max_locks_add       : 0,
        cap_max_add         : 0.0,
        cap_recharge_add    : 0.0,
    };

    pub fn add(&self, other: &StatDelta) -> StatDelta {
        StatDelta {
            max_speed_add       : self.max_speed_add       + other.max_speed_add,
            thrust_add          : self.thrust_add          + other.thrust_add,
            max_shield_add      : self.max_shield_add      + other.max_shield_add,
            max_armor_add       : self.max_armor_add       + other.max_armor_add,
            max_hull_add        : self.max_hull_add        + other.max_hull_add,
            weapon_damage_add   : self.weapon_damage_add   + other.weapon_damage_add,
            weapon_range_add    : self.weapon_range_add    + other.weapon_range_add,
            weapon_cooldown_add : self.weapon_cooldown_add + other.weapon_cooldown_add,
            lock_time_add       : self.lock_time_add       + other.lock_time_add,
            max_locks_add       : self.max_locks_add       + other.max_locks_add,
            cap_max_add         : self.cap_max_add         + other.cap_max_add,
            cap_recharge_add    : self.cap_recharge_add    + other.cap_recharge_add,
        }
    }
}

// ── ModuleDefinition ──────────────────────────────────────────────────────────

/// Immutable definition ("blueprint") for one module type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleDefinition {
    pub id              : ModuleId,
    pub name            : String,
    pub kind            : ModuleKind,
    pub slot            : SlotKind,
    pub stat_delta      : StatDelta,
    /// Passive = always effective / Active = toggled by player.
    pub activation_mode : ActivationMode,
    /// Capacitor consumed once at the start of each activation cycle (GJ).
    /// Zero for Passive modules.
    pub cap_cost_per_cycle: f32,
    /// Duration of one activation cycle (ticks).
    /// Zero for Passive modules (cycle concept does not apply).
    pub cycle_time_ticks  : u64,
}

// ── FittingSnapshot ───────────────────────────────────────────────────────────

/// スナップショット内の 1 スロット分のエントリ。
/// `is_active` を含めることで Replay 時の活性化状態も復元できる（INV-002）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlotEntry {
    pub module_id : ModuleId,
    pub is_active : bool,
}

/// 装備スロット全体のスナップショット。
///
/// `ShipFitted` イベントに含めることで Event Replay 時に
/// 完全に Fitting 状態が復元される（INV-002）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FittingSnapshot {
    pub high : Vec<SlotEntry>,
    pub mid  : Vec<SlotEntry>,
    pub low  : Vec<SlotEntry>,
    pub rig  : Vec<SlotEntry>,
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
            max_shield_add      : 50.0,
            max_armor_add       : 30.0,
            max_hull_add        : 20.0,
            weapon_damage_add   : 20.0,
            weapon_range_add    : 500.0,
            weapon_cooldown_add : -1,
            lock_time_add       : -1,
            max_locks_add       : 1,
            cap_max_add         : 0.0,
            cap_recharge_add    : 0.0,
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
            id                : ModuleId(1),
            name              : "150mm Railgun".to_string(),
            kind              : ModuleKind::Weapon,
            slot              : SlotKind::High,
            activation_mode   : ActivationMode::Active,
            cap_cost_per_cycle: 60.0,
            cycle_time_ticks  : 10,
            stat_delta        : StatDelta {
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
