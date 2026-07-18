//! Fitting system — aggregates FittingComp StatDeltas into ShipStatsComp.
//!
//! # Propulsion model (ADR-0023)
//!
//! Two propulsion fields are handled specially and NOT through the simple
//! additive delta path:
//!
//! - `mass`:      base.mass + Σ(ALL slots' mass_add)   — passive, always applied
//! - `max_speed`: base.max_speed × Π(effective slots' speed_multiplier)
//!
//! All other fields use the standard additive delta from effective slots only.

use crate::components::{FittingComp, HullComp, ShipStatsComp};
use crate::world::SimWorld;
use dawn_core::{fitting::StatDelta, ShipId};

/// Converts (mass_kg × inertia_modifier) to τ in ticks.
/// Tuned so that a Magpie (mass=12M kg, inertia=0.3) yields τ ≈ 36 ticks
/// → align time ≈ 50 ticks ≈ 5 seconds at 10 ticks/s (ADR-0023 §5).
/// Compatibility re-export for callers that historically imported the tuning
/// constant from the fitting system. The movement policy owns the value now.
pub const MASS_SCALE: f32 = dawn_core::MASS_SCALE;

/// Recompute `ShipStatsComp` for `ship_id` from its current `FittingComp`.
///
/// Must be called after any fitting change (module fit / activate / deactivate).
/// Returns the new stats, or `None` if the ship does not exist.
pub fn apply_fitting(
    world: &mut SimWorld,
    ship_id: ShipId,
    base: ShipStatsComp,
) -> Option<ShipStatsComp> {
    let entity = world.find_entity(ship_id)?;

    // ── Propulsion stats (ADR-0023 special cases) ─────────────────────────────

    let (total_mass_add, speed_mult) = world
        .get::<FittingComp>(entity)
        .map(|f| {
            // mass_add: passive — sum from ALL slots regardless of is_effective().
            let mass_add: f32 = f.iter_slots().map(|s| s.def.stat_delta.mass_add).sum();

            // speed_multiplier: product of EFFECTIVE slots only (AB must be ON).
            let speed_mult: f32 = f
                .iter_slots()
                .filter(|s| s.is_effective())
                .map(|s| s.def.stat_delta.speed_multiplier)
                .product();

            (mass_add, speed_mult)
        })
        .unwrap_or((0.0, 1.0));

    // ── Additive delta from effective slots (standard fields) ─────────────────

    let delta: StatDelta = world
        .get::<FittingComp>(entity)
        .map(|f| f.total_delta())
        .unwrap_or(StatDelta::ZERO);

    // ── Build new stats ───────────────────────────────────────────────────────

    let old_stats = world
        .get::<ShipStatsComp>(entity)
        .map(|s| *s)
        .unwrap_or(base);
    let mut new_stats = apply_delta(base, &delta);

    // Override propulsion fields with correctly computed values.
    new_stats.max_speed = (base.max_speed * speed_mult).max(0.0);
    new_stats.mass = base.mass + total_mass_add;
    // inertia_modifier comes from the hull; no module changes it yet.
    new_stats.inertia_modifier = base.inertia_modifier;

    world.set_ship_stats(entity, new_stats);

    // Scale current HP proportionally when max HP changes.
    if let Some(mut hull) = world.get_mut::<HullComp>(entity) {
        hull.rescale(&old_stats, &new_stats);
    }

    Some(new_stats)
}

