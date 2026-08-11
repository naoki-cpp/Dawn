use crate::node::SimulationNode;
use dawn_core::{
    commands::TransitCommand, DawnError, DomainEvent, Position, SectorId, ShipId, ShipTypeId,
    TransitHandoffState,
};
use dawn_ecs::{components::VelocityComp, TransitState};

use crate::persistence::StateSnapshot;
use dawn_core::{NodeId, SectorBounds, Tick, Velocity};
use dawn_storage::{EventStore, FileEventStore, InMemoryEventStore};

fn mem_node() -> SimulationNode {
    SimulationNode::new_test(
        NodeId(0),
        SectorId(0),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
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

    let entity = *node.simulation.ships.index.get(&ship_id).unwrap();
    assert_eq!(
        node.simulation.world.transit_state(entity),
        TransitState::InTransit { to: SectorId(1) }
    );

    let last = node.pending_events().last().unwrap();
    match last {
        DomainEvent::SectorTransitRequested(e) => {
            assert_eq!(e.ship_id, ship_id);
            assert_eq!(e.from, node.sector_id());
            assert_eq!(e.to, SectorId(1));
        }
        other => panic!("expected SectorTransitRequested, got {other:?}"),
    }
}

#[test]
fn prepare_transit_commit_rolls_back_when_handoff_snapshot_is_incomplete() {
    let mut node = mem_node();
    let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
    let entity = *node.simulation.ships.index.get(&ship_id).unwrap();
    let _ = node
        .simulation
        .world
        .remove_one::<VelocityComp>(entity)
        .unwrap();
    let event_count = node.total_event_count();

    assert!(node
        .prepare_transit_commit(ship_id, SectorId(1), None)
        .is_none());
    assert_eq!(
        node.simulation.world.transit_state(entity),
        TransitState::None
    );
    assert_eq!(node.total_event_count(), event_count);
    assert!(node.can_propose_transit(ship_id));
    assert!(!node
        .pending_events()
        .iter()
        .any(|event| matches!(event, DomainEvent::SectorTransitRequested(_))));
}

#[test]
fn transit_attempt_counter_exhaustion_does_not_freeze_the_ship() {
    let mut node = mem_node();
    let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
    let entity = *node.simulation.ships.index.get(&ship_id).unwrap();
    node.transit.transit_attempt_counter = u64::MAX;
    let event_count = node.total_event_count();

    assert!(node
        .prepare_transit_commit(ship_id, SectorId(1), None)
        .is_none());
    assert_eq!(
        node.simulation.world.transit_state(entity),
        TransitState::None
    );
    assert_eq!(node.total_event_count(), event_count);
    assert!(node.can_propose_transit(ship_id));
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
fn export_transit_handoff_without_removing_the_ship_or_appending_an_event() {
    // Issue #204: export no longer removes the Ship or appends
    // SectorTransitCompleted -- that used to happen here, durably, before
    // the destination's TransitOp::Commit had even been proposed to Raft.
    // A crash in that window could lose the Ship (source's log said it
    // left, destination's log had nothing). Now the Ship stays put,
    // frozen (InTransit), until a matching Ack is committed and
    // complete_outgoing_transit finalizes the source half.
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

    let snapshot = node.export_transit(ship_id).expect("ship should export");
    assert_eq!(snapshot.ship_id, ship_id);

    assert!(
        node.simulation.ships.index.contains_key(&ship_id),
        "the ship must stay in this Sector's ECS until complete_outgoing_transit"
    );
    assert!(
        !node
            .pending_events()
            .iter()
            .any(|event| matches!(event, DomainEvent::SectorTransitCompleted(_))),
        "export alone must not append SectorTransitCompleted"
    );
}

#[test]
fn complete_outgoing_transit_removes_ship_and_appends_completed_event() {
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
    let snapshot = node.export_transit(ship_id).unwrap();
    let entity = *node.simulation.ships.index.get(&ship_id).unwrap();
    node.simulation
        .world
        .get_mut::<VelocityComp>(entity)
        .expect("transit ship must retain its velocity component")
        .0 = Velocity::new(99.0, 0.0, 0.0);

    let entry_pos = dawn_core::AbsolutePosition::new(500.0, 0.0, 0.0);
    node.complete_outgoing_transit(snapshot.ship_id, SectorId(1), entry_pos, Tick::ZERO);

    assert!(
        !node.simulation.ships.index.contains_key(&ship_id),
        "ship must leave the from-sector ECS"
    );
    assert_eq!(node.ship_count(), 0);

    let last = node.pending_events().last().unwrap();
    match last {
        DomainEvent::SectorTransitCompleted(e) => {
            assert_eq!(e.handoff.ship_id, ship_id);
            assert_eq!(e.handoff, snapshot, "Ack cleanup must use Saga handoff");
            assert_eq!(e.from, node.sector_id());
            assert_eq!(e.to, SectorId(1));
            assert_eq!(e.entry_pos, entry_pos);
        }
        other => panic!("expected SectorTransitCompleted, got {other:?}"),
    }
}

#[test]
fn complete_outgoing_transit_is_idempotent() {
    let mut node = mem_node();
    let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
    node.propose_transit(TransitCommand {
        ship_id,
        to: SectorId(1),
    })
    .unwrap();
    let snapshot = node.export_transit(ship_id).unwrap();
    let entry_pos = dawn_core::AbsolutePosition::ORIGIN;

    node.complete_outgoing_transit(snapshot.ship_id, SectorId(1), entry_pos, Tick::ZERO);
    node.complete_outgoing_transit(snapshot.ship_id, SectorId(1), entry_pos, Tick::ZERO);

    let completed_count = node
        .pending_events()
        .iter()
        .filter(|event| matches!(event, DomainEvent::SectorTransitCompleted(_)))
        .count();
    assert_eq!(
        completed_count, 1,
        "a repeated Commit observation must not double-append"
    );
}

#[test]
fn export_transit_clears_ownership_maps_for_a_player_ship() {
    // Regression test (architecture review 2026-07-03): export_transit
    // used to hand-roll index/type_ids/base_stats removal and forgot
    // owners/active_ship, leaving a dangling ownership entry for a
    // transited player ship. Now routed through ShipRegistry::remove
    // via SimulationNode::remove_ship (called by complete_outgoing_transit,
    // issue #204), which clears all four maps.
    let mut node = mem_node();
    let player_id = node.next_player_id();
    let ship_id = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);
    node.propose_transit(TransitCommand {
        ship_id,
        to: SectorId(1),
    })
    .unwrap();
    let snapshot = node.export_transit(ship_id).expect("ship should export");

    node.complete_outgoing_transit(
        snapshot.ship_id,
        SectorId(1),
        dawn_core::AbsolutePosition::new(500.0, 0.0, 0.0),
        Tick::ZERO,
    );

    assert!(
        !node.owns_ship(player_id, ship_id),
        "owners map must not retain a dangling entry after transit"
    );
    assert!(
        !node.players.owners.contains_key(&ship_id),
        "owners map must be cleared"
    );
    assert!(
        !node.players.active_ship.contains_key(&player_id),
        "active_ship map must be cleared"
    );
}

#[test]
fn export_transit_returns_none_for_ship_not_in_transit() {
    let mut node = mem_node();
    let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
    assert!(node.export_transit(ship_id).is_none());
    assert_eq!(node.ship_count(), 1, "ship must remain when not in transit");
}

#[test]
fn import_transit_restores_ship_with_same_id_at_entry_position_and_appends_completed_event() {
    let mut from_node = SimulationNode::new_test(
        NodeId(0),
        SectorId(0),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
    );
    let mut to_node = SimulationNode::new_test(
        NodeId(1),
        SectorId(1),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
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
    let snapshot = from_node.export_transit(ship_id).unwrap();

    to_node.import_transit(&snapshot, SectorId(0), entry_pos.into(), Tick::ZERO);

    assert_eq!(to_node.ship_count(), 1);
    assert_eq!(to_node.get_ship_position(ship_id), Some(entry_pos));

    let last = to_node.pending_events().last().unwrap();
    match last {
        DomainEvent::SectorTransitCompleted(e) => {
            assert_eq!(e.handoff.ship_id, ship_id);
            assert_eq!(e.from, SectorId(0));
            assert_eq!(e.to, SectorId(1));
            assert_eq!(e.entry_pos, entry_pos.into());
        }
        other => panic!("expected SectorTransitCompleted, got {other:?}"),
    }
}

/// Regression: a Ship that jumps through a Gate must land within the
/// *return* Gate's `activation_radius`, so it can jump straight back.
/// `entry_pos` alone is not sufficient to re-anchor against the destination
/// body — `import_transit` must use
/// the precise `entry_pos` (the gate's `abs_m`) to set up the arriving
/// Ship's anchor in the destination Sector (ADR-0029). Without that
/// re-anchoring, the Ship keeps its *source*-Sector anchor and its
/// absolute position computes to nonsense, so `can_propose_jump` for the
/// return Gate (and every other Gate in the destination Sector) falsely
/// fails.
#[test]
fn ship_arriving_through_a_gate_can_immediately_jump_back_through_the_return_gate() {
    let galaxy = crate::galaxy::Galaxy::demo();
    let mut from_node = SimulationNode::new_test(
        NodeId(0),
        SectorId(0),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
    );
    let mut to_node = SimulationNode::new_test(
        NodeId(1),
        SectorId(1),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
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
    let entry_pos = return_gate.abs_m;
    let snapshot = from_node.export_transit(ship_id).unwrap();
    to_node.import_transit(&snapshot, SectorId(0), entry_pos, Tick::ZERO);

    assert!(
        to_node.can_propose_jump(ship_id, return_gate.id),
        "ship must land within the return gate's activation_radius, not just \
         at its `position` interpreted against the wrong anchor"
    );
}

/// Same regression as above, but through the consolidated
/// `prepare_transit_commit`/`handle_transit_commit` pair instead of the
/// individual primitives — exercises the Gate-lookup/entry-point logic
/// those two methods now own, mirroring exactly what
/// `transit::apply_committed_raft_entries` calls in production.
#[test]
fn the_consolidated_request_commit_pair_reproduces_the_same_arrival() {
    let mut from_node = SimulationNode::new_test(
        NodeId(0),
        SectorId(0),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
    );
    let mut to_node = SimulationNode::new_test(
        NodeId(1),
        SectorId(1),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
    );

    let outbound_gate = crate::galaxy::Galaxy::demo()
        .gates_in_sector(SectorId(0))
        .into_iter()
        .find(|g| g.to_sector == SectorId(1))
        .expect("Sector 0 has a gate to Sector 1");
    let return_gate = crate::galaxy::Galaxy::demo()
        .gates_in_sector(SectorId(1))
        .into_iter()
        .find(|g| g.to_sector == SectorId(0))
        .expect("Sector 1 has a gate back to Sector 0");

    let ship_id = from_node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);

    let data = from_node
        .prepare_transit_commit(ship_id, SectorId(1), Some(outbound_gate.id))
        .expect("transit must be accepted and the ship exported");
    assert_eq!(
        data.entry_pos, return_gate.abs_m,
        "the arrival point must be the return gate's precise abs_m, not Sector 0's"
    );

    to_node.handle_transit_commit(
        &data.handoff,
        SectorId(0),
        data.entry_pos,
        Some(outbound_gate.id),
        data.request_tick,
    );

    assert!(
        to_node.can_propose_jump(ship_id, return_gate.id),
        "the consolidated pair must reproduce the same anchor-fix as the primitives"
    );
    let records = to_node.pending_events();
    let jump_used = records
        .iter()
        .find_map(|event| match event {
            DomainEvent::JumpGateUsed(e) => Some(e),
            _ => None,
        })
        .expect("handle_transit_commit must append JumpGateUsed");
    assert_eq!(jump_used.ship_id, ship_id);
    assert_eq!(jump_used.gate_id, outbound_gate.id);
    assert!(
        records
            .iter()
            .any(|event| matches!(event, DomainEvent::StarSystemChanged(_))),
        "Sector 0 (Alpha) and Sector 1 (Beta) are different Star Systems, \
         so StarSystemChanged must also be appended"
    );
}

#[test]
fn inventory_survives_a_cross_sector_transit() {
    let mut from_node = SimulationNode::new_test(
        NodeId(0),
        SectorId(0),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
    );
    let mut to_node = SimulationNode::new_test(
        NodeId(1),
        SectorId(1),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
    );

    let player_id = from_node.next_player_id();
    let ship_id = from_node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);
    let before_entity = *from_node.simulation.ships.index.get(&ship_id).unwrap();
    let before_len = from_node
        .simulation
        .world
        .get::<dawn_ecs::components::InventoryComp>(before_entity)
        .unwrap()
        .items
        .values()
        .copied()
        .sum::<u64>();
    assert!(before_len > 0, "player ships spawn with a seeded inventory");

    from_node
        .propose_transit(TransitCommand {
            ship_id,
            to: SectorId(1),
        })
        .unwrap();
    let entry_pos = Position::new(500.0, 0.0, 0.0);
    let snapshot = from_node.export_transit(ship_id).unwrap();
    to_node.import_transit(&snapshot, SectorId(0), entry_pos.into(), Tick::ZERO);

    let after_entity = *to_node.simulation.ships.index.get(&ship_id).unwrap();
    let after = to_node
        .simulation
        .world
        .get::<dawn_ecs::components::InventoryComp>(after_entity)
        .unwrap();
    assert_eq!(
        after.items.values().copied().sum::<u64>(),
        before_len,
        "inventory must carry over the gate, unlike tackle state"
    );
}

#[test]
fn adopted_player_ship_accepts_owned_commands_on_the_destination_node() {
    let mut from_node = SimulationNode::new_test(
        NodeId(0),
        SectorId(0),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
    );
    let mut to_node = SimulationNode::new_test(
        NodeId(1),
        SectorId(1),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
    );

    let player_id = from_node.next_player_id();
    let ship_id = from_node.spawn_player_ship(player_id);
    from_node
        .propose_transit(TransitCommand {
            ship_id,
            to: SectorId(1),
        })
        .unwrap();
    let snapshot = from_node.export_transit(ship_id).unwrap();
    to_node.import_transit(
        &snapshot,
        SectorId(0),
        dawn_core::AbsolutePosition::ORIGIN,
        Tick::ZERO,
    );

    // The durable handoff establishes ownership before any client resume.
    assert!(to_node.apply_stop_command_owned(player_id, ship_id));

    assert!(to_node.adopt_player_ship(ship_id, player_id));
    assert!(to_node.apply_stop_command_owned(player_id, ship_id));
}

/// Normal-path Sector Transit: ownership ends up in exactly one Sector,
/// and at no point do both Sectors hold the Ship at once (INV-003).
#[test]
fn transit_moves_ship_ownership_to_destination_sector_exactly_once() {
    // Issue #204 strengthened this invariant: ownership now stays with
    // exactly one Sector for the *entire* Transit, never dropping to zero
    // in between. Before, `export_transit` removed the ship immediately
    // at Request-commit time, so there was a real window (until the
    // destination's Commit landed) where the sum below was 0 -- which is
    // exactly the crash-loses-the-ship window this issue closed. Now the
    // source keeps the ship (frozen out of Movement/Combat) until it
    // observes its own Commit, so the sum is always 1.
    let mut from_node = SimulationNode::new_test(
        NodeId(0),
        SectorId(0),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
    );
    let mut to_node = SimulationNode::new_test(
        NodeId(1),
        SectorId(1),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
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
    let snapshot = from_node.export_transit(ship_id).unwrap();

    // Exporting a snapshot for the Commit proposal does not move
    // ownership either -- the ship is still durably owned by `from`,
    // just frozen (`TransitState::InTransit`) until the Commit lands.
    assert_eq!(
        from_node.ship_count() + to_node.ship_count(),
        1,
        "export must not create a window where neither Sector owns the ship"
    );
    assert_eq!(from_node.ship_count(), 1);
    assert_eq!(to_node.ship_count(), 0);

    to_node.import_transit(&snapshot, SectorId(0), entry_pos.into(), Tick::ZERO);
    from_node.complete_outgoing_transit(
        snapshot.ship_id,
        SectorId(1),
        entry_pos.into(),
        Tick::ZERO,
    );

    // Final state: destination sector owns the ship, exactly once overall.
    assert_eq!(from_node.ship_count(), 0);
    assert_eq!(to_node.ship_count(), 1);
    assert_eq!(to_node.get_ship_position(ship_id), Some(entry_pos));
}

/// Legacy recovery regression: after a Sector Transit, the destination
/// Sector's state can be reproduced by the current snapshot + Event Log
/// replay path. ADR-0049 replaces this operational mechanism with a
/// versioned checkpoint plus authoritative RecoveryDelta tail; the test
/// remains while that migration is implemented.
#[test]
fn destination_sector_state_after_transit_is_fully_restored_from_snapshot_and_replay() {
    let dir = tempfile::tempdir().unwrap();
    let event_path = dir.path().join("events.log");
    let snap_path = dir.path().join("snapshot.bin");

    let mut from_node = SimulationNode::new_test(
        NodeId(0),
        SectorId(0),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
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
    let snapshot = from_node.export_transit(ship_id).unwrap();

    {
        let store = FileEventStore::open(&event_path).unwrap();
        let mut to_node = SimulationNode::with_test_store(
            NodeId(1),
            SectorId(1),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            store,
        );
        to_node.import_transit(&snapshot, SectorId(0), entry_pos.into(), Tick::ZERO);

        let snap = to_node.take_snapshot();
        snap.save(&snap_path).unwrap();
    } // node drops; FileEventStore flushes via BufWriter

    let snap = StateSnapshot::load(&snap_path).unwrap();
    let store2 = FileEventStore::open(&event_path).unwrap();
    let restored = SimulationNode::restore_from_test(
        store2,
        &snap,
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        &[],
        &[],
    );

    assert_eq!(restored.ship_count(), 1);
    assert_eq!(restored.get_ship_position(ship_id), Some(entry_pos));
}

/// ADR-0014 Task 9: measures the cost of a single Sector Transit
/// (propose + export + import), excluding Raft commit latency.
///
/// Ignored by default (it's a benchmark, not a correctness check).
/// Run with: `cargo test -p dawn-server --release transit_latency_benchmark -- --ignored --nocapture`
#[test]
#[ignore]
fn transit_latency_benchmark() {
    use std::time::Instant;

    const ITERATIONS: u32 = 1_000;
    let mut total = std::time::Duration::ZERO;

    for i in 0..ITERATIONS {
        let mut from_node = SimulationNode::new_test(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        );
        let mut to_node = SimulationNode::new_test(
            NodeId(1),
            SectorId(1),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
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
        let snapshot = from_node.export_transit(ship_id).unwrap();
        to_node.import_transit(&snapshot, SectorId(0), entry_pos.into(), Tick::ZERO);
        total += start.elapsed();

        let _ = i;
    }

    let avg = total / ITERATIONS;
    println!("transit (propose+export+import) avg over {ITERATIONS} iterations: {avg:?}");
}

// ── Snapshot + tail replay recovery (issue #204) ────────────────────────

#[test]
fn replaying_requested_marks_the_ship_in_transit() {
    let mut node = mem_node();
    let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);

    node.apply_event_pub(DomainEvent::SectorTransitRequested(
        dawn_core::events::SectorTransitRequested {
            ship_id,
            from: SectorId(0),
            to: SectorId(1),
            request_tick: Tick(1),
            gate_id: None,
            entry_pos: dawn_core::AbsolutePosition::ORIGIN,
            tick: Tick(1),
        },
    ));

    let entity = *node.simulation.ships.index.get(&ship_id).unwrap();
    assert_eq!(
        node.simulation.world.transit_state(entity),
        TransitState::InTransit { to: SectorId(1) }
    );
}

#[test]
fn replaying_an_incoming_request_marker_does_not_freeze_the_destination_ship() {
    let mut node = SimulationNode::new_test(
        NodeId(1),
        SectorId(1),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
    );
    let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
    node.apply_event_pub(DomainEvent::SectorTransitRequested(
        dawn_core::events::SectorTransitRequested {
            ship_id,
            from: SectorId(0),
            to: SectorId(1),
            request_tick: Tick(7),
            gate_id: None,
            entry_pos: dawn_core::AbsolutePosition::ORIGIN,
            tick: Tick(3),
        },
    ));

    assert!(node.can_propose_transit(ship_id));
}

#[test]
fn replaying_aborted_clears_the_in_transit_marker() {
    let mut node = mem_node();
    let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
    node.apply_event_pub(DomainEvent::SectorTransitRequested(
        dawn_core::events::SectorTransitRequested {
            ship_id,
            from: SectorId(0),
            to: SectorId(1),
            request_tick: Tick(1),
            gate_id: None,
            entry_pos: dawn_core::AbsolutePosition::ORIGIN,
            tick: Tick(1),
        },
    ));

    node.apply_event_pub(DomainEvent::SectorTransitAborted(
        dawn_core::events::SectorTransitAborted {
            ship_id,
            from: SectorId(0),
            to: SectorId(1),
            tick: Tick(2),
        },
    ));

    let entity = *node.simulation.ships.index.get(&ship_id).unwrap();
    assert_eq!(
        node.simulation.world.transit_state(entity),
        TransitState::None
    );
}

#[test]
fn replaying_stale_abort_for_an_old_route_keeps_the_current_marker() {
    let mut node = mem_node();
    let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
    for event in [
        DomainEvent::SectorTransitRequested(dawn_core::events::SectorTransitRequested {
            ship_id,
            from: SectorId(0),
            to: SectorId(1),
            request_tick: Tick(1),
            gate_id: None,
            entry_pos: dawn_core::AbsolutePosition::ORIGIN,
            tick: Tick(1),
        }),
        DomainEvent::SectorTransitRequested(dawn_core::events::SectorTransitRequested {
            ship_id,
            from: SectorId(0),
            to: SectorId(2),
            request_tick: Tick(2),
            gate_id: None,
            entry_pos: dawn_core::AbsolutePosition::ORIGIN,
            tick: Tick(2),
        }),
        DomainEvent::SectorTransitAborted(dawn_core::events::SectorTransitAborted {
            ship_id,
            from: SectorId(0),
            to: SectorId(1),
            tick: Tick(3),
        }),
    ] {
        node.apply_event_pub(event);
    }

    let entity = *node.simulation.ships.index.get(&ship_id).unwrap();
    assert_eq!(
        node.simulation.world.transit_state(entity),
        TransitState::InTransit { to: SectorId(2) }
    );
}

#[test]
fn replaying_completed_on_the_source_sector_removes_the_ship() {
    let mut node = mem_node();
    let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);

    node.apply_event_pub(DomainEvent::SectorTransitCompleted(
        dawn_core::events::SectorTransitCompleted {
            handoff: sample_transit_handoff(ship_id, Velocity::ZERO),
            from: SectorId(0), // matches node.sector_id() -- this is the source
            to: SectorId(1),
            request_tick: dawn_core::Tick::ZERO,
            entry_pos: dawn_core::AbsolutePosition::ORIGIN,
            tick: Tick(1),
        },
    ));

    assert!(
        !node.simulation.ships.index.contains_key(&ship_id),
        "a Sector replaying its own SectorTransitCompleted as the source \
         must not resurrect the ship it exported"
    );
}

#[test]
fn replaying_completed_on_the_destination_sector_materializes_the_ship() {
    let mut node = SimulationNode::new_test(
        NodeId(1),
        SectorId(1), // matches `to` below -- this is the destination
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
    );
    let ship_id = ShipId::new(NodeId(0), 7);

    node.apply_event_pub(DomainEvent::SectorTransitCompleted(
        dawn_core::events::SectorTransitCompleted {
            handoff: sample_transit_handoff(ship_id, Velocity::new(1.0, 0.0, 0.0)),
            from: SectorId(0),
            to: SectorId(1),
            request_tick: dawn_core::Tick::ZERO,
            entry_pos: dawn_core::AbsolutePosition::new(500.0, 0.0, 0.0),
            tick: Tick(1),
        },
    ));

    assert!(
        node.simulation.ships.index.contains_key(&ship_id),
        "a Sector replaying SectorTransitCompleted as the destination \
         must materialize the imported ship from the event alone"
    );
    let entity = *node.simulation.ships.index.get(&ship_id).unwrap();
    assert_eq!(
        node.simulation.world.get::<VelocityComp>(entity).unwrap().0,
        Velocity::new(1.0, 0.0, 0.0)
    );
}

#[test]
fn replaying_completed_on_the_destination_sector_is_idempotent() {
    // Guards against a double-materialize if the event were ever
    // replayed twice (e.g. a snapshot taken mid-tail-replay in a future
    // refactor) -- mirrors the `!contains_key` guard every other
    // ship-materializing replay arm already has (ShipSpawned, etc).
    let mut node = SimulationNode::new_test(
        NodeId(1),
        SectorId(1),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
    );
    let ship_id = ShipId::new(NodeId(0), 7);
    let event = DomainEvent::SectorTransitCompleted(dawn_core::events::SectorTransitCompleted {
        handoff: sample_transit_handoff(ship_id, Velocity::ZERO),
        from: SectorId(0),
        to: SectorId(1),
        request_tick: dawn_core::Tick::ZERO,
        entry_pos: dawn_core::AbsolutePosition::new(500.0, 0.0, 0.0),
        tick: Tick(1),
    });

    node.apply_event_pub(event.clone());
    node.apply_event_pub(event);

    assert_eq!(node.ship_count(), 1);
}

fn sample_transit_handoff(ship_id: ShipId, velocity: Velocity) -> TransitHandoffState {
    TransitHandoffState {
        ship_id,
        owner_player_id: None,
        resume_ticket: None,
        pending_resume_ticket: None,
        ship_type_id: ShipTypeId(1),
        velocity,
        current_shield: 80.0,
        current_armor: 90.0,
        current_hull: 100.0,
        is_destroyed: false,
        capacitor: Some(40.0),
        fitting: dawn_core::fitting::FittingSnapshot::empty(),
        inventory: std::collections::BTreeMap::new(),
    }
}

/// The end-to-end acceptance test from issue #204: a completed
/// cross-Sector Transit must survive a simulated restart of *both*
/// Sectors, and ownership must land on exactly one of them afterward --
/// never both (a resurrected source ship) and never neither (a lost
/// import).
#[test]
fn a_completed_transit_survives_snapshot_plus_tail_replay_on_both_sectors() {
    let mut from_node = SimulationNode::new_test(
        NodeId(0),
        SectorId(0),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
    );
    let mut to_node = SimulationNode::new_test(
        NodeId(1),
        SectorId(1),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
    );

    // Snapshot both Sectors *before* the transit -- the ship only exists
    // in the post-snapshot tail, on both sides, the same shape as issue
    // #197's regression.
    let ship_id = from_node.spawn_ship(
        ShipTypeId(1),
        Position::ORIGIN,
        Velocity::new(1.0, 0.0, 0.0),
    );
    let from_snapshot_before = from_node.take_snapshot();
    let to_snapshot_before = to_node.take_snapshot();

    from_node
        .propose_transit(TransitCommand {
            ship_id,
            to: SectorId(1),
        })
        .unwrap();
    let entry_offset = Position::new(500.0, 0.0, 0.0);
    let entry_pos = dawn_core::AbsolutePosition::from(entry_offset);
    let exported = from_node.export_transit(ship_id).unwrap();
    to_node.import_transit(&exported, SectorId(0), entry_pos, Tick::ZERO);
    // The durability fix (issue #204) this test targets: `from_node` only
    // removes the ship and records SectorTransitCompleted once it
    // observes the *same* Commit the destination acted on -- mirroring
    // `transit::apply_committed_raft_entries`'s `from == node.sector_id()`
    // branch, not the old immediate removal at export time.
    from_node.complete_outgoing_transit(exported.ship_id, SectorId(1), entry_pos, Tick::ZERO);

    // Simulate a restart of both Sectors: snapshot + tail-log replay,
    // exactly as `restore_from` is used in production recovery.
    let mut from_store2 = InMemoryEventStore::new();
    for event in from_node.pending_events() {
        from_store2.append(event.clone());
    }
    let restored_from = SimulationNode::restore_from_test(
        from_store2,
        &from_snapshot_before,
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        &[],
        &[],
    );

    let mut to_store2 = InMemoryEventStore::new();
    for event in to_node.pending_events() {
        to_store2.append(event.clone());
    }
    let restored_to = SimulationNode::restore_from_test(
        to_store2,
        &to_snapshot_before,
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        &[],
        &[],
    );

    let owned_by_source = restored_from.simulation.ships.index.contains_key(&ship_id);
    let owned_by_destination = restored_to.simulation.ships.index.contains_key(&ship_id);
    assert!(
        !owned_by_source,
        "the source Sector must not resurrect a ship it transferred away"
    );
    assert!(
        owned_by_destination,
        "the destination Sector must reconstruct the imported ship from \
         its own tail log alone"
    );
    assert_ne!(
        owned_by_source, owned_by_destination,
        "ownership must exist on exactly one Sector after restart, never \
         both and never neither"
    );

    let entity = *restored_to.simulation.ships.index.get(&ship_id).unwrap();
    assert_eq!(
        restored_to
            .simulation
            .world
            .get::<VelocityComp>(entity)
            .unwrap()
            .0,
        Velocity::new(1.0, 0.0, 0.0),
        "velocity"
    );
    assert_eq!(
        restored_to.get_ship_position(ship_id),
        Some(entry_offset),
        "the imported ship must land at the transit entry position, \
         the same as the live import_transit path"
    );
    // The load-bearing check: the destination replay must redo the
    // anchor rebase itself (rebase_ship_anchor_state), not rely on the
    // already-logged AnchorRebased entry -- that entry replays *before*
    // the ship exists on this Sector (see that method's doc comment) and
    // silently no-ops. Comparing against the live `to_node`'s anchor
    // (produced by the same import through the normal, working path)
    // catches a dropped rebase that a position-only check cannot: with no
    // nearby body to rebase onto, this Sector's `AnchorTable` falls back
    // to the same default anchor either way, so position alone reads
    // identical whether or not the rebase ran. Anchor identity does not.
    assert_eq!(
        restored_to.get_ship_anchor(ship_id),
        to_node.get_ship_anchor(ship_id),
        "the restored ship's anchor must match what the live import produced"
    );
}

/// The crash window a review of the first version of this fix caught
/// (issue #204): a cluster restart between the source's `TransitOp::Request`
/// commit and the destination's `TransitOp::Commit` commit must not lose
/// the ship. Before deferring `complete_outgoing_transit` to Commit-time,
/// `export_transit` removed the ship and appended `SectorTransitCompleted`
/// immediately at Request-commit time -- durably, on the source's own
/// log -- before the destination's Commit had even been *proposed* to
/// Raft, let alone committed. A restart in that gap left the source log
/// saying the ship was gone and the destination log with nothing at all:
/// the ship existed nowhere. This test stops at exactly that point --
/// `export_transit` runs (building the Commit proposal payload) but
/// neither `complete_outgoing_transit` nor `import_transit` ever does --
/// and asserts the source Sector still owns the ship after a simulated
/// restart.
#[test]
fn a_ship_survives_a_restart_between_request_commit_and_transit_commit() {
    let mut from_node = SimulationNode::new_test(
        NodeId(0),
        SectorId(0),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
    );

    let ship_id = from_node.spawn_ship(
        ShipTypeId(1),
        Position::ORIGIN,
        Velocity::new(1.0, 0.0, 0.0),
    );
    let snapshot_before = from_node.take_snapshot();

    from_node
        .propose_transit(TransitCommand {
            ship_id,
            to: SectorId(1),
        })
        .unwrap();
    // Mirrors `prepare_transit_commit`'s snapshot step, run for a
    // `TransitOp::Commit` that -- in this test -- is never proposed,
    // never mind committed. Nothing past this point ever runs:
    // no `complete_outgoing_transit`, no destination `import_transit`.
    let _snapshot_for_commit_proposal = from_node.export_transit(ship_id).unwrap();

    // Simulate a whole-cluster restart at exactly this point.
    let mut store2 = InMemoryEventStore::new();
    for event in from_node.pending_events() {
        store2.append(event.clone());
    }
    let restored = SimulationNode::restore_from_test(
        store2,
        &snapshot_before,
        std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        &[],
        &[],
    );

    assert!(
        restored.simulation.ships.index.contains_key(&ship_id),
        "a restart before the Commit lands must not lose the ship -- it \
         is still owned by the source Sector, just pending"
    );
    let entity = *restored.simulation.ships.index.get(&ship_id).unwrap();
    assert_eq!(
        restored.simulation.world.transit_state(entity),
        TransitState::InTransit { to: SectorId(1) },
        "the ship must still be marked InTransit, so it stays frozen \
         (Movement/Combat) and a retried Commit is still meaningful"
    );
}
