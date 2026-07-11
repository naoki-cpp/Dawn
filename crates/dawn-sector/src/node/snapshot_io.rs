use std::collections::BTreeMap;

use dawn_ecs::components::{
    CapacitorComp, FittingComp, HullComp, InventoryComp, PositionComp, TackledComp, VelocityComp,
};
use dawn_event_store::store::EventStore;

use crate::persistence::{ShipSnapshot, StateSnapshot};

use super::SimulationNode;

impl<S: EventStore> SimulationNode<S> {
    /// Capture the current ECS state as a `StateSnapshot`.
    ///
    /// The snapshot covers all events with `log_index < event_store.len()`.
    /// Pair with the event log to reconstruct this exact state on restart.
    pub fn take_snapshot(&self) -> StateSnapshot {
        let mut ships: Vec<ShipSnapshot> = self
            .ships
            .index
            .iter()
            .filter_map(|(&ship_id, &entity)| {
                let pos = self.world.get::<PositionComp>(entity)?.0;
                let vel = self.world.get::<VelocityComp>(entity)?.0;
                let hull = self.world.get::<HullComp>(entity)?;
                let capacitor = self.world.get::<CapacitorComp>(entity).map(|c| c.current);
                let fitting = self
                    .world
                    .get::<FittingComp>(entity)
                    .map(|f| f.to_snapshot())
                    .unwrap_or_else(dawn_core::fitting::FittingSnapshot::empty);
                let tackled_by = self
                    .world
                    .get::<TackledComp>(entity)
                    .map(|t| t.tacklers.clone())
                    .unwrap_or_default();
                let inventory = self
                    .world
                    .get::<InventoryComp>(entity)
                    .map(|inv| inv.items.clone())
                    .unwrap_or_default();
                let ship_type_id = self
                    .ships
                    .type_ids
                    .get(&ship_id)
                    .copied()
                    .unwrap_or(dawn_core::ShipTypeId(0));
                let anchor = self.world.ship_anchor(entity).unwrap_or_default();
                Some(ShipSnapshot {
                    ship_id,
                    ship_type_id,
                    position: pos,
                    anchor,
                    velocity: vel,
                    current_shield: hull.shield(),
                    current_armor: hull.armor(),
                    current_hull: hull.hull(),
                    is_destroyed: hull.is_destroyed(),
                    capacitor,
                    fitting,
                    tackled_by,
                    inventory,
                })
            })
            .collect();

        // Canonical ordering: a snapshot of a given state must serialise
        // identically regardless of HashMap iteration order, so it can be
        // byte-compared (INV-002: verifiable snapshot / round-trip).
        // `ShipId: Ord` is canonical id order (node_id then counter).
        ships.sort_by_key(|s| s.ship_id);

        StateSnapshot {
            node_id: self.node_id,
            sector_id: self.sector_id,
            bounds: self.bounds,
            log_index: self.event_store.len() as u64,
            tick: self.current_tick,
            id_counter: self.id_counter,
            ships,
            // ADR-0038: Station inventory is durable in SQLite now, decoupled
            // from the tick-log snapshot cadence -- never populated here
            // going forward. Kept on `StateSnapshot` only so old snapshots
            // (from before this change) still deserialize; `restore_from`
            // migrates a non-empty one into SQLite once.
            station_inventories: BTreeMap::new(),
            docked_ships: self.docked_ships.clone(),
            docked_players: self.docked_players.clone(),
        }
    }
}

// ── Checkpointing (ADR-0017 8A-7) ──────────────────────────────────────────────

