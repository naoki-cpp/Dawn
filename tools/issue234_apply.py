from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"{label}: expected one match, found {count}")
    return text.replace(old, new)


# dawn-sector: make observer resolution an explicit failure.
path = Path("crates/dawn-sector/src/node/serialization.rs")
text = path.read_text()
text = replace_once(
    text,
    "#[derive(Debug)]\n"
    "pub struct HandoffPayload {\n"
    "    pub initial_state: InitialStateWire,\n"
    "    pub player_loadout: Option<PlayerLoadoutWire>,\n"
    "}\n",
    "#[derive(Debug)]\n"
    "pub struct HandoffPayload {\n"
    "    pub initial_state: InitialStateWire,\n"
    "    pub player_loadout: Option<PlayerLoadoutWire>,\n"
    "}\n"
    "\n"
    "/// The observer ship needed to scope an InitialState could not be resolved.\n"
    "///\n"
    "/// Network admission, resume, and post-transit handoff must reject this\n"
    "/// condition instead of substituting an empty or full-world payload.\n"
    "#[derive(Debug, Clone, Copy, PartialEq, Eq)]\n"
    "pub struct MissingObserverShip {\n"
    "    pub ship_id: ShipId,\n"
    "}\n"
    "\n"
    "impl std::fmt::Display for MissingObserverShip {\n"
    "    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\n"
    "        write!(f, \"observer ship #{} could not be resolved\", self.ship_id.raw())\n"
    "    }\n"
    "}\n"
    "\n"
    "impl std::error::Error for MissingObserverShip {}\n",
    "observer error type",
)
text = replace_once(
    text,
    "    /// Build the `InitialState` + `PlayerLoadout` pair to hand a client once\n"
    "    /// its identity (fresh or resumed) has already been decided by the caller.\n"
    "    pub fn build_handoff_payload(&self, ship_id: ShipId, aoi_cell_size: f64) -> HandoffPayload {\n"
    "        let initial_state = self\n"
    "            .ship_absolute_pos(ship_id)\n"
    "            .map(|pos| self.build_initial_state_json_for(pos, aoi_cell_size))\n"
    "            .unwrap_or_else(|| self.build_initial_state_json());\n"
    "        let player_loadout = self.build_player_loadout_json(ship_id);\n"
    "        HandoffPayload {\n"
    "            initial_state,\n"
    "            player_loadout,\n"
    "        }\n"
    "    }\n"
    "\n"
    "    /// Full-world `InitialState` (every ship). Used for non-AoI callers.\n"
    "    pub fn build_initial_state_json(&self) -> InitialStateWire {\n",
    "    /// Build the observer-scoped `InitialState` + `PlayerLoadout` pair to\n"
    "    /// hand a client once its identity (fresh or resumed) has been selected.\n"
    "    pub fn build_handoff_payload(\n"
    "        &self,\n"
    "        ship_id: ShipId,\n"
    "        aoi_cell_size: f64,\n"
    "    ) -> Result<HandoffPayload, MissingObserverShip> {\n"
    "        let initial_state = self.build_initial_state_for_observer(ship_id, aoi_cell_size)?;\n"
    "        let player_loadout = self.build_player_loadout_json(ship_id);\n"
    "        Ok(HandoffPayload {\n"
    "            initial_state,\n"
    "            player_loadout,\n"
    "        })\n"
    "    }\n"
    "\n"
    "    /// Build an `InitialState` scoped to `observer_ship`'s 27-cell AoI.\n"
    "    pub fn build_initial_state_for_observer(\n"
    "        &self,\n"
    "        observer_ship: ShipId,\n"
    "        cell_size: f64,\n"
    "    ) -> Result<InitialStateWire, MissingObserverShip> {\n"
    "        let observer_abs = self\n"
    "            .ship_absolute_pos(observer_ship)\n"
    "            .ok_or(MissingObserverShip {\n"
    "                ship_id: observer_ship,\n"
    "            })?;\n"
    "        Ok(self.build_initial_state_json_for(observer_abs, cell_size))\n"
    "    }\n"
    "\n"
    "    /// Full-world state for diagnostics and non-network tests. Admission,\n"
    "    /// resume, and handoff paths must use the observer-scoped builders above.\n"
    "    pub fn build_initial_state_json(&self) -> InitialStateWire {\n",
    "strict handoff builder",
)
text = replace_once(
    text,
    "        let payload = node.build_handoff_payload(ship_id, cell);\n",
    "        let payload = node\n"
    "            .build_handoff_payload(ship_id, cell)\n"
    "            .expect(\"known observer ship\");\n",
    "handoff success test",
)
text = replace_once(
    text,
    "        assert!(\n"
    "            payload.player_loadout.is_some(),\n"
    "            \"every ship with a FittingComp gets a PlayerLoadout payload\"\n"
    "        );\n"
    "    }\n"
    "}\n",
    "        assert!(\n"
    "            payload.player_loadout.is_some(),\n"
    "            \"every ship with a FittingComp gets a PlayerLoadout payload\"\n"
    "        );\n"
    "    }\n"
    "\n"
    "    #[test]\n"
    "    fn handoff_payload_rejects_an_unresolved_observer() {\n"
    "        let node = mem_node();\n"
    "        let missing = ShipId::new(NodeId(9), 999);\n"
    "\n"
    "        let error = node\n"
    "            .build_handoff_payload(missing, 1_000.0)\n"
    "            .expect_err(\"missing observer must not receive full-world state\");\n"
    "\n"
    "        assert_eq!(error, MissingObserverShip { ship_id: missing });\n"
    "    }\n"
    "}\n",
    "handoff missing-observer test",
)
path.write_text(text)

