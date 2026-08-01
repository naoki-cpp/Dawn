## connection_test.gd
##
## Signal and redirect wiring tests for connection.gd. Wire decoding and
## variant projection are covered in Rust; these tests intentionally call the
## typed `_accept_*` boundary instead of hand-building wire-shaped Dictionaries.
extends GdUnitTestSuite

const Connection = preload("res://scripts/connection.gd")


func test_reconnect_logging_is_emitted_on_first_attempt_and_after_interval() -> void:
	assert_bool(Connection.should_log_reconnect(1, 0.0, 30.0)).is_true()
	assert_bool(Connection.should_log_reconnect(10, 29.9, 30.0)).is_false()
	assert_bool(Connection.should_log_reconnect(10, 30.0, 30.0)).is_true()


func test_normalize_ws_url_adds_ws_scheme_to_host_port() -> void:
	var connection: Node = Connection.new()
	assert_str(connection._normalize_ws_url("127.0.0.1:7880")).is_equal("ws://127.0.0.1:7880")
	connection.free()


func test_normalize_ws_url_keeps_existing_ws_scheme() -> void:
	var connection: Node = Connection.new()
	assert_str(connection._normalize_ws_url("ws://127.0.0.1:7880")).is_equal("ws://127.0.0.1:7880")
	connection.free()


func test_normalize_ws_url_keeps_existing_wss_scheme() -> void:
	var connection: Node = Connection.new()
	assert_str(connection._normalize_ws_url("wss://example.test/ws")).is_equal("wss://example.test/ws")
	connection.free()


func test_welcome_outcome_updates_identity_and_emits_signal() -> void:
	var connection: Node = Connection.new()
	var received: Array = []
	connection.welcomed.connect(func(player_id: int, ship_id: int) -> void:
		received.append({"player_id": player_id, "ship_id": ship_id})
	)

	connection._accept_welcome(5, 11)

	assert_int(connection.player_id).is_equal(5)
	assert_int(connection.ship_id).is_equal(11)
	assert_bool(connection._welcomed).is_true()
	assert_int(received.size()).is_equal(1)
	connection.free()


func test_module_activated_outcome_emits_module_signal() -> void:
	var connection: Node = Connection.new()
	var received: Array = []
	connection.module_activated.connect(func(ship_id: int, module_id: int, slot: String) -> void:
		received.append({"ship_id": ship_id, "module_id": module_id, "slot": slot})
	)

	connection._accept_module_activated(11, 7, "Mid")

	assert_int(received.size()).is_equal(1)
	assert_int((received[0] as Dictionary)["ship_id"]).is_equal(11)
	assert_int((received[0] as Dictionary)["module_id"]).is_equal(7)
	assert_str((received[0] as Dictionary)["slot"]).is_equal("Mid")
	connection.free()


func test_player_loadout_outcome_emits_raw_bytes_unchanged() -> void:
	var connection: Node = Connection.new()
	var received: Array = []
	connection.player_fitting_received.connect(func(bytes: PackedByteArray) -> void:
		received.append(bytes)
	)

	var bytes := PackedByteArray([1, 2, 3])
	connection._accept_player_loadout(bytes)

	assert_int(received.size()).is_equal(1)
	assert_that(received[0]).is_equal(bytes)
	connection.free()


func test_market_snapshot_outcome_emits_market_signal() -> void:
	var connection: Node = Connection.new()
	var received: Array = []
	connection.market_snapshot_received.connect(func(snapshot: Dictionary) -> void:
		received.append(snapshot)
	)

	connection._accept_market_snapshot({
		"balance": 250,
		"orders": [],
		"notice": "Order placed",
	})

	assert_int(received.size()).is_equal(1)
	assert_int((received[0] as Dictionary)["balance"]).is_equal(250)
	assert_str((received[0] as Dictionary)["notice"]).is_equal("Order placed")
	connection.free()


func test_motion_correction_outcome_emits_prediction_signal() -> void:
	var connection: Node = Connection.new()
	var received: Array = []
	connection.motion_correction_received.connect(func(payload: Dictionary) -> void:
		received.append(payload)
	)

	connection._accept_motion_correction({
		"ship_id": 11,
		"tick": 42,
		"position": {"x": 100.0, "y": 20.0, "z": 300.0},
		"velocity": {"dx": 4.0, "dy": 5.0, "dz": -6.0},
	})

	assert_int(received.size()).is_equal(1)
	assert_int((received[0] as Dictionary)["ship_id"]).is_equal(11)
	assert_int((received[0] as Dictionary)["tick"]).is_equal(42)
	connection.free()


class EventDispatchTarget:
	extends RefCounted
	var left_ship_id: int = -1

	func _handle_aoi_leave(payload: Dictionary) -> void:
		left_ship_id = payload.get("ship_id", -1) as int


func test_real_outcome_dispatches_welcome_and_detects_missing_handler() -> void:
	var decoder := ServerMessageDecoder.new()
	var outcome: ServerMessageOutcome = decoder.test_outcome("Welcome")
	assert_object(outcome).is_not_null()

	var missing_target := Node.new()
	assert_bool(outcome.dispatch(missing_target)).is_false()
	missing_target.free()

	var connection: Node = Connection.new()
	assert_bool(outcome.dispatch(connection)).is_true()
	assert_int(connection.player_id).is_equal(5)
	assert_int(connection.ship_id).is_equal(11)
	connection.free()


func test_real_world_event_outcome_dispatches_to_handler() -> void:
	var decoder := ServerMessageDecoder.new()
	var top_level: ServerMessageOutcome = decoder.test_outcome("AoiLeave")
	var connection: Node = Connection.new()
	var events: Array = []
	connection.event_received.connect(func(event: ServerEventOutcome) -> void:
		events.append(event)
	)
	assert_bool(top_level.dispatch(connection)).is_true()
	assert_int(events.size()).is_equal(1)

	var target := EventDispatchTarget.new()
	assert_bool((events[0] as ServerEventOutcome).dispatch(target)).is_true()
	assert_int(target.left_ship_id).is_equal(19)
	connection.free()


func test_real_initial_state_outcome_preserves_nested_payload() -> void:
	var decoder := ServerMessageDecoder.new()
	var outcome: ServerMessageOutcome = decoder.test_outcome("InitialState")
	var connection: Node = Connection.new()
	var states: Array = []
	connection.initial_state_received.connect(func(state: Dictionary) -> void:
		states.append(state)
	)
	assert_bool(outcome.dispatch(connection)).is_true()
	assert_int(states.size()).is_equal(1)
	assert_str((states[0] as Dictionary).get("system_name", "") as String).is_equal("Alpha")
	connection.free()


func test_real_market_outcome_preserves_every_item_variant() -> void:
	var decoder := ServerMessageDecoder.new()
	var outcome: ServerMessageOutcome = decoder.test_outcome("MarketSnapshot")
	var connection: Node = Connection.new()
	var snapshots: Array = []
	connection.market_snapshot_received.connect(func(snapshot: Dictionary) -> void:
		snapshots.append(snapshot)
	)
	assert_bool(outcome.dispatch(connection)).is_true()
	var orders: Array = (snapshots[0] as Dictionary).get("orders", []) as Array
	assert_int(orders.size()).is_equal(3)
	assert_str((orders[0] as Dictionary).get("item_id", "") as String).is_equal("ScrapMetal")
	assert_bool(((orders[1] as Dictionary).get("item_id", {}) as Dictionary).has("Module")).is_true()
	assert_bool(((orders[2] as Dictionary).get("item_id", {}) as Dictionary).has("PackagedShip")).is_true()
	connection.free()
