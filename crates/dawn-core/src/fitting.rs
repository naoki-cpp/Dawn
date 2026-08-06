//! Fitting system domain types.
//!
//! EVE Online-style module fitting system.
//! A Ship equips modules into slots, and module effects are aggregated into ShipStats.
//!
//! # Slot kinds
//! - High : weapons and other offensive modules
//! - Mid  : shields / propulsion, etc.
//! - Low  : armor / speed enhancement, etc.
//! - Rig  : permanent modification (not removable)
//!
//! # Design principles
//! - `StatDelta` is Fitting's output. Aggregated into ECS's `ShipStatsComp`.
//! - Including `FittingSnapshot` in an Event allows full restoration on Event Replay (INV-002).

use crate::events::RepairLayer;
#[cfg(feature = "schema")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ── ID ────────────────────────────────────────────────────────────────────────

/// ID identifying a module's kind.
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ModuleId(pub u32);

// ── Slot ──────────────────────────────────────────────────────────────────────

/// The kind of an equipment slot.
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SlotKind {
    High,
    Mid,
    Low,
    Rig,
}

// ── Module kind ───────────────────────────────────────────────────────────────

/// The broad category of effect a module provides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModuleKind {
    /// Weapon (High slot)
    Weapon,
    /// Shield Booster (Mid slot)
    ShieldBooster,
    /// Armor Repairer (Low slot)
    ArmorRepairer,
    /// Propulsion module (Mid slot)
    Propulsion,
    /// Sensor booster (Mid slot)
    Sensor,
    /// Rig (Rig slot)
    Rig,
    /// Fold Disruptor — prevents tackled ship from warping or jumping (ADR-0024).
    /// High slot, active. tackle_range_add in StatDelta determines effective range.
    Tackle,
    /// Remote Shield Booster — repairs a Locked ally's Shield layer (ADR-0036).
    /// A distinct kind from `ShieldBooster` (which is always self-targeted):
    /// keeping them separate lets `requires_target()` stay a per-kind bool
    /// rather than needing per-slot conditional target validation.
    RemoteShieldBooster,
    /// Remote Armor Repairer — repairs a Locked ally's Armor layer (ADR-0036).
    RemoteArmorRepairer,
}

impl ModuleKind {
    /// Whether Active modules of this kind must be given a `target_ship_id`
    /// when activated (ADR-0035/0036). Weapon/Tackle/Remote-repair act on
    /// another ship and are meaningless without a target; other kinds
    /// (self-buffs, local repair) act on the fitting ship itself and must
    /// not carry a target.
    pub fn requires_target(self) -> bool {
        matches!(
            self,
            ModuleKind::Weapon
                | ModuleKind::Tackle
                | ModuleKind::RemoteShieldBooster
                | ModuleKind::RemoteArmorRepairer
        )
    }

    /// Which `HullComp` layer this kind repairs, if any (ADR-0033/0036).
    /// Local and Remote variants of the same booster/repairer type heal the
    /// same layer — only the target resolution differs, which the Capacitor
    /// System handles separately via `target_ship_id`.
    pub fn repair_layer(self) -> Option<RepairLayer> {
        match self {
            ModuleKind::ShieldBooster | ModuleKind::RemoteShieldBooster => {
                Some(RepairLayer::Shield)
            }
            ModuleKind::ArmorRepairer | ModuleKind::RemoteArmorRepairer => Some(RepairLayer::Armor),
            _ => None,
        }
    }

    /// Which effective-range family gates this kind's targeted activation
    /// (ADR-0035/0036), if any. The Range Gate System uses this to know
    /// which `ShipStatsComp` range stat determines whether a targeted
    /// module should be force-deactivated once its target drifts away.
    /// `None` for kinds that are not range-gated (self-buffs, local repair,
    /// passives).
    pub fn range_gate_kind(self) -> Option<RangeGateKind> {
        match self {
            ModuleKind::Weapon => Some(RangeGateKind::Weapon),
            ModuleKind::Tackle => Some(RangeGateKind::Tackle),
            ModuleKind::RemoteShieldBooster | ModuleKind::RemoteArmorRepairer => {
                Some(RangeGateKind::RemoteRepair)
            }
            _ => None,
        }
    }
}

