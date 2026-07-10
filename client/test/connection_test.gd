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
	connection._handle_message(payload, JSON.stringify(payload))

	assert_int(received.size()).is_equal(1)
	assert_int((received[0] as Dictionary)["ship_id"]).is_equal(11)
	assert_int((received[0] as Dictionary)["module_id"]).is_equal(7)
	assert_str((received[0] as Dictionary)["slot"]).is_equal("Mid")
	connection.free()


## player_fitting_received carries the raw JSON line (not a parsed
## Dictionary) so PlayerLoadout.apply_payload (dawn-client-gdext) gets the
## server's exact wire text -- round-tripping a Dictionary through
## JSON.stringify() would corrupt integers into floats (e.g. 20 -> "20.0"),
## which serde_json rejects for u32/u64 fields.
func test_player_loadout_message_emits_player_fitting_signal() -> void:
	var connection: Node = Connection.new()
	var received: Array = []
	connection.player_fitting_received.connect(func(raw_json: String) -> void:
		received.append(raw_json)
	)

	var payload := {
		"type": "PlayerLoadout",
		"modules": [{"module_id": 3}],
		"inventory": [],
	}
	connection._handle_message(payload, JSON.stringify(payload))

	assert_int(received.size()).is_equal(1)
	var parsed: Dictionary = JSON.parse_string(received[0] as String) as Dictionary
	assert_int((((parsed["modules"] as Array)[0] as Dictionary)["module_id"] as float) as int).is_equal(3)
	connection.free()


func test_legacy_player_fitting_message_still_emits_player_fitting_signal() -> void:
	var connection: Node = Connection.new()
	var received: Array = []
	connection.player_fitting_received.connect(func(raw_json: String) -> void:
		received.append(raw_json)
	)

	var payload := {
		"type": "PlayerFitting",
		"modules": [{"module_id": 7}],
		"inventory": [],
	}
	connection._handle_message(payload, JSON.stringify(payload))

	assert_int(received.size()).is_equal(1)
	var parsed: Dictionary = JSON.parse_string(received[0] as String) as Dictionary
	assert_int((((parsed["modules"] as Array)[0] as Dictionary)["module_id"] as float) as int).is_equal(7)
	connection.free()
