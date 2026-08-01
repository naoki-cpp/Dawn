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
