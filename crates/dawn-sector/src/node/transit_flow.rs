//! `SimulationNode` state transitions for Sector Transit (ADR-0014).
//!
//! The top-level `dawn-sector::transit` module owns the Raft payload
//! (`TransitOp`) and Step 7.5 orchestration. This module keeps the ECS and
//! EventStore mutations close to `SimulationNode`, where the required private
//! state already lives.

use dawn_core::{
    commands::TransitCommand,
    events::{JumpGateUsed, SectorTransitCompleted, SectorTransitRequested, StarSystemChanged},
    fitting::FittingSnapshot,
    DawnError, DomainEvent, JumpGateId, Position, SectorId, ShipId, ShipTypeId,
};
use dawn_ecs::{
    components::{CapacitorComp, FittingComp, HullComp, PositionComp, VelocityComp},
    TransitState,
};
use dawn_event_store::store::EventStore;

use crate::persistence::ShipSnapshot;

use super::SimulationNode;

impl<S: EventStore> SimulationNode<S> {
    /// Validate and begin a Sector Transit (CLAUDE.md §4 Step 2).
    ///
    /// On success, marks the Ship `TransitState::InTransit` and appends a
    /// `SectorTransitRequested` event (ownership stays with this Sector).
    /// On failure, no event is appended (CommandRejected per INV-006).
    ///
    /// In the Raft pipeline (ADR-0014) this is invoked when a committed
    /// `TransitOp::Request` is applied at Step 7.5 — never directly from a
    /// client command.
    pub fn propose_transit(&mut self, cmd: TransitCommand) -> Result<(), DawnError> {
        let &entity = self.ships.index.get(&cmd.ship_id)
            .ok_or(DawnError::ShipNotFound(cmd.ship_id))?;

        if self.world.transit_state(entity).is_in_transit() {
            return Err(DawnError::ShipInTransit(cmd.ship_id));
        }

        self.world.set_transit_state(entity, TransitState::InTransit { to: cmd.to });

        self.event_store.append(DomainEvent::SectorTransitRequested(SectorTransitRequested {
            ship_id: cmd.ship_id,
            from   : self.sector_id,
            to     : cmd.to,
            tick   : self.current_tick,
        }));

        Ok(())
    }

    /// Whether a `TransitCommand` for `ship_id` would currently be accepted
    /// (Ship exists and is not already in transit). Used to reject commands
    /// up front, before proposing to the Raft Log (INV-006).
    pub fn can_propose_transit(&self, ship_id: ShipId) -> bool {
        self.ships.index
            .get(&ship_id)
            .is_some_and(|&entity| !self.world.transit_state(entity).is_in_transit())
    }

    /// Append `JumpGateUsed` (and `StarSystemChanged` if the destination
    /// Sector belongs to a different Star System) for a Ship that just
    /// completed a Jump-Gate Transit (ADR-0009).
    ///
    /// Called from Step 7.5 on the destination node, after
    /// [`import_transit`](Self::import_transit) appends
    /// `SectorTransitCompleted` — `JumpGateUsed` records *how* the Ship
    /// moved, in addition to (not instead of) `SectorTransitCompleted`.
    pub fn append_jump_events(&mut self, ship_id: ShipId, gate_id: JumpGateId, from: SectorId, to: SectorId, entry_pos: Position) {
        self.event_store.append(DomainEvent::JumpGateUsed(JumpGateUsed {
            ship_id,
            gate_id,
            from_sector: from,
            to_sector  : to,
            entry_pos,
            tick       : self.current_tick,
        }));

        let from_system = self.sector_map.star_map.system_for_sector(from);
        let to_system   = self.sector_map.star_map.system_for_sector(to);
        if from_system != to_system {
            self.event_store.append(DomainEvent::StarSystemChanged(StarSystemChanged {
                ship_id,
                from_system,
                to_system,
                tick: self.current_tick,
            }));
        }
    }

