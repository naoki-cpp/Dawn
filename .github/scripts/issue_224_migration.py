from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def write(path: str, content: str) -> None:
    (ROOT / path).write_text(content, encoding="utf-8")


lib = read("crates/dawn-client-gdext/src/lib.rs")
lib = lib.replace(
    "mod client_command_gd;\n",
    "mod client_command_gd;\nmod client_outcome;\n",
)
lib = lib.replace(
    "pub use server_message_gd::ServerMessageDecoder;",
    "pub use server_message_gd::{ServerEventOutcome, ServerMessageDecoder, ServerMessageOutcome};",
)
write("crates/dawn-client-gdext/src/lib.rs", lib)

json_variant = read("crates/dawn-client-gdext/src/json_variant.rs")
json_variant = json_variant.replace(
    "/// callers). Shared by\n"
    "/// [`crate::ServerMessageDecoder`] (server -> client) and\n"
    "/// [`crate::client_command_gd::ClientMessageDecoder`] (test-only,\n"
    "/// client -> server) since both decode a serde type into the same shape.\n",
    "/// callers). This remains only for the test-only\n"
    "/// [`crate::client_command_gd::ClientMessageDecoder`]; runtime inbound\n"
    "/// server messages now project directly into typed outcomes.\n",
)
start = json_variant.index("/// Converts any `Serialize` struct into a `Dict`")
end = json_variant.index("/// Converts an externally tagged serde value", start)
json_variant = json_variant[:start] + json_variant[end:]
json_variant = json_variant.replace(
    "the shape `ClientCommandWire`/`EventWire` serialize to since ADR-0042",
    "the shape `ClientCommandWire` serializes to since ADR-0042",
)
json_variant = json_variant.replace(
    "consumers were already written against (back when these types were\n"
    "/// internally tagged JSON).",
    "test assertions use for the client -> server decoder.",
)
write("crates/dawn-client-gdext/src/json_variant.rs", json_variant)

server_event = read("crates/dawn-wire/src/server_event.rs")
server_event = server_event.replace(
    "/// tagged enum (no `deserialize_any`). `dawn-client-gdext`'s\n"
    "/// `ServerMessageDecoder` converts the externally tagged shape back into a\n"
    "/// `{\"type\": ..., ...}` Dictionary so existing GDScript consumers (written\n"
    "/// against the old JSON shape) don't need to change.\n",
    "/// tagged enum (no `deserialize_any`). `dawn-client-gdext` decodes this\n"
    "/// enum into a typed client outcome and performs all variant dispatch in\n"
    "/// Rust; GDScript never reconstructs this enum from a string tag.\n",
)
write("crates/dawn-wire/src/server_event.rs", server_event)

navigation = read("crates/dawn-client-gdext/src/navigation_gd.rs")
navigation = navigation.replace(
    "//! `InitialState`'s navigation payload (system names, jump gates, stations,\n"
    "//! celestial bodies, buildable ship types) is a nested structure, not flat\n"
    "//! scalars, so it can't take the same \"just pass typed args\" fix as\n"
    "//! `WorldSession::apply_health_event`. Instead this walks the `Dictionary`\n"
    "//! `ServerMessageDecoder` already produced from the decoded `InitialStateWire`\n"
    "//! directly, so `main.gd` never needs to `JSON.stringify` it back into text\n"
    "//! only for `serde_json` to parse it again on this side of the FFI boundary.\n",
    "//! `InitialState`'s navigation payload (system names, jump gates, stations,\n"
    "//! celestial bodies, buildable ship types) is a nested structure, not flat\n"
    "//! scalars. The inbound typed outcome projects it directly from\n"
    "//! `InitialStateWire` into this Godot-facing `Dictionary`; this module then\n"
    "//! converts that boundary object into the pure Rust client model. No JSON\n"
    "//! value or text round-trip exists on the runtime path.\n",
)
write("crates/dawn-client-gdext/src/navigation_gd.rs", navigation)