impl SimulationNode<dawn_event_store::FileEventStore> {
    /// Take a snapshot, persist it durably, then compact the hot log behind it.
    ///
    /// This is the operational checkpoint of ADR-0017: after it returns, recovery
    /// only needs `snapshot_path` + the post-snapshot tail of the hot log; the
    /// prefix it covers lives in the append-only cold archive at `cold_path`.
    ///
    /// Ordering is load-bearing for crash safety: the snapshot is saved **before**
    /// the hot log is compacted. A crash between the two leaves the snapshot
    /// written and the hot log untouched (a redundant but safe state). Compacting
    /// first could strand a snapshot whose `log_index` is older than the new
    /// `base_index`, which would make `iter_from` silently skip events.
    pub fn checkpoint(
        &mut self,
        snapshot_path: impl AsRef<std::path::Path>,
        cold_path: impl AsRef<std::path::Path>,
    ) -> std::io::Result<StateSnapshot> {
        let snapshot = self.take_snapshot();
        snapshot.save(&snapshot_path)?;
        self.event_store.compact(snapshot.log_index, cold_path)?;
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::station::StationOperationOutcome;
    use crate::persistence::StateSnapshot;
    use dawn_core::{NodeId, Position, SectorBounds, SectorId, ShipId, Tick, Velocity};
    use dawn_event_store::{FileEventStore, InMemoryEventStore};

    fn mem_node() -> SimulationNode {
        SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        )
    }

    fn node_with_modules() -> SimulationNode {
        use crate::{modules, ship_types};
        let mut node = mem_node();
        for def in modules::all_modules() {
            node.register_module(def);
        }
        for def in ship_types::all_ship_types() {
            node.register_ship_type(def);
        }
        node
    }

    #[test]
    fn snapshot_records_correct_ship_count_and_tick() {
        let mut node = mem_node();
        for i in 0..3 {
            node.spawn_ship(
                dawn_core::ShipTypeId(1),
                Position::new(i as f32 * 100.0, 0.0, 0.0),
                Velocity::new(1.0, 0.0, 0.0),
            );
        }
        for _ in 0..5 {
            node.tick();
        }

        let snap = node.take_snapshot();
        assert_eq!(snap.ships.len(), 3);
        assert_eq!(snap.tick, Tick(5));
        assert_eq!(snap.log_index, node.total_event_count() as u64);
    }

    #[test]
    fn ecs_state_is_fully_restored_from_snapshot_and_event_replay_after_simulated_restart() {
        let dir = tempfile::tempdir().unwrap();
        let event_path = dir.path().join("events.log");
        let snapshot_path = dir.path().join("snapshot.bin");

        // ── Session 1: run, snapshot mid-way, continue, shut down ───────────
        let ship_ids: Vec<ShipId>;
        let final_tick: Tick;
        let final_positions: Vec<Position>;
        {
            let store = FileEventStore::open(&event_path).unwrap();
            let mut node = SimulationNode::with_store(
                NodeId(0),
                SectorId(0),
                SectorBounds::centered(SectorBounds::DEFAULT_HALF),
                store,
            );

            // Spawn 5 ships as players with thrust so they emit VelocityChanged events.
            // (ADR-0008: NPC ships at constant velocity emit no events, so tick cannot
            //  be restored from the event log alone. Using player ships with thrust
            //  ensures VelocityChanged events carry the tick for replay.)
            ship_ids = (0..5u64)
                .map(|i| {
                    let id = node.spawn_ship(
                        dawn_core::ShipTypeId(1),
                        Position::new(i as f32 * 100.0, 0.0, 0.0),
                        Velocity::ZERO,
                    );
                    node.set_player_ship(id);
                    node.apply_move_command(id, Position::new(10_000.0, 0.0, 0.0));
                    id
                })
                .collect();

            for _ in 0..5 {
                node.tick();
            }
            let snap = node.take_snapshot();
            snap.save(&snapshot_path).unwrap();

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
        // ADR-0008: position is derived state — restore replays velocity events,
        // then we re-run the remaining ticks to reach the exact final position.
        let snap = StateSnapshot::load(&snapshot_path).unwrap();
        let store2 = FileEventStore::open(&event_path).unwrap();
        let mut node2 = SimulationNode::restore_from(store2, &snap, &[], &[]);

        let remaining = final_tick.value() - node2.current_tick().value();
        for _ in 0..remaining {
            node2.tick();
        }

        assert_eq!(
            node2.current_tick(),
            final_tick,
            "tick must match after restore + replay ticks"
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
                "position of ship {} must match after restore + replay",
                id
            );
        }
    }

