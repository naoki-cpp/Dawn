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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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
    pub high: u8,
    pub mid: u8,
    pub low: u8,
    pub rig: u8,
}

impl SlotLayout {
    pub const FRIGATE: Self = Self {
        high: 3,
        mid: 3,
        low: 2,
        rig: 3,
    };
    pub const CRUISER: Self = Self {
        high: 5,
        mid: 4,
        low: 3,
        rig: 3,
    };
    pub const BATTLESHIP: Self = Self {
        high: 8,
        mid: 4,
        low: 5,
        rig: 3,
    };

    /// Max slot count for `kind` (ADR-0032): the player-facing Fit path
    /// enforces this so a ship cannot accumulate more modules in a slot kind
    /// than its hull supports.
    pub fn capacity_for(&self, kind: crate::fitting::SlotKind) -> u8 {
        use crate::fitting::SlotKind;
        match kind {
            SlotKind::High => self.high,
            SlotKind::Mid => self.mid,
            SlotKind::Low => self.low,
            SlotKind::Rig => self.rig,
        }
    }
}

// ── ベーススタット ────────────────────────────────────────────────────────────

/// Base stats for a ship type before any module fitting is applied.
///
/// `ShipStatsComp` = `ShipBaseStats` + Σ(active module `StatDelta`).
///
/// Weapon stats are intentionally absent — weapon capability is granted only
/// through High-slot weapon modules (base value is zero).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ShipBaseStats {
    /// Base max speed without any modules (units/tick). AB/MWD multiply this
    /// via StatDelta.speed_multiplier to produce the effective max speed.
    pub max_speed: f64,
    /// Hull mass (kg). Together with inertia_modifier determines τ (time
    /// constant for velocity changes) and therefore align time (ADR-0023).
    pub mass: f64,
    /// Inertia modifier (dimensionless). Lower = more agile. Controls only
    /// align time; has no direct effect on max speed.
    pub inertia_modifier: f64,
    /// Max Shield HP.
    pub max_shield: f32,
    /// Max Armor HP.
    pub max_armor: f32,
    /// Max Hull HP.
    pub max_hull: f32,
    /// Ticks required to complete a lock-on.
    pub lock_time: u64,
    /// Maximum simultaneous lock targets.
    pub max_locks: u32,
    /// Capacitor pool size (GJ).
    pub cap_max: f32,
    /// Capacitor recovered per tick (GJ/tick).
    pub cap_recharge_per_tick: f32,
    /// Signature radius (arbitrary units). Larger values are easier to track.
    /// Frigate ≈ 40, Cruiser ≈ 125, Battleship ≈ 360.
    pub sig_radius: f32,
}

// ── 船種定義 ──────────────────────────────────────────────────────────────────

/// 船種 1 種類の定義（不変のテンプレート）。
///
/// モジュールの `ModuleDefinition` に対応する。
/// サーバー起動時にレジストリに登録し、`spawn_ship(ship_type_id, ...)` で参照する。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShipTypeDefinition {
    pub id: ShipTypeId,
    pub name: String,
    pub class: ShipClass,
    pub base_stats: ShipBaseStats,
    pub slot_layout: SlotLayout,
    /// Whether `BuildPackagedShipCommand` may target this type (ADR-0034 9B).
    /// NPC-only hulls (e.g. the starter Frigate) are never player-buildable.
    pub buildable: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frigate_def() -> ShipTypeDefinition {
        ShipTypeDefinition {
            id: ShipTypeId(1),
            name: "Starter Frigate".to_string(),
            class: ShipClass::Frigate,
            slot_layout: SlotLayout::FRIGATE,
            buildable: false,
            base_stats: ShipBaseStats {
                max_speed: 400.0,
                mass: 1_500_000.0,
                inertia_modifier: 0.4,
                max_shield: 200.0,
                max_armor: 150.0,
                max_hull: 150.0,
                lock_time: 5,
                max_locks: 1,
                cap_max: 300.0,
                cap_recharge_per_tick: 6.0,
                sig_radius: 40.0,
            },
        }
    }

    #[test]
    fn ship_type_definition_has_correct_total_hp() {
        let def = frigate_def();
        let total = def.base_stats.max_shield + def.base_stats.max_armor + def.base_stats.max_hull;
        assert_eq!(total, 500.0);
    }

    #[test]
    fn frigate_slot_layout_has_three_high_slots() {
        assert_eq!(SlotLayout::FRIGATE.high, 3);
    }

    #[test]
    fn capacity_for_returns_the_matching_slot_kind_field() {
        use crate::fitting::SlotKind;
        let layout = SlotLayout::FRIGATE;
        assert_eq!(layout.capacity_for(SlotKind::High), layout.high);
        assert_eq!(layout.capacity_for(SlotKind::Mid), layout.mid);
        assert_eq!(layout.capacity_for(SlotKind::Low), layout.low);
        assert_eq!(layout.capacity_for(SlotKind::Rig), layout.rig);
    }
}