connection = read("client/scripts/connection.gd")
connection = connection.replace(
    "signal event_received(payload: Dictionary)",
    "signal event_received(outcome: ServerEventOutcome)",
)
connection = connection.replace(
    "## ClientCommand/ServerMessageDecoder are GDExtension classes (dawn-wire/\n"
    "## dawn-client-gdext, ADR-0041/ADR-0042) -- globally registered, no preload\n"
    "## needed. Their methods take &self (matching every other GDExtension class\n"
    "## in this project, e.g. PlayerLoadout), so callers need an instance rather\n"
    "## than calling the class name directly.\n",
    "## ClientCommand/ServerMessageDecoder are GDExtension classes (dawn-wire/\n"
    "## dawn-client-gdext, ADR-0041/ADR-0042) -- globally registered, no preload\n"
    "## needed. The decoder returns a typed ServerMessageOutcome that owns all\n"
    "## Rust-side variant projection.\n",
)
start = connection.index("## One WebSocket frame always carries exactly one message")
end = connection.index("func _normalize_ws_url", start)
connection_runtime = '''## One WebSocket frame always carries exactly one postcard `ServerMessage`
## (ADR-0042). `ServerMessageDecoder` returns a typed Rust outcome; the
## outcome itself owns variant projection and calls one fixed `_accept_*`
## method below. No runtime path rebuilds the wire enum as a Dictionary or
## dispatches by a string `"type"` field.
func _receive_messages() -> void:
\twhile _ws.get_available_packet_count() > 0:
\t\tvar packet: PackedByteArray = _ws.get_packet()
\t\tvar outcome: ServerMessageOutcome = _decoder.decode(packet)
\t\tif outcome == null:
\t\t\tpush_warning("[Connection] failed to decode binary ServerMessage")
\t\t\tcontinue
\t\tif not outcome.dispatch(self):
\t\t\tpush_warning("[Connection] failed to dispatch typed ServerMessage outcome")


func _accept_welcome(p_player_id: int, p_ship_id: int) -> void:
\tplayer_id = p_player_id
\tship_id = p_ship_id
\t_welcomed = true
\tprint("[Connection] Welcome: player_id=%d ship_id=%d" % [player_id, ship_id])
\twelcomed.emit(player_id, ship_id)


func _accept_initial_state(state: Dictionary) -> void:
\tvar ships: Array = state.get("ships", []) as Array
\tprint("[Connection] InitialState: %d ships" % ships.size())
\tinitial_state_received.emit(state)


func _accept_event(outcome: ServerEventOutcome) -> void:
\tevent_received.emit(outcome)


func _accept_player_loadout(bytes: PackedByteArray) -> void:
\tprint("[Connection] PlayerLoadout received")
\tplayer_fitting_received.emit(bytes)


func _accept_redirect(ws_addr: String, p_player_id: int, p_ship_id: int) -> void:
\tif ws_addr.is_empty():
\t\tpush_warning("[Connection] Redirect without ws_addr")
\t\treturn
\tplayer_id = p_player_id
\tship_id = p_ship_id
\t_server_url = _normalize_ws_url(ws_addr)
\t_welcomed = false
\t_connected = false
\t_reconnect_timer = RECONNECT_INTERVAL
\tprint("[Connection] Redirect: reconnecting to %s as player_id=%d ship_id=%d" % [
\t\t_server_url, player_id, ship_id])
\tconnection_changed.emit(false)
\t_ws.close()


func _accept_module_activated(p_ship_id: int, p_module_id: int, slot: String) -> void:
\tmodule_activated.emit(p_ship_id, p_module_id, slot)


func _accept_module_deactivated(
\tp_ship_id: int,
\tp_module_id: int,
\tslot: String,
\treason: String
) -> void:
\tmodule_deactivated.emit(p_ship_id, p_module_id, slot, reason)


func _accept_market_snapshot(snapshot: Dictionary) -> void:
\tmarket_snapshot_received.emit(snapshot)


func _accept_motion_correction(payload: Dictionary) -> void:
\tmotion_correction_received.emit(payload)


'''
connection = connection[:start] + connection_runtime + connection[end:]
write("client/scripts/connection.gd", connection)

main = read("client/scripts/main.gd")
old_start = main.index("func _on_event_received(payload: Dictionary) -> void:")
old_end = main.index("# -- Position snap (ADR-0029)", old_start)
new_event_entry = '''func _on_event_received(outcome: ServerEventOutcome) -> void:
\t_session.increment_event_count()
\t_sync_session_state()
\tif not outcome.dispatch(self):
\t\tpush_warning("[World] failed to dispatch typed server event outcome")


'''
main = main[:old_start] + new_event_entry + main[old_end:]
write("client/scripts/main.gd", main)

