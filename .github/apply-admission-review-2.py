from pathlib import Path
import re


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text()
    if text.count(old) != 1:
        raise SystemExit(f"{path}: expected one exact match, found {text.count(old)}")
    file.write_text(text.replace(old, new, 1))


def replace_regex(path: str, pattern: str, replacement: str) -> None:
    file = Path(path)
    text = file.read_text()
    updated, count = re.subn(pattern, replacement, text, count=1, flags=re.S)
    if count != 1:
        raise SystemExit(f"{path}: expected one regex match, found {count}")
    file.write_text(updated)


# Durable allocation watermark event. Append the enum variant at the end so
# existing postcard discriminants remain stable.
events = "crates/dawn-core/src/events.rs"
replace_once(
    events,
    "use crate::{AbsolutePosition, Position, SectorId, ShipId, Tick, Velocity};",
    "use crate::{AbsolutePosition, PlayerId, Position, SectorId, ShipId, Tick, Velocity};",
)
replace_once(
    events,
    "    ShipAssembled(ShipAssembled),\n}",
    "    ShipAssembled(ShipAssembled),\n\n"
    "    /// A fresh client admission durably consumed a PlayerId/ShipId pair.\n"
    "    /// No Ship is materialized by this event; replay only advances the\n"
    "    /// allocation watermarks so identities are never reused after a crash.\n"
    "    ClientAdmissionIdentityReserved(ClientAdmissionIdentityReserved),\n}",
)
replace_once(
    events,
    "            Self::ShipAssembled(e) => e.ship_id,\n",
    "            Self::ShipAssembled(e) => e.ship_id,\n"
    "            Self::ClientAdmissionIdentityReserved(e) => e.ship_id,\n",
)
replace_once(
    events,
    "            Self::ShipAssembled(e) => e.tick,\n",
    "            Self::ShipAssembled(e) => e.tick,\n"
    "            Self::ClientAdmissionIdentityReserved(e) => e.tick,\n",
)
replace_once(
    events,
    "pub struct ShipSpawned {\n"
    "    pub ship_id: ShipId,\n"
    "    pub sector_id: SectorId,\n"
    "    /// Authoritative Sector-frame spawn position.\n"
    "    pub initial_position: AbsolutePosition,\n"
    "    /// 船種 ID。Replay 時に base_stats を復元するために必須（INV-002）。\n"
    "    pub ship_type_id: ShipTypeId,\n"
    "    pub tick: Tick,\n"
    "}\n",
    "pub struct ShipSpawned {\n"
    "    pub ship_id: ShipId,\n"
    "    pub sector_id: SectorId,\n"
    "    /// Authoritative Sector-frame spawn position.\n"
    "    pub initial_position: AbsolutePosition,\n"
    "    /// 船種 ID。Replay 時に base_stats を復元するために必須（INV-002）。\n"
    "    pub ship_type_id: ShipTypeId,\n"
    "    pub tick: Tick,\n"
    "}\n\n"
    "/// Durable allocation watermark for one fresh client admission.\n"
    "#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\n"
    "pub struct ClientAdmissionIdentityReserved {\n"
    "    pub player_id: PlayerId,\n"
    "    pub ship_id: ShipId,\n"
    "    pub tick: Tick,\n"
    "}\n",
)

# Node-local in-flight reservations.
node_mod = "crates/dawn-sector/src/node/mod.rs"
replace_once(
    node_mod,
    "    pending_fresh_admissions: HashSet<ShipId>,\n",
    "    pending_fresh_admissions: HashSet<ShipId>,\n"
    "    /// Ship-level lock held by an in-flight resume handshake.\n"
    "    /// Non-durable: authoritative ownership is still unchanged until commit.\n"
    "    pending_resume_admissions: HashMap<ShipId, PlayerId>,\n",
)
replace_once(
    node_mod,
    "            pending_fresh_admissions: HashSet::new(),\n",
    "            pending_fresh_admissions: HashSet::new(),\n"
    "            pending_resume_admissions: HashMap::new(),\n",
)

