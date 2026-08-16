//! Combat ECS components.

use super::movement::ShipStatsComp;
use dawn_core::{ShipId, Tick};

// ── CapacitorComp ─────────────────────────────────────────────────────────────

/// Live capacitor state for a ship.
///
/// The maximum and recharge rate are stored in `ShipStatsComp`; this component
/// tracks only the current charge level so it can be modified each tick without
/// touching the stat block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CapacitorComp {
    /// Current capacitor charge (GJ).  Clamped to `[0, cap_max]` each tick.
    pub current: f32,
}

/// A Ship's current HP state (Shield / Armor / Hull layers).
///
/// Damage order: Shield → Armor → Hull. `is_destroyed` becomes `true` when
/// Hull reaches 0.
///
/// Fields are private (`/improve-codebase-architecture` HullComp deepening,
/// 2026-07-05): every caller that restores or overwrites HP already has the
/// authoritative (shield, armor, hull) triple in hand (event replay, snapshot
/// restore, batch write-back) and used to hand-write all three fields plus
/// `is_destroyed` individually -- five call sites duplicating the same write
/// and, in two of them, deriving `is_destroyed` by hand instead of reusing
/// `apply_damage`'s own `hull <= 0.0` rule. `set_hp` is the one place that
/// invariant lives now.
#[derive(Debug, Clone, Copy)]
pub struct HullComp {
    current_shield: f32,
    current_armor: f32,
    current_hull: f32,
    is_destroyed: bool,
}

impl HullComp {
    /// Initialize at full HP across all three layers.
    pub fn new(max_shield: f32, max_armor: f32, max_hull: f32) -> Self {
        Self {
            current_shield: max_shield,
            current_armor: max_armor,
            current_hull: max_hull,
            is_destroyed: false,
        }
    }

    pub fn shield(&self) -> f32 {
        self.current_shield
    }

    pub fn armor(&self) -> f32 {
        self.current_armor
    }

    pub fn hull(&self) -> f32 {
        self.current_hull
    }

    pub fn is_destroyed(&self) -> bool {
        self.is_destroyed
    }

    /// Sum of all three layers. Used for HP-fraction checks (e.g. Bot AI's
    /// flee threshold); does not weight layers, since callers already have
    /// each max separately when they need a fraction.
    pub fn total_hp(&self) -> f32 {
        self.current_shield + self.current_armor + self.current_hull
    }

    /// Apply damage in Shield → Armor → Hull order.
    /// Returns (current_shield, current_armor, current_hull).
    pub fn apply_damage(&mut self, amount: f32) -> (f32, f32, f32) {
        let mut remaining = amount;

        // 1. Absorb from Shield.
        let shield_absorbed = remaining.min(self.current_shield);
        self.current_shield -= shield_absorbed;
        remaining -= shield_absorbed;

        // 2. Absorb from Armor.
        if remaining > 0.0 {
            let armor_absorbed = remaining.min(self.current_armor);
            self.current_armor -= armor_absorbed;
            remaining -= armor_absorbed;
        }

        // 3. Absorb from Hull.
        if remaining > 0.0 {
            self.current_hull = (self.current_hull - remaining).max(0.0);
        }

        if self.current_hull <= 0.0 {
            self.is_destroyed = true;
        }

        (self.current_shield, self.current_armor, self.current_hull)
    }

    pub fn repair_shield(&mut self, amount: f32, max_shield: f32) -> f32 {
        if self.is_destroyed || amount <= 0.0 {
            return self.current_shield;
        }
        self.current_shield = (self.current_shield + amount).clamp(0.0, max_shield.max(0.0));
        self.current_shield
    }

    pub fn repair_armor(&mut self, amount: f32, max_armor: f32) -> f32 {
        if self.is_destroyed || amount <= 0.0 {
            return self.current_armor;
        }
        self.current_armor = (self.current_armor + amount).clamp(0.0, max_armor.max(0.0));
        self.current_armor
    }

    /// Overwrite all three HP layers with already-known authoritative values
    /// (event replay, snapshot restore, batch write-back). Derives
    /// `is_destroyed = hull <= 0.0` internally -- no caller has ever needed
    /// it to differ (a destroyed Ship's entity is removed outright, so a live
    /// `HullComp` is never snapshotted or replayed mid-destruction).
    pub fn set_hp(&mut self, shield: f32, armor: f32, hull: f32) {
        self.current_shield = shield;
        self.current_armor = armor;
        self.current_hull = hull;
        self.is_destroyed = hull <= 0.0;
    }

