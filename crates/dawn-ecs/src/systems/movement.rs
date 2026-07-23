//! Movement system adapter for the shared EVE-style movement policy (ADR-0023).
//!
//! `dawn_core::MovementProfile::step` owns the one-tick exponential approach
//! calculation. This system supplies ECS state, integrates the returned
//! displacement, skips committed warp, and emits `VelocityChanged` events.
//!
//! # Align time (EVE-compatible, ADR-0022 / ADR-0023)
//!
//! Warp engages when speed_toward(gate) ≥ max_speed × 0.75.
//! Time to reach 75% of max_speed = −ln(0.25) × τ ≈ 1.386 × τ ticks.
//! τ is controlled by mass × inertia_modifier, which emerges from the hull and
//! fitted modules (including passive mass_add from oversized ABs — ADR-0023).
//!
//! # Authoritative Event (ADR-0008 / INV-MOVE)
//!
//! Emits `VelocityChanged` only when velocity actually differs from the previous tick.

use crate::{
    components::{PositionComp, ShipIdComp, ShipStatsComp, ThrustComp, VelocityComp, WarpComp},
    SimWorld,
};
use dawn_core::{
    events::VelocityChanged, DomainEvent, MovementInput, MovementProfile, Tick, Velocity,
};

#[derive(Debug)]
pub struct MovementSystem;

impl MovementSystem {
    /// Run one tick of movement for all non-warping ships.
    ///
    /// Returns `VelocityChanged` events for ships whose velocity changed.
    pub fn run(world: &mut SimWorld, tick: Tick) -> Vec<DomainEvent> {
        let mut events = Vec::new();

        for (_entity, (id_comp, pos_comp, vel_comp, thrust_comp, stats_comp, warp_comp)) in
            world.inner_mut().query_mut::<(
                &ShipIdComp,
                &mut PositionComp,
                &mut VelocityComp,
                &mut ThrustComp,
                &ShipStatsComp,
                Option<&WarpComp>,
            )>()
        {
            // Ships in the committed warping phase are owned by process_warp;
            // skip them so warp speed is not clamped (ADR-0022 §6).
            if warp_comp.is_some_and(|w| w.is_warping()) {
                continue;
            }

            let old_velocity = vel_comp.0;

            let Ok(profile) = MovementProfile::new(
                stats_comp.max_speed,
                stats_comp.mass,
                stats_comp.inertia_modifier,
            ) else {
                // Fitting and ship-type loading validate these values before
                // they reach the hot path. Do not advance malformed state.
                continue;
            };
            let input = if thrust_comp.is_braking {
                MovementInput::Brake
            } else {
                MovementInput::Thrust(thrust_comp.direction)
            };
            let step = profile.step(vel_comp.0, input);
            vel_comp.0 = step.velocity;
            if step.braking_complete {
                thrust_comp.is_braking = false;
            }

            // ── Integrate position ────────────────────────────────────────────
            pos_comp.0.x += vel_comp.0.dx;
            pos_comp.0.y += vel_comp.0.dy;
            pos_comp.0.z += vel_comp.0.dz;

            // ── Emit VelocityChanged only when velocity actually changed ───────
            if velocity_changed(old_velocity, vel_comp.0) {
                events.push(DomainEvent::VelocityChanged(VelocityChanged {
                    ship_id: id_comp.0,
                    velocity: vel_comp.0,
                    tick,
                }));
            }
        }

        events
    }
}

