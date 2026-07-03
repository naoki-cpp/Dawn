//! Capacitor system — cycle-based activation cost and per-tick recharge.
//!
//! # Activation cycle model
//!
//! Each Active module runs in discrete cycles:
//!
//!   1. When `cycle_remaining == 0` and the module is ON, a new cycle starts:
//!      - If `cap >= cap_cost_per_cycle`: deduct cap, set `cycle_remaining = cycle_time_ticks`.
//!      - If cap is insufficient: force the module OFF, emit `ModuleDeactivated`.
//!   2. Each tick that the module is running: `cycle_remaining -= 1`.
//!   3. Cap recharges by `cap_recharge_per_tick` every tick (before cycle logic).
//!
//! # Processing order (must run AFTER Movement, BEFORE Combat)
//!
//! # Contract
//!
//! - Pure computation: no I/O, no global state.
//! - Returned events must be appended to the EventStore by the caller.
//! - After forced deactivation, `apply_fitting()` must be called by the caller
//!   so that `ShipStatsComp` reflects the new module state.

use crate::{
    components::{CapacitorComp, FittingComp, ShipIdComp, ShipStatsComp},
    systems::repair::RepairCycle,
    SimWorld,
};
use dawn_core::{
    events::ModuleDeactivated,
    fitting::{ActivationMode, ModuleKind},
    DomainEvent, ShipId, Tick,
};

// ── Result type ───────────────────────────────────────────────────────────────

/// Result returned by the capacitor system for one tick.
pub struct CapacitorResult {
    pub events: Vec<DomainEvent>,
    /// Ships that had at least one module force-deactivated.
    /// The caller must call `apply_fitting()` on each to update `ShipStatsComp`.
    pub refitted: Vec<ShipId>,
    /// Ships whose Weapon module started a new cycle this tick.
    /// The Combat system uses this list to decide which ships fire.
    pub weapon_cycles_started: Vec<ShipId>,
    /// Local repair module cycles that started this tick.
    pub repair_cycles_started: Vec<RepairCycle>,
}

// ── Internal snapshot ─────────────────────────────────────────────────────────

struct SlotInfo {
    /// Flat index into high→mid→low→rig ordering.
    flat_idx: usize,
    kind: ModuleKind,
    cap_cost: f32,
    repair_amount: f32,
    /// Per-slot target (ADR-0035/0036). `None` for self-only repair kinds;
    /// `Some` for Weapon/Tackle/Remote-repair kinds.
    target_ship_id: Option<ShipId>,
    cycle_time: u64,
    cycle_remaining: u64,
    is_active: bool,
}

struct ShipSnap {
    entity: hecs::Entity,
    ship_id: ShipId,
    cap_current: f32,
    cap_max: f32,
    cap_recharge: f32,
    slots: Vec<SlotInfo>,
}

// ── System entry point ────────────────────────────────────────────────────────

