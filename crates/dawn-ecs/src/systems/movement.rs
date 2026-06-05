//! Movement system — acceleration-based space physics.
//!
//! # Physics model
//!
//! Every tick:
//!   1. Apply thrust: `velocity += normalize(thrust) * thrust_magnitude`
//!   2. Clamp speed:  `|velocity| > max_speed  →  scale down`
//!   3. Apply velocity: `position += velocity`
//!
//! No walls. Space is infinite. Ships drift forever unless thrust is applied.
//!
//! # Authoritative Event (ADR-0008)
//!
//! Emits `VelocityChanged` when velocity differs from the previous Tick.
//! Ships with unchanged velocity emit no event.
//! Position is derived state and is NOT recorded in any event.
//!
//! # Contract
//!
//! - Pure computation: no I/O, no global state.
//! - The caller appends the returned events to the EventStore.

use crate::{
    components::{PositionComp, ShipIdComp, ShipStatsComp, ThrustComp, VelocityComp},
    SimWorld,
};
use dawn_core::{events::VelocityChanged, DomainEvent, Tick, Velocity};

pub struct MovementSystem;

impl MovementSystem {
    /// Run one tick of movement for all ships.
    ///
    /// Returns `VelocityChanged` events for ships whose velocity changed this tick.
    /// Ships at rest or with unchanged velocity emit no event.
    pub fn run(world: &mut SimWorld, tick: Tick) -> Vec<DomainEvent> {
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
            let old_velocity = vel_comp.0;

            // ── 1. Apply thrust ───────────────────────────────────────────────
            if stats_comp.thrust_magnitude > 0.0 {
                let t   = thrust_comp.0;
                let mag = magnitude(t);
                if mag > f32::EPSILON {
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

            // ── 4. Emit VelocityChanged only when velocity actually changed ────
            let new_velocity = vel_comp.0;
            if velocity_changed(old_velocity, new_velocity) {
                events.push(DomainEvent::VelocityChanged(VelocityChanged {
                    ship_id : id_comp.0,
                    velocity: new_velocity,
                    tick,
                }));
            }
        }

        events
    }
}

/// Returns true if velocity changed by more than float epsilon.
fn velocity_changed(old: Velocity, new: Velocity) -> bool {
    (new.dx - old.dx).abs() > f32::EPSILON
        || (new.dy - old.dy).abs() > f32::EPSILON
        || (new.dz - old.dz).abs() > f32::EPSILON
}

fn magnitude(v: Velocity) -> f32 {
    (v.dx * v.dx + v.dy * v.dy + v.dz * v.dz).sqrt()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ShipStatsComp;
    use dawn_core::{NodeId, Position, SectorId, Velocity};

    fn spawn(world: &mut SimWorld, i: u64, pos: Position, vel: Velocity) {
        let id = dawn_core::ShipId::new(NodeId(0), i);
        world.spawn_ship(id, pos, vel);
    }

    #[test]
    fn ship_with_zero_velocity_produces_no_event() {
        let mut w = SimWorld::new(SectorId(0));
        spawn(&mut w, 1, Position::new(100.0, 100.0, 100.0), Velocity::ZERO);
        assert!(MovementSystem::run(&mut w, Tick(1)).is_empty());
    }

    #[test]
    fn ship_with_nonzero_velocity_produces_no_event_when_velocity_unchanged() {
        // 等速直線運動: thrust=0, 速度変化なし → イベントなし
        let mut w = SimWorld::new(SectorId(0));
        spawn(&mut w, 1, Position::new(100.0, 100.0, 100.0), Velocity::new(1.0, 0.0, 0.0));
        // NPC (thrust_magnitude=0) は速度が変わらない
        assert!(MovementSystem::run(&mut w, Tick(1)).is_empty(),
            "等速直線運動では VelocityChanged は発行しない");
    }

    #[test]
    fn velocity_changed_event_emitted_when_thrust_changes_velocity() {
        let mut w  = SimWorld::new(SectorId(0));
        let id     = dawn_core::ShipId::new(NodeId(0), 1);
        let entity = w.spawn_ship(id, Position::new(500.0, 500.0, 500.0), Velocity::ZERO);
        w.set_ship_stats(entity, ShipStatsComp::PLAYER);
        w.inner_mut().get::<&mut ThrustComp>(entity).unwrap().0 = Velocity::new(1.0, 0.0, 0.0);
        let events = MovementSystem::run(&mut w, Tick(1));
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], DomainEvent::VelocityChanged(_)));
    }

    #[test]
    fn velocity_changed_event_carries_new_velocity_and_tick() {
        let mut w  = SimWorld::new(SectorId(0));
        let id     = dawn_core::ShipId::new(NodeId(0), 1);
        let entity = w.spawn_ship(id, Position::new(500.0, 500.0, 500.0), Velocity::ZERO);
        w.set_ship_stats(entity, ShipStatsComp::PLAYER);
        w.inner_mut().get::<&mut ThrustComp>(entity).unwrap().0 = Velocity::new(1.0, 0.0, 0.0);
        let events = MovementSystem::run(&mut w, Tick(7));
        if let DomainEvent::VelocityChanged(e) = &events[0] {
            assert!(e.velocity.dx > 0.0, "velocity.dx should be positive after thrust");
            assert_eq!(e.tick, Tick(7));
        } else {
            panic!("expected VelocityChanged");
        }
    }

    #[test]
    fn position_advances_by_velocity_each_tick() {
        let start = Position::new(100.0, 200.0, 300.0);
        let vel   = Velocity::new(5.0, -3.0, 1.0);
        let mut w = SimWorld::new(SectorId(0));
        spawn(&mut w, 1, start, vel);
        MovementSystem::run(&mut w, Tick(1));
        // position should have advanced by vel (NPC: no thrust, velocity unchanged)
        for (_, pos) in w.inner().query::<&PositionComp>().iter() {
            assert!((pos.0.x - (start.x + vel.dx)).abs() < f32::EPSILON);
            assert!((pos.0.y - (start.y + vel.dy)).abs() < f32::EPSILON);
        }
    }

    #[test]
    fn ship_continues_past_old_sector_boundary_without_bouncing() {
        let mut w = SimWorld::new(SectorId(0));
        spawn(&mut w, 1, Position::new(9999.0, 0.0, 0.0), Velocity::new(100.0, 0.0, 0.0));
        MovementSystem::run(&mut w, Tick(1));
        for (_e, vel) in w.inner().query::<&VelocityComp>().iter() {
            assert!(vel.0.dx > 0.0, "velocity must not be reversed — no walls in space");
        }
    }

    #[test]
    fn ten_ships_with_thrust_each_produce_one_event_per_tick() {
        let mut w = SimWorld::new(SectorId(0));
        for i in 0..10 {
            let id     = dawn_core::ShipId::new(NodeId(0), i);
            let entity = w.spawn_ship(id, Position::new(i as f32 * 10.0, 0.0, 0.0), Velocity::ZERO);
            w.set_ship_stats(entity, ShipStatsComp::PLAYER);
            w.inner_mut().get::<&mut ThrustComp>(entity).unwrap().0 = Velocity::new(1.0, 0.0, 0.0);
        }
        assert_eq!(MovementSystem::run(&mut w, Tick(1)).len(), 10);
    }

    #[test]
    fn velocity_changed_event_tick_matches_the_tick_passed_to_run() {
        let mut w  = SimWorld::new(SectorId(0));
        let id     = dawn_core::ShipId::new(NodeId(0), 1);
        let entity = w.spawn_ship(id, Position::new(100.0, 100.0, 100.0), Velocity::ZERO);
        w.set_ship_stats(entity, ShipStatsComp::PLAYER);
        w.inner_mut().get::<&mut ThrustComp>(entity).unwrap().0 = Velocity::new(1.0, 0.0, 0.0);
        assert_eq!(MovementSystem::run(&mut w, Tick(999))[0].tick(), Tick(999));
    }

    #[test]
    fn thrust_accumulates_velocity_each_tick() {
        let mut w  = SimWorld::new(SectorId(0));
        let id     = dawn_core::ShipId::new(NodeId(0), 1);
        let entity = w.spawn_ship(id, Position::new(500.0, 500.0, 500.0), Velocity::ZERO);
        w.set_ship_stats(entity, ShipStatsComp::PLAYER);
        w.inner_mut().get::<&mut ThrustComp>(entity).unwrap().0 = Velocity::new(1.0, 0.0, 0.0);
        MovementSystem::run(&mut w, Tick(1));
        let vel = *w.inner().get::<&VelocityComp>(entity).unwrap();
        assert!(vel.0.dx > 0.0);
        assert_eq!(vel.0.dy, 0.0);
        assert_eq!(vel.0.dz, 0.0);
    }

    #[test]
    fn velocity_is_clamped_to_max_speed() {
        let mut w  = SimWorld::new(SectorId(0));
        let id     = dawn_core::ShipId::new(NodeId(0), 1);
        let entity = w.spawn_ship(id, Position::new(500.0, 500.0, 500.0),
                                  Velocity::new(10000.0, 0.0, 0.0));
        w.set_ship_stats(entity, ShipStatsComp::PLAYER);
        MovementSystem::run(&mut w, Tick(1));
        let vel   = *w.inner().get::<&VelocityComp>(entity).unwrap();
        let stats = *w.inner().get::<&ShipStatsComp>(entity).unwrap();
        assert!(magnitude(vel.0) <= stats.max_speed + f32::EPSILON);
    }
}