snapshot = "crates/dawn-sector/src/node/snapshot_io.rs"
replace_once(
    snapshot,
    "            pending_fresh_admissions: _,\n",
    "            pending_fresh_admissions: _,\n"
    "            // Resume locks protect only live asynchronous handshakes.\n"
    "            pending_resume_admissions: _,\n",
)

provisional = "crates/dawn-sector/src/node/admission_provisional.rs"
replace_once(
    provisional,
    "    events::ShipSpawned, fitting::ActivationMode, DomainEvent, ItemId, PlayerId, Position, ShipId,\n",
    "    events::{ClientAdmissionIdentityReserved, ShipSpawned}, fitting::ActivationMode, DomainEvent,\n"
    "    ItemId, PlayerId, Position, ShipId,\n",
)
replace_once(
    provisional,
    "    pub(crate) fn reserve_fresh_admission_identity(&mut self) -> (PlayerId, ShipId) {\n"
    "        let player_id = self.next_player_id();\n"
    "        let ship_id = ShipId::new(self.node_id, self.id_counter);\n"
    "        self.id_counter += 1;\n"
    "        let inserted = self.pending_fresh_admissions.insert(ship_id);\n"
    "        debug_assert!(\n"
    "            inserted,\n"
    "            \"fresh admission ShipId reservation must be unique\"\n"
    "        );\n"
    "        (player_id, ship_id)\n"
    "    }\n",
    "    pub(crate) fn reserve_fresh_admission_identity(&mut self) -> (PlayerId, ShipId) {\n"
    "        let player_id = self.next_player_id();\n"
    "        let ship_id = ShipId::new(self.node_id, self.id_counter);\n"
    "        self.id_counter += 1;\n"
    "        let inserted = self.pending_fresh_admissions.insert(ship_id);\n"
    "        debug_assert!(\n"
    "            inserted,\n"
    "            \"fresh admission ShipId reservation must be unique\"\n"
    "        );\n"
    "        self.event_store\n"
    "            .append(DomainEvent::ClientAdmissionIdentityReserved(\n"
    "                ClientAdmissionIdentityReserved {\n"
    "                    player_id,\n"
    "                    ship_id,\n"
    "                    tick: self.current_tick,\n"
    "                },\n"
    "            ));\n"
    "        (player_id, ship_id)\n"
    "    }\n",
)
replace_once(
    provisional,
    "    fn materialize_admission_player_ship(\n",
    "    /// Acquire a Ship-level lock for one in-flight resume handshake.\n"
    "    pub(crate) fn reserve_resume_admission(\n"
    "        &mut self,\n"
    "        player_id: PlayerId,\n"
    "        ship_id: ShipId,\n"
    "    ) -> bool {\n"
    "        if !self.ships.index.contains_key(&ship_id)\n"
    "            || self.pending_resume_admissions.contains_key(&ship_id)\n"
    "        {\n"
    "            return false;\n"
    "        }\n"
    "        self.pending_resume_admissions.insert(ship_id, player_id);\n"
    "        true\n"
    "    }\n\n"
    "    pub(crate) fn release_resume_admission(\n"
    "        &mut self,\n"
    "        player_id: PlayerId,\n"
    "        ship_id: ShipId,\n"
    "    ) {\n"
    "        if self.pending_resume_admissions.get(&ship_id) == Some(&player_id) {\n"
    "            self.pending_resume_admissions.remove(&ship_id);\n"
    "        }\n"
    "    }\n\n"
    "    pub(crate) fn commit_reserved_resume_admission(\n"
    "        &mut self,\n"
    "        player_id: PlayerId,\n"
    "        ship_id: ShipId,\n"
    "    ) -> bool {\n"
    "        if self.pending_resume_admissions.get(&ship_id) != Some(&player_id) {\n"
    "            return false;\n"
    "        }\n"
    "        self.pending_resume_admissions.remove(&ship_id);\n"
    "        self.resume_player_ship(ship_id, player_id)\n"
    "    }\n\n"
    "    fn materialize_admission_player_ship(\n",
)

