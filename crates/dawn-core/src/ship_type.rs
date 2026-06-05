//! 船種定義（ShipType）— ADR-0006 Option B。
//!
//! モジュールの `ModuleDefinition` に対応する概念。
//! `ShipTypeDefinition` がテンプレートで、船 spawn 時に `ShipTypeId` を指定する。
//!
//! # 船種とクラスの関係
//! ShipClass は「フリゲートかクルーザーか」という抽象カテゴリ。
//! ShipTypeDefinition は「どの具体的な船種か」（Merlin / Rifter など）。
//! 同じクラスでも ShipTypeDefinition が異なれば base_stats が異なる。

use serde::{Deserialize, Serialize};

// ── ID ────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ShipTypeId(pub u32);

// ── クラス ────────────────────────────────────────────────────────────────────

/// 船のクラス（抽象カテゴリ）。
/// スロット制限やモジュールサイズ制限に使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShipClass {
    /// 小型・高速・少スロット
    Frigate,
    /// 中型・バランス
    Cruiser,
    /// 大型・低速・多スロット
    Battleship,
}

// ── スロットレイアウト ─────────────────────────────────────────────────────────

/// 装備スロットの最大数（船種固有）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SlotLayout {
    pub high : u8,
    pub mid  : u8,
    pub low  : u8,
    pub rig  : u8,
}

impl SlotLayout {
    pub const FRIGATE: Self = Self { high: 3, mid: 3, low: 2, rig: 3 };
    pub const CRUISER: Self = Self { high: 5, mid: 4, low: 3, rig: 3 };
    pub const BATTLESHIP: Self = Self { high: 8, mid: 4, low: 5, rig: 3 };
}

// ── ベーススタット ────────────────────────────────────────────────────────────

/// 装備なし時の船種固有ベーススタット。
///
/// `ShipStatsComp` = `ShipBaseStats` + Σ(有効モジュールの `StatDelta`)。
///
/// # 設計メモ
/// weapon_* フィールドはここに含まない。
/// 武器能力はモジュール装備によってのみ付与される（ベースはゼロ）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ShipBaseStats {
    /// 最大速度（units/tick）
    pub max_speed        : f32,
    /// 推力加速度（units/tick²）。0 = NPC（等速）
    pub thrust_magnitude : f32,
    /// シールド最大 HP
    pub max_shield       : f32,
    /// アーマー最大 HP
    pub max_armor        : f32,
    /// ハル（構造）最大 HP
    pub max_hull         : f32,
    /// ロック完了までの Tick 数
    pub lock_time        : u64,
    /// 同時ロック上限
    pub max_locks        : u32,
}

// ── 船種定義 ──────────────────────────────────────────────────────────────────

/// 船種 1 種類の定義（不変のテンプレート）。
///
/// モジュールの `ModuleDefinition` に対応する。
/// サーバー起動時にレジストリに登録し、`spawn_ship(ship_type_id, ...)` で参照する。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShipTypeDefinition {
    pub id          : ShipTypeId,
    pub name        : String,
    pub class       : ShipClass,
    pub base_stats  : ShipBaseStats,
    pub slot_layout : SlotLayout,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frigate_def() -> ShipTypeDefinition {
        ShipTypeDefinition {
            id         : ShipTypeId(1),
            name       : "Starter Frigate".to_string(),
            class      : ShipClass::Frigate,
            slot_layout: SlotLayout::FRIGATE,
            base_stats : ShipBaseStats {
                max_speed        : 400.0,
                thrust_magnitude : 0.0,
                max_shield       : 200.0,
                max_armor        : 150.0,
                max_hull         : 150.0,
                lock_time        : 5,
                max_locks        : 1,
            },
        }
    }

    #[test]
    fn ship_type_definition_has_correct_total_hp() {
        let def = frigate_def();
        let total = def.base_stats.max_shield
            + def.base_stats.max_armor
            + def.base_stats.max_hull;
        assert_eq!(total, 500.0);
    }

    #[test]
    fn frigate_slot_layout_has_three_high_slots() {
        assert_eq!(SlotLayout::FRIGATE.high, 3);
    }
}
