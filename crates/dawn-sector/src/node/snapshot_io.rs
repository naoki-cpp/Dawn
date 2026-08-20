use crate::persistence::StateSnapshot;
use dawn_storage::{PublicEventIndex, RecoveryIndex};

use super::{
    state::{GameData, SectorTopology, SimulationState},
    SimulationNode,
};

impl SimulationNode {
    /// Capture the current ECS state as a `StateSnapshot` with no journal
    /// coverage. Runtime code must use [`Self::take_snapshot_with_cursors`]
    /// with both committed stream positions it owns.
    ///
    /// The node is destructured exhaustively (no `..`) on purpose: adding a
    /// field to `SimulationNode` breaks this function until someone decides
    /// whether it survives a restart. Deciding by memory is what silently lost
    /// `player_id_counter` before. `apply_snapshot` is the matching read side.
    #[cfg(test)]
    pub(crate) fn take_snapshot(&self) -> StateSnapshot {
        self.take_snapshot_at(0)
    }

    /// Capture a recovery fixture with no covered public events.
    ///
    /// This helper exists for recovery tests and integration fixtures that do
    /// not own a public event store. Production checkpointing must use
    /// [`Self::take_snapshot_with_cursors`] so a valid public cursor cannot be
    /// silently replaced by zero.
    pub fn take_snapshot_at(
        &self,
        covered_recovery_index: impl Into<RecoveryIndex>,
    ) -> StateSnapshot {
        self.take_snapshot_with_cursors(covered_recovery_index.into(), PublicEventIndex::ZERO)
    }

    /// Capture the authoritative state with both independent durable stream
    /// positions. Runtime checkpointing must provide the public-event cursor
    /// from the public store instead of deriving it from recovery coverage.
    pub fn take_snapshot_with_cursors(
        &self,
        covered_recovery_index: RecoveryIndex,
        public_event_next_index: PublicEventIndex,
    ) -> StateSnapshot {
        let Self {
            node_id,
            sector_id,
            bounds,
            simulation,
            // Covered by capture_node_state() below.
            players: _,
            stations: _,
            transit: _,
            topology,
            game_data,
            // Covered by capture_node_state() below.
            frame_outputs: _,
            persistence: _,
        } = self;
        let SimulationState {
            // Read again inside capture_tick_ship_state below (via `self`),
            // which needs the full `&SimulationNode` for anchor resolution.
            world: _,
            current_tick,
            // Covered by capture_node_state() below.
            id_counter: _,
            ships: ship_registry,
            base_stats: _,
            pending_bot_lock_commands: _,
            applied_market_settlements: _,
        } = simulation;
        let SectorTopology {
            sector_map: _,
            anchor_table: _,
        } = topology;
        let GameData {
            module_registry: _,
            ship_type_registry: _,
            catalog_fingerprint,
        } = game_data;

        // One capture per ship, through the same optional-component list the
        // tick-prepare rollback and TickRecoveryDelta use (ADR-0049, issue
        // #312) -- this used to be a fourth independent hand-rolled reader
        // that only captured capacitor/fitting-without-cycle/tackled/
        // inventory, silently dropping flight-mode state, lock countdowns,
        // and module cycle counters on every checkpoint restore.
        let mut ships: Vec<crate::transition::ShipState> = ship_registry
            .index
            .keys()
            .filter_map(|&ship_id| self.capture_tick_ship_state(ship_id))
            .collect();

        // Canonical ordering: a snapshot of a given state must serialise
        // identically regardless of HashMap iteration order, so it can be
        // byte-compared (INV-002: verifiable snapshot / round-trip).
        // `ShipId: Ord` is canonical id order (node_id then counter).
        ships.sort_by_key(|s| s.snapshot.ship_id);

        StateSnapshot {
            node_id: *node_id,
            sector_id: *sector_id,
            bounds: *bounds,
            covered_recovery_index,
            public_event_next_index,
            tick: *current_tick,
            catalog_fingerprint: *catalog_fingerprint,
            ships,
            node_state: self.capture_node_state(),
        }
    }

