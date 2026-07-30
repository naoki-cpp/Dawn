from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    return text.replace(old, new, 1)


def edit(path: str, transform) -> None:
    file = Path(path)
    old = file.read_text()
    new = transform(old)
    if new == old:
        raise SystemExit(f"{path}: no change")
    file.write_text(new)


def patch_snapshot(text: str) -> str:
    text = replace_once(
        text,
        "use std::collections::BTreeMap;",
        "use std::collections::{BTreeMap, BTreeSet};",
        "snapshot imports",
    )
    marker = "// ── Node-level snapshot ───────────────────────────────────────────────────────\n"
    receipt = """/// Durable destination-side receipt for an imported Sector Transit.\n///\n/// Ship presence is not a valid deduplication marker because the imported Ship\n/// may be destroyed or transit onward before an old Commit retry arrives.\n#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]\npub struct CompletedIncomingTransit {\n    pub ship_id: ShipId,\n    pub from: SectorId,\n    pub to: SectorId,\n    pub request_tick: Tick,\n}\n\n"""
    text = replace_once(text, marker, receipt + marker, "receipt type")
    text = replace_once(
        text,
        "    /// Current docked station per ship.\n    pub docked_ships:",
        "    /// Destination-side receipts that survive checkpoint compaction.\n"
        "    #[serde(default)]\n"
        "    pub completed_incoming_transits: BTreeSet<CompletedIncomingTransit>,\n"
        "    /// Current docked station per ship.\n"
        "    pub docked_ships:",
        "snapshot receipt field",
    )
    # Add an empty/default field to every StateSnapshot literal in this file.
    text = text.replace(
        "            docked_ships: BTreeMap::from([",
        "            completed_incoming_transits: BTreeSet::new(),\n"
        "            docked_ships: BTreeMap::from([",
    )
    return text


edit("crates/dawn-sector/src/persistence/snapshot.rs", patch_snapshot)


def patch_persistence_mod(text: str) -> str:
    return replace_once(
        text,
        "pub use snapshot::{ShipSnapshot, StateSnapshot};",
        "pub use snapshot::{CompletedIncomingTransit, ShipSnapshot, StateSnapshot};",
        "snapshot export",
    )


edit("crates/dawn-sector/src/persistence/mod.rs", patch_persistence_mod)


def patch_node_mod(text: str) -> str:
    text = replace_once(
        text,
        "use crate::persistence::StateSnapshot;",
        "use crate::persistence::{CompletedIncomingTransit, StateSnapshot};",
        "node receipt import",
    )
    text = replace_once(
        text,
        "    completed_warps: Vec<ShipId>,\n}",
        "    completed_warps: Vec<ShipId>,\n"
        "    /// Durable destination-side transit receipts used for Commit deduplication.\n"
        "    completed_incoming_transits: std::collections::BTreeSet<CompletedIncomingTransit>,\n"
        "}",
        "node receipt field",
    )
    text = replace_once(
        text,
        "            completed_warps: Vec::new(),\n        }",
        "            completed_warps: Vec::new(),\n"
        "            completed_incoming_transits: std::collections::BTreeSet::new(),\n"
        "        }",
        "node receipt init",
    )
    return text


edit("crates/dawn-sector/src/node/mod.rs", patch_node_mod)


def patch_snapshot_io(text: str) -> str:
    text = replace_once(
        text,
        "            docked_players,\n            // `log_index` is derived",
        "            docked_players,\n"
        "            completed_incoming_transits,\n"
        "            // `log_index` is derived",
        "take_snapshot destructure",
    )
    text = replace_once(
        text,
        "            docked_players: docked_players.clone(),\n        }",
        "            docked_players: docked_players.clone(),\n"
        "            completed_incoming_transits: completed_incoming_transits.clone(),\n"
        "        }",
        "take_snapshot output",
    )
    text = replace_once(
        text,
        "            docked_players,\n            // Consumed by `restore_from`",
        "            docked_players,\n"
        "            completed_incoming_transits,\n"
        "            // Consumed by `restore_from`",
        "apply_snapshot destructure",
    )
    text = replace_once(
        text,
        "        self.docked_players = docked_players.clone();\n",
        "        self.docked_players = docked_players.clone();\n"
        "        self.completed_incoming_transits = completed_incoming_transits.clone();\n",
        "apply_snapshot receipt assignment",
    )
    text = text.replace(
        "            docked_ships: std::collections::BTreeMap::from([",
        "            completed_incoming_transits: std::collections::BTreeSet::new(),\n"
        "            docked_ships: std::collections::BTreeMap::from([",
    )
    return text


edit("crates/dawn-sector/src/node/snapshot_io.rs", patch_snapshot_io)


