## connection_test.gd
##
## Pure tests for connection.gd redirect helpers. WebSocket I/O itself stays
## manual per docs/process/godot-client-testing.md.
extends GdUnitTestSuite

const Connection = preload("res://scripts/connection.gd")


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


func test_module_activated_message_emits_module_signal() -> void:
	var connection: Node = Connection.new()
	var received: Array = []
	connection.module_activated.connect(func(ship_id: int, module_id: int, slot: String) -> void:
		received.append({"ship_id": ship_id, "module_id": module_id, "slot": slot})
	)

	var payload := {
		"type": "ModuleActivated",
		"ship_id": 11,
		"module_id": 7,
		"slot": "Mid",
		"tick": 4,
	}
	connection._handle_message(payload, PackedByteArray())

	assert_int(received.size()).is_equal(1)
	assert_int((received[0] as Dictionary)["ship_id"]).is_equal(11)
	assert_int((received[0] as Dictionary)["module_id"]).is_equal(7)
	assert_str((received[0] as Dictionary)["slot"]).is_equal("Mid")
	connection.free()


## player_fitting_received carries the raw postcard bytes (ADR-0042), not a
## parsed Dictionary -- PlayerLoadout.apply_wire_bytes (dawn-client-gdext)
## decodes them directly into typed Rust state, with no lossy Dictionary/
## JSON round-trip in between. `_handle_message` just needs to forward
## whatever bytes it was given unchanged.
func test_player_loadout_message_emits_player_fitting_signal_with_the_raw_bytes() -> void:
	var connection: Node = Connection.new()
	var received: Array = []
	connection.player_fitting_received.connect(func(bytes: PackedByteArray) -> void:
		received.append(bytes)
	)

	var payload := {"type": "PlayerLoadout"}
	var bytes := PackedByteArray([1, 2, 3])
	connection._handle_message(payload, bytes)

	assert_int(received.size()).is_equal(1)
	assert_that(received[0]).is_equal(bytes)
	connection.free()


func test_market_snapshot_message_emits_market_signal() -> void:
	var connection: Node = Connection.new()
	var received: Array = []
	connection.market_snapshot_received.connect(func(snapshot: Dictionary) -> void:
		received.append(snapshot)
	)

	connection._handle_message({
		"type": "MarketSnapshot",
		"balance": 250,
		"orders": [],
		"notice": "Order placed",
	}, PackedByteArray())

	assert_int(received.size()).is_equal(1)
	assert_int((received[0] as Dictionary)["balance"]).is_equal(250)
	assert_str((received[0] as Dictionary)["notice"]).is_equal("Order placed")
	connection.free()


func test_legacy_player_fitting_message_still_emits_player_fitting_signal() -> void:
	var connection: Node = Connection.new()
	var received: Array = []
	connection.player_fitting_received.connect(func(bytes: PackedByteArray) -> void:
		received.append(bytes)
	)

	var payload := {"type": "PlayerFitting"}
	var bytes := PackedByteArray([4, 5, 6])
	connection._handle_message(payload, bytes)

	assert_int(received.size()).is_equal(1)
	assert_that(received[0]).is_equal(bytes)
	connection.free()