    /// Complete an outgoing Sector Transit: remove the Ship from this node's
    /// ECS and return a snapshot for the destination node to import.
    ///
    /// Appends `SectorTransitCompleted` from this (the `from`) Sector's
    /// perspective. Returns `None` if `ship_id` is unknown or not currently
    /// `InTransit`.
    pub fn export_transit(&mut self, ship_id: ShipId, entry_pos: Position) -> Option<ShipSnapshot> {
        let &entity = self.ships.index.get(&ship_id)?;
        let to = match self.world.transit_state(entity) {
            TransitState::InTransit { to } => to,
            TransitState::None => return None,
        };

        let pos  = self.world.inner().get::<&PositionComp>(entity).ok()?.0;
        let vel  = self.world.inner().get::<&VelocityComp>(entity).ok()?.0;
        let (current_shield, current_armor, current_hull, is_destroyed) = {
            let hull = self.world.inner().get::<&HullComp>(entity).ok()?;
            (hull.current_shield, hull.current_armor, hull.current_hull, hull.is_destroyed)
        };
        let capacitor = self.world.inner().get::<&CapacitorComp>(entity).ok().map(|c| c.current);
        let fitting = self.world.inner().get::<&FittingComp>(entity)
            .map(|f| f.to_snapshot())
            .unwrap_or_else(|_| FittingSnapshot::empty());
        let ship_type_id = self.ships.type_ids.get(&ship_id).copied().unwrap_or(ShipTypeId(0));

        // Tackle state is not transferred on sector transit (tacklers are in
        // this sector; they lose the tackle as the ship leaves).
        let snapshot = ShipSnapshot {
            ship_id,
            ship_type_id,
            position: pos,
            velocity: vel,
            current_shield,
            current_armor,
            current_hull,
            is_destroyed,
            capacitor,
            fitting,
            tackled_by: Vec::new(),
        };

        self.ships.index.remove(&ship_id);
        self.world.despawn_ship(entity);
        self.ships.type_ids.remove(&ship_id);
        self.base_stats.remove(&ship_id);

        self.event_store.append(DomainEvent::SectorTransitCompleted(SectorTransitCompleted {
            ship_id,
            from     : self.sector_id,
            to,
            entry_pos,
            velocity : vel,
            tick     : self.current_tick,
        }));

        Some(snapshot)
    }

    /// Complete an incoming Sector Transit: restore `ship` (exported from the
    /// `from` Sector via [`export_transit`](Self::export_transit)) into this
    /// node's ECS at `entry_pos`, preserving its `ShipId` (INV-004 — no ID
    /// reuse, the same Ship simply changes Sector ownership).
    ///
    /// Appends `SectorTransitCompleted` from this (the `to`) Sector's
    /// perspective.
    pub fn import_transit(&mut self, ship: &ShipSnapshot, from: SectorId, entry_pos: Position) {
        let mut ship = ship.clone();
        ship.position = entry_pos;
        self.restore_ship_from_snapshot(&ship);

        self.event_store.append(DomainEvent::SectorTransitCompleted(SectorTransitCompleted {
            ship_id  : ship.ship_id,
            from,
            to       : self.sector_id,
            entry_pos,
            velocity : ship.velocity,
            tick     : self.current_tick,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::StateSnapshot;
    use dawn_core::{NodeId, SectorBounds, Velocity};
    use dawn_event_store::FileEventStore;

    fn mem_node() -> SimulationNode {
        SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        )
    }

    #[test]
    fn propose_transit_marks_ship_in_transit_and_appends_requested_event() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);

        node.propose_transit(TransitCommand { ship_id, to: SectorId(1) }).unwrap();

        let entity = *node.ships.index.get(&ship_id).unwrap();
        assert_eq!(node.world.transit_state(entity), TransitState::InTransit { to: SectorId(1) });

        let last = node.event_store().all_records().last().unwrap();
        match &last.event {
            DomainEvent::SectorTransitRequested(e) => {
                assert_eq!(e.ship_id, ship_id);
                assert_eq!(e.from, node.sector_id());
                assert_eq!(e.to, SectorId(1));
            }
            other => panic!("expected SectorTransitRequested, got {other:?}"),
        }
    }

    #[test]
    fn propose_transit_is_rejected_when_ship_is_already_in_transit() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        node.propose_transit(TransitCommand { ship_id, to: SectorId(1) }).unwrap();

        let err = node.propose_transit(TransitCommand { ship_id, to: SectorId(2) }).unwrap_err();
        assert!(matches!(err, DawnError::ShipInTransit(id) if id == ship_id));
    }

    #[test]
    fn propose_transit_is_rejected_for_unknown_ship() {
        let mut node = mem_node();
        let unknown = ShipId::new(NodeId(99), 0);
        let err = node.propose_transit(TransitCommand { ship_id: unknown, to: SectorId(1) }).unwrap_err();
        assert!(matches!(err, DawnError::ShipNotFound(id) if id == unknown));
    }