path = Path("crates/dawn-sector/src/node/mod.rs")
text = path.read_text()
text = replace_once(
    text,
    "pub use commands::{ClientCommandFollowup, ModuleActivationRejection};\n"
    "pub use jump::JumpOutcome;\n",
    "pub use commands::{ClientCommandFollowup, ModuleActivationRejection};\n"
    "pub use jump::JumpOutcome;\n"
    "pub use serialization::{HandoffPayload, MissingObserverShip};\n",
    "node serialization exports",
)
path.write_text(text)

# dawn-simulation: fresh single/cluster handshakes reject observer failure.
path = Path("crates/dawn-simulation/src/serve/single.rs")
text = path.read_text()
text = replace_once(
    text,
    "            let payload = node.build_handoff_payload(ship_id, AOI_CELL_SIZE);\n"
    "            let tx = ready_sess_tx.clone();\n",
    "            let payload = match node.build_handoff_payload(ship_id, AOI_CELL_SIZE) {\n"
    "                Ok(payload) => payload,\n"
    "                Err(error) => {\n"
    "                    eprintln!(\n"
    "                        \"[Server] fresh handshake from {addr} refused: {error}\"\n"
    "                    );\n"
    "                    node.despawn_incomplete_handshake_spawn(ship_id);\n"
    "                    drop(stream);\n"
    "                    continue;\n"
    "                }\n"
    "            };\n"
    "            let tx = ready_sess_tx.clone();\n",
    "single fresh handoff",
)
path.write_text(text)