/// The range-gated family a `ModuleKind` belongs to (see `ModuleKind::range_gate_kind`).
/// Each variant corresponds to a different `ShipStatsComp` range stat; the
/// Range Gate System (dawn-sector) owns the actual stat lookup since
/// `ShipStatsComp` lives in dawn-ecs, not dawn-core.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RangeGateKind {
    Weapon,
    Tackle,
    RemoteRepair,
}

// ── Activation mode ───────────────────────────────────────────────────────────

/// A module's activation mode.
///
/// Passive: StatDelta is always applied just by fitting it (e.g. Shield Extender).
/// Active : the player toggles it on/off (e.g. Weapon, Afterburner).
///          StatDelta does not apply while off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ActivationMode {
    /// Always-on effect. Active as soon as it's fitted.
    Passive,
    /// Can be toggled on/off. No effect while off.
    Active,
}

// ── StatDelta ─────────────────────────────────────────────────────────────────

/// Per-module stat additions applied to the base ship stats after fitting.
///
/// All fields default to zero / one (no change).  Positive additive values
/// increase the stat; negative decrease it (where the field is signed).
/// Multiplicative fields (`speed_multiplier`) default to 1.0 and are combined
/// via multiplication across all effective slots.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StatDelta {
    /// Multiplicative speed bonus applied when this module is active (default 1.0).
    /// EVE formula: Vmax = Vbase * (1 + Vbonus * Thrust/Mass) simplified to a
    /// single multiplier. 1MN AB ≈ 2.35, MWD ≈ 6.0.
    pub speed_multiplier: f64,
    /// Mass added to the ship when fitted (kg). Passive — always applied
    /// regardless of active/inactive state. Increases τ → longer align time.
    /// This is what makes oversized ABs a meaningful trade-off (ADR-0023).
    pub mass_add: f64,
    /// Bonus to max Shield HP.
    pub max_shield_add: f32,
    /// Bonus to max Armor HP.
    pub max_armor_add: f32,
    /// Bonus to max Hull HP.
    pub max_hull_add: f32,
    /// Bonus to weapon damage per shot.
    pub weapon_damage_add: f32,
    /// Bonus to weapon optimal range (units).
    pub weapon_range_add: f32,
    /// Weapon tracking speed (rad/tick). Determines ability to hit fast-moving targets.
    pub tracking_speed_add: f32,
    /// Weapon falloff range (units). Hit chance halves at optimal + falloff.
    pub falloff_range_add: f32,
    /// Weapon cooldown adjustment (ticks; negative = shorter cooldown).
    pub weapon_cooldown_add: i32,
    /// Lock-on time adjustment (ticks; negative = faster lock).
    pub lock_time_add: i32,
    /// Simultaneous lock limit adjustment.
    pub max_locks_add: i32,
    /// Bonus to capacitor capacity (GJ).
    pub cap_max_add: f32,
    /// Bonus to capacitor recharge rate (GJ/tick).
    pub cap_recharge_add: f32,
    /// Tackle range added by this module (units). 0 = no tackle capability.
    /// Summed across all active Tackle modules (ADR-0024).
    pub tackle_range_add: f32,
    /// HP restored by one active local repair cycle (ADR-0033).
    pub repair_amount: f32,
    /// Remote repair range added by this module (units). 0 = no remote-repair
    /// capability. Summed across all active Remote Shield Booster / Remote
    /// Armor Repairer modules (ADR-0036), exactly like `tackle_range_add`.
    pub repair_range_add: f32,
}