    #[test]
    fn export_transit_removes_ship_and_appends_completed_event() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::new(1.0, 0.0, 0.0));
        node.propose_transit(TransitCommand { ship_id, to: SectorId(1) }).unwrap();

        let entry_pos = Position::new(500.0, 0.0, 0.0);
        let snapshot = node.export_transit(ship_id, entry_pos).expect("ship should export");
        assert_eq!(snapshot.ship_id, ship_id);

        assert!(node.ships.index.get(&ship_id).is_none(), "ship must leave the from-sector ECS");
        assert_eq!(node.ship_count(), 0);

        let last = node.event_store().all_records().last().unwrap();
        match &last.event {
            DomainEvent::SectorTransitCompleted(e) => {
                assert_eq!(e.ship_id, ship_id);
                assert_eq!(e.from, node.sector_id());
                assert_eq!(e.to, SectorId(1));
                assert_eq!(e.entry_pos, entry_pos);
            }
            other => panic!("expected SectorTransitCompleted, got {other:?}"),
        }
    }

    #[test]
    fn export_transit_returns_none_for_ship_not_in_transit() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        assert!(node.export_transit(ship_id, Position::ORIGIN).is_none());
        assert_eq!(node.ship_count(), 1, "ship must remain when not in transit");
    }

    #[test]
    fn import_transit_restores_ship_with_same_id_at_entry_position_and_appends_completed_event() {
        let mut from_node = SimulationNode::new(NodeId(0), SectorId(0), SectorBounds::centered(SectorBounds::DEFAULT_HALF));
        let mut to_node   = SimulationNode::new(NodeId(1), SectorId(1), SectorBounds::centered(SectorBounds::DEFAULT_HALF));

        let ship_id = from_node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::new(1.0, 0.0, 0.0));
        from_node.propose_transit(TransitCommand { ship_id, to: SectorId(1) }).unwrap();

        let entry_pos = Position::new(500.0, 0.0, 0.0);
        let snapshot = from_node.export_transit(ship_id, entry_pos).unwrap();

        to_node.import_transit(&snapshot, SectorId(0), entry_pos);

        assert_eq!(to_node.ship_count(), 1);
        assert_eq!(to_node.get_ship_position(ship_id), Some(entry_pos));

        let last = to_node.event_store().all_records().last().unwrap();
        match &last.event {
            DomainEvent::SectorTransitCompleted(e) => {
                assert_eq!(e.ship_id, ship_id);
                assert_eq!(e.from, SectorId(0));
                assert_eq!(e.to, SectorId(1));
                assert_eq!(e.entry_pos, entry_pos);
            }
            other => panic!("expected SectorTransitCompleted, got {other:?}"),
        }
    }

    #[test]
    fn adopted_player_ship_accepts_owned_commands_on_the_destination_node() {
        let mut from_node = SimulationNode::new(NodeId(0), SectorId(0), SectorBounds::centered(SectorBounds::DEFAULT_HALF));
        let mut to_node   = SimulationNode::new(NodeId(1), SectorId(1), SectorBounds::centered(SectorBounds::DEFAULT_HALF));

        let player_id = from_node.next_player_id();
        let ship_id   = from_node.spawn_player_ship(player_id);
        from_node.propose_transit(TransitCommand { ship_id, to: SectorId(1) }).unwrap();
        let snapshot = from_node.export_transit(ship_id, Position::ORIGIN).unwrap();
        to_node.import_transit(&snapshot, SectorId(0), Position::ORIGIN);

        // Before the handoff, the destination node rejects owned commands.
        assert!(!to_node.apply_stop_command_owned(player_id, ship_id));

        assert!(to_node.adopt_player_ship(ship_id, player_id));
        assert!(to_node.apply_stop_command_owned(player_id, ship_id));
    }

    /// Normal-path Sector Transit: ownership ends up in exactly one Sector,
    /// and at no point do both Sectors hold the Ship at once (INV-003).
    #[test]
    fn transit_moves_ship_ownership_to_destination_sector_exactly_once() {
        let mut from_node = SimulationNode::new(NodeId(0), SectorId(0), SectorBounds::centered(SectorBounds::DEFAULT_HALF));
        let mut to_node   = SimulationNode::new(NodeId(1), SectorId(1), SectorBounds::centered(SectorBounds::DEFAULT_HALF));

        let ship_id = from_node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::new(1.0, 0.0, 0.0));
        assert_eq!(from_node.ship_count() + to_node.ship_count(), 1, "ship starts owned by exactly one sector");

        from_node.propose_transit(TransitCommand { ship_id, to: SectorId(1) }).unwrap();
        // Proposal alone does not move ownership yet.
        assert_eq!(from_node.ship_count() + to_node.ship_count(), 1);

        let entry_pos = Position::new(500.0, 0.0, 0.0);
        let snapshot = from_node.export_transit(ship_id, entry_pos).unwrap();

        // In flight: neither sector owns the ship (no split-brain double-ownership).
        assert_eq!(from_node.ship_count(), 0);
        assert_eq!(to_node.ship_count(), 0);

        to_node.import_transit(&snapshot, SectorId(0), entry_pos);

        // Final state: destination sector owns the ship, exactly once overall.
        assert_eq!(from_node.ship_count(), 0);
        assert_eq!(to_node.ship_count(), 1);
        assert_eq!(to_node.get_ship_position(ship_id), Some(entry_pos));
    }

    /// INV-002: after a Sector Transit, the destination Sector's state can be
    /// fully reproduced from a snapshot + Event Log replay (node restart).
    #[test]
    fn destination_sector_state_after_transit_is_fully_restored_from_snapshot_and_replay() {
        let dir        = tempfile::tempdir().unwrap();
        let event_path = dir.path().join("events.log");
        let snap_path  = dir.path().join("snapshot.bin");

        let mut from_node = SimulationNode::new(NodeId(0), SectorId(0), SectorBounds::centered(SectorBounds::DEFAULT_HALF));
        let ship_id = from_node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::new(1.0, 0.0, 0.0));
        from_node.propose_transit(TransitCommand { ship_id, to: SectorId(1) }).unwrap();
        let entry_pos = Position::new(500.0, 0.0, 0.0);
        let snapshot = from_node.export_transit(ship_id, entry_pos).unwrap();

        {
            let store = FileEventStore::open(&event_path).unwrap();
            let mut to_node = SimulationNode::with_store(
                NodeId(1), SectorId(1),
                SectorBounds::centered(SectorBounds::DEFAULT_HALF),
                store,
            );
            to_node.import_transit(&snapshot, SectorId(0), entry_pos);

            let snap = to_node.take_snapshot();
            snap.save(&snap_path).unwrap();
        } // node drops; FileEventStore flushes via BufWriter

        let snap   = StateSnapshot::load(&snap_path).unwrap();
        let store2 = FileEventStore::open(&event_path).unwrap();
        let restored = SimulationNode::restore_from(store2, &snap, &[], &[]);

        assert_eq!(restored.ship_count(), 1);
        assert_eq!(restored.get_ship_position(ship_id), Some(entry_pos));
    }

    /// ADR-0014 Task 9: measures the cost of a single Sector Transit
    /// (propose + export + import), excluding Raft commit latency.
    ///
    /// Ignored by default (it's a benchmark, not a correctness check).
    /// Run with: `cargo test -p dawn-simulation --release transit_latency_benchmark -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn transit_latency_benchmark() {
        use std::time::Instant;

        const ITERATIONS: u32 = 1_000;
        let mut total = std::time::Duration::ZERO;

        for i in 0..ITERATIONS {
            let mut from_node = SimulationNode::new(NodeId(0), SectorId(0), SectorBounds::centered(SectorBounds::DEFAULT_HALF));
            let mut to_node   = SimulationNode::new(NodeId(1), SectorId(1), SectorBounds::centered(SectorBounds::DEFAULT_HALF));
            let ship_id = from_node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::new(1.0, 0.0, 0.0));
            let entry_pos = Position::new(500.0, 0.0, 0.0);

            let start = Instant::now();
            from_node.propose_transit(TransitCommand { ship_id, to: SectorId(1) }).unwrap();
            let snapshot = from_node.export_transit(ship_id, entry_pos).unwrap();
            to_node.import_transit(&snapshot, SectorId(0), entry_pos);
            total += start.elapsed();

            let _ = i;
        }

        let avg = total / ITERATIONS;
        println!("transit (propose+export+import) avg over {ITERATIONS} iterations: {avg:?}");
    }
}