    /// INV-002 / ADR-0017 (8A-1) — a snapshot is a verifiable checkpoint:
    /// restoring it and snapshotting again yields a byte-identical snapshot.
    #[test]
    fn snapshot_round_trips_through_restore_byte_for_byte() {
        use crate::{modules, ship_types};

        let mut node = node_with_modules();

        for i in 0..4u64 {
            let id = node.spawn_ship(
                dawn_core::ShipTypeId(1),
                Position::new(i as f32 * 50.0, 0.0, 0.0),
                Velocity::ZERO,
            );
            node.set_player_ship(id);
            node.apply_move_command(id, Position::new(9_000.0, 1_000.0, 0.0));
        }
        for _ in 0..6 {
            node.tick();
        }

        let snap1 = node.take_snapshot();

        let mut store2 = InMemoryEventStore::new();
        for rec in node.event_store().all_records() {
            store2.append(rec.event.clone());
        }
        let node2 = SimulationNode::restore_from(
            store2,
            &snap1,
            &modules::all_modules(),
            &ship_types::all_ship_types(),
        );
        let snap2 = node2.take_snapshot();

        assert_eq!(
            postcard::to_stdvec(&snap1).unwrap(),
            postcard::to_stdvec(&snap2).unwrap(),
            "snapshot must round-trip through restore byte-for-byte (INV-002)"
        );
    }

    /// INV-002 / ADR-0017 (8A-1) — snapshot + re-running the tail ticks reproduces
    /// the live state, including transient derived state (capacitor) not event-sourced.
    ///
    /// Ships coast at constant velocity with no move command so there is no
    /// thrust intent (transient, not snapshotted). Both live and restored nodes
    /// coast identically, isolating the snapshot round-trip property.
    #[test]
    fn snapshot_plus_tail_tick_reexecution_matches_live_including_capacitor() {
        use crate::{modules, ship_types};

        let mut live = node_with_modules();

        for i in 0..3u64 {
            live.spawn_ship(
                dawn_core::ShipTypeId(1),
                Position::new(i as f32 * 100.0, 0.0, 0.0),
                Velocity::new(120.0, -40.0, 0.0),
            );
        }

        for _ in 0..12 {
            live.tick();
        }
        let snap = live.take_snapshot();
        let events_up_to_snapshot: Vec<_> = live
            .event_store()
            .all_records()
            .iter()
            .take(snap.log_index as usize)
            .map(|r| r.event.clone())
            .collect();

        for _ in 0..4 {
            live.tick();
        }
        let live_final = live.take_snapshot();

        let mut store2 = InMemoryEventStore::new();
        for e in events_up_to_snapshot {
            store2.append(e);
        }
        let mut restored = SimulationNode::restore_from(
            store2,
            &snap,
            &modules::all_modules(),
            &ship_types::all_ship_types(),
        );
        for _ in 0..4 {
            restored.tick();
        }
        let restored_final = restored.take_snapshot();

        assert_eq!(
            postcard::to_stdvec(&live_final).unwrap(),
            postcard::to_stdvec(&restored_final).unwrap(),
            "snapshot + tail tick re-execution must match live, including capacitor"
        );
    }

