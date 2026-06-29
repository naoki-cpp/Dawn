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
    components::{CapacitorComp, FittingComp, HullComp, InventoryComp, PositionComp, VelocityComp},
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
        let &entity = self
            .ships
            .index
            .get(&cmd.ship_id)
            .ok_or(DawnError::ShipNotFound(cmd.ship_id))?;

        if self.world.transit_state(entity).is_in_transit() {
            return Err(DawnError::ShipInTransit(cmd.ship_id));
        }

        self.world
            .set_transit_state(entity, TransitState::InTransit { to: cmd.to });

        self.event_store.append(DomainEvent::SectorTransitRequested(
            SectorTransitRequested {
                ship_id: cmd.ship_id,
                from: self.sector_id,
                to: cmd.to,
                tick: self.current_tick,
            },
        ));

        Ok(())
    }

    /// Whether a `TransitCommand` for `ship_id` would currently be accepted
    /// (Ship exists and is not already in transit). Used to reject commands
    /// up front, before proposing to the Raft Log (INV-006).
    pub fn can_propose_transit(&self, ship_id: ShipId) -> bool {
        self.ships
            .index
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
    pub fn append_jump_events(
        &mut self,
        ship_id: ShipId,
        gate_id: JumpGateId,
        from: SectorId,
        to: SectorId,
        entry_pos: Position,
    ) {
        self.event_store
            .append(DomainEvent::JumpGateUsed(JumpGateUsed {
                ship_id,
                gate_id,
                from_sector: from,
                to_sector: to,
                entry_pos,
                tick: self.current_tick,
            }));

        let from_system = self.sector_map.galaxy.system_for_sector(from);
        let to_system = self.sector_map.galaxy.system_for_sector(to);
        if from_system != to_system {
            self.event_store
                .append(DomainEvent::StarSystemChanged(StarSystemChanged {
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

        let pos = self.world.inner().get::<&PositionComp>(entity).ok()?.0;
        let vel = self.world.inner().get::<&VelocityComp>(entity).ok()?.0;
        let (current_shield, current_armor, current_hull, is_destroyed) = {
            let hull = self.world.inner().get::<&HullComp>(entity).ok()?;
            (
                hull.current_shield,
                hull.current_armor,
                hull.current_hull,
                hull.is_destroyed,
            )
        };
        let capacitor = self
            .world
            .inner()
            .get::<&CapacitorComp>(entity)
            .ok()
            .map(|c| c.current);
        let fitting = self
            .world
            .inner()
            .get::<&FittingComp>(entity)
            .map(|f| f.to_snapshot())
            .unwrap_or_else(|_| FittingSnapshot::empty());
        let ship_type_id = self
            .ships
            .type_ids
            .get(&ship_id)
            .copied()
            .unwrap_or(ShipTypeId(0));
        // Inventory must follow the ship across Sectors (ADR-0032) -- unlike
        // tackle, it's the pilot's possessions, not Sector-local state.
        let inventory = self
            .world
            .inner()
            .get::<&InventoryComp>(entity)
            .map(|inv| inv.items.clone())
            .unwrap_or_default();

        // Tackle state is not transferred on sector transit (tacklers are in
        // this sector; they lose the tackle as the ship leaves).
        let snapshot = ShipSnapshot {
            ship_id,
            ship_type_id,
            position: pos,
            // Placeholder only: `AnchorId(0)` is a real, specific anchor
            // (Helios, Sector 0's star — see `anchor.rs`), not a "this
            // Sector's star" sentinel, so it is only ever correct by
            // coincidence for a transit out of Sector 0. `import_transit`'s
            // `rebase_after_transit` overwrites both this and `position`
            // with the real destination anchor before anything reads them
            // (ADR-0029), so the value here never survives past restore.
            anchor: dawn_core::AnchorId(0),
            velocity: vel,
            current_shield,
            current_armor,
            current_hull,
            is_destroyed,
            capacitor,
            fitting,
            tackled_by: Vec::new(),
            inventory,
        };

        self.ships.index.remove(&ship_id);
        self.world.despawn_ship(entity);
        self.ships.type_ids.remove(&ship_id);
        self.base_stats.remove(&ship_id);

        self.event_store.append(DomainEvent::SectorTransitCompleted(
            SectorTransitCompleted {
                ship_id,
                from: self.sector_id,
                to,
                entry_pos,
                velocity: vel,
                tick: self.current_tick,
            },
        ));

        Some(snapshot)
    }

    /// Complete an incoming Sector Transit: restore `ship` (exported from the
    /// `from` Sector via [`export_transit`](Self::export_transit)) into this
    /// node's ECS at `entry_pos`, preserving its `ShipId` (INV-004 — no ID
    /// reuse, the same Ship simply changes Sector ownership).
    ///
    /// `restore_ship_from_snapshot` re-applies the Ship's *old* (source-Sector)
    /// anchor, and `entry_pos` alone is too coarse (one f32 ulp is ~16 km at
    /// true-AU magnitudes — far past a Gate's 2 km `activation_radius`) to use
    /// as a raw offset against it. `entry_pos_abs` is the precise f64
    /// Sector-frame arrival point (the destination Gate's `abs_m`, or the
    /// origin for a non-Gate Transit); `rebase_after_transit` re-anchors
    /// against it (appending the authoritative `AnchorRebased` event, ADR-0029)
    /// so the Ship can immediately jump back out (ADR-0009).
    ///
    /// Appends `SectorTransitCompleted` from this (the `to`) Sector's
    /// perspective.
    pub fn import_transit(
        &mut self,
        ship: &ShipSnapshot,
        from: SectorId,
        entry_pos: Position,
        entry_pos_abs: [f64; 3],
    ) {
        let mut ship = ship.clone();
        ship.position = entry_pos;
        self.restore_ship_from_snapshot(&ship);
        self.rebase_after_transit(ship.ship_id, entry_pos_abs);

        self.event_store.append(DomainEvent::SectorTransitCompleted(
            SectorTransitCompleted {
                ship_id: ship.ship_id,
                from,
                to: self.sector_id,
                entry_pos,
                velocity: ship.velocity,
                tick: self.current_tick,
            },
        ));
    }

    /// Re-anchor a Ship that just arrived in this Sector via Sector Transit
    /// to the nearest body anchor to `entry_pos_abs`, appending the
    /// authoritative `AnchorRebased` event (ADR-0029). No-op if the Ship or
    /// an anchor candidate in this Sector can't be found.
    ///
    /// Unlike `warp::rebase_arrival_event` (which uses an all-zero
    /// `[f64; 3]` as an "arrival not engaged yet" sentinel), `entry_pos_abs`
    /// here is always a deliberate absolute point — a Gate's `abs_m`, or the
    /// Sector origin for a non-Gate Transit — so there's no fallback-compose
    /// branch to skip.
    fn rebase_after_transit(&mut self, ship_id: ShipId, entry_pos_abs: [f64; 3]) {
        let Some(&entity) = self.ships.index.get(&ship_id) else {
            return;
        };
        let Some(to) = self
            .anchor_table
            .nearest_anchor(self.sector_id, entry_pos_abs)
        else {
            return;
        };
        let Some(to_abs) = self.anchor_table.abs(to) else {
            return;
        };
        let offset = Position::new(
            (entry_pos_abs[0] - to_abs[0]) as f32,
            (entry_pos_abs[1] - to_abs[1]) as f32,
            (entry_pos_abs[2] - to_abs[2]) as f32,
        );
        self.world.set_ship_anchor(entity, to);
        if let Ok(mut p) = self.world.inner_mut().get::<&mut PositionComp>(entity) {
            p.0 = offset;
        }
        self.event_store.append(DomainEvent::AnchorRebased(
            dawn_core::events::AnchorRebased {
                ship_id,
                anchor: to,
                offset,
                tick: self.current_tick,
            },
        ));
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

        node.propose_transit(TransitCommand {
            ship_id,
            to: SectorId(1),
        })
        .unwrap();

        let entity = *node.ships.index.get(&ship_id).unwrap();
        assert_eq!(
            node.world.transit_state(entity),
            TransitState::InTransit { to: SectorId(1) }
        );

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
        node.propose_transit(TransitCommand {
            ship_id,
            to: SectorId(1),
        })
        .unwrap();

        let err = node
            .propose_transit(TransitCommand {
                ship_id,
                to: SectorId(2),
            })
            .unwrap_err();
        assert!(matches!(err, DawnError::ShipInTransit(id) if id == ship_id));
    }

    #[test]
    fn propose_transit_is_rejected_for_unknown_ship() {
        let mut node = mem_node();
        let unknown = ShipId::new(NodeId(99), 0);
        let err = node
            .propose_transit(TransitCommand {
                ship_id: unknown,
                to: SectorId(1),
            })
            .unwrap_err();
        assert!(matches!(err, DawnError::ShipNotFound(id) if id == unknown));
    }

    #[test]
    fn export_transit_removes_ship_and_appends_completed_event() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(
            ShipTypeId(1),
            Position::ORIGIN,
            Velocity::new(1.0, 0.0, 0.0),
        );
        node.propose_transit(TransitCommand {
            ship_id,
            to: SectorId(1),
        })
        .unwrap();

        let entry_pos = Position::new(500.0, 0.0, 0.0);
        let snapshot = node
            .export_transit(ship_id, entry_pos)
            .expect("ship should export");
        assert_eq!(snapshot.ship_id, ship_id);

        assert!(
            !node.ships.index.contains_key(&ship_id),
            "ship must leave the from-sector ECS"
        );
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
        let mut from_node = SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        );
        let mut to_node = SimulationNode::new(
            NodeId(1),
            SectorId(1),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        );

        let ship_id = from_node.spawn_ship(
            ShipTypeId(1),
            Position::ORIGIN,
            Velocity::new(1.0, 0.0, 0.0),
        );
        from_node
            .propose_transit(TransitCommand {
                ship_id,
                to: SectorId(1),
            })
            .unwrap();

        let entry_pos = Position::new(500.0, 0.0, 0.0);
        let snapshot = from_node.export_transit(ship_id, entry_pos).unwrap();

        to_node.import_transit(
            &snapshot,
            SectorId(0),
            entry_pos,
            [entry_pos.x as f64, entry_pos.y as f64, entry_pos.z as f64],
        );

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

    /// Regression: a Ship that jumps through a Gate must land within the
    /// *return* Gate's `activation_radius`, so it can jump straight back.
    /// `entry_pos` alone (the f32 `JumpGateDef::position`) is too coarse to
    /// re-anchor against at true-AU magnitudes — `import_transit` must use
    /// the precise `entry_pos_abs` (the gate's `abs_m`) to set up the arriving
    /// Ship's anchor in the destination Sector (ADR-0029). Without that
    /// re-anchoring, the Ship keeps its *source*-Sector anchor and its
    /// absolute position computes to nonsense, so `can_propose_jump` for the
    /// return Gate (and every other Gate in the destination Sector) falsely
    /// fails.
    #[test]
    fn ship_arriving_through_a_gate_can_immediately_jump_back_through_the_return_gate() {
        let galaxy = crate::galaxy::Galaxy::demo();
        let mut from_node = SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        );
        let mut to_node = SimulationNode::new(
            NodeId(1),
            SectorId(1),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        );

        let return_gate = galaxy
            .gates_in_sector(SectorId(1))
            .into_iter()
            .find(|g| g.to_sector == SectorId(0))
            .expect("Sector 1 has a gate back to Sector 0");

        let ship_id = from_node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        from_node
            .propose_transit(TransitCommand {
                ship_id,
                to: SectorId(1),
            })
            .unwrap();

        // Mirrors `transit::apply_committed_raft_entries`'s Request handler:
        // arrive at the return gate's position so the player can jump
        // straight back (ADR-0009).
        let entry_pos = return_gate.position;
        let entry_pos_abs = return_gate.abs_m;
        let snapshot = from_node.export_transit(ship_id, entry_pos).unwrap();
        to_node.import_transit(&snapshot, SectorId(0), entry_pos, entry_pos_abs);

        assert!(
            to_node.can_propose_jump(ship_id, return_gate.id),
            "ship must land within the return gate's activation_radius, not just \
             at its f32-coarse `position` interpreted against the wrong anchor"
        );
    }

    #[test]
    fn inventory_survives_a_cross_sector_transit() {
        let mut from_node = SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        );
        let mut to_node = SimulationNode::new(
            NodeId(1),
            SectorId(1),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        );

        for def in crate::modules::all_modules() {
            from_node.register_module(def);
        }
        for def in crate::ship_types::all_ship_types() {
            from_node.register_ship_type(def);
        }
        let player_id = from_node.next_player_id();
        let ship_id = from_node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);
        let before_entity = *from_node.ships.index.get(&ship_id).unwrap();
        let before_len = from_node
            .world
            .inner()
            .get::<&dawn_ecs::components::InventoryComp>(before_entity)
            .unwrap()
            .items
            .len();
        assert!(before_len > 0, "player ships spawn with a seeded inventory");

        from_node
            .propose_transit(TransitCommand {
                ship_id,
                to: SectorId(1),
            })
            .unwrap();
        let entry_pos = Position::new(500.0, 0.0, 0.0);
        let snapshot = from_node.export_transit(ship_id, entry_pos).unwrap();
        to_node.import_transit(
            &snapshot,
            SectorId(0),
            entry_pos,
            [entry_pos.x as f64, entry_pos.y as f64, entry_pos.z as f64],
        );

        let after_entity = *to_node.ships.index.get(&ship_id).unwrap();
        let after = to_node
            .world
            .inner()
            .get::<&dawn_ecs::components::InventoryComp>(after_entity)
            .unwrap();
        assert_eq!(
            after.items.len(),
            before_len,
            "inventory must carry over the gate, unlike tackle state"
        );
    }

    #[test]
    fn adopted_player_ship_accepts_owned_commands_on_the_destination_node() {
        let mut from_node = SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        );
        let mut to_node = SimulationNode::new(
            NodeId(1),
            SectorId(1),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        );

        let player_id = from_node.next_player_id();
        let ship_id = from_node.spawn_player_ship(player_id);
        from_node
            .propose_transit(TransitCommand {
                ship_id,
                to: SectorId(1),
            })
            .unwrap();
        let snapshot = from_node.export_transit(ship_id, Position::ORIGIN).unwrap();
        to_node.import_transit(&snapshot, SectorId(0), Position::ORIGIN, [0.0, 0.0, 0.0]);

        // Before the handoff, the destination node rejects owned commands.
        assert!(!to_node.apply_stop_command_owned(player_id, ship_id));

        assert!(to_node.adopt_player_ship(ship_id, player_id));
        assert!(to_node.apply_stop_command_owned(player_id, ship_id));
    }

    /// Normal-path Sector Transit: ownership ends up in exactly one Sector,
    /// and at no point do both Sectors hold the Ship at once (INV-003).
    #[test]
    fn transit_moves_ship_ownership_to_destination_sector_exactly_once() {
        let mut from_node = SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        );
        let mut to_node = SimulationNode::new(
            NodeId(1),
            SectorId(1),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        );

        let ship_id = from_node.spawn_ship(
            ShipTypeId(1),
            Position::ORIGIN,
            Velocity::new(1.0, 0.0, 0.0),
        );
        assert_eq!(
            from_node.ship_count() + to_node.ship_count(),
            1,
            "ship starts owned by exactly one sector"
        );

        from_node
            .propose_transit(TransitCommand {
                ship_id,
                to: SectorId(1),
            })
            .unwrap();
        // Proposal alone does not move ownership yet.
        assert_eq!(from_node.ship_count() + to_node.ship_count(), 1);

        let entry_pos = Position::new(500.0, 0.0, 0.0);
        let snapshot = from_node.export_transit(ship_id, entry_pos).unwrap();

        // In flight: neither sector owns the ship (no split-brain double-ownership).
        assert_eq!(from_node.ship_count(), 0);
        assert_eq!(to_node.ship_count(), 0);

        to_node.import_transit(
            &snapshot,
            SectorId(0),
            entry_pos,
            [entry_pos.x as f64, entry_pos.y as f64, entry_pos.z as f64],
        );

        // Final state: destination sector owns the ship, exactly once overall.
        assert_eq!(from_node.ship_count(), 0);
        assert_eq!(to_node.ship_count(), 1);
        assert_eq!(to_node.get_ship_position(ship_id), Some(entry_pos));
    }

    /// INV-002: after a Sector Transit, the destination Sector's state can be
    /// fully reproduced from a snapshot + Event Log replay (node restart).
    #[test]
    fn destination_sector_state_after_transit_is_fully_restored_from_snapshot_and_replay() {
        let dir = tempfile::tempdir().unwrap();
        let event_path = dir.path().join("events.log");
        let snap_path = dir.path().join("snapshot.bin");

        let mut from_node = SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        );
        let ship_id = from_node.spawn_ship(
            ShipTypeId(1),
            Position::ORIGIN,
            Velocity::new(1.0, 0.0, 0.0),
        );
        from_node
            .propose_transit(TransitCommand {
                ship_id,
                to: SectorId(1),
            })
            .unwrap();
        let entry_pos = Position::new(500.0, 0.0, 0.0);
        let snapshot = from_node.export_transit(ship_id, entry_pos).unwrap();

        {
            let store = FileEventStore::open(&event_path).unwrap();
            let mut to_node = SimulationNode::with_store(
                NodeId(1),
                SectorId(1),
                SectorBounds::centered(SectorBounds::DEFAULT_HALF),
                store,
            );
            to_node.import_transit(
                &snapshot,
                SectorId(0),
                entry_pos,
                [entry_pos.x as f64, entry_pos.y as f64, entry_pos.z as f64],
            );

            let snap = to_node.take_snapshot();
            snap.save(&snap_path).unwrap();
        } // node drops; FileEventStore flushes via BufWriter

        let snap = StateSnapshot::load(&snap_path).unwrap();
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
            let mut from_node = SimulationNode::new(
                NodeId(0),
                SectorId(0),
                SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            );
            let mut to_node = SimulationNode::new(
                NodeId(1),
                SectorId(1),
                SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            );
            let ship_id = from_node.spawn_ship(
                ShipTypeId(1),
                Position::ORIGIN,
                Velocity::new(1.0, 0.0, 0.0),
            );
            let entry_pos = Position::new(500.0, 0.0, 0.0);

            let start = Instant::now();
            from_node
                .propose_transit(TransitCommand {
                    ship_id,
                    to: SectorId(1),
                })
                .unwrap();
            let snapshot = from_node.export_transit(ship_id, entry_pos).unwrap();
            to_node.import_transit(
                &snapshot,
                SectorId(0),
                entry_pos,
                [entry_pos.x as f64, entry_pos.y as f64, entry_pos.z as f64],
            );
            total += start.elapsed();

            let _ = i;
        }

        let avg = total / ITERATIONS;
        println!("transit (propose+export+import) avg over {ITERATIONS} iterations: {avg:?}");
    }
}