connection_test = '''## connection_test.gd
##
## Signal and redirect wiring tests for connection.gd. Wire decoding and
## variant projection are covered in Rust; these tests intentionally call the
## typed `_accept_*` boundary instead of hand-building wire-shaped Dictionaries.
extends GdUnitTestSuite

const Connection = preload("res://scripts/connection.gd")


func test_reconnect_logging_is_emitted_on_first_attempt_and_after_interval() -> void:
\tassert_bool(Connection.should_log_reconnect(1, 0.0, 30.0)).is_true()
\tassert_bool(Connection.should_log_reconnect(10, 29.9, 30.0)).is_false()
\tassert_bool(Connection.should_log_reconnect(10, 30.0, 30.0)).is_true()


func test_normalize_ws_url_adds_ws_scheme_to_host_port() -> void:
\tvar connection: Node = Connection.new()
\tassert_str(connection._normalize_ws_url("127.0.0.1:7880")).is_equal("ws://127.0.0.1:7880")
\tconnection.free()


func test_normalize_ws_url_keeps_existing_ws_scheme() -> void:
\tvar connection: Node = Connection.new()
\tassert_str(connection._normalize_ws_url("ws://127.0.0.1:7880")).is_equal("ws://127.0.0.1:7880")
\tconnection.free()


func test_normalize_ws_url_keeps_existing_wss_scheme() -> void:
\tvar connection: Node = Connection.new()
\tassert_str(connection._normalize_ws_url("wss://example.test/ws")).is_equal("wss://example.test/ws")
\tconnection.free()


func test_welcome_outcome_updates_identity_and_emits_signal() -> void:
\tvar connection: Node = Connection.new()
\tvar received: Array = []
\tconnection.welcomed.connect(func(player_id: int, ship_id: int) -> void:
\t\treceived.append({"player_id": player_id, "ship_id": ship_id})
\t)

\tconnection._accept_welcome(5, 11)

\tassert_int(connection.player_id).is_equal(5)
\tassert_int(connection.ship_id).is_equal(11)
\tassert_bool(connection._welcomed).is_true()
\tassert_int(received.size()).is_equal(1)
\tconnection.free()


func test_module_activated_outcome_emits_module_signal() -> void:
\tvar connection: Node = Connection.new()
\tvar received: Array = []
\tconnection.module_activated.connect(func(ship_id: int, module_id: int, slot: String) -> void:
\t\treceived.append({"ship_id": ship_id, "module_id": module_id, "slot": slot})
\t)

\tconnection._accept_module_activated(11, 7, "Mid")

\tassert_int(received.size()).is_equal(1)
\tassert_int((received[0] as Dictionary)["ship_id"]).is_equal(11)
\tassert_int((received[0] as Dictionary)["module_id"]).is_equal(7)
\tassert_str((received[0] as Dictionary)["slot"]).is_equal("Mid")
\tconnection.free()


func test_player_loadout_outcome_emits_raw_bytes_unchanged() -> void:
\tvar connection: Node = Connection.new()
\tvar received: Array = []
\tconnection.player_fitting_received.connect(func(bytes: PackedByteArray) -> void:
\t\treceived.append(bytes)
\t)

\tvar bytes := PackedByteArray([1, 2, 3])
\tconnection._accept_player_loadout(bytes)

\tassert_int(received.size()).is_equal(1)
\tassert_that(received[0]).is_equal(bytes)
\tconnection.free()


func test_market_snapshot_outcome_emits_market_signal() -> void:
\tvar connection: Node = Connection.new()
\tvar received: Array = []
\tconnection.market_snapshot_received.connect(func(snapshot: Dictionary) -> void:
\t\treceived.append(snapshot)
\t)

\tconnection._accept_market_snapshot({
\t\t"balance": 250,
\t\t"orders": [],
\t\t"notice": "Order placed",
\t})

\tassert_int(received.size()).is_equal(1)
\tassert_int((received[0] as Dictionary)["balance"]).is_equal(250)
\tassert_str((received[0] as Dictionary)["notice"]).is_equal("Order placed")
\tconnection.free()


func test_motion_correction_outcome_emits_prediction_signal() -> void:
\tvar connection: Node = Connection.new()
\tvar received: Array = []
\tconnection.motion_correction_received.connect(func(payload: Dictionary) -> void:
\t\treceived.append(payload)
\t)

\tconnection._accept_motion_correction({
\t\t"ship_id": 11,
\t\t"tick": 42,
\t\t"position": {"x": 100.0, "y": 20.0, "z": 300.0},
\t\t"velocity": {"dx": 4.0, "dy": 5.0, "dz": -6.0},
\t})

\tassert_int(received.size()).is_equal(1)
\tassert_int((received[0] as Dictionary)["ship_id"]).is_equal(11)
\tassert_int((received[0] as Dictionary)["tick"]).is_equal(42)
\tconnection.free()
'''
write("client/test/connection_test.gd", connection_test)

for temporary in [
    ".github/scripts/issue_224_migration.py",
    ".github/workflows/issue-224-migration.yml",
]:
    path = ROOT / temporary
    if path.exists():
        path.unlink()