fn velocity_changed(old: Velocity, new: Velocity) -> bool {
    (new.dx - old.dx).abs() > f32::EPSILON
        || (new.dy - old.dy).abs() > f32::EPSILON
        || (new.dz - old.dz).abs() > f32::EPSILON
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ShipStatsComp;
    use dawn_core::{MovementInput, MovementProfile, NodeId, Position, SectorId, Velocity};

    fn spawn(world: &mut SimWorld, i: u64, pos: Position, vel: Velocity) {
        let id = dawn_core::ShipId::new(NodeId(0), i);
        world.spawn_ship(id, pos, vel);
    }

    #[test]
    fn ship_with_zero_velocity_produces_no_event() {
        let mut w = SimWorld::new(SectorId(0));
        spawn(
            &mut w,
            1,
            Position::new(100.0, 100.0, 100.0),
            Velocity::ZERO,
        );
        assert!(MovementSystem::run(&mut w, Tick(1)).is_empty());
    }

    #[test]
    fn ship_with_nonzero_velocity_coasts_without_event() {
        // Constant-velocity coasting: no thrust, velocity unchanged → no event.
        let mut w = SimWorld::new(SectorId(0));
        spawn(
            &mut w,
            1,
            Position::new(100.0, 100.0, 100.0),
            Velocity::new(1.0, 0.0, 0.0),
        );
        // NPC has no thrust direction set, so v_target = v → Δv = 0.
        assert!(MovementSystem::run(&mut w, Tick(1)).is_empty());
    }

    #[test]
    fn warping_ship_is_skipped_by_movement_so_warp_speed_is_not_clamped() {
        use crate::components::{WarpComp, WarpPhase};
        let mut w = SimWorld::new(SectorId(0));
        let id = dawn_core::ShipId::new(NodeId(0), 1);
        let warp_vel = Velocity::new(5000.0, 0.0, 0.0);
        let entity = w.spawn_ship(id, Position::ORIGIN, warp_vel);
        w.set_ship_stats(entity, ShipStatsComp::PLAYER);
        w.inner_mut()
            .insert_one(
                entity,
                WarpComp {
                    target: dawn_core::WarpTarget::Gate(dawn_core::JumpGateId(0)),
                    phase: WarpPhase::Warping,
                    auto_jump: false,
                    warp_start_abs: dawn_core::AbsolutePosition::ORIGIN,
                    warp_total: 1,
                    warp_elapsed: 0,
                    warp_arrival_abs: dawn_core::AbsolutePosition::ORIGIN,
                    warp_start_vel: Velocity::ZERO,
                },
            )
            .unwrap();

        let events = MovementSystem::run(&mut w, Tick(1));
        assert!(events.is_empty(), "movement must not touch a warping ship");
        let pos = w.inner().get::<&PositionComp>(entity).unwrap().0;
        assert_eq!(
            pos,
            Position::ORIGIN,
            "movement must not integrate a warping ship"
        );
        let vel = w.inner().get::<&VelocityComp>(entity).unwrap().0;
        assert_eq!(vel, warp_vel, "movement must not clamp warp speed");
    }

    #[test]
    fn velocity_changed_event_emitted_when_thrust_changes_velocity() {
        let mut w = SimWorld::new(SectorId(0));
        let id = dawn_core::ShipId::new(NodeId(0), 1);
        let entity = w.spawn_ship(id, Position::new(500.0, 500.0, 500.0), Velocity::ZERO);
        w.set_ship_stats(entity, ShipStatsComp::PLAYER);
        w.inner_mut()
            .get::<&mut ThrustComp>(entity)
            .unwrap()
            .direction = Velocity::new(1.0, 0.0, 0.0);
        let events = MovementSystem::run(&mut w, Tick(1));
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], DomainEvent::VelocityChanged(_)));
    }

    #[test]
    fn velocity_changed_event_carries_new_velocity_and_tick() {
        let mut w = SimWorld::new(SectorId(0));
        let id = dawn_core::ShipId::new(NodeId(0), 1);
        let entity = w.spawn_ship(id, Position::new(500.0, 500.0, 500.0), Velocity::ZERO);
        w.set_ship_stats(entity, ShipStatsComp::PLAYER);
        w.inner_mut()
            .get::<&mut ThrustComp>(entity)
            .unwrap()
            .direction = Velocity::new(1.0, 0.0, 0.0);
        let events = MovementSystem::run(&mut w, Tick(7));
        if let DomainEvent::VelocityChanged(e) = &events[0] {
            assert!(e.velocity.dx > 0.0);
            assert_eq!(e.tick, Tick(7));
        } else {
            panic!("expected VelocityChanged");
        }
    }

    #[test]
    fn position_advances_by_velocity_each_tick() {
        let start = Position::new(100.0, 200.0, 300.0);
        let vel = Velocity::new(5.0, -3.0, 1.0);
        let mut w = SimWorld::new(SectorId(0));
        spawn(&mut w, 1, start, vel);
        MovementSystem::run(&mut w, Tick(1));
        for (_, pos) in w.inner().query::<&PositionComp>().iter() {
            assert!((pos.0.x - (start.x + vel.dx)).abs() < 0.001);
            assert!((pos.0.y - (start.y + vel.dy)).abs() < 0.001);
        }
    }

    #[test]
    fn ship_continues_past_old_sector_boundary_without_bouncing() {
        let mut w = SimWorld::new(SectorId(0));
        spawn(
            &mut w,
            1,
            Position::new(9999.0, 0.0, 0.0),
            Velocity::new(100.0, 0.0, 0.0),
        );
        MovementSystem::run(&mut w, Tick(1));
        for (_e, vel) in w.inner().query::<&VelocityComp>().iter() {
            assert!(
                vel.0.dx > 0.0,
                "velocity must not be reversed — no walls in space"
            );
        }
    }

    #[test]
    fn ten_ships_with_thrust_each_produce_one_event_per_tick() {
        let mut w = SimWorld::new(SectorId(0));
        for i in 0..10 {
            let id = dawn_core::ShipId::new(NodeId(0), i);
            let entity = w.spawn_ship(id, Position::new(i as f32 * 10.0, 0.0, 0.0), Velocity::ZERO);
            w.set_ship_stats(entity, ShipStatsComp::PLAYER);
            w.inner_mut()
                .get::<&mut ThrustComp>(entity)
                .unwrap()
                .direction = Velocity::new(1.0, 0.0, 0.0);
        }
        assert_eq!(MovementSystem::run(&mut w, Tick(1)).len(), 10);
    }

    #[test]
    fn velocity_changed_event_tick_matches_the_tick_passed_to_run() {
        let mut w = SimWorld::new(SectorId(0));
        let id = dawn_core::ShipId::new(NodeId(0), 1);
        let entity = w.spawn_ship(id, Position::new(100.0, 100.0, 100.0), Velocity::ZERO);
        w.set_ship_stats(entity, ShipStatsComp::PLAYER);
        w.inner_mut()
            .get::<&mut ThrustComp>(entity)
            .unwrap()
            .direction = Velocity::new(1.0, 0.0, 0.0);
        assert_eq!(MovementSystem::run(&mut w, Tick(999))[0].tick(), Tick(999));
    }

    #[test]
    fn thrust_accumulates_velocity_toward_max_speed_exponentially() {
        let mut w = SimWorld::new(SectorId(0));
        let id = dawn_core::ShipId::new(NodeId(0), 1);
        let entity = w.spawn_ship(id, Position::new(500.0, 500.0, 500.0), Velocity::ZERO);
        w.set_ship_stats(entity, ShipStatsComp::PLAYER);
        w.inner_mut()
            .get::<&mut ThrustComp>(entity)
            .unwrap()
            .direction = Velocity::new(1.0, 0.0, 0.0);
        MovementSystem::run(&mut w, Tick(1));
        let vel = *w.inner().get::<&VelocityComp>(entity).unwrap();
        assert!(vel.0.dx > 0.0);
        assert_eq!(vel.0.dy, 0.0);
        assert_eq!(vel.0.dz, 0.0);
        // Must not exceed max_speed immediately.
        assert!(vel.0.dx <= ShipStatsComp::PLAYER.max_speed + f32::EPSILON);
    }

    #[test]
    fn adapter_matches_the_shared_policy_for_one_tick() {
        let stats = ShipStatsComp::PLAYER;
        let direction = Velocity::new(1.0, 0.0, 0.0);
        let expected = MovementProfile::new(stats.max_speed, stats.mass, stats.inertia_modifier)
            .unwrap()
            .step(Velocity::ZERO, MovementInput::Thrust(direction))
            .velocity;

        let mut w = SimWorld::new(SectorId(0));
        let id = dawn_core::ShipId::new(NodeId(0), 1);
        let entity = w.spawn_ship(id, Position::ORIGIN, Velocity::ZERO);
        w.set_ship_stats(entity, stats);
        w.inner_mut()
            .get::<&mut ThrustComp>(entity)
            .unwrap()
            .direction = direction;

        MovementSystem::run(&mut w, Tick(1));

        let actual = w.inner().get::<&VelocityComp>(entity).unwrap().0;
        assert_eq!(actual, expected);
    }

    #[test]
    fn braking_decelerates_ship_and_stops_without_overshoot() {
        let mut w = SimWorld::new(SectorId(0));
        let id = dawn_core::ShipId::new(NodeId(0), 1);
        let entity = w.spawn_ship(id, Position::ORIGIN, Velocity::new(100.0, 0.0, 0.0));
        w.set_ship_stats(entity, ShipStatsComp::PLAYER);
        w.inner_mut()
            .get::<&mut ThrustComp>(entity)
            .unwrap()
            .is_braking = true;

        for _ in 0..500 {
            MovementSystem::run(&mut w, Tick(1));
        }

        let vel = *w.inner().get::<&VelocityComp>(entity).unwrap();
        assert_eq!(vel.0, Velocity::ZERO, "ship must come to a complete stop");
        let thrust = *w.inner().get::<&ThrustComp>(entity).unwrap();
        assert!(
            !thrust.is_braking,
            "is_braking must be cleared once stopped"
        );
    }

    #[test]
    fn braking_emits_velocity_changed_events_until_stopped() {
        let mut w = SimWorld::new(SectorId(0));
        let id = dawn_core::ShipId::new(NodeId(0), 1);
        let entity = w.spawn_ship(id, Position::ORIGIN, Velocity::new(200.0, 0.0, 0.0));
        w.set_ship_stats(entity, ShipStatsComp::PLAYER);
        w.inner_mut()
            .get::<&mut ThrustComp>(entity)
            .unwrap()
            .is_braking = true;
        let events = MovementSystem::run(&mut w, Tick(1));
        assert!(
            !events.is_empty(),
            "first braking tick must emit VelocityChanged"
        );
    }

    /// Verify that align time (ticks to reach 75% of max_speed) ≈ 1.386 × τ_ticks.
    #[test]
    fn align_time_matches_eve_formula_1386_times_tau() {
        let mut w = SimWorld::new(SectorId(0));
        let id = dawn_core::ShipId::new(NodeId(0), 1);
        let entity = w.spawn_ship(id, Position::ORIGIN, Velocity::ZERO);
        // Use PLAYER stats: mass=10M, inertia=0.3 → τ = 10M*0.3/100_000 = 30 ticks.
        w.set_ship_stats(entity, ShipStatsComp::PLAYER);
        w.inner_mut()
            .get::<&mut ThrustComp>(entity)
            .unwrap()
            .direction = Velocity::new(1.0, 0.0, 0.0);

        let tau_ticks = ShipStatsComp::PLAYER.mass * ShipStatsComp::PLAYER.inertia_modifier
            / dawn_core::MASS_SCALE;
        let expected_align = (-0.25_f32.ln() * tau_ticks).ceil() as u32;
        let threshold = ShipStatsComp::PLAYER.max_speed * 0.75;

        let mut actual_align = 0u32;
        for t in 1..=500 {
            MovementSystem::run(&mut w, Tick(t as u64));
            let vel = w.inner().get::<&VelocityComp>(entity).unwrap().0;
            if vel.dx >= threshold {
                actual_align = t;
                break;
            }
        }
        assert!(actual_align > 0, "ship never reached 75% of max_speed");
        // Allow ±2 ticks tolerance for discrete-time approximation.
        assert!(
            actual_align.abs_diff(expected_align) <= 2,
            "align_time actual={actual_align} expected≈{expected_align} (τ={tau_ticks:.1})"
        );
    }

    #[test]
    fn velocity_never_exceeds_max_speed() {
        let mut w = SimWorld::new(SectorId(0));
        let id = dawn_core::ShipId::new(NodeId(0), 1);
        let entity = w.spawn_ship(id, Position::new(500.0, 500.0, 500.0), Velocity::ZERO);
        w.set_ship_stats(entity, ShipStatsComp::PLAYER);
        w.inner_mut()
            .get::<&mut ThrustComp>(entity)
            .unwrap()
            .direction = Velocity::new(1.0, 0.0, 0.0);
        for t in 1..=200 {
            MovementSystem::run(&mut w, Tick(t));
        }
        let vel = *w.inner().get::<&VelocityComp>(entity).unwrap();
        let stats = *w.inner().get::<&ShipStatsComp>(entity).unwrap();
        assert!(
            vel.0.speed() <= stats.max_speed + 0.001,
            "speed {} exceeds max_speed {}",
            vel.0.speed(),
            stats.max_speed
        );
    }
}