    /// Capture every node-level scalar/collection this Sector must reproduce
    /// exactly after a tick rollback or a checkpoint restore (ADR-0049,
    /// issue #312) -- the single function `take_snapshot_at` and
    /// `prepare_tick_state_transition_with_result` (tick.rs) both call,
    /// instead of each re-deriving the same fields by hand. `transit_saga`
    /// is captured here for completeness but restored separately by
    /// `restore_transit_saga`, which does Saga reconciliation beyond a plain
    /// field copy.
    pub(super) fn capture_node_state(&self) -> crate::transition::NodeState {
        let mut applied_market_settlements: Vec<u64> = self
            .simulation
            .applied_market_settlements
            .iter()
            .copied()
            .collect();
        applied_market_settlements.sort_unstable();

        crate::transition::NodeState {
            id_counter: self.simulation.id_counter,
            player_id_counter: self.players.player_id_counter,
            transit_attempt_counter: self.transit.transit_attempt_counter,
            active_ships: self
                .players
                .active_ship
                .iter()
                .map(|(&player, &ship)| (player, ship))
                .collect(),
            owners: self
                .players
                .owners
                .iter()
                .map(|(&ship, &player)| (ship, player))
                .collect(),
            docked_ships: self.stations.snapshot_docked_ships(),
            docked_players: self.stations.snapshot_docked_players(),
            pending_bot_lock_commands: self.simulation.pending_bot_lock_commands.clone(),
            pending_auto_jumps: self.frame_outputs.pending_auto_jumps.clone(),
            applied_market_settlements,
            transit_saga: self.transit_saga_snapshot(),
        }
    }

    /// Restore every field `capture_node_state` captures except
    /// `transit_saga`, which the caller applies separately via
    /// `restore_transit_saga`.
    pub(super) fn restore_node_state(&mut self, node: &crate::transition::NodeState) {
        let crate::transition::NodeState {
            id_counter,
            player_id_counter,
            transit_attempt_counter,
            active_ships,
            owners,
            docked_ships,
            docked_players,
            pending_bot_lock_commands,
            pending_auto_jumps,
            applied_market_settlements,
            transit_saga: _,
        } = node;

        self.simulation.id_counter = *id_counter;
        self.players.player_id_counter = *player_id_counter;
        self.transit.transit_attempt_counter = *transit_attempt_counter;
        self.players.active_ship = active_ships
            .iter()
            .map(|(&player, &ship)| (player, ship))
            .collect();
        self.players.owners = owners
            .iter()
            .map(|(&ship, &player)| (ship, player))
            .collect();
        self.stations
            .restore(docked_ships.clone(), docked_players.clone());
        self.simulation.pending_bot_lock_commands = pending_bot_lock_commands.clone();
        self.frame_outputs.pending_auto_jumps = pending_auto_jumps.clone();
        self.simulation.applied_market_settlements =
            applied_market_settlements.iter().copied().collect();
    }

    /// Apply the node-level scalars and maps carried by `snapshot`.
    ///
    /// The read half of the seam `take_snapshot` writes. Covers only the state
    /// this struct owns directly — ships are materialised separately by
    /// `restore_ship_from_snapshot`. The external recovery journal is applied
    /// by the runtime after this checkpoint is loaded; this method does not
    /// interpret public events as authoritative state.
    ///
    /// `StateSnapshot` is destructured exhaustively (no `..`) for the same
    /// reason `take_snapshot` destructures the node: adding a field there must
    /// not compile until it is read back somewhere.
    pub(super) fn apply_snapshot(&mut self, snapshot: &StateSnapshot) {
        let StateSnapshot {
            node_id,
            sector_id,
            bounds,
            tick,
            catalog_fingerprint: _,
            node_state,
            // `ships` needs the module and ship-type registries in place first.
            // `covered_recovery_index` is external journal coverage, not an event cursor for
            // this storage-independent restore operation.
            ships: _,
            covered_recovery_index: _,
            public_event_next_index: _,
        } = snapshot;

        self.node_id = *node_id;
        self.sector_id = *sector_id;
        self.bounds = *bounds;
        self.simulation.current_tick = *tick;

        // Restore the canonical node state first. Transit Saga reconciliation
        // remains a separate step because it validates and rebuilds the
        // handoff journal rather than copying fields blindly.
        self.restore_node_state(node_state);
        self.transit.transit_journal = crate::transit::handoff::TransitJournal::from_snapshot(
            *sector_id,
            node_state.transit_saga.clone(),
        )
        .expect("checkpoint contains an invalid Transit Saga for this Sector");
    }
}