/// Apply additive `delta` on top of `base`. Propulsion fields (max_speed, mass,
/// inertia_modifier) are handled by `apply_fitting()` and left at base values here.
pub fn apply_delta(base: ShipStatsComp, delta: &StatDelta) -> ShipStatsComp {
    ShipStatsComp {
        // Propulsion — caller overrides these; keep base values as placeholder.
        max_speed: base.max_speed,
        mass: base.mass,
        inertia_modifier: base.inertia_modifier,

        max_shield: (base.max_shield + delta.max_shield_add).max(0.0),
        max_armor: (base.max_armor + delta.max_armor_add).max(0.0),
        max_hull: (base.max_hull + delta.max_hull_add).max(1.0),
        weapon_damage: (base.weapon_damage + delta.weapon_damage_add).max(0.0),
        weapon_range: (base.weapon_range + delta.weapon_range_add).max(0.0),
        weapon_tracking: (base.weapon_tracking + delta.tracking_speed_add).max(0.0),
        weapon_falloff: (base.weapon_falloff + delta.falloff_range_add).max(0.0),
        sig_radius: base.sig_radius,
        weapon_cooldown: {
            let raw = base.weapon_cooldown as i64 + delta.weapon_cooldown_add as i64;
            raw.max(1) as u64
        },
        lock_time: {
            let raw = base.lock_time as i64 + delta.lock_time_add as i64;
            raw.max(1) as u64
        },
        max_locks: {
            let raw = base.max_locks as i64 + delta.max_locks_add as i64;
            raw.max(0) as u32
        },
        cap_max: (base.cap_max + delta.cap_max_add).max(0.0),
        cap_recharge_per_tick: (base.cap_recharge_per_tick + delta.cap_recharge_add).max(0.0),
        tackle_range: (base.tackle_range + delta.tackle_range_add).max(0.0),
        repair_amount: (base.repair_amount + delta.repair_amount).max(0.0),
        repair_range: (base.repair_range + delta.repair_range_add).max(0.0),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{FittedSlot, FittingComp};
    use crate::world::SimWorld;
    use dawn_core::SectorId;
    use dawn_core::{
        fitting::{ActivationMode, ModuleDefinition, ModuleId, ModuleKind, SlotKind, StatDelta},
        NodeId, Position, ShipId, Velocity,
    };

    fn ship_id() -> ShipId {
        ShipId::new(NodeId(0), 1)
    }

    fn make_world_with_ship() -> (SimWorld, ShipId) {
        let mut world = SimWorld::new(SectorId(0));
        let id = ship_id();
        world.spawn_ship(id, Position::ORIGIN, Velocity::ZERO);
        (world, id)
    }

    #[test]
    fn apply_fitting_with_no_modules_leaves_base_stats_unchanged() {
        let (mut world, id) = make_world_with_ship();
        let result = apply_fitting(&mut world, id, ShipStatsComp::NPC);
        let stats = result.unwrap();
        assert_eq!(stats.max_speed, ShipStatsComp::NPC.max_speed);
        assert_eq!(stats.weapon_damage, ShipStatsComp::NPC.weapon_damage);
    }

    #[test]
    fn apply_fitting_adds_weapon_module_damage_to_base_stats() {
        let (mut world, id) = make_world_with_ship();
        let entity = world.find_entity(id).unwrap();
        let weapon_slot = FittedSlot {
            def: ModuleDefinition {
                id: ModuleId(1),
                name: "Railgun".to_string(),
                kind: ModuleKind::Weapon,
                slot: SlotKind::High,
                stat_delta: StatDelta {
                    weapon_damage_add: 15.0,
                    ..StatDelta::ZERO
                },
                activation_mode: ActivationMode::Active,
                cap_cost_per_cycle: 60.0,
                cycle_time_ticks: 10,
            },
            is_active: true,
            cycle_remaining: 0,
            target_ship_id: None,
        };
        world
            .get_mut::<FittingComp>(entity)
            .unwrap()
            .high
            .push(weapon_slot);
        let stats = apply_fitting(&mut world, id, ShipStatsComp::NPC).unwrap();
        assert_eq!(stats.weapon_damage, ShipStatsComp::NPC.weapon_damage + 15.0);
    }

    #[test]
    fn active_afterburner_multiplies_max_speed() {
        let (mut world, id) = make_world_with_ship();
        let entity = world.find_entity(id).unwrap();
        let ab_slot = FittedSlot {
            def: ModuleDefinition {
                id: ModuleId(5),
                name: "1MN AB".to_string(),
                kind: ModuleKind::Propulsion,
                slot: SlotKind::Mid,
                stat_delta: StatDelta {
                    speed_multiplier: 2.35,
                    ..StatDelta::ZERO
                },
                activation_mode: ActivationMode::Active,
                cap_cost_per_cycle: 100.0,
                cycle_time_ticks: 10,
            },
            is_active: true,
            cycle_remaining: 0,
            target_ship_id: None,
        };
        world
            .get_mut::<FittingComp>(entity)
            .unwrap()
            .mid
            .push(ab_slot);
        let stats = apply_fitting(&mut world, id, ShipStatsComp::NPC).unwrap();
        let expected = ShipStatsComp::NPC.max_speed * 2.35;
        assert!(
            (stats.max_speed - expected).abs() < 0.01,
            "AB ON: expected max_speed={expected}, got {}",
            stats.max_speed
        );
    }

    #[test]
    fn inactive_afterburner_does_not_affect_max_speed() {
        let (mut world, id) = make_world_with_ship();
        let entity = world.find_entity(id).unwrap();
        let ab_slot = FittedSlot {
            def: ModuleDefinition {
                id: ModuleId(5),
                name: "1MN AB".to_string(),
                kind: ModuleKind::Propulsion,
                slot: SlotKind::Mid,
                stat_delta: StatDelta {
                    speed_multiplier: 2.35,
                    ..StatDelta::ZERO
                },
                activation_mode: ActivationMode::Active,
                cap_cost_per_cycle: 100.0,
                cycle_time_ticks: 10,
            },
            is_active: false, // OFF
            cycle_remaining: 0,
            target_ship_id: None,
        };
        world
            .get_mut::<FittingComp>(entity)
            .unwrap()
            .mid
            .push(ab_slot);
        let stats = apply_fitting(&mut world, id, ShipStatsComp::NPC).unwrap();
        assert!(
            (stats.max_speed - ShipStatsComp::NPC.max_speed).abs() < 0.01,
            "AB OFF: max_speed must not change"
        );
    }

    #[test]
    fn mass_add_is_always_applied_regardless_of_module_active_state() {
        let (mut world, id) = make_world_with_ship();
        let entity = world.find_entity(id).unwrap();
        let oaab = FittedSlot {
            def: ModuleDefinition {
                id: ModuleId(10),
                name: "10MN AB".to_string(),
                kind: ModuleKind::Propulsion,
                slot: SlotKind::Mid,
                stat_delta: StatDelta {
                    speed_multiplier: 2.35,
                    mass_add: 8_000_000.0,
                    ..StatDelta::ZERO
                },
                activation_mode: ActivationMode::Active,
                cap_cost_per_cycle: 150.0,
                cycle_time_ticks: 10,
            },
            is_active: false, // OFF — but mass_add must still apply
            cycle_remaining: 0,
            target_ship_id: None,
        };
        world.get_mut::<FittingComp>(entity).unwrap().mid.push(oaab);
        let stats = apply_fitting(&mut world, id, ShipStatsComp::NPC).unwrap();
        let expected_mass = ShipStatsComp::NPC.mass + 8_000_000.0;
        assert!(
            (stats.mass - expected_mass).abs() < 1.0,
            "mass_add must apply even when module is OFF; expected={expected_mass}, got={}",
            stats.mass
        );
    }

    #[test]
    fn apply_delta_clamps_weapon_cooldown_to_minimum_one_tick() {
        let base = ShipStatsComp::NPC;
        let delta = StatDelta {
            weapon_cooldown_add: -100,
            ..StatDelta::ZERO
        };
        let result = apply_delta(base, &delta);
        assert_eq!(result.weapon_cooldown, 1);
    }

    #[test]
    fn apply_delta_does_not_allow_negative_damage() {
        let base = ShipStatsComp::NPC;
        let delta = StatDelta {
            weapon_damage_add: -9999.0,
            ..StatDelta::ZERO
        };
        let result = apply_delta(base, &delta);
        assert_eq!(result.weapon_damage, 0.0);
    }

    #[test]
    fn apply_fitting_returns_none_for_nonexistent_ship() {
        let mut world = SimWorld::new(SectorId(0));
        let fake_id = ShipId::new(NodeId(0), 999);
        assert!(apply_fitting(&mut world, fake_id, ShipStatsComp::NPC).is_none());
    }
}
