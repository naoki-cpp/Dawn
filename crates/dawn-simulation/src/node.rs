//! `SimulationNode` — the self-contained simulation unit for one Sector.
//!
//! Ties together `SimWorld` (ECS), `InMemoryEventStore`, and the
//! `MovementSystem` into a runnable tick loop.
//!
//! No networking.  No async.  No global state.
//! This is the core that must work in isolation before any distribution layer
//! is added — the entire rationale for this crate existing.

use dawn_core::{
    events::ShipSpawned,
    DomainEvent, NodeId, Position, SectorBounds, SectorId, ShipId, Tick, Velocity,
};
use dawn_ecs::{systems::MovementSystem, SimWorld};
use dawn_event_store::{store::EventStore, InMemoryEventStore};

/// Result returned after executing one tick.
#[derive(Debug)]
pub struct TickResult {
    /// The tick that was just completed.
    pub tick          : Tick,
    /// Number of `ShipMoved` events emitted this tick.
    pub events_emitted: usize,
    /// The actual events produced (used by Actor layer for replication).
    pub events        : Vec<DomainEvent>,
}

/// A single-Sector simulation node.
///
/// Owns the ECS world and event store for one Sector.
/// Designed to run entirely without network communication.
pub struct SimulationNode {
    node_id      : NodeId,
    sector_id    : SectorId,
    bounds       : SectorBounds,
    world        : SimWorld,
    event_store  : InMemoryEventStore,
    current_tick : Tick,
    id_counter   : u64,
}

impl SimulationNode {
    pub fn new(node_id: NodeId, sector_id: SectorId, bounds: SectorBounds) -> Self {
        Self {
            node_id,
            sector_id,
            bounds,
            world        : SimWorld::new(sector_id),
            event_store  : InMemoryEventStore::new(),
            current_tick : Tick::ZERO,
            id_counter   : 0,
        }
    }

    // ── Identity ──────────────────────────────────────────────────────────────

    pub fn node_id(&self)    -> NodeId   { self.node_id }
    pub fn sector_id(&self)  -> SectorId { self.sector_id }

    // ── Spawn / Despawn ───────────────────────────────────────────────────────

    /// Spawn a Ship and append a `ShipSpawned` event.
    ///
    /// The `ShipId` is generated internally using the Node's counter.
    /// See CLAUDE.md INV-004: IDs are never reused.
    pub fn spawn_ship(&mut self, position: Position, velocity: Velocity) -> ShipId {
        let ship_id = ShipId::new(self.node_id, self.id_counter);
        self.id_counter += 1;

        self.world.spawn_ship(ship_id, position, velocity);

        self.event_store.append(DomainEvent::ShipSpawned(ShipSpawned {
            ship_id,
            sector_id       : self.sector_id,
            initial_position: position,
            tick            : self.current_tick,
        }));

        ship_id
    }

    // ── Tick ──────────────────────────────────────────────────────────────────

    /// Execute one simulation tick.
    ///
    /// Processing order (see CLAUDE.md §6 — must not be reordered):
    ///
    /// 1. Advance the logical tick counter.
    /// 2. Run the Movement System (pure ECS computation).
    /// 3. Append all produced events to the Event Store.
    /// 4. Return a `TickResult`.
    pub fn tick(&mut self) -> TickResult {
        // Step 1: advance tick.
        self.current_tick = self.current_tick.next();
        let tick = self.current_tick;

        // Step 2: run systems.
        let events = MovementSystem::run(&mut self.world, &self.bounds, tick);

        // Step 3: persist — all events are appended before any external
        // notification would go out (CLAUDE.md §4, forbidden pattern 3).
        let count = events.len();
        self.event_store.append_batch(events.iter().cloned());

        // Step 4: return result (events are returned for Actor-layer replication).
        TickResult { tick, events_emitted: count, events }
    }

    // ── Observation ───────────────────────────────────────────────────────────