// ── Checkpointing (ADR-0017 8A-7) ──────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::station::StationOperationOutcome;
    use crate::persistence::{ShipSnapshot, StateSnapshot};
    use dawn_core::{NodeId, PlayerId, Position, SectorBounds, SectorId, ShipId, Tick, Velocity};

    fn mem_node() -> SimulationNode {
        SimulationNode::new_test(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        )
    }

    fn node_with_modules() -> SimulationNode {
        mem_node()
    }

    /// Restoring a snapshot and re-capturing must reproduce it byte for byte.
    ///
    /// This is the read-side half of the seam's enforcement. `take_snapshot`
    /// destructures the node exhaustively, so a field cannot silently fail to
    /// be *written*; this covers the other direction, where a field is written
    /// but `apply_snapshot` never reads it back — which is how
    /// `player_id_counter` stayed at zero across a restart while `id_counter`
    /// survived.
    ///
    /// Compared on the encoded bytes rather than field by field so the
    /// assertion cannot go stale: `snapshot_fixture`'s struct literal is what
    /// enforces coverage, and it stops compiling when `StateSnapshot` grows a
    /// field.
    #[test]
    fn restoring_a_snapshot_and_recapturing_reproduces_it_exactly() {
        let original = snapshot_fixture(0);
        let node = SimulationNode::restore_from(
            &original,
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            crate::game_data::test_catalog_arc(),
        );

        assert_eq!(
            postcard::to_stdvec(&node.take_snapshot_at(original.covered_recovery_index)).unwrap(),
            postcard::to_stdvec(&original).unwrap(),
            "restore lost or altered state that take_snapshot had captured"
        );
    }

    /// A restarted node must not re-issue a `PlayerId` it already handed out.
    ///
    /// The snapshot carries established ownership bindings so a restarted
    /// node cannot let a different resume identity claim a restored Ship.
    /// The allocation counter still prevents a restart from issuing a PlayerId
    /// already handed out on the real admission path.
    #[test]
    fn player_ids_are_not_reissued_after_a_restore() {
        let mut node = node_with_modules();
        let first = node.next_player_id();
        let second = node.next_player_id();
        let snapshot = node.take_snapshot();

        let mut restored = SimulationNode::restore_from(
            &snapshot,
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            crate::game_data::test_catalog_arc(),
        );

        let after_restart = restored.next_player_id();
        assert!(
            after_restart != first && after_restart != second,
            "restart re-issued {after_restart:?}, already held by a restored player"
        );
    }

    #[test]
    fn restore_rejects_a_checkpoint_from_a_different_catalog() {
        let mut snapshot = snapshot_fixture(0);
        snapshot.catalog_fingerprint ^= 1;

        let result = SimulationNode::restore_from_checked(
            &snapshot,
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            std::sync::Arc::new(crate::game_data::test_catalog().clone()),
        );

        let error = result.expect_err("an incompatible catalog must fence restore");
        assert!(error.contains("catalog fingerprint"));
    }

    /// Representative NodeState fields are non-default so an omitted value
    /// changes the encoded bytes. The exhaustive StateSnapshot and NodeState
    /// literals also catch newly added fields at compile time.
    fn snapshot_fixture(covered_recovery_index: u64) -> StateSnapshot {
        StateSnapshot {
            node_id: NodeId(0),
            sector_id: SectorId(0),
            bounds: SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            covered_recovery_index: covered_recovery_index.into(),
            public_event_next_index: 0.into(),
            tick: Tick(17),
            catalog_fingerprint: crate::game_data::test_catalog().fingerprint(),
            ships: vec![crate::transition::ShipState {
                snapshot: ShipSnapshot {
                    ship_id: ShipId::new(NodeId(0), 0),
                    ship_type_id: dawn_core::ShipTypeId(1),
                    absolute_position: Some(dawn_core::AbsolutePosition::new(100.0, 200.0, 300.0)),
                    position: Position::new(100.0, 200.0, 300.0),
                    anchor: dawn_core::AnchorId(0),
                    velocity: Velocity::new(1.0, 2.0, 3.0),
                    current_shield: 50.0,
                    current_armor: 60.0,
                    current_hull: 70.0,
                    is_destroyed: false,
                    capacitor: Some(250.0),
                    fitting: dawn_core::fitting::FittingSnapshot::empty(),
                    tackled_by: vec![ShipId::new(NodeId(0), 1)],
                    inventory: std::collections::BTreeMap::from([(
                        dawn_core::ItemId::ScrapMetal,
                        4,
                    )]),
                },
                fitting_present: false,
                inventory_present: true,
                movement: crate::transition::TickMovementState {
                    thrust: None,
                    approach: None,
                    orbit: None,
                    keep_at_range: None,
                    warp: None,
                },
                fitting: crate::transition::TickFittingState {
                    high: Vec::new(),
                    mid: Vec::new(),
                    low: Vec::new(),
                    rig: Vec::new(),
                },
                locks: None,
                weapon_last_fired_tick: None,
            }],
            node_state: crate::transition::NodeState {
                id_counter: 5,
                player_id_counter: 3,
                transit_attempt_counter: 8,
                active_ships: std::collections::BTreeMap::from([(
                    dawn_core::PlayerId(9),
                    ShipId::new(NodeId(0), 0),
                )]),
                owners: std::collections::BTreeMap::from([(
                    ShipId::new(NodeId(0), 0),
                    dawn_core::PlayerId(9),
                )]),
                docked_ships: std::collections::BTreeMap::from([(
                    ShipId::new(NodeId(0), 0),
                    dawn_core::StationId(0),
                )]),
                docked_players: std::collections::BTreeMap::from([(
                    dawn_core::PlayerId(9),
                    dawn_core::StationId(0),
                )]),
                pending_bot_lock_commands: vec![dawn_core::LockOnCommand {
                    ship_id: ShipId::new(NodeId(0), 0),
                    target_id: ShipId::new(NodeId(0), 1),
                }],
                pending_auto_jumps: vec![(ShipId::new(NodeId(0), 0), dawn_core::JumpGateId(7))],
                applied_market_settlements: vec![7, 11],
                transit_saga: crate::persistence::TransitSagaSnapshot::default(),
            },
        }
    }

    #[test]
    fn snapshot_records_correct_ship_count_and_tick() {
        const COVERED_RECOVERY_INDEX: u64 = 41;
        let mut node = mem_node();
        for i in 0..3 {
            node.spawn_ship(
                dawn_core::ShipTypeId(1),
                Position::new(i as f64 * 100.0, 0.0, 0.0),
                Velocity::new(1.0, 0.0, 0.0),
            );
        }
        for _ in 0..5 {
            node.tick();
        }

        let snap = node.take_snapshot_at(COVERED_RECOVERY_INDEX);
        assert_eq!(snap.ships.len(), 3);
        assert_eq!(snap.tick, Tick(5));
        assert_eq!(snap.covered_recovery_index, COVERED_RECOVERY_INDEX);
    }

    #[test]
    fn ecs_state_is_fully_restored_from_snapshot_after_simulated_restart() {
        // ── Session 1: run, snapshot mid-way, continue, shut down ───────────
        let ship_ids: Vec<ShipId>;
        let snap: StateSnapshot;
        let final_tick: Tick;
        let final_positions: Vec<Position>;
        {
            let mut node = SimulationNode::new_test(
                NodeId(0),
                SectorId(0),
                SectorBounds::centered(SectorBounds::DEFAULT_HALF),
                std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            );

            // Spawn owned player ships with a stable velocity. The checkpoint
            // restores instantaneous movement state; an active thrust command
            // belongs to the RecoveryDelta write set instead.
            ship_ids = (0..5u64)
                .map(|i| {
                    let id = node.spawn_player_ship_at_pub(
                        PlayerId(i),
                        Position::new(i as f64 * 100.0, 0.0, 0.0),
                    );
                    let entity = *node.simulation.ships.index.get(&id).unwrap();
                    node.simulation
                        .world
                        .get_mut::<dawn_ecs::components::VelocityComp>(entity)
                        .unwrap()
                        .0 = Velocity::new(120.0, 0.0, 0.0);
                    id
                })
                .collect();

            for _ in 0..5 {
                node.tick();
            }
            snap = node.take_snapshot();
            for _ in 0..3 {
                node.tick();
            }

            final_tick = node.current_tick();
            final_positions = ship_ids
                .iter()
                .map(|id| node.get_ship_position(*id).unwrap())
                .collect();
        }

        // ── Session 2: restart, restore, verify ─────────────────────────────
        //
        // The snapshot contains the current position, velocity, and tick, so
        // the remaining constant-velocity ticks must reach the same position.
        let mut node2 = SimulationNode::restore_from(
            &snap,
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            crate::game_data::test_catalog_arc(),
        );

        let remaining = final_tick.value() - node2.current_tick().value();
        for _ in 0..remaining {
            node2.tick();
        }

        assert_eq!(
            node2.current_tick(),
            final_tick,
            "tick must match after restore + subsequent ticks"
        );
        assert_eq!(
            node2.ship_count(),
            ship_ids.len(),
            "ship count must match after restore"
        );
        for (id, expected_pos) in ship_ids.iter().zip(final_positions.iter()) {
            let restored_pos = node2
                .get_ship_position(*id)
                .expect("ship must exist after restore");
            assert_eq!(
                restored_pos, *expected_pos,
                "position of ship {} must match after restore + subsequent ticks",
                id
            );
        }
    }

    /// INV-002 / ADR-0017 (8A-1) — a snapshot is a verifiable checkpoint:
    /// restoring it and snapshotting again yields a byte-identical snapshot.
    #[test]
    fn snapshot_round_trips_through_restore_byte_for_byte() {
        const COVERED_RECOVERY_INDEX: u64 = 73;
        let mut node = node_with_modules();

        for i in 0..4u64 {
            let id = node.spawn_ship(
                dawn_core::ShipTypeId(1),
                Position::new(i as f64 * 50.0, 0.0, 0.0),
                Velocity::ZERO,
            );
            node.set_player_ship(id);
            node.apply_move_command(id, Position::new(9_000.0, 1_000.0, 0.0));
        }
        for _ in 0..6 {
            node.tick();
        }

        let snap1 = node.take_snapshot_at(COVERED_RECOVERY_INDEX);

        let node2 = SimulationNode::restore_from(
            &snap1,
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            crate::game_data::test_catalog_arc(),
        );
        let snap2 = node2.take_snapshot_at(snap1.covered_recovery_index);

        assert_eq!(snap2.covered_recovery_index, COVERED_RECOVERY_INDEX);

        assert_eq!(
            postcard::to_stdvec(&snap1).unwrap(),
            postcard::to_stdvec(&snap2).unwrap(),
            "snapshot must round-trip through restore byte-for-byte (INV-002)"
        );
    }

    /// INV-002 / ADR-0017 (8A-4) — operational recovery does NOT require genesis
    /// public-event history. Restoring from the checkpoint still reproduces
    /// the live state before subsequent committed ticks are applied.
    #[test]
    fn recovery_does_not_require_genesis_public_event_history() {
        let snap;
        let live_final;
        {
            let mut node = SimulationNode::new_test(
                NodeId(0),
                SectorId(0),
                SectorBounds::centered(SectorBounds::DEFAULT_HALF),
                std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            );
            for i in 0..3u64 {
                node.spawn_ship(
                    dawn_core::ShipTypeId(1),
                    Position::new(i as f64 * 100.0, 0.0, 0.0),
                    Velocity::new(40.0, -15.0, 0.0),
                );
            }
            for _ in 0..8 {
                node.tick();
            }
            snap = node.take_snapshot();
            for _ in 0..4 {
                node.tick();
            }
            live_final = node.take_snapshot();
        }

        let mut restored = SimulationNode::restore_from(
            &snap,
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            crate::game_data::test_catalog_arc(),
        );
        assert_eq!(restored.current_tick(), snap.tick);
        for _ in 0..4 {
            restored.tick();
        }
        let mut restored_final = restored.take_snapshot_at(0);
        let mut live_final = live_final;
        live_final.covered_recovery_index = 0.into();
        restored_final.covered_recovery_index = 0.into();

        assert_eq!(
            postcard::to_stdvec(&live_final).unwrap(),
            postcard::to_stdvec(&restored_final).unwrap(),
            "recovery from the checkpoint plus subsequent committed ticks must match live"
        );
    }

    #[test]
    fn hull_capacitor_and_fitting_state_are_restored_from_snapshot() {
        use crate::{modules, ship_types};
        use dawn_core::fitting::{FittingSnapshot, SlotEntry};
        use dawn_ecs::components::{FittingComp, HullComp};

        let dir = tempfile::tempdir().unwrap();
        let snapshot_path = dir.path().join("snapshot.bin");

        let ship_id: ShipId;
        {
            let mut node = node_with_modules();

            ship_id = node.spawn_ship(
                ship_types::SHIP_TYPE_MAGPIE,
                Position::ORIGIN,
                Velocity::ZERO,
            );

            let entity = *node.simulation.ships.index.get(&ship_id).unwrap();
            node.simulation
                .world
                .get_mut::<HullComp>(entity)
                .unwrap()
                .set_hp(30.0, 90.0, 100.0);
            let fitting = FittingComp::from_snapshot(
                &FittingSnapshot {
                    high: vec![],
                    mid: vec![SlotEntry {
                        module_id: modules::MODULE_AFTERBURNER,
                        is_active: true,
                    }],
                    low: vec![],
                    rig: vec![],
                },
                &node.game_data.module_registry,
            );
            node.simulation.world.insert_one(entity, fitting);
            node.reapply_fitting(ship_id);

            let snap = node.take_snapshot();
            snap.save(&snapshot_path).unwrap();
        }

        let snap = StateSnapshot::load(&snapshot_path).unwrap();
        let node2 = SimulationNode::restore_from(
            &snap,
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            crate::game_data::test_catalog_arc(),
        );

        let hp = node2.get_ship_hp(ship_id).unwrap();
        assert_eq!(
            hp,
            30.0 + 90.0 + 100.0,
            "Hull HP layers must survive restore"
        );

        let cap = node2
            .get_ship_capacitor(ship_id)
            .expect("capacitor must be restored");
        assert_eq!(
            cap,
            node2.get_ship_stats(ship_id).unwrap().cap_max,
            "capacitor must be restored to its snapshot value"
        );

        let fitted = node2.get_fitted_module_ids(ship_id);
        assert!(
            fitted.iter().any(|module| {
                module.module_id == modules::MODULE_AFTERBURNER && module.is_active
            }),
            "Afterburner must remain fitted and active after restore, got {:?}",
            fitted
        );
    }

    /// ADR-0049 / issue #312: this regression test proves that a checkpoint
    /// reproduces flight-mode state, lock countdowns, and module cycle
    /// counters exactly (recovery-contract.md rows 193/196/198), alongside
    /// the position/velocity/HP/fitting state covered by the surrounding
    /// snapshot restore tests.
    #[test]
    fn warp_lock_and_module_cycle_state_survive_snapshot_restore() {
        use crate::{modules, ship_types};
        use dawn_core::fitting::{FittingSnapshot, SlotEntry};
        use dawn_core::navigation::WarpTarget;
        use dawn_core::{AbsolutePosition, JumpGateId};
        use dawn_ecs::components::{
            FittingComp, LockComp, LockEntry, LockState, WarpComp, WarpPhase,
        };

        let mut node = mem_node();

        let ship_id = node.spawn_ship(
            ship_types::SHIP_TYPE_MAGPIE,
            Position::ORIGIN,
            Velocity::ZERO,
        );
        let target_id = node.spawn_ship(
            ship_types::SHIP_TYPE_MAGPIE,
            Position::new(500.0, 0.0, 0.0),
            Velocity::ZERO,
        );
        let entity = *node.simulation.ships.index.get(&ship_id).unwrap();

        // Mid-warp.
        node.simulation.world.insert_one(
            entity,
            WarpComp {
                target: WarpTarget::Gate(JumpGateId(1)),
                phase: WarpPhase::Warping,
                auto_jump: false,
                warp_start_abs: AbsolutePosition([0.0, 0.0, 0.0]),
                warp_total: 20,
                warp_elapsed: 7,
                warp_arrival_abs: AbsolutePosition([1000.0, 0.0, 0.0]),
                warp_start_vel: Velocity::ZERO,
            },
        );

        // Active lock on target_id.
        node.simulation.world.insert_one(
            entity,
            LockComp {
                entries: vec![LockEntry {
                    target_id,
                    state: LockState::Locked,
                }],
            },
        );

        // Fitted, active, partially-cycled module.
        let fitting = FittingComp::from_snapshot(
            &FittingSnapshot {
                high: vec![SlotEntry {
                    module_id: modules::MODULE_AFTERBURNER,
                    is_active: true,
                }],
                mid: vec![],
                low: vec![],
                rig: vec![],
            },
            &node.game_data.module_registry,
        );
        node.simulation.world.insert_one(entity, fitting);
        node.reapply_fitting(ship_id);
        {
            let mut fitting = node
                .simulation
                .world
                .get_mut::<FittingComp>(entity)
                .expect("fitting was just set by ShipFitted");
            fitting.high[0].cycle_remaining = 3;
        }

        let snap = node.take_snapshot();
        let node2 = SimulationNode::restore_from(
            &snap,
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            crate::game_data::test_catalog_arc(),
        );

        assert_eq!(
            node2.warp_phase(ship_id),
            Some(WarpPhase::Warping),
            "warp state must survive a checkpoint restore (recovery-contract.md row 193)"
        );

        let entity2 = *node2
            .simulation
            .ships
            .index
            .get(&ship_id)
            .expect("ship must exist after restore");
        let lock = node2.simulation.world.get::<LockComp>(entity2);
        assert!(
            lock.as_deref().is_some_and(|lock| lock
                .entries
                .iter()
                .any(|entry| entry.target_id == target_id
                    && matches!(entry.state, LockState::Locked))),
            "an active lock must survive a checkpoint restore (recovery-contract.md row 198)"
        );

        let fitting2 = node2
            .simulation
            .world
            .get::<FittingComp>(entity2)
            .expect("fitting must survive a checkpoint restore");
        assert_eq!(
            fitting2.high[0].cycle_remaining, 3,
            "module cycle_remaining must survive a checkpoint restore (recovery-contract.md row 196)"
        );
    }

    /// INV-002 / ADR-0029 — a warp-to-body rebases the ship onto the body's
    /// anchor via an authoritative `AnchorRebased` event, leaving its raw
    /// `PositionComp` body-relative. The snapshot must capture that anchor (not
    /// just the offset) so restore reproduces the same *absolute* position; a
    /// snapshot that dropped the anchor would silently relocate the ship by the
    /// body's absolute position (~10^5+ units) on restart. This is the
    /// new-schema check the pre-anchor round-trip tests can't make.
    #[test]
    fn warp_arrival_anchor_and_absolute_position_survive_snapshot_restore() {
        use dawn_core::WarpTarget;

        let mut node = node_with_modules();
        let player = node.next_player_id();
        let ship_id = node.spawn_player_ship_at_pub(player, Position::new(0.0, 0.0, 0.0));

        let body_id = dawn_core::CelestialBodyId(1);
        assert!(
            node.apply_warp_command(ship_id, WarpTarget::Body(body_id), false),
            "warp to body should be accepted",
        );
        for _ in 0..5_000 {
            node.tick();
            if node.warp_phase(ship_id).is_none() {
                break;
            }
        }
        assert!(
            node.warp_phase(ship_id).is_none(),
            "warp should have completed"
        );

        let live_anchor = node.get_ship_anchor(ship_id).expect("ship has an anchor");
        assert_eq!(
            live_anchor,
            dawn_core::AnchorId::from(body_id),
            "arrival should have rebased onto the body anchor"
        );
        let live_abs = node.ship_absolute(ship_id).expect("ship exists");

        // Anchor and offset are checkpoint state, not a public-event replay.
        let snap = node.take_snapshot();
        let node2 = SimulationNode::restore_from(
            &snap,
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            crate::game_data::test_catalog_arc(),
        );

        assert_eq!(
            node2.get_ship_anchor(ship_id),
            Some(live_anchor),
            "anchor must survive snapshot + restore"
        );
        let restored_abs = node2
            .ship_absolute(ship_id)
            .expect("ship exists after restore");
        assert_eq!(
            restored_abs, live_abs,
            "absolute position must be identical after restore (anchor + offset both restored)"
        );
    }

    /// ADR-0038: Station inventory survives a restart because it lives in its
    /// own durable SQLite file, independent of the checkpoint lifecycle -- not
    /// because `take_snapshot()`/`restore_from()` carry it. Simulates a real
    /// restart: `node` and `node2` are otherwise-independent
    /// `SimulationNode`s, but both point `open_repositories` at the same
    /// on-disk file.
    #[test]
    fn station_inventory_survives_snapshot_restore() {
        use dawn_core::{ItemId, PlayerId, StationId};

        let db_path = tempfile::NamedTempFile::new().unwrap();
        let db_path = db_path.path().to_str().unwrap();

        let mut node = node_with_modules();
        node.open_repositories(db_path).unwrap();
        node.credit_station_item(PlayerId(7), StationId(0), ItemId::ScrapMetal, 4);
        node.credit_station_item(
            PlayerId(7),
            StationId(0),
            ItemId::PackagedShip(dawn_core::ShipTypeId(1)),
            1,
        );

        let snap = node.take_snapshot();
        let mut node2 = SimulationNode::restore_from(
            &snap,
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            crate::game_data::test_catalog_arc(),
        );
        node2.open_repositories(db_path).unwrap();

        assert_eq!(
            node2.station_item_count(PlayerId(7), StationId(0), ItemId::ScrapMetal),
            4
        );
        assert_eq!(
            node2.station_item_count(
                PlayerId(7),
                StationId(0),
                ItemId::PackagedShip(dawn_core::ShipTypeId(1)),
            ),
            1
        );
    }

    // `restore_from_migrates_a_pre_adr_0038_snapshots_station_inventories` was
    // deleted with the field it covered. It built the snapshot in memory and
    // mutated the field directly, so it never crossed the `load` path it
    // claimed to test — and a real pre-ADR-0038 snapshot cannot reach that
    // branch at all, since postcard rejects the shorter buffer outright
    // (ADR-0017 format compatibility).

    #[test]
    fn docked_station_state_survives_snapshot_restore() {
        use dawn_core::{DockCommand, StationId};

        let mut node = node_with_modules();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);
        let station = node.station(StationId(0)).expect("demo station exists");
        node.set_spawn_anchor_abs(ship_id, station.abs_m);
        assert!(matches!(
            node.dock_owned(
                player_id,
                ship_id,
                DockCommand {
                    station_id: StationId(0),
                }
            ),
            StationOperationOutcome::Accepted { .. }
        ));

        let snap = node.take_snapshot();
        let node2 = SimulationNode::restore_from(
            &snap,
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            crate::game_data::test_catalog_arc(),
        );

        assert_eq!(node2.docked_station(ship_id), Some(StationId(0)));
        assert_eq!(node2.player_docked_station(player_id), Some(StationId(0)));
    }
}