    /// INV-002 / ADR-0017 (8A-4) — operational recovery does NOT require genesis
    /// replay. After compacting the hot log behind the snapshot, reopening and
    /// restoring from the snapshot still reproduces the live state.
    #[test]
    fn recovery_does_not_require_genesis_replay_after_compaction() {
        let dir = tempfile::tempdir().unwrap();
        let hot = dir.path().join("events.log");
        let cold = dir.path().join("cold.log");

        let snap;
        let live_final;
        {
            let store = FileEventStore::open(&hot).unwrap();
            let mut node = SimulationNode::with_store(
                NodeId(0),
                SectorId(0),
                SectorBounds::centered(SectorBounds::DEFAULT_HALF),
                store,
            );
            for i in 0..3u64 {
                node.spawn_ship(
                    dawn_core::ShipTypeId(1),
                    Position::new(i as f32 * 100.0, 0.0, 0.0),
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

        {
            let mut store = FileEventStore::open(&hot).unwrap();
            store.compact(snap.log_index, &cold).unwrap();
            assert_eq!(store.base_index(), snap.log_index);
            assert_eq!(
                store.records_on_disk(),
                0,
                "events behind the snapshot are archived, not in the hot log"
            );
        }

        let store2 = FileEventStore::open(&hot).unwrap();
        assert_eq!(
            store2.base_index(),
            snap.log_index,
            "hot log holds no genesis events"
        );
        let mut restored = SimulationNode::restore_from(store2, &snap, &[], &[]);
        assert_eq!(restored.current_tick(), snap.tick);
        for _ in 0..4 {
            restored.tick();
        }
        let restored_final = restored.take_snapshot();

        assert_eq!(
            postcard::to_stdvec(&live_final).unwrap(),
            postcard::to_stdvec(&restored_final).unwrap(),
            "recovery from snapshot + bounded hot tail (no genesis) must match live"
        );
    }

    #[test]
    fn hull_capacitor_and_fitting_state_are_restored_from_snapshot() {
        use crate::{modules, ship_types};
        use dawn_core::events::{DamageTaken, ShipFitted};
        use dawn_core::fitting::{FittingSnapshot, SlotEntry};
        use dawn_core::DomainEvent;

        let dir = tempfile::tempdir().unwrap();
        let event_path = dir.path().join("events.log");
        let snapshot_path = dir.path().join("snapshot.bin");

        let ship_id: ShipId;
        {
            let store = FileEventStore::open(&event_path).unwrap();
            let mut node = SimulationNode::with_store(
                NodeId(0),
                SectorId(0),
                SectorBounds::centered(SectorBounds::DEFAULT_HALF),
                store,
            );
            for def in modules::all_modules() {
                node.register_module(def);
            }
            for def in ship_types::all_ship_types() {
                node.register_ship_type(def);
            }

            ship_id = node.spawn_ship(
                ship_types::SHIP_TYPE_MAGPIE,
                Position::ORIGIN,
                Velocity::ZERO,
            );

            node.apply_event_pub(DomainEvent::DamageTaken(DamageTaken {
                ship_id,
                damage: 50.0,
                current_shield: 30.0,
                current_armor: 90.0,
                current_hull: 100.0,
                tick: Tick(1),
            }));

            node.apply_event_pub(DomainEvent::ShipFitted(ShipFitted {
                ship_id,
                fitting: FittingSnapshot {
                    high: vec![],
                    mid: vec![SlotEntry {
                        module_id: modules::MODULE_AFTERBURNER,
                        is_active: true,
                    }],
                    low: vec![],
                    rig: vec![],
                },
                inventory: vec![],
                tick: Tick(1),
            }));

            let snap = node.take_snapshot();
            snap.save(&snapshot_path).unwrap();
        }

        let snap = StateSnapshot::load(&snapshot_path).unwrap();
        let store2 = FileEventStore::open(&event_path).unwrap();
        let node2 = SimulationNode::restore_from(
            store2,
            &snap,
            &modules::all_modules(),
            &ship_types::all_ship_types(),
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

    /// INV-002 / ADR-0029 — a warp-to-body rebases the ship onto the body's
    /// anchor via an authoritative `AnchorRebased` event, leaving its raw
    /// `PositionComp` body-relative. The snapshot must capture that anchor (not
    /// just the offset) so restore reproduces the same *absolute* position; a
    /// snapshot that dropped the anchor would silently relocate the ship by the
    /// body's absolute position (~10^5+ units) on restart. This is the
    /// new-schema check the pre-anchor round-trip tests can't make.
    #[test]
    fn warp_arrival_anchor_and_absolute_position_survive_snapshot_restore() {
        use crate::{modules, ship_types};
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

        // Snapshot + restore from the event log (which includes AnchorRebased).
        let snap = node.take_snapshot();
        let mut store2 = InMemoryEventStore::new();
        for rec in node.event_store().all_records() {
            store2.append(rec.event.clone());
        }
        let node2 = SimulationNode::restore_from(
            store2,
            &snap,
            &modules::all_modules(),
            &ship_types::all_ship_types(),
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

    /// ADR-0038: Station inventory now survives a restart because it lives in
    /// its own durable SQLite file, independent of the snapshot/event-log
    /// lifecycle -- not because `take_snapshot()`/`restore_from()` carry it
    /// (they don't, going forward). Simulates a real restart: `node` and
    /// `node2` are otherwise-independent `SimulationNode`s (fresh in-memory
    /// event store, like a real process restart would use a fresh
    /// `FileEventStore` handle), but both point `open_station_inventory_db`
    /// at the same on-disk file.
    #[test]
    fn station_inventory_survives_snapshot_restore() {
        use crate::{modules, ship_types};
        use dawn_core::{ItemId, PlayerId, StationId};

        let db_path = tempfile::NamedTempFile::new().unwrap();
        let db_path = db_path.path().to_str().unwrap();

        let mut node = node_with_modules();
        node.open_station_inventory_db(db_path).unwrap();
        node.credit_station_item(PlayerId(7), StationId(0), ItemId::ScrapMetal, 4);
        node.credit_station_item(
            PlayerId(7),
            StationId(0),
            ItemId::PackagedShip(dawn_core::ShipTypeId(1)),
            1,
        );

        let snap = node.take_snapshot();
        let mut store2 = InMemoryEventStore::new();
        for rec in node.event_store().all_records() {
            store2.append(rec.event.clone());
        }
        let mut node2 = SimulationNode::restore_from(
            store2,
            &snap,
            &modules::all_modules(),
            &ship_types::all_ship_types(),
        );
        node2.open_station_inventory_db(db_path).unwrap();

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

    /// ADR-0038 back-compat: a `StateSnapshot` taken before this change still
    /// carries a populated `station_inventories` field. `restore_from` must
    /// migrate it into the (fresh, in-memory) SQLite database once.
    #[test]
    fn restore_from_migrates_a_pre_adr_0038_snapshots_station_inventories() {
        use crate::{modules, ship_types};
        use dawn_core::{ItemId, PlayerId, StationId};

        let node = node_with_modules();
        let mut snap = node.take_snapshot();
        snap.station_inventories =
            BTreeMap::from([(PlayerId(7), BTreeMap::from([(ItemId::ScrapMetal, 9)]))]);

        let store2 = InMemoryEventStore::new();
        let node2 = SimulationNode::restore_from(
            store2,
            &snap,
            &modules::all_modules(),
            &ship_types::all_ship_types(),
        );

        assert_eq!(
            node2.station_item_count(PlayerId(7), StationId(0), ItemId::ScrapMetal),
            9
        );
    }

    #[test]
    fn docked_station_state_survives_snapshot_restore() {
        use crate::{modules, ship_types};
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
        let mut store2 = InMemoryEventStore::new();
        for rec in node.event_store().all_records() {
            store2.append(rec.event.clone());
        }
        let node2 = SimulationNode::restore_from(
            store2,
            &snap,
            &modules::all_modules(),
            &ship_types::all_ship_types(),
        );

        assert_eq!(node2.docked_station(ship_id), Some(StationId(0)));
        assert_eq!(node2.player_docked_station(player_id), Some(StationId(0)));
    }
}