path = Path("crates/dawn-simulation/src/serve/cluster.rs")
text = path.read_text()
text = replace_once(
    text,
    "            let initial_state = match nodes[0].ship_absolute_pos(ship_id) {\n"
    "                Some(pos) => nodes[0].build_initial_state_json_for(pos, AOI_CELL_SIZE),\n"
    "                None => nodes[0].build_initial_state_json(),\n"
    "            };\n"
    "            let player_loadout = nodes[0].build_player_loadout_json(ship_id);\n"
    "            let tx = ready_sess_tx.clone();\n",
    "            let payload = match nodes[0].build_handoff_payload(ship_id, AOI_CELL_SIZE) {\n"
    "                Ok(payload) => payload,\n"
    "                Err(error) => {\n"
    "                    eprintln!(\n"
    "                        \"[Server] clustered fresh handshake from {addr} refused: {error}\"\n"
    "                    );\n"
    "                    nodes[0].despawn_incomplete_handshake_spawn(ship_id);\n"
    "                    drop(stream);\n"
    "                    continue;\n"
    "                }\n"
    "            };\n"
    "            let tx = ready_sess_tx.clone();\n",
    "cluster fresh handoff",
)
text = replace_once(
    text,
    "                    initial_state,\n"
    "                    player_loadout,\n",
    "                    payload.initial_state,\n"
    "                    payload.player_loadout,\n",
    "cluster handshake payload fields",
)
path.write_text(text)

# production admission: selected fresh/resume identities still need a resolvable observer.
path = Path("crates/dawn-sector-node/src/client_admission.rs")
text = path.read_text()
text = replace_once(
    text,
    "use dawn_sector::node::SimulationNode;\n",
    "use dawn_sector::node::{HandoffPayload, MissingObserverShip, SimulationNode};\n",
    "client admission imports",
)
text = replace_once(
    text,
    "            let player_id = handshake_identity.player_id;\n"
    "            let ship_id = handshake_identity.ship_id;\n"
    "            let payload = node.build_handoff_payload(ship_id, aoi_cell_size);\n"
    "            let tx = self.ready_sess_tx.clone();\n",
    "            let player_id = handshake_identity.player_id;\n"
    "            let ship_id = handshake_identity.ship_id;\n"
    "            let payload = match build_handshake_payload(\n"
    "                node,\n"
    "                &handshake_identity,\n"
    "                aoi_cell_size,\n"
    "            ) {\n"
    "                Ok(payload) => payload,\n"
    "                Err(error) => {\n"
    "                    eprintln!(\n"
    "                        \"[Node] handshake refused from {}: {error}\",\n"
    "                        request.peer_addr\n"
    "                    );\n"
    "                    if let Some(ship_id) =\n"
    "                        should_despawn_on_completion_failure(&handshake_identity)\n"
    "                    {\n"
    "                        node.despawn_incomplete_handshake_spawn(ship_id);\n"
    "                    }\n"
    "                    continue;\n"
    "                }\n"
    "            };\n"
    "            let tx = self.ready_sess_tx.clone();\n",
    "strict admission payload",
)
text = replace_once(
    text,
    "fn should_despawn_on_completion_failure(identity: &HandshakeIdentity) -> Option<ShipId> {\n",
    "fn build_handshake_payload<S: EventStore>(\n"
    "    node: &SimulationNode<S>,\n"
    "    identity: &HandshakeIdentity,\n"
    "    aoi_cell_size: f64,\n"
    ") -> Result<HandoffPayload, MissingObserverShip> {\n"
    "    node.build_handoff_payload(identity.ship_id, aoi_cell_size)\n"
    "}\n"
    "\n"
    "fn should_despawn_on_completion_failure(identity: &HandshakeIdentity) -> Option<ShipId> {\n",
    "admission payload helper",
)
text = replace_once(
    text,
    "    #[test]\n"
    "    fn fresh_handshake_respects_population_cap() {\n",
    "    #[test]\n"
    "    fn fresh_handshake_payload_rejects_a_missing_observer() {\n"
    "        let node = test_node();\n"
    "        let identity = HandshakeIdentity {\n"
    "            player_id: PlayerId(1),\n"
    "            ship_id: ShipId::new(NodeId(7), 999),\n"
    "            resumed: false,\n"
    "        };\n"
    "\n"
    "        let error = build_handshake_payload(&node, &identity, 1_000.0)\n"
    "            .expect_err(\"fresh identity without an observer must be refused\");\n"
    "\n"
    "        assert_eq!(error.ship_id, identity.ship_id);\n"
    "    }\n"
    "\n"
    "    #[test]\n"
    "    fn resumed_handshake_payload_rejects_a_missing_observer() {\n"
    "        let node = test_node();\n"
    "        let identity = HandshakeIdentity {\n"
    "            player_id: PlayerId(12),\n"
    "            ship_id: ShipId::new(NodeId(7), 999),\n"
    "            resumed: true,\n"
    "        };\n"
    "\n"
    "        let error = build_handshake_payload(&node, &identity, 1_000.0)\n"
    "            .expect_err(\"resume identity without an observer must be refused\");\n"
    "\n"
    "        assert_eq!(error.ship_id, identity.ship_id);\n"
    "    }\n"
    "\n"
    "    #[test]\n"
    "    fn fresh_handshake_respects_population_cap() {\n",
    "fresh/resume failure tests",
)
path.write_text(text)