admission = "crates/dawn-sector/src/client_admission.rs"
replace_once(
    admission,
    "    ResumeShipMissing {\n        player_id: PlayerId,\n        ship_id: ShipId,\n    },\n",
    "    ResumeShipMissing {\n        player_id: PlayerId,\n        ship_id: ShipId,\n    },\n"
    "    /// Another handshake already holds the Ship-level resume lock.\n"
    "    ResumeAlreadyPending {\n"
    "        player_id: PlayerId,\n"
    "        ship_id: ShipId,\n"
    "    },\n",
)
replace_once(
    admission,
    "            Self::MissingObserver(error) => write!(f, \"{error}\"),\n",
    "            Self::ResumeAlreadyPending { player_id, ship_id } => write!(\n"
    "                f,\n"
    "                \"resume refused for {player_id}: ship #{} already has an in-flight resume\",\n"
    "                ship_id.raw()\n"
    "            ),\n"
    "            Self::MissingObserver(error) => write!(f, \"{error}\"),\n",
)
replace_once(
    admission,
    "            AdmissionOrigin::Resume => node.resume_player_ship(self.ship_id, self.player_id),\n",
    "            AdmissionOrigin::Resume => {\n"
    "                node.commit_reserved_resume_admission(self.player_id, self.ship_id)\n"
    "            }\n",
)
replace_once(
    admission,
    "        if !present {\n"
    "            if matches!(self.origin, AdmissionOrigin::Fresh { .. }) {\n"
    "                node.abort_reserved_fresh_admission(self.ship_id);\n"
    "            }\n",
    "        if !present {\n"
    "            match self.origin {\n"
    "                AdmissionOrigin::Fresh { .. } => {\n"
    "                    node.abort_reserved_fresh_admission(self.ship_id);\n"
    "                }\n"
    "                AdmissionOrigin::Resume => {\n"
    "                    node.release_resume_admission(self.player_id, self.ship_id);\n"
    "                }\n"
    "            }\n",
)
replace_once(
    admission,
    "    pub fn abort<S: EventStore>(self, node: &mut SimulationNode<S>) {\n"
    "        if matches!(self.origin, AdmissionOrigin::Fresh { .. }) {\n"
    "            node.abort_reserved_fresh_admission(self.ship_id);\n"
    "        }\n"
    "    }\n",
    "    pub fn abort<S: EventStore>(self, node: &mut SimulationNode<S>) {\n"
    "        match self.origin {\n"
    "            AdmissionOrigin::Fresh { .. } => {\n"
    "                node.abort_reserved_fresh_admission(self.ship_id);\n"
    "            }\n"
    "            AdmissionOrigin::Resume => {\n"
    "                node.release_resume_admission(self.player_id, self.ship_id);\n"
    "            }\n"
    "        }\n"
    "    }\n",
)
replace_regex(
    admission,
    r"            ClientAdmissionIntent::Resume \{ player_id, ship_id \} => \{.*?\n            \}\n        \}\n    \}\n\}",
    """            ClientAdmissionIntent::Resume { player_id, ship_id } => {
                // ADR-0007: validate the exact requested Ship and never fall
                // back to a fresh spawn. A Ship-level reservation serializes
                // concurrent resume handshakes until this attempt commits or aborts.
                if self.ship_absolute_pos(ship_id).is_none() {
                    return Err(ClientAdmissionRefusal::ResumeShipMissing {
                        player_id,
                        ship_id,
                    });
                }
                if !self.reserve_resume_admission(player_id, ship_id) {
                    return Err(ClientAdmissionRefusal::ResumeAlreadyPending {
                        player_id,
                        ship_id,
                    });
                }

                let result = (|| {
                    let mut handoff = self
                        .build_handoff_payload(ship_id, aoi_cell_size)
                        .map_err(|_| ClientAdmissionRefusal::ResumeShipMissing {
                            player_id,
                            ship_id,
                        })?;

                    if let Some(observer) = handoff
                        .initial_state
                        .ships
                        .iter_mut()
                        .find(|ship| ship.ship_id == ship_id.raw())
                    {
                        observer.is_player = true;
                    }

                    let loadout = self
                        .build_player_loadout_json_for_admission(player_id, ship_id)
                        .ok_or(ClientAdmissionRefusal::ResumeShipMissing {
                            player_id,
                            ship_id,
                        })?;
                    handoff.player_loadout = Some(loadout);
                    Ok(handoff)
                })();

                match result {
                    Ok(handoff) => Ok(ClientAdmissionAttempt {
                        player_id,
                        ship_id,
                        origin: AdmissionOrigin::Resume,
                        handoff: Some(handoff),
                    }),
                    Err(refusal) => {
                        self.release_resume_admission(player_id, ship_id);
                        Err(refusal)
                    }
                }
            }
        }
    }
}""",
)
# Watermark event is now the only durable state during fresh begin.
replace_once(
    admission,
    "        assert!(node.event_store().all_records().is_empty());\n\n"
    "        let committed = attempt.commit(&mut node).expect(\"fresh commit\");\n",
    "        assert!(matches!(\n"
    "            node.event_store().all_records(),\n"
    "            [record] if matches!(record.event, DomainEvent::ClientAdmissionIdentityReserved(_))\n"
    "        ));\n\n"
    "        let committed = attempt.commit(&mut node).expect(\"fresh commit\");\n",
)
replace_once(
    admission,
    "        assert!(node.event_store().all_records().is_empty());\n\n"
    "        attempt.abort(&mut node);\n\n"
    "        assert_eq!(node.ship_count(), 0);\n"
    "        assert!(node.event_store().all_records().is_empty());\n",
    "        assert!(matches!(\n"
    "            node.event_store().all_records(),\n"
    "            [record] if matches!(record.event, DomainEvent::ClientAdmissionIdentityReserved(_))\n"
    "        ));\n\n"
    "        attempt.abort(&mut node);\n\n"
    "        assert_eq!(node.ship_count(), 0);\n"
    "        assert_eq!(node.event_store().all_records().len(), 1);\n",
)
replace_once(
    admission,
    "    #[test]\n    fn missing_resume_never_falls_back_to_fresh_spawn() {\n",
    "    #[test]\n"
    "    fn concurrent_resume_attempts_for_one_ship_are_serialized() {\n"
    "        let mut node = node();\n"
    "        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);\n"
    "        let first_player = PlayerId(12);\n"
    "        let second_player = PlayerId(13);\n"
    "        let first = node\n"
    "            .begin_client_admission(\n"
    "                ClientAdmissionIntent::Resume {\n"
    "                    player_id: first_player,\n"
    "                    ship_id,\n"
    "                },\n"
    "                AOI_CELL_SIZE,\n"
    "            )\n"
    "            .expect(\"first resume obtains the Ship lock\");\n\n"
    "        let second = node\n"
    "            .begin_client_admission(\n"
    "                ClientAdmissionIntent::Resume {\n"
    "                    player_id: second_player,\n"
    "                    ship_id,\n"
    "                },\n"
    "                AOI_CELL_SIZE,\n"
    "            )\n"
    "            .expect_err(\"second concurrent resume must be refused\");\n"
    "        assert_eq!(\n"
    "            second,\n"
    "            ClientAdmissionRefusal::ResumeAlreadyPending {\n"
    "                player_id: second_player,\n"
    "                ship_id,\n"
    "            }\n"
    "        );\n\n"
    "        first.abort(&mut node);\n"
    "        let retry = node\n"
    "            .begin_client_admission(\n"
    "                ClientAdmissionIntent::Resume {\n"
    "                    player_id: second_player,\n"
    "                    ship_id,\n"
    "                },\n"
    "                AOI_CELL_SIZE,\n"
    "            )\n"
    "            .expect(\"abort releases the Ship-level resume lock\");\n"
    "        retry.abort(&mut node);\n"
    "    }\n\n"
    "    #[test]\n    fn missing_resume_never_falls_back_to_fresh_spawn() {\n",
)
# Bring DomainEvent into the test module for the reservation assertion.
replace_once(
    admission,
    "    use dawn_core::{NodeId, SectorBounds, SectorId, ShipTypeId, Velocity};\n",
    "    use dawn_core::{DomainEvent, NodeId, SectorBounds, SectorId, ShipTypeId, Velocity};\n",
)