/// Run the capacitor system for one tick.
pub fn run(world: &mut SimWorld, tick: Tick) -> CapacitorResult {
    // ── 1. Read-only snapshot ─────────────────────────────────────────────────
    let snaps: Vec<ShipSnap> = world
        .query::<(&ShipIdComp, &CapacitorComp, &ShipStatsComp, &FittingComp)>()
        .iter()
        .map(|(entity, (sid, cap, stats, fit))| {
            let slots: Vec<SlotInfo> = fit
                .iter_slots()
                .enumerate()
                .filter(|(_, slot)| slot.def.activation_mode == ActivationMode::Active)
                .map(|(i, slot)| SlotInfo {
                    flat_idx: i,
                    kind: slot.def.kind,
                    cap_cost: slot.def.cap_cost_per_cycle,
                    repair_amount: slot.def.stat_delta.repair_amount,
                    target_ship_id: slot.target_ship_id,
                    cycle_time: slot.def.cycle_time_ticks,
                    cycle_remaining: slot.cycle_remaining,
                    is_active: slot.is_active,
                })
                .collect();

            ShipSnap {
                entity,
                ship_id: sid.0,
                cap_current: cap.current,
                cap_max: stats.cap_max,
                cap_recharge: stats.cap_recharge_per_tick,
                slots,
            }
        })
        .collect();

    // ── 2. Compute new cap, cycle updates, and forced deactivations ───────────
    let mut events: Vec<DomainEvent> = Vec::new();
    let mut refitted: Vec<ShipId> = Vec::new();
    let mut weapon_cycles_started: Vec<ShipId> = Vec::new();
    let mut repair_cycles_started: Vec<RepairCycle> = Vec::new();

    for snap in snaps {
        // Recharge first (every tick, before cycle logic).
        let mut cap = (snap.cap_current + snap.cap_recharge).min(snap.cap_max);

        let mut forced_off: Vec<usize> = Vec::new();
        let mut new_cycles: Vec<(usize, u64)> = Vec::new(); // (flat_idx, new remaining)
        let mut weapon_fired = false;

        for slot in &snap.slots {
            if !slot.is_active {
                continue; // Module is OFF; no cycle progression.
            }

            if slot.cycle_remaining == 0 {
                // Cycle ended — try to start a new one.
                if slot.cap_cost <= 0.0 || cap >= slot.cap_cost {
                    // Enough cap: consume and start cycle.
                    cap -= slot.cap_cost;
                    new_cycles.push((slot.flat_idx, slot.cycle_time.max(1)));
                    // Record weapon cycle start so Combat system can fire.
                    if slot.kind == ModuleKind::Weapon {
                        weapon_fired = true;
                    } else if slot.kind.repair_layer().is_some() {
                        // Local repair kinds never carry a target (ADR-0035
                        // requires_target() == false), so this falls back to
                        // self; Remote repair kinds always carry one (ADR-0036).
                        repair_cycles_started.push(RepairCycle {
                            target_ship_id: slot.target_ship_id.unwrap_or(snap.ship_id),
                            module_kind: slot.kind,
                            repair_amount: slot.repair_amount,
                        });
                    }
                } else {
                    // Insufficient cap: force the module OFF.
                    forced_off.push(slot.flat_idx);
                }
            } else {
                // Cycle in progress: tick down.
                new_cycles.push((slot.flat_idx, slot.cycle_remaining - 1));
            }
        }

        if weapon_fired {
            weapon_cycles_started.push(snap.ship_id);
        }

        let new_cap = cap.max(0.0);

        // Write cap back to ECS.
        apply_cap(world, snap.entity, new_cap);

        // Write cycle_remaining updates back to ECS.
        update_cycles(world, snap.entity, &new_cycles);

        // Force-deactivate modules and emit events.
        if !forced_off.is_empty() {
            deactivate_modules(
                world,
                snap.entity,
                snap.ship_id,
                &forced_off,
                tick,
                &mut events,
            );
            refitted.push(snap.ship_id);
        }
    }

    CapacitorResult {
        events,
        refitted,
        weapon_cycles_started,
        repair_cycles_started,
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn apply_cap(world: &mut SimWorld, entity: hecs::Entity, new_cap: f32) {
    if let Some(mut cap) = world.get_mut::<CapacitorComp>(entity) {
        cap.current = new_cap;
    }
}

fn update_cycles(world: &mut SimWorld, entity: hecs::Entity, updates: &[(usize, u64)]) {
    if updates.is_empty() {
        return;
    }

    let Some(mut fitting) = world.get_mut::<FittingComp>(entity) else {
        return;
    };

    for &(flat_idx, new_remaining) in updates {
        if let Some(slot) = fitting.slot_at_flat_mut(flat_idx) {
            slot.cycle_remaining = new_remaining;
        }
    }
}

fn deactivate_modules(
    world: &mut SimWorld,
    entity: hecs::Entity,
    ship_id: ShipId,
    flat_indices: &[usize],
    tick: Tick,
    events: &mut Vec<DomainEvent>,
) {
    let Some(mut fitting) = world.get_mut::<FittingComp>(entity) else {
        return;
    };

    for &flat_idx in flat_indices {
        if let Some(slot) = fitting.slot_at_flat_mut(flat_idx) {
            if slot.is_active {
                let module_id = slot.force_off();
                events.push(DomainEvent::ModuleDeactivated(ModuleDeactivated {
                    ship_id,
                    module_id,
                    slot: slot.def.slot,
                    forced_reason: Some(
                        dawn_core::events::ModuleDeactivationReason::CapacitorExhausted,
                    ),
                    tick,
                }));
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{FittedSlot, HullComp};
    use dawn_core::{
        fitting::{ActivationMode, ModuleDefinition, ModuleId, ModuleKind, SlotKind, StatDelta},
        NodeId, Position, SectorId, ShipId, Tick, Velocity,
    };

    fn test_ship_id() -> ShipId {
        ShipId::new(NodeId(0), 1)
    }

    /// Active weapon module: cap_cost=60, cycle_time=10 ticks.
    fn railgun_slot(active: bool) -> FittedSlot {
        FittedSlot {
            def: ModuleDefinition {
                id: ModuleId(1),
                name: "Railgun".to_string(),
                kind: ModuleKind::Weapon,
                slot: SlotKind::High,
                activation_mode: ActivationMode::Active,
                cap_cost_per_cycle: 60.0,
                cycle_time_ticks: 10,
                stat_delta: StatDelta {
                    weapon_damage_add: 25.0,
                    ..StatDelta::ZERO
                },
            },
            is_active: active,
            cycle_remaining: 0,
            target_ship_id: None,
        }
    }

    fn shield_booster_slot(active: bool) -> FittedSlot {
        FittedSlot {
            def: ModuleDefinition {
                id: ModuleId(13),
                name: "Small Shield Booster".to_string(),
                kind: ModuleKind::ShieldBooster,
                slot: SlotKind::Mid,
                activation_mode: ActivationMode::Active,
                cap_cost_per_cycle: 30.0,
                cycle_time_ticks: 8,
                stat_delta: StatDelta {
                    repair_amount: 45.0,
                    ..StatDelta::ZERO
                },
            },
            is_active: active,
            cycle_remaining: 0,
            target_ship_id: None,
        }
    }

    fn make_world(cap: f32, fitting: FittingComp, cap_max: f32, recharge: f32) -> SimWorld {
        let id = test_ship_id();
        let mut world = SimWorld::new(SectorId(0));
        let entity = world.spawn_ship(id, Position::ORIGIN, Velocity::ZERO);

        let stats = ShipStatsComp {
            cap_max,
            cap_recharge_per_tick: recharge,
            ..ShipStatsComp::PLAYER
        };
        world
            .inner_mut()
            .insert(
                entity,
                (
                    fitting,
                    HullComp::new(stats.max_shield, stats.max_armor, stats.max_hull),
                    CapacitorComp { current: cap },
                ),
            )
            .unwrap();
        world.set_ship_stats(entity, stats);
        world
    }

    fn read_cap(world: &SimWorld) -> f32 {
        let id = test_ship_id();
        world
            .query::<(&ShipIdComp, &CapacitorComp)>()
            .iter()
            .find(|(_, (sid, _))| sid.0 == id)
            .map(|(_, (_, cap))| cap.current)
            .expect("ship not found")
    }

    fn read_cycle(world: &SimWorld) -> u64 {
        let id = test_ship_id();
        let entity = world.find_entity(id).expect("ship not found");
        world.get::<FittingComp>(entity).unwrap().high[0].cycle_remaining
    }

    // ── Recharge ──────────────────────────────────────────────────────────────

    #[test]
    fn cap_recharges_every_tick_even_when_no_module_is_active() {
        let mut fitting = FittingComp::empty();
        fitting.high.push(railgun_slot(false)); // OFF
        let mut world = make_world(200.0, fitting, 500.0, 10.0);

        run(&mut world, Tick(1));

        assert_eq!(read_cap(&world), 210.0, "recharge: 200 + 10 = 210");
    }

    #[test]
    fn cap_does_not_exceed_cap_max() {
        let mut world = make_world(500.0, FittingComp::empty(), 500.0, 10.0);
        run(&mut world, Tick(1));
        assert_eq!(read_cap(&world), 500.0, "cap is clamped at cap_max");
    }

    // ── Cycle start ───────────────────────────────────────────────────────────

    #[test]
    fn new_cycle_consumes_cap_and_sets_cycle_remaining() {
        let mut fitting = FittingComp::empty();
        fitting.high.push(railgun_slot(true)); // ON, cycle_remaining=0
                                               // cap: 400 + 10 recharge = 410, then consume 60 → 350
        let mut world = make_world(400.0, fitting, 500.0, 10.0);

        run(&mut world, Tick(1));

        assert_eq!(
            read_cap(&world),
            350.0,
            "410 recharge - 60 cycle cost = 350"
        );
        assert_eq!(
            read_cycle(&world),
            10,
            "cycle_remaining set to cycle_time=10"
        );
    }

    #[test]
    fn repair_cycle_start_is_reported_with_module_repair_amount() {
        let mut fitting = FittingComp::empty();
        fitting.mid.push(shield_booster_slot(true));
        let mut world = make_world(100.0, fitting, 500.0, 0.0);

        let result = run(&mut world, Tick(1));

        assert_eq!(result.repair_cycles_started.len(), 1);
        let cycle = result.repair_cycles_started[0];
        assert_eq!(
            cycle.target_ship_id,
            test_ship_id(),
            "local repair has no target_ship_id, so it falls back to self"
        );
        assert_eq!(cycle.module_kind, ModuleKind::ShieldBooster);
        assert_eq!(cycle.repair_amount, 45.0);
    }

    #[test]
    fn cycle_counts_down_without_consuming_cap() {
        let mut fitting = FittingComp::empty();
        let mut slot = railgun_slot(true);
        slot.cycle_remaining = 5; // mid-cycle
        fitting.high.push(slot);
        let mut world = make_world(300.0, fitting, 500.0, 10.0);

        run(&mut world, Tick(1));

        // Cap only recharges; no cost consumed mid-cycle.
        assert_eq!(
            read_cap(&world),
            310.0,
            "mid-cycle: only recharge, no cap cost"
        );
        assert_eq!(read_cycle(&world), 4, "cycle_remaining decremented: 5→4");
    }

    #[test]
    fn new_cycle_starts_automatically_when_countdown_reaches_zero() {
        let mut fitting = FittingComp::empty();
        let mut slot = railgun_slot(true);
        slot.cycle_remaining = 1; // will hit 0 after this tick → next cycle starts
        fitting.high.push(slot);
        let mut world = make_world(400.0, fitting, 500.0, 10.0);

        run(&mut world, Tick(1)); // cycle_remaining: 1 → 0 (tick down, no cost)

        assert_eq!(read_cycle(&world), 0, "cycle just ended");
        assert_eq!(read_cap(&world), 410.0, "only recharge this tick");

        run(&mut world, Tick(2)); // cycle_remaining=0 → new cycle: 410+10-60=360

        assert_eq!(read_cap(&world), 360.0, "new cycle consumed 60 GJ");
        assert_eq!(read_cycle(&world), 10, "new cycle started");
    }

    // ── Force deactivation ────────────────────────────────────────────────────

    #[test]
    fn module_is_force_deactivated_and_event_emitted_when_cap_is_insufficient() {
        let mut fitting = FittingComp::empty();
        fitting.high.push(railgun_slot(true)); // ON, cycle_remaining=0, cost=60
                                               // cap: 10 + 10 recharge = 20 < 60 → force OFF
        let mut world = make_world(10.0, fitting, 500.0, 10.0);

        let result = run(&mut world, Tick(1));

        assert_eq!(result.refitted, vec![test_ship_id()]);
        assert_eq!(result.events.len(), 1);
        match &result.events[0] {
            DomainEvent::ModuleDeactivated(e) => {
                assert_eq!(e.module_id, ModuleId(1));
                assert_eq!(e.tick, Tick(1));
            }
            other => panic!("expected ModuleDeactivated, got {other:?}"),
        }
    }

    #[test]
    fn cap_is_not_negative_after_forced_deactivation() {
        let mut fitting = FittingComp::empty();
        fitting.high.push(railgun_slot(true));
        let mut world = make_world(0.0, fitting, 500.0, 5.0); // recharge < cost
        run(&mut world, Tick(1));
        assert!(read_cap(&world) >= 0.0, "cap must never be negative");
    }

    #[test]
    fn off_module_does_not_start_a_cycle() {
        let mut fitting = FittingComp::empty();
        fitting.high.push(railgun_slot(false)); // OFF
        let mut world = make_world(500.0, fitting, 500.0, 0.0);
        run(&mut world, Tick(1));
        assert_eq!(read_cycle(&world), 0, "OFF module: cycle_remaining stays 0");
        assert_eq!(read_cap(&world), 500.0, "OFF module: no cap consumed");
    }
}