# clustered post-transit handoff: disconnect the session when the destination
# cannot resolve the observer rather than sending full-world InitialState.
path = Path("crates/dawn-simulation/src/serve/runtime.rs")
text = path.read_text()
text = replace_once(
    text,
    "use dawn_core::{DomainEvent, PlayerId, ShipId};\n"
    "use dawn_sector::node::SimulationNode;\n"
    "use dawn_sector::transit;\n"
    "use dawn_wire::ServerMessage;\n",
    "use dawn_core::{DomainEvent, PlayerId, ShipId};\n"
    "use dawn_event_store::store::EventStore;\n"
    "use dawn_sector::node::SimulationNode;\n"
    "use dawn_sector::transit;\n"
    "use dawn_wire::{InitialStateWire, ServerMessage};\n",
    "runtime imports",
)
text = replace_once(
    text,
    "    resend_jump_initial_state(ctx.nodes, ctx.sessions, &handoff);\n"
    "    tick_outputs\n",
    "    let failed_players = resend_jump_initial_state(ctx.nodes, ctx.sessions, &handoff);\n"
    "    for player_id in failed_players {\n"
    "        ctx.player_sector.remove(&player_id);\n"
    "    }\n"
    "    tick_outputs\n",
    "post-transit failure cleanup",
)
old_function = '''fn resend_jump_initial_state(
    nodes: &[SimulationNode],
    sessions: &[ws_server::PlayerSession],
    handoff: &JumpHandoff,
) {
    for (player_id, dest) in &handoff.jumped_players {
        if let Some(sess) = sessions.iter().find(|s| s.player_id == *player_id) {
            if let Some(events) = handoff.own_events.get(player_id) {
                sess.send_events(events);
            }
            let initial_state = nodes[*dest]
                .ship_absolute_pos(sess.ship_id)
                .map(|pos| nodes[*dest].build_initial_state_json_for(pos, AOI_CELL_SIZE))
                .unwrap_or_else(|| nodes[*dest].build_initial_state_json());
            sess.send_message(&ServerMessage::InitialState(initial_state));
        }
    }
}
'''
new_function = '''trait JumpHandoffSession {
    fn player_id(&self) -> PlayerId;
    fn ship_id(&self) -> ShipId;
    fn send_events(&mut self, events: &[DomainEvent]);
    fn send_initial_state(&mut self, initial_state: InitialStateWire);
}

impl JumpHandoffSession for ws_server::PlayerSession {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn ship_id(&self) -> ShipId {
        self.ship_id
    }

    fn send_events(&mut self, events: &[DomainEvent]) {
        let _ = ws_server::PlayerSession::send_events(self, events);
    }

    fn send_initial_state(&mut self, initial_state: InitialStateWire) {
        let _ = self.send_message(&ServerMessage::InitialState(initial_state));
    }
}

fn resend_jump_initial_state<S: EventStore, T: JumpHandoffSession>(
    nodes: &[SimulationNode<S>],
    sessions: &mut Vec<T>,
    handoff: &JumpHandoff,
) -> Vec<PlayerId> {
    let destinations: HashMap<PlayerId, usize> =
        handoff.jumped_players.iter().copied().collect();
    let mut failed_players = Vec::new();

    sessions.retain_mut(|session| {
        let player_id = session.player_id();
        let Some(&dest) = destinations.get(&player_id) else {
            return true;
        };

        let initial_state = match nodes[dest]
            .build_initial_state_for_observer(session.ship_id(), AOI_CELL_SIZE)
        {
            Ok(initial_state) => initial_state,
            Err(error) => {
                eprintln!(
                    "[Server] post-transit handoff failed for {player_id:?} in Sector {dest}: {error}"
                );
                failed_players.push(player_id);
                return false;
            }
        };

        if let Some(events) = handoff.own_events.get(&player_id) {
            session.send_events(events);
        }
        session.send_initial_state(initial_state);
        true
    });

    failed_players
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{NodeId, Position, SectorBounds, SectorId, Velocity};
    use dawn_sector::ship_types::SHIP_TYPE_NPC_FRIGATE;

    struct FakeSession {
        player_id: PlayerId,
        ship_id: ShipId,
        initial_state_ship_ids: Vec<Vec<u64>>,
    }

    impl FakeSession {
        fn new(player_id: PlayerId, ship_id: ShipId) -> Self {
            Self {
                player_id,
                ship_id,
                initial_state_ship_ids: Vec::new(),
            }
        }
    }

    impl JumpHandoffSession for FakeSession {
        fn player_id(&self) -> PlayerId {
            self.player_id
        }

        fn ship_id(&self) -> ShipId {
            self.ship_id
        }

        fn send_events(&mut self, _events: &[DomainEvent]) {}

        fn send_initial_state(&mut self, initial_state: InitialStateWire) {
            self.initial_state_ship_ids.push(
                initial_state
                    .ships
                    .into_iter()
                    .map(|ship| ship.ship_id)
                    .collect(),
            );
        }
    }

    fn node() -> SimulationNode {
        SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        )
    }

    fn jump_handoff(player_id: PlayerId, dest: usize) -> JumpHandoff {
        JumpHandoff {
            jumped_players: vec![(player_id, dest)],
            own_events: HashMap::new(),
        }
    }

    #[test]
    fn post_transit_handoff_drops_a_session_with_a_missing_observer() {
        let player_id = PlayerId(1);
        let missing = ShipId::new(NodeId(9), 999);
        let nodes = vec![node()];
        let mut sessions = vec![FakeSession::new(player_id, missing)];

        let failed = resend_jump_initial_state(
            &nodes,
            &mut sessions,
            &jump_handoff(player_id, 0),
        );

        assert_eq!(failed, vec![player_id]);
        assert!(sessions.is_empty());
    }

    #[test]
    fn post_transit_handoff_keeps_valid_observer_state_scoped() {
        let player_id = PlayerId(1);
        let mut destination = node();
        let observer = destination.spawn_ship(
            SHIP_TYPE_NPC_FRIGATE,
            Position::ORIGIN,
            Velocity::ZERO,
        );
        let far = destination.spawn_ship(
            SHIP_TYPE_NPC_FRIGATE,
            Position::new(100_000.0, 0.0, 0.0),
            Velocity::ZERO,
        );
        let nodes = vec![destination];
        let mut sessions = vec![FakeSession::new(player_id, observer)];

        let failed = resend_jump_initial_state(
            &nodes,
            &mut sessions,
            &jump_handoff(player_id, 0),
        );

        assert!(failed.is_empty());
        assert_eq!(sessions.len(), 1);
        let ids = &sessions[0].initial_state_ship_ids[0];
        assert!(ids.contains(&observer.raw()));
        assert!(!ids.contains(&far.raw()));
    }
}
'''
text = replace_once(text, old_function, new_function, "post-transit handoff function")
path.write_text(text)

