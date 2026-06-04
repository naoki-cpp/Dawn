//! Movement system.
//!
//! Applies velocity to every Ship's position, enforces Sector wall-bounce,
//! and returns one `ShipMoved` event per ship that actually moved.
//!
//! # Contract
//!
//! - Pure computation: no I/O, no global state.
//! - The caller appends the returned events to the EventStore.
//! - A ship that starts and ends at the same position emits no event.
//!   (A stationary ship with `Velocity::ZERO` produces zero events.)

use crate::{
    components::{PositionComp, ShipIdComp, VelocityComp},
    SimWorld,
};
use dawn_core::{
    events::{ShipMoved},
    DomainEvent, SectorBounds, Tick,
};

pub struct MovementSystem;

impl MovementSystem {
    /// Run movement for all ships in `world` and return the resulting events.
    ///
    /// `bounds` determines the Sector walls for bounce-back.
    /// `tick` is stamped onto every emitted event.
    ///
    /// # Complexity
    ///
    /// O(N) where N is the number of Ship entities in the world.
    pub fn run(world: &mut SimWorld, bounds: &SectorBounds, tick: Tick) -> Vec<DomainEvent> {
        let mut events = Vec::new();

        for (_entity, (id_comp, pos_comp, vel_comp)) in world
            .inner_mut()
            .query_mut::<(&ShipIdComp, &mut PositionComp, &mut VelocityComp)>()
        {
            let from = pos_comp.0;

            // Apply velocity.
            pos_comp.0.x += vel_comp.0.dx;
            pos_comp.0.y += vel_comp.0.dy;
            pos_comp.0.z += vel_comp.0.dz;

            // Enforce bounds with elastic wall-bounce.
            bounds.clamp_and_reflect(&mut pos_comp.0, &mut vel_comp.0);

            let to = pos_comp.0;

            // Skip ships that did not actually move (e.g. Velocity::ZERO).
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{NodeId, Position, SectorId, Velocity};

    fn bounds() -> SectorBounds {
        SectorBounds::cube(SectorBounds::DEFAULT_SIZE)
    }

    fn spawn_ship(
        world   : &mut SimWorld,
        counter : u64,
        pos     : Position,
        vel     : Velocity,
    ) {
        let id = dawn_core::ShipId::new(NodeId(0), counter);
        world.spawn_ship(id, pos, vel);
    }

    #[test]
    fn ship_with_zero_velocity_produces_no_event() {
        let mut world = SimWorld::new(SectorId(0));
        spawn_ship(&mut world, 1, Position::new(100.0, 100.0, 100.0), Velocity::ZERO);

        let events = MovementSystem::run(&mut world, &bounds(), Tick(1));
        assert!(events.is_empty(), "stationary ship must not emit ShipMoved");
    }

    #[test]
    fn ship_with_nonzero_velocity_produces_exactly_one_event_per_tick() {
        let mut world = SimWorld::new(SectorId(0));
        spawn_ship(
            &mut world, 1,
            Position::new(100.0, 100.0, 100.0),
            Velocity::new(1.0, 0.0, 0.0),
        );

        let events = MovementSystem::run(&mut world, &bounds(), Tick(1));
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn ship_moved_event_carries_correct_from_and_to_positions() {
        let start = Position::new(100.0, 200.0, 300.0);
        let vel   = Velocity::new(5.0, -3.0, 1.0);

        let mut world = SimWorld::new(SectorId(0));
        spawn_ship(&mut world, 1, start, vel);

        let events = MovementSystem::run(&mut world, &bounds(), Tick(7));

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
        // Place the ship just outside the max X boundary.
        let mut world = SimWorld::new(SectorId(0));
        spawn_ship(
            &mut world, 1,
            Position::new(SectorBounds::DEFAULT_SIZE - 1.0, 500.0, 500.0),
            Velocity::new(5.0, 0.0, 0.0), // moving toward max X wall
        );

        MovementSystem::run(&mut world, &bounds(), Tick(1));

        // After bounce, velocity on X must be negative (moving away from wall).
        for (_e, vel) in world.inner().query::<&VelocityComp>().iter() {
            assert!(vel.0.dx < 0.0, "velocity should be reflected after wall bounce");
        }
    }

    #[test]
    fn ten_ships_each_produce_one_event_per_tick() {
        let mut world = SimWorld::new(SectorId(0));
        for i in 0..10 {
            spawn_ship(
                &mut world,
                i,
                Position::new(i as f32 * 10.0, 0.0, 0.0),
                Velocity::new(1.0, 1.0, 0.0),
            );
        }

        let events = MovementSystem::run(&mut world, &bounds(), Tick(1));
        assert_eq!(events.len(), 10);
    }

    #[test]
    fn ship_moved_event_tick_matches_the_tick_passed_to_run() {
        let mut world = SimWorld::new(SectorId(0));
        spawn_ship(&mut world, 1, Position::new(100.0, 100.0, 100.0), Velocity::new(1.0, 0.0, 0.0));

        let events = MovementSystem::run(&mut world, &bounds(), Tick(999));
        assert_eq!(events[0].tick(), Tick(999));
    }
}
