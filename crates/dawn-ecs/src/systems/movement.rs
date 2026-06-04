//! Movement system — acceleration-based space physics.
//!
//! # Physics model (Cycle 2+)
//!
//! Every tick:
//!   1. Apply thrust: `velocity += normalize(thrust) * thrust_magnitude`
//!   2. Clamp speed:  `|velocity| > max_speed  →  scale down`
//!   3. Apply velocity: `position += velocity`
//!   4. Wall bounce: reflect velocity (and cancel thrust toward wall)
//!
//! NPC ships have `thrust_magnitude = 0.0` so they continue at constant
//! velocity (unchanged from Phase 0–1 behaviour).
//!
//! # Contract
//!
//! - Pure computation: no I/O, no global state.
//! - The caller appends the returned events to the EventStore.
//! - A ship that starts and ends at the same position emits no event.

use crate::{
    components::{PositionComp, ShipIdComp, ShipStatsComp, ThrustComp, VelocityComp},
    SimWorld,
};
use dawn_core::{events::ShipMoved, DomainEvent, SectorBounds, Tick, Velocity};

pub struct MovementSystem;

impl MovementSystem {
    /// Run one tick of movement for all ships and return `ShipMoved` events.
    pub fn run(world: &mut SimWorld, bounds: &SectorBounds, tick: Tick) -> Vec<DomainEvent> {
        let mut events = Vec::new();

        for (_entity, (id_comp, pos_comp, vel_comp, thrust_comp, stats_comp)) in world
            .inner_mut()
            .query_mut::<(
                &ShipIdComp,
                &mut PositionComp,
                &mut VelocityComp,
                &ThrustComp,
                &ShipStatsComp,
            )>()
        {
            let from = pos_comp.0;

            // ── 1. Apply thrust ───────────────────────────────────────────────
            if stats_comp.thrust_magnitude > 0.0 {
                let t = thrust_comp.0;
                let mag = magnitude(t);
                if mag > f32::EPSILON {
                    // Normalize thrust direction, scale by thrust_magnitude
                    let scale = stats_comp.thrust_magnitude / mag;
                    vel_comp.0.dx += t.dx * scale;
                    vel_comp.0.dy += t.dy * scale;
                    vel_comp.0.dz += t.dz * scale;
                }
            }

            // ── 2. Clamp to max_speed ─────────────────────────────────────────
            let speed = magnitude(vel_comp.0);
            if speed > stats_comp.max_speed && speed > f32::EPSILON {
                let scale = stats_comp.max_speed / speed;
                vel_comp.0.dx *= scale;
                vel_comp.0.dy *= scale;
                vel_comp.0.dz *= scale;
            }

            // ── 3. Apply velocity to position ─────────────────────────────────
            pos_comp.0.x += vel_comp.0.dx;
            pos_comp.0.y += vel_comp.0.dy;
            pos_comp.0.z += vel_comp.0.dz;

            // ── 4. Elastic wall bounce ────────────────────────────────────────
            bounds.clamp_and_reflect(&mut pos_comp.0, &mut vel_comp.0);

            let to = pos_comp.0;

            if (to.x - from.x).abs() > f32::EPSILON
                || (to.y - from.y).abs() > f32::EPSILON
                || (to.z - from.z).abs() > f32::EPSILON
            {
                events.push(DomainEvent::ShipMoved(ShipMoved {
                    ship_id: id_comp.0,
                    from,
                    to,
                    tick,
                }));
            }
        }

        events
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn magnitude(v: Velocity) -> f32 {
    (v.dx * v.dx + v.dy * v.dy + v.dz * v.dz).sqrt()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{NodeId, Position, SectorId, Velocity};
    use crate::components::ShipStatsComp;

    fn bounds() -> SectorBounds { SectorBounds::cube(SectorBounds::DEFAULT_SIZE) }

    fn spawn(world: &mut SimWorld, i: u64, pos: Position, vel: Velocity) {
        let id = dawn_core::ShipId::new(NodeId(0), i);
        world.spawn_ship(id, pos, vel);
    }

    #[test]
    fn ship_with_zero_velocity_produces_no_event() {
        let mut w = SimWorld::new(SectorId(0));
        spawn(&mut w, 1, Position::new(100.0, 100.0, 100.0), Velocity::ZERO);
        assert!(MovementSystem::run(&mut w, &bounds(), Tick(1)).is_empty());
    }

    #[test]
    fn ship_with_nonzero_velocity_produces_exactly_one_event_per_tick() {
        let mut w = SimWorld::new(SectorId(0));
        spawn(&mut w, 1, Position::new(100.0, 100.0, 100.0), Velocity::new(1.0, 0.0, 0.0));
        assert_eq!(MovementSystem::run(&mut w, &bounds(), Tick(1)).len(), 1);
    }

    #[test]
    fn ship_moved_event_carries_correct_from_and_to_positions() {
        let start = Position::new(100.0, 200.0, 300.0);
        let vel   = Velocity::new(5.0, -3.0, 1.0);
        let mut w = SimWorld::new(SectorId(0));
        spawn(&mut w, 1, start, vel);
        let events = MovementSystem::run(&mut w, &bounds(), Tick(7));
        match &events[0] {
            DomainEvent::ShipMoved(e) => {
                assert_eq!(e.from, start);
                assert_eq!(e.to.x, start.x + vel.dx);
                assert_eq!(e.to.y, start.y + vel.dy);
                assert_eq!(e.to.z, start.z + vel.dz);
                assert_eq!(e.tick, Tick(7));
            }
            other => panic!("expected ShipMoved, got {other:?}"),
        }
    }

    #[test]
    fn ship_bounces_off_wall_and_velocity_is_reflected() {
        let mut w = SimWorld::new(SectorId(0));
        spawn(
            &mut w, 1,
            Position::new(SectorBounds::DEFAULT_SIZE - 1.0, 500.0, 500.0),
            Velocity::new(5.0, 0.0, 0.0),
        );
        MovementSystem::run(&mut w, &bounds(), Tick(1));
        for (_e, vel) in w.inner().query::<&VelocityComp>().iter() {
            assert!(vel.0.dx < 0.0, "velocity should be reflected after wall bounce");
        }
    }

    #[test]
    fn ten_ships_each_produce_one_event_per_tick() {
        let mut w = SimWorld::new(SectorId(0));
        for i in 0..10 {
            spawn(&mut w, i, Position::new(i as f32 * 10.0, 0.0, 0.0), Velocity::new(1.0, 1.0, 0.0));
        }
        assert_eq!(MovementSystem::run(&mut w, &bounds(), Tick(1)).len(), 10);
    }

    #[test]
    fn ship_moved_event_tick_matches_the_tick_passed_to_run() {
        let mut w = SimWorld::new(SectorId(0));
        spawn(&mut w, 1, Position::new(100.0, 100.0, 100.0), Velocity::new(1.0, 0.0, 0.0));
        let events = MovementSystem::run(&mut w, &bounds(), Tick(999));
        assert_eq!(events[0].tick(), Tick(999));
    }

    // thrust が加速度として velocity に加算される
    #[test]
    fn thrust_accumulates_velocity_each_tick() {
        let mut w = SimWorld::new(SectorId(0));
        let id = dawn_core::ShipId::new(NodeId(0), 1);
        let entity = w.spawn_ship(id, Position::new(500.0, 500.0, 500.0), Velocity::ZERO);

        // Set player stats and thrust toward +X
        w.set_ship_stats(entity, ShipStatsComp::PLAYER);
        w.inner_mut()
            .get::<&mut ThrustComp>(entity)
            .unwrap()
            .0 = Velocity::new(1.0, 0.0, 0.0); // thrust direction

        MovementSystem::run(&mut w, &bounds(), Tick(1));

        // After one tick, velocity.dx should equal thrust_magnitude
        let vel = *w.inner().get::<&VelocityComp>(entity).unwrap();
        assert!(vel.0.dx > 0.0, "thrust should have added positive dx velocity");
        assert_eq!(vel.0.dy, 0.0);
        assert_eq!(vel.0.dz, 0.0);
    }

    // velocity が max_speed を超えないよう clamp される
    #[test]
    fn velocity_is_clamped_to_max_speed() {
        let mut w  = SimWorld::new(SectorId(0));
        let id     = dawn_core::ShipId::new(NodeId(0), 1);
        let entity = w.spawn_ship(
            id,
            Position::new(500.0, 500.0, 500.0),
            Velocity::new(10000.0, 0.0, 0.0), // far above any max_speed
        );
        w.set_ship_stats(entity, ShipStatsComp::PLAYER);

        MovementSystem::run(&mut w, &bounds(), Tick(1));

        let vel   = *w.inner().get::<&VelocityComp>(entity).unwrap();
        let stats = *w.inner().get::<&ShipStatsComp>(entity).unwrap();
        let speed = (vel.0.dx * vel.0.dx + vel.0.dy * vel.0.dy + vel.0.dz * vel.0.dz).sqrt();
        assert!(speed <= stats.max_speed + f32::EPSILON,
            "speed {speed} must not exceed max_speed {}", stats.max_speed);
    }
}