# ADR and playtest documentation.
path = Path("docs/adr/ADR-0019-spatial-index-and-aoi.md")
text = path.read_text()
text = replace_once(
    text,
    "InitialState : 全 Ship ではなく、その 27 セルの船のみを送る。\n"
    "購読更新     : 前 frame の可視集合と現在の可視集合を比較し、Enter/Leave を ShipId 順に送る。\n",
    "InitialState : 全 Ship ではなく、その 27 セルの船のみを送る。\n"
    "observer失敗 : 自船を解決できない admission / resume / handoff は明示的に失敗させ、\n"
    "               空 payload や全 Ship payload へフォールバックしない。\n"
    "購読更新     : 前 frame の可視集合と現在の可視集合を比較し、Enter/Leave を ShipId 順に送る。\n",
    "ADR observer failure rule",
)
text = replace_once(
    text,
    "- **27セル規則を全runtimeで共有する**。runtime独自のvisible-set policyを禁止する。\n"
    "- **戦闘の射程判定は厳密距離のまま**。AoI候補集合は権威判定を置き換えない。\n",
    "- **27セル規則を全runtimeで共有する**。runtime独自のvisible-set policyを禁止する。\n"
    "- **observer identityを成功payloadで隠さない**。自船を解決できなければ接続・resume・\n"
    "  post-transit handoffを拒否し、全world InitialStateを送らない。\n"
    "- **戦闘の射程判定は厳密距離のまま**。AoI候補集合は権威判定を置き換えない。\n",
    "ADR design constraint",
)
text = replace_once(
    text,
    "- [x] admission/resume/recovery時に権威状態から再構築してseedするテストを追加\n",
    "- [x] admission/resume/recovery時に権威状態から再構築してseedするテストを追加\n"
    "- [x] missing observerをfresh/resume/post-transitで明示拒否し、full-world fallbackを削除\n",
    "ADR checklist",
)
text = replace_once(
    text,
    "*提案: 2026-06-15。人間承認済み 2026-06-15。AoI frame lifecycle 統合: 2026-08-01（Issue #225）。*\n",
    "*提案: 2026-06-15。人間承認済み 2026-06-15。AoI frame lifecycle 統合: 2026-08-01（Issue #225）。missing observer拒否: 2026-08-01（Issue #234）。*\n",
    "ADR footer",
)
path.write_text(text)