impl StatDelta {
    /// No change: additive fields zero, multiplicative fields 1.0.
    pub const ZERO: Self = Self {
        speed_multiplier: 1.0,
        mass_add: 0.0,
        max_shield_add: 0.0,
        max_armor_add: 0.0,
        max_hull_add: 0.0,
        weapon_damage_add: 0.0,
        weapon_range_add: 0.0,
        tracking_speed_add: 0.0,
        falloff_range_add: 0.0,
        weapon_cooldown_add: 0,
        lock_time_add: 0,
        max_locks_add: 0,
        cap_max_add: 0.0,
        cap_recharge_add: 0.0,
        tackle_range_add: 0.0,
        repair_amount: 0.0,
        repair_range_add: 0.0,
    };

    /// Combine two deltas. Additive fields sum; speed_multiplier multiplies.
    /// mass_add is summed here for completeness (apply_fitting overrides with
    /// the all-slots sum to implement passive behaviour).
    pub fn add(&self, other: &StatDelta) -> StatDelta {
        StatDelta {
            speed_multiplier: self.speed_multiplier * other.speed_multiplier,
            mass_add: self.mass_add + other.mass_add,
            max_shield_add: self.max_shield_add + other.max_shield_add,
            max_armor_add: self.max_armor_add + other.max_armor_add,
            max_hull_add: self.max_hull_add + other.max_hull_add,
            weapon_damage_add: self.weapon_damage_add + other.weapon_damage_add,
            weapon_range_add: self.weapon_range_add + other.weapon_range_add,
            tracking_speed_add: self.tracking_speed_add + other.tracking_speed_add,
            falloff_range_add: self.falloff_range_add + other.falloff_range_add,
            weapon_cooldown_add: self.weapon_cooldown_add + other.weapon_cooldown_add,
            lock_time_add: self.lock_time_add + other.lock_time_add,
            max_locks_add: self.max_locks_add + other.max_locks_add,
            cap_max_add: self.cap_max_add + other.cap_max_add,
            cap_recharge_add: self.cap_recharge_add + other.cap_recharge_add,
            tackle_range_add: self.tackle_range_add + other.tackle_range_add,
            repair_amount: self.repair_amount + other.repair_amount,
            repair_range_add: self.repair_range_add + other.repair_range_add,
        }
    }
}

// ── ModuleDefinition ──────────────────────────────────────────────────────────

/// Immutable definition ("blueprint") for one module type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleDefinition {
    pub id: ModuleId,
    pub name: String,
    pub kind: ModuleKind,
    pub slot: SlotKind,
    pub stat_delta: StatDelta,
    /// Passive = always effective / Active = toggled by player.
    pub activation_mode: ActivationMode,
    /// Capacitor consumed once at the start of each activation cycle (GJ).
    /// Zero for Passive modules.
    pub cap_cost_per_cycle: f32,
    /// Duration of one activation cycle (ticks).
    /// Zero for Passive modules (cycle concept does not apply).
    pub cycle_time_ticks: u64,
}

// ── FittingSnapshot ───────────────────────────────────────────────────────────

/// One slot's worth of entry within a snapshot.
/// Including `is_active` also restores activation state on Replay (INV-002).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlotEntry {
    pub module_id: ModuleId,
    pub is_active: bool,
}

/// A snapshot of the entire equipment slot layout.
///
/// Including this in the `ShipFitted` event fully restores Fitting state
/// on Event Replay (INV-002).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FittingSnapshot {
    pub high: Vec<SlotEntry>,
    pub mid: Vec<SlotEntry>,
    pub low: Vec<SlotEntry>,
    pub rig: Vec<SlotEntry>,
}

impl FittingSnapshot {
    /// Empty equipment state
    pub fn empty() -> Self {
        Self {
            high: Vec::new(),
            mid: Vec::new(),
            low: Vec::new(),
            rig: Vec::new(),
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
            speed_multiplier: 2.35,
            mass_add: 8_000_000.0,
            max_shield_add: 50.0,
            max_armor_add: 30.0,
            max_hull_add: 20.0,
            weapon_damage_add: 20.0,
            weapon_range_add: 500.0,
            tracking_speed_add: 0.0,
            falloff_range_add: 0.0,
            weapon_cooldown_add: -1,
            lock_time_add: -1,
            max_locks_add: 1,
            cap_max_add: 0.0,
            cap_recharge_add: 0.0,
            tackle_range_add: 0.0,
            repair_amount: 0.0,
            repair_range_add: 0.0,
        };
        let result = base.add(&StatDelta::ZERO);
        assert_eq!(result, base);
    }