    /// Proportionally rescale current HP when max HP changes (refit).
    /// `is_destroyed` is left untouched: a refit cannot destroy or revive a
    /// Ship, only change its maxima.
    pub fn rescale(&mut self, old: &ShipStatsComp, new: &ShipStatsComp) {
        let scale = |cur: f32, old_max: f32, new_max: f32| -> f32 {
            if old_max <= 0.0 {
                return new_max;
            }
            (cur / old_max * new_max).clamp(0.0, new_max)
        };
        self.current_shield = scale(self.current_shield, old.max_shield, new.max_shield);
        self.current_armor = scale(self.current_armor, old.max_armor, new.max_armor);
        self.current_hull = scale(self.current_hull, old.max_hull, new.max_hull);
    }
}

/// 武器クールダウン追跡コンポーネント。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeaponComp {
    /// 最後に発射した Tick。`Tick::ZERO` は未発射。
    pub last_fired_tick: Tick,
}

impl Default for WeaponComp {
    fn default() -> Self {
        Self::new()
    }
}

impl WeaponComp {
    pub fn new() -> Self {
        Self {
            last_fired_tick: Tick::ZERO,
        }
    }

    /// 現在 Tick で発射可能かどうかを判定する。
    pub fn can_fire(&self, current_tick: Tick, cooldown: u64) -> bool {
        current_tick.value() >= self.last_fired_tick.value() + cooldown
    }
}

// ── Lock-on ───────────────────────────────────────────────────────────────────

/// ロックオン状態。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LockState {
    /// ロック中：あと `remaining_ticks` Tick でロック完了。
    Locking { remaining_ticks: u64 },
    /// ロック完了：このターゲットに発射可能。
    Locked,
}

/// 1ターゲットへのロックエントリ。
#[derive(Debug, Clone, PartialEq)]
pub struct LockEntry {
    pub target_id: ShipId,
    pub state: LockState,
}

/// Ship のロックオン状態全体を保持するコンポーネント。
///
/// 最大エントリ数は `ShipStatsComp::max_locks` で決まる。
#[derive(Debug, Clone, PartialEq)]
pub struct LockComp {
    pub entries: Vec<LockEntry>,
}

impl Default for LockComp {
    fn default() -> Self {
        Self::new()
    }
}

impl LockComp {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// `target_id` がすでにロック中またはロック済みか。
    pub fn has_target(&self, target_id: ShipId) -> bool {
        self.entries.iter().any(|e| e.target_id == target_id)
    }

    /// スロットに空きがあるか。
    pub fn has_capacity(&self, max_locks: u32) -> bool {
        self.entries.len() < max_locks as usize
    }