apply_event = "crates/dawn-sector/src/node/apply_event.rs"
replace_once(
    apply_event,
    "        match event {\n            DomainEvent::ShipSpawned(e) => {\n",
    "        match event {\n"
    "            DomainEvent::ClientAdmissionIdentityReserved(e) => {\n"
    "                self.player_id_counter = self.player_id_counter.max(e.player_id.0 + 1);\n"
    "                self.id_counter = self.id_counter.max(e.ship_id.0.counter() + 1);\n"
    "            }\n\n"
    "            DomainEvent::ShipSpawned(e) => {\n",
)

wire = "crates/dawn-wire/src/server_event.rs"
replace_once(
    wire,
    "        DomainEvent::ShipDisassembled(_) => return None,\n",
    "        DomainEvent::ShipDisassembled(_) => return None,\n"
    "        DomainEvent::ClientAdmissionIdentityReserved(_) => return None,\n",
)

# Runtime refusal logging for the explicit concurrency result.
production = "crates/dawn-sector-node/src/client_admission.rs"
replace_once(
    production,
    "                Err(ClientAdmissionRefusal::FreshAtPopulationCap) => {\n",
    "                Err(ClientAdmissionRefusal::ResumeAlreadyPending { ship_id, .. }) => {\n"
    "                    eprintln!(\n"
    "                        \"[Node] resume refused from {}: ship #{} already has an in-flight resume\",\n"
    "                        request.peer_addr,\n"
    "                        ship_id.raw()\n"
    "                    );\n"
    "                    continue;\n"
    "                }\n"
    "                Err(ClientAdmissionRefusal::FreshAtPopulationCap) => {\n",
)