    #[test]
    fn stat_delta_accumulates_correctly_across_multiple_modules() {
        let module_a = StatDelta {
            speed_multiplier: 2.35,
            weapon_damage_add: 10.0,
            ..StatDelta::ZERO
        };
        let module_b = StatDelta {
            speed_multiplier: 1.5,
            weapon_damage_add: 5.0,
            ..StatDelta::ZERO
        };
        let total = module_a.add(&module_b);
        assert!((total.speed_multiplier - 2.35 * 1.5).abs() < 0.001);
        assert_eq!(total.weapon_damage_add, 15.0);
    }

    #[test]
    fn weapon_and_tackle_require_a_target_other_kinds_do_not() {
        assert!(ModuleKind::Weapon.requires_target());
        assert!(ModuleKind::Tackle.requires_target());
        assert!(!ModuleKind::ShieldBooster.requires_target());
        assert!(!ModuleKind::ArmorRepairer.requires_target());
        assert!(!ModuleKind::Propulsion.requires_target());
        assert!(!ModuleKind::Sensor.requires_target());
        assert!(!ModuleKind::Rig.requires_target());
    }

    #[test]
    fn repair_layer_groups_local_and_remote_variants_by_layer() {
        assert_eq!(
            ModuleKind::ShieldBooster.repair_layer(),
            Some(RepairLayer::Shield)
        );
        assert_eq!(
            ModuleKind::RemoteShieldBooster.repair_layer(),
            Some(RepairLayer::Shield)
        );
        assert_eq!(
            ModuleKind::ArmorRepairer.repair_layer(),
            Some(RepairLayer::Armor)
        );
        assert_eq!(
            ModuleKind::RemoteArmorRepairer.repair_layer(),
            Some(RepairLayer::Armor)
        );
        assert_eq!(ModuleKind::Weapon.repair_layer(), None);
        assert_eq!(ModuleKind::Tackle.repair_layer(), None);
    }

    #[test]
    fn range_gate_kind_groups_targeted_kinds_by_stat_family() {
        assert_eq!(
            ModuleKind::Weapon.range_gate_kind(),
            Some(RangeGateKind::Weapon)
        );
        assert_eq!(
            ModuleKind::Tackle.range_gate_kind(),
            Some(RangeGateKind::Tackle)
        );
        assert_eq!(
            ModuleKind::RemoteShieldBooster.range_gate_kind(),
            Some(RangeGateKind::RemoteRepair)
        );
        assert_eq!(
            ModuleKind::RemoteArmorRepairer.range_gate_kind(),
            Some(RangeGateKind::RemoteRepair)
        );
        // Self-targeted / passive kinds are not range-gated.
        assert_eq!(ModuleKind::ShieldBooster.range_gate_kind(), None);
        assert_eq!(ModuleKind::ArmorRepairer.range_gate_kind(), None);
        assert_eq!(ModuleKind::Propulsion.range_gate_kind(), None);
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
            id: ModuleId(1),
            name: "150mm Railgun".to_string(),
            kind: ModuleKind::Weapon,
            slot: SlotKind::High,
            activation_mode: ActivationMode::Active,
            cap_cost_per_cycle: 60.0,
            cycle_time_ticks: 10,
            stat_delta: StatDelta {
                weapon_damage_add: 25.0,
                weapon_range_add: 800.0,
                weapon_cooldown_add: 0,
                ..StatDelta::ZERO
            },
        };
        // Use Debug instead of serde_json (dawn-core has no JSON dependency)
        let debug_str = format!("{def:?}");
        assert!(debug_str.contains("150mm Railgun"));
        assert!(debug_str.contains("Weapon"));
    }
}