    /// `Locked` 状態のターゲット一覧。
    pub fn locked_targets(&self) -> impl Iterator<Item = ShipId> + '_ {
        self.entries
            .iter()
            .filter(|e| e.state == LockState::Locked)
            .map(|e| e.target_id)
    }

    /// 最初の `Locked` ターゲットを返す（武器発射先として使う）。
    pub fn first_locked(&self) -> Option<ShipId> {
        self.locked_targets().next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hull_comp_initialized_with_full_hp() {
        let hull = HullComp::new(300.0, 200.0, 100.0);
        assert_eq!(hull.current_shield, 300.0);
        assert_eq!(hull.current_armor, 200.0);
        assert_eq!(hull.current_hull, 100.0);
        assert!(!hull.is_destroyed);
    }

    #[test]
    fn damage_depletes_shield_first() {
        let mut hull = HullComp::new(300.0, 200.0, 100.0);
        hull.apply_damage(100.0);
        assert_eq!(hull.current_shield, 200.0);
        assert_eq!(hull.current_armor, 200.0);
        assert_eq!(hull.current_hull, 100.0);
        assert!(!hull.is_destroyed);
    }

    #[test]
    fn damage_overflows_shield_into_armor() {
        let mut hull = HullComp::new(100.0, 200.0, 100.0);
        hull.apply_damage(150.0);
        assert_eq!(hull.current_shield, 0.0);
        assert_eq!(hull.current_armor, 150.0);
        assert_eq!(hull.current_hull, 100.0);
    }

    #[test]
    fn hull_comp_is_destroyed_when_hull_reaches_zero() {
        let mut hull = HullComp::new(0.0, 0.0, 100.0);
        hull.apply_damage(100.0);
        assert_eq!(hull.current_hull, 0.0);
        assert!(hull.is_destroyed);
    }

    #[test]
    fn hull_comp_hp_does_not_go_below_zero() {
        let mut hull = HullComp::new(0.0, 0.0, 50.0);
        let (_, _, h) = hull.apply_damage(200.0);
        assert_eq!(h, 0.0);
        assert!(hull.is_destroyed);
    }

    #[test]
    fn hull_comp_partial_damage_does_not_destroy_ship() {
        let mut hull = HullComp::new(200.0, 100.0, 100.0);
        hull.apply_damage(150.0);
        assert!(!hull.is_destroyed);
    }

    #[test]
    fn shield_repair_clamps_to_max_shield() {
        let mut hull = HullComp::new(100.0, 100.0, 100.0);
        hull.apply_damage(75.0);
        let repaired = hull.repair_shield(50.0, 100.0);
        assert_eq!(repaired, 75.0);
        assert_eq!(hull.current_shield, 75.0);
    }

    #[test]
    fn armor_repair_clamps_to_max_armor() {
        let mut hull = HullComp::new(0.0, 100.0, 100.0);
        hull.apply_damage(40.0);
        let repaired = hull.repair_armor(80.0, 100.0);
        assert_eq!(repaired, 100.0);
        assert_eq!(hull.current_armor, 100.0);
    }

    #[test]
    fn set_hp_overwrites_all_three_layers() {
        let mut hull = HullComp::new(300.0, 200.0, 100.0);
        hull.set_hp(10.0, 20.0, 30.0);
        assert_eq!(hull.shield(), 10.0);
        assert_eq!(hull.armor(), 20.0);
        assert_eq!(hull.hull(), 30.0);
        assert!(!hull.is_destroyed());
    }

    #[test]
    fn set_hp_derives_is_destroyed_from_hull_reaching_zero() {
        let mut hull = HullComp::new(300.0, 200.0, 100.0);
        hull.set_hp(10.0, 20.0, 0.0);
        assert!(hull.is_destroyed());
    }

    #[test]
    fn total_hp_sums_all_three_layers() {
        let hull = HullComp::new(300.0, 200.0, 100.0);
        assert_eq!(hull.total_hp(), 600.0);
    }

    #[test]
    fn rescale_scales_current_hp_proportionally_to_new_maxima() {
        use crate::components::movement::ShipStatsComp;

        let mut hull = HullComp::new(100.0, 100.0, 100.0);
        hull.apply_damage(50.0); // shield 50 / 100
        let old = ShipStatsComp {
            max_shield: 100.0,
            ..ShipStatsComp::PLAYER
        };
        let new = ShipStatsComp {
            max_shield: 200.0,
            ..ShipStatsComp::PLAYER
        };
        hull.rescale(&old, &new);
        assert_eq!(hull.shield(), 100.0); // 50/100 * 200
    }

    #[test]
    fn rescale_leaves_is_destroyed_untouched() {
        use crate::components::movement::ShipStatsComp;

        let mut hull = HullComp::new(0.0, 0.0, 100.0);
        hull.apply_damage(100.0);
        assert!(hull.is_destroyed());
        hull.rescale(&ShipStatsComp::PLAYER, &ShipStatsComp::PLAYER);
        assert!(hull.is_destroyed());
    }

    #[test]
    fn weapon_comp_can_fire_after_cooldown_elapsed() {
        let mut weapon = WeaponComp::new();
        weapon.last_fired_tick = Tick(10);
        assert!(weapon.can_fire(Tick(15), 5)); // 10 + 5 = 15 → OK
        assert!(!weapon.can_fire(Tick(14), 5)); // 10 + 5 = 15 > 14 → NG
    }

    #[test]
    fn weapon_comp_can_fire_on_first_shot_from_tick_zero() {
        let weapon = WeaponComp::new();
        // last_fired_tick = ZERO(0), cooldown = 5 → can fire from tick 5
        assert!(weapon.can_fire(Tick(5), 5));
        assert!(weapon.can_fire(Tick(1), 1));
    }
}