single = "crates/dawn-simulation/src/serve/single.rs"
replace_once(
    single,
    "        ClientAdmissionRefusal::MissingObserver(error) => {\n",
    "        ClientAdmissionRefusal::ResumeAlreadyPending { ship_id, .. } => {\n"
    "            eprintln!(\n"
    "                \"[Server] resume from {addr} refused: ship #{} already has an in-flight resume\",\n"
    "                ship_id.raw()\n"
    "            );\n"
    "        }\n"
    "        ClientAdmissionRefusal::MissingObserver(error) => {\n",
)
cluster = "crates/dawn-simulation/src/serve/cluster.rs"
replace_once(
    cluster,
    "        ClientAdmissionRefusal::MissingObserver(error) => {\n",
    "        ClientAdmissionRefusal::ResumeAlreadyPending { ship_id, .. } => {\n"
    "            eprintln!(\n"
    "                \"[Server] clustered resume from {addr} refused: ship #{} already has an in-flight resume\",\n"
    "                ship_id.raw()\n"
    "            );\n"
    "        }\n"
    "        ClientAdmissionRefusal::MissingObserver(error) => {\n",
)

replay = "crates/dawn-sector/tests/client_admission_replay.rs"
replace_once(
    replay,
    "use dawn_event_store::InMemoryEventStore;",
    "use dawn_event_store::{store::EventStore, InMemoryEventStore};",
)
replace_regex(
    replay,
    r"#\[test\]\nfn in_flight_fresh_admission_is_absent_from_snapshot_and_event_replay\(\) \{.*?\n\}\n\n#\[test\]",
    """#[test]
fn in_flight_fresh_admission_keeps_only_a_durable_identity_watermark() {
    let galaxy = Arc::new(Galaxy::demo());
    let mut node = SimulationNode::new(
        NodeId(0),
        SectorId(0),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        Arc::clone(&galaxy),
    );
    let pre_begin_snapshot = node.take_snapshot();

    let attempt = node
        .begin_client_admission(
            ClientAdmissionIntent::Fresh {
                spawn_position: Position::ORIGIN,
            },
            AOI_CELL_SIZE,
        )
        .expect("fresh admission should begin");
    let reserved_player_id = attempt.player_id();
    let reserved_ship_id = attempt.ship_id();
    let snapshot_during_handshake = node.take_snapshot();

    assert_eq!(node.ship_count(), 0);
    assert!(snapshot_during_handshake.ships.is_empty());
    assert!(matches!(
        node.event_store().all_records(),
        [record]
            if matches!(
                &record.event,
                dawn_core::DomainEvent::ClientAdmissionIdentityReserved(event)
                    if event.player_id == reserved_player_id && event.ship_id == reserved_ship_id
            )
    ));

    let mut replay_store = InMemoryEventStore::new();
    for record in node.event_store().all_records() {
        replay_store.append(record.event.clone());
    }
    drop(attempt);

    let catalog = repository_catalog();
    let mut restored = SimulationNode::restore_from(
        replay_store,
        &pre_begin_snapshot,
        galaxy,
        catalog.modules(),
        catalog.ship_types(),
    );

    assert_eq!(restored.ship_count(), 0);
    assert!(restored.ship_absolute_pos(reserved_ship_id).is_none());
    let retry = restored
        .begin_client_admission(
            ClientAdmissionIntent::Fresh {
                spawn_position: Position::ORIGIN,
            },
            AOI_CELL_SIZE,
        )
        .expect("restored node may admit a new client");
    assert_ne!(retry.player_id(), reserved_player_id);
    assert_ne!(retry.ship_id(), reserved_ship_id);
    retry.abort(&mut restored);
}

#[test]""",
)

