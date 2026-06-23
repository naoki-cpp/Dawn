//! Combat ECS components.

use dawn_core::{ShipId, Tick};

// ── CapacitorComp ─────────────────────────────────────────────────────────────

/// Live capacitor state for a ship.
///
/// The maximum and recharge rate are stored in `ShipStatsComp`; this component
/// tracks only the current charge level so it can be modified each tick without
/// touching the stat block.
#[derive(Debug, Clone, Copy)]
pub struct CapacitorComp {
    /// Current capacitor charge (GJ).  Clamped to `[0, cap_max]` each tick.
    pub current: f32,
}

/// Ship の現在 HP 状態（Shield / Armor / Hull の 3 層）。
///
/// ダメージ適用順序: Shield → Armor → Hull
/// Hull が 0 になると `is_destroyed = true`。
#[derive(Debug, Clone, Copy)]
pub struct HullComp {
    pub current_shield: f32,
    pub current_armor: f32,
    pub current_hull: f32,
    pub is_destroyed: bool,
}

impl HullComp {
    /// 初期 HP を 3 層で指定して初期化する。
    pub fn new(max_shield: f32, max_armor: f32, max_hull: f32) -> Self {
        Self {
            current_shield: max_shield,
            current_armor: max_armor,
            current_hull: max_hull,
            is_destroyed: false,
        }
    }

    /// ダメージを Shield → Armor → Hull の順に適用する。
    /// 返り値: (current_shield, current_armor, current_hull)
    pub fn apply_damage(&mut self, amount: f32) -> (f32, f32, f32) {
        let mut remaining = amount;

        // 1. Shield から引く
        let shield_absorbed = remaining.min(self.current_shield);
        self.current_shield -= shield_absorbed;
        remaining -= shield_absorbed;

        // 2. Armor から引く
        if remaining > 0.0 {
            let armor_absorbed = remaining.min(self.current_armor);
            self.current_armor -= armor_absorbed;
            remaining -= armor_absorbed;
        }

        // 3. Hull から引く
        if remaining > 0.0 {
            self.current_hull = (self.current_hull - remaining).max(0.0);
        }

        if self.current_hull <= 0.0 {
            self.is_destroyed = true;
        }

        (self.current_shield, self.current_armor, self.current_hull)
    }
}

/// 武器クールダウン追跡コンポーネント。
#[derive(Debug, Clone, Copy)]
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
#[derive(Debug, Clone)]
pub struct LockEntry {
    pub target_id: ShipId,
    pub state: LockState,
}

/// Ship のロックオン状態全体を保持するコンポーネント。
///
/// 最大エントリ数は `ShipStatsComp::max_locks` で決まる。
#[derive(Debug, Clone)]
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