path = Path("docs/process/playtest-guide.md")
text = path.read_text()
text = replace_once(
    text,
    "[実装済み] InitialState にプレイヤー船識別フラグ（is_player）\n"
    "  → Godot が「相手プレイヤーの船」と NPC を区別するために使用\n"
    "  → node.rs の build_initial_state_json() が is_player を出力する\n",
    "[実装済み] InitialState にプレイヤー船識別フラグ（is_player）\n"
    "  → Godot が「相手プレイヤーの船」と NPC を区別するために使用\n"
    "  → node/serialization.rs の observer-scoped handoff が 27 セル内の船だけを出力する\n"
    "  → observer shipを解決できない接続・resume・handoffは失敗し、全world状態を送らない\n",
    "playtest InitialState wording",
)
path.write_text(text)

# Source audit: no network fallback may substitute full-world InitialState.
for source_path in [
    Path("crates/dawn-sector/src/node/serialization.rs"),
    Path("crates/dawn-simulation/src/serve/cluster.rs"),
    Path("crates/dawn-simulation/src/serve/runtime.rs"),
]:
    source = source_path.read_text()
    for marker in [
        ".unwrap_or_else(|| self.build_initial_state_json())",
        "None => nodes[0].build_initial_state_json()",
        ".unwrap_or_else(|| nodes[*dest].build_initial_state_json())",
    ]:
        if marker in source:
            raise RuntimeError(f"{source_path}: fallback remains: {marker}")