    pub fn current_tick(&self)    -> Tick  { self.current_tick }
    pub fn ship_count(&self)      -> usize { self.world.ship_count() }
    pub fn total_event_count(&self) -> usize { self.event_store.len() }

    /// Borrow the event store for inspection (tests, replay, snapshotting).
    pub fn event_store(&self) -> &InMemoryEventStore {
        &self.event_store
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::Velocity;

    fn node() -> SimulationNode {
        SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::cube(SectorBounds::DEFAULT_SIZE),
        )
    }

    // ── Ship lifecycle ───────────────────────────────────────────────────────

    #[test]
    fn spawning_a_ship_appends_a_ship_spawned_event() {
        let mut node = node();
        node.spawn_ship(Position::ORIGIN, Velocity::ZERO);

        assert_eq!(node.total_event_count(), 1);
        assert!(matches!(
            node.event_store().all_records()[0].event,
            DomainEvent::ShipSpawned(_)
        ));
    }

    #[test]
    fn spawned_ships_receive_unique_ids() {
        let mut node = node();
        let id_a = node.spawn_ship(Position::ORIGIN, Velocity::ZERO);
        let id_b = node.spawn_ship(Position::ORIGIN, Velocity::ZERO);
        assert_ne!(id_a, id_b);
    }

    // ── Tick behaviour ───────────────────────────────────────────────────────

    #[test]
    fn tick_advances_the_logical_tick_counter_by_one() {
        let mut node = node();
        assert_eq!(node.current_tick(), Tick::ZERO);
        node.tick();
        assert_eq!(node.current_tick(), Tick(1));
        node.tick();
        assert_eq!(node.current_tick(), Tick(2));
    }

    #[test]
    fn each_moving_ship_produces_one_ship_moved_event_per_tick() {
        let mut node = node();
        node.spawn_ship(Position::new(100.0, 100.0, 100.0), Velocity::new(1.0, 0.0, 0.0));
        node.spawn_ship(Position::new(200.0, 100.0, 100.0), Velocity::new(0.0, 1.0, 0.0));

        let result = node.tick();
        assert_eq!(result.events_emitted, 2);
    }

    #[test]
    fn stationary_ships_produce_no_ship_moved_events() {
        let mut node = node();
        node.spawn_ship(Position::ORIGIN, Velocity::ZERO);
        let result = node.tick();
        assert_eq!(result.events_emitted, 0);
    }

    #[test]
    fn ship_moved_events_carry_the_current_tick_value() {
        let mut node = node();
        node.spawn_ship(Position::new(100.0, 100.0, 100.0), Velocity::new(1.0, 0.0, 0.0));
        node.tick(); // tick becomes 1
        node.tick(); // tick becomes 2

        let last = node.event_store().all_records().last().unwrap();
        assert_eq!(last.event.tick(), Tick(2));
    }

    // ── INV-002: replay ──────────────────────────────────────────────────────

    #[test]
    fn total_event_count_grows_monotonically_across_ticks() {
        let mut node = node();
        node.spawn_ship(Position::new(100.0, 100.0, 100.0), Velocity::new(1.0, 1.0, 1.0));
        let mut last = node.total_event_count();
        for _ in 0..10 {
            node.tick();
            assert!(
                node.total_event_count() >= last,
                "event count must never decrease"
            );
            last = node.total_event_count();
        }
    }

    #[test]
    fn replaying_events_reproduces_correct_spawn_count() {
        let mut node = node();
        for i in 0..5 {
            node.spawn_ship(
                Position::new(i as f32 * 100.0, 0.0, 0.0),
                Velocity::new(1.0, 0.0, 0.0),
            );
        }
        node.tick();

        let spawned = node
            .event_store()
            .iter_from(0)
            .filter(|r| matches!(r.event, DomainEvent::ShipSpawned(_)))
            .count();

        assert_eq!(spawned, 5, "replay must recover the number of spawned ships");
    }
}