# Documentation.
client_doc = "docs/architecture/client-admission.md"
replace_once(
    client_doc,
    "reservation against the cap. It materializes the Ship only inside the begin call\n"
    "to build observer-scoped `InitialState`/`PlayerLoadout`, then removes that preview\n"
    "before returning. The reservation is non-durable and snapshots never include it.\n",
    "reservation against the cap. The consumed `PlayerId`/`ShipId` watermark is appended\n"
    "durably before any frame can be sent. Begin materializes the Ship only inside the\n"
    "call to build observer-scoped `InitialState`/`PlayerLoadout`, then removes that\n"
    "preview before returning. The in-flight reservation is non-durable and snapshots\n"
    "never include it; the allocation watermark survives through snapshot or event replay.\n",
)
replace_once(
    client_doc,
    "- **Process loss before resolution:** loses the non-durable reservation and\n"
    "  cannot resurrect a Ship from either snapshot or event replay.\n",
    "- **Process loss before resolution:** loses the non-durable reservation and\n"
    "  cannot resurrect a Ship, while the durable watermark prevents either ID\n"
    "  from being issued to a later client.\n",
)
replace_once(
    client_doc,
    "Resume names an exact `(PlayerId, ShipId)`. A missing Ship is refused and never\n"
    "falls back to fresh spawn. Begin validates the Ship and builds the observer-\n",
    "Resume names an exact `(PlayerId, ShipId)`. A missing Ship is refused and never\n"
    "falls back to fresh spawn. Begin first acquires a non-durable Ship-level resume\n"
    "reservation; a concurrent attempt for the same Ship is refused until the first\n"
    "attempt commits or aborts. Begin then validates the Ship and builds the observer-\n",
)

catalog = "docs/architecture/event-catalog.md"
replace_once(
    catalog,
    "| `ShipSpawned` | Ship appeared in the world | `SimulationNode::spawn_ship()` | ✅ implemented |\n",
    "| `ShipSpawned` | Ship appeared in the world | `SimulationNode::spawn_ship()` | ✅ implemented |\n"
    "| `ClientAdmissionIdentityReserved` | Fresh admission durably consumed a `PlayerId`/`ShipId` pair without materializing a Ship; Replay advances allocation watermarks only | `SimulationNode::reserve_fresh_admission_identity()` | ✅ implemented |\n",
)