def patch_transit_flow(text: str) -> str:
    text = replace_once(
        text,
        "use crate::persistence::ShipSnapshot;",
        "use crate::persistence::{CompletedIncomingTransit, ShipSnapshot};",
        "transit receipt import",
    )
    anchor = "impl<S: EventStore> SimulationNode<S> {\n"
    methods = """impl<S: EventStore> SimulationNode<S> {\n    pub(crate) fn has_completed_incoming_transit(\n        &self,\n        ship_id: ShipId,\n        from: SectorId,\n        to: SectorId,\n        request_tick: Tick,\n    ) -> bool {\n        self.completed_incoming_transits\n            .contains(&CompletedIncomingTransit { ship_id, from, to, request_tick })\n    }\n\n    fn record_completed_incoming_transit(\n        &mut self,\n        ship_id: ShipId,\n        from: SectorId,\n        to: SectorId,\n        request_tick: Tick,\n    ) {\n        self.completed_incoming_transits\n            .insert(CompletedIncomingTransit { ship_id, from, to, request_tick });\n    }\n\n    fn replayed_transit_request_tick(\n        &self,\n        ship_id: ShipId,\n        from: SectorId,\n        to: SectorId,\n    ) -> Option<Tick> {\n        self.event_store\n            .iter_from(0)\n            .filter_map(|record| match &record.event {\n                DomainEvent::SectorTransitRequested(event)\n                    if event.ship_id == ship_id && event.from == from && event.to == to =>\n                {\n                    Some(event.request_tick)\n                }\n                _ => None,\n            })\n            .last()\n    }\n\n"""
    text = replace_once(text, anchor, methods, "receipt methods")

    # Destination commit path receives the request identity from the Raft Commit.
    old_signature = """        entry_pos_abs: dawn_core::AbsolutePosition,\n        gate_id: Option<JumpGateId>,\n    ) {\n        let ship_id = ship.ship_id;\n        self.import_transit(ship, from, entry_pos, entry_pos_abs);"""
    new_signature = """        entry_pos_abs: dawn_core::AbsolutePosition,\n        gate_id: Option<JumpGateId>,\n        request_tick: Tick,\n    ) {\n        let ship_id = ship.ship_id;\n        self.record_completed_incoming_transit(\n            ship_id,\n            from,\n            self.sector_id,\n            request_tick,\n        );\n        self.import_transit(ship, from, entry_pos, entry_pos_abs);"""
    text = replace_once(text, old_signature, new_signature, "commit handler identity")

    # Tail replay reconstructs the durable receipt from the matching Requested event.
    replay_old = "        } else if self.sector_id == e.to && !self.ships.index.contains_key(&e.ship_id) {"
    replay_new = """        } else if self.sector_id == e.to {\n            if let Some(request_tick) =\n                self.replayed_transit_request_tick(e.ship_id, e.from, e.to)\n            {\n                self.record_completed_incoming_transit(\n                    e.ship_id,\n                    e.from,\n                    e.to,\n                    request_tick,\n                );\n            }\n            if self.ships.index.contains_key(&e.ship_id) {\n                if e.tick > self.current_tick {\n                    self.current_tick = e.tick;\n                }\n                return;\n            }"""
    text = replace_once(text, replay_old, replay_new, "completed replay receipt")
    return text


edit("crates/dawn-sector/src/node/transit_flow.rs", patch_transit_flow)


def patch_transit(text: str) -> str:
    text = replace_once(
        text,
        "    let mut marker_seen = false;\n",
        "    if node.has_completed_incoming_transit(ship_id, from, to, request_tick) {\n"
        "        return true;\n"
        "    }\n"
        "    let mut marker_seen = false;\n",
        "durable receipt lookup",
    )
    text = replace_once(
        text,
        "                        node.handle_transit_commit(&ship, from, entry_pos, entry_pos_abs, gate_id);",
        "                        node.handle_transit_commit(\n"
        "                            &ship,\n"
        "                            from,\n"
        "                            entry_pos,\n"
        "                            entry_pos_abs,\n"
        "                            gate_id,\n"
        "                            request_tick,\n"
        "                        );",
        "commit call identity",
    )

    needle = """    #[test]\n    fn decode_returns_none_for_garbage_payload() {"""
    test = """    #[test]\n    fn duplicate_commit_after_checkpoint_does_not_resurrect_removed_ship() {\n        let mut source = node(0, 0);\n        let mut destination = node(1, 1);\n        let ship_id = source.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);\n        let data = source\n            .prepare_transit_commit(ship_id, SectorId(1), None)\n            .unwrap();\n        let commit = TransitOp::Commit {\n            ship: data.ship,\n            from: SectorId(0),\n            to: SectorId(1),\n            entry_pos: data.entry_pos,\n            entry_pos_abs: data.entry_pos_abs,\n            gate_id: None,\n            request_tick: data.request_tick,\n        };\n\n        let (raft, mut proposals) = raft_handle();\n        let (tx, mut rx) = mpsc::unbounded_channel();\n        tx.send(commit.encode()).unwrap();\n        apply_committed_raft_entries(&mut destination, &raft, &mut rx);\n        assert!(matches!(\n            decode_proposed_transit(&mut proposals),\n            TransitOp::Ack { .. }\n        ));\n\n        let mut checkpoint = destination.take_snapshot();\n        checkpoint.ships.retain(|ship| ship.ship_id != ship_id);\n        let mut restored =\n            SimulationNode::restore_from(InMemoryEventStore::new(), &checkpoint, &[], &[]);\n        assert!(restored.get_ship_position(ship_id).is_none());\n\n        let (dup_tx, mut dup_rx) = mpsc::unbounded_channel();\n        dup_tx.send(commit.encode()).unwrap();\n        apply_committed_raft_entries(&mut restored, &raft, &mut dup_rx);\n\n        assert!(matches!(\n            decode_proposed_transit(&mut proposals),\n            TransitOp::Ack { .. }\n        ));\n        assert!(restored.get_ship_position(ship_id).is_none());\n        assert_eq!(restored.event_store().len(), 0);\n    }\n\n"""
    text = replace_once(text, needle, test + needle, "resurrection regression test")
    return text


edit("crates/dawn-sector/src/transit.rs", patch_transit)
