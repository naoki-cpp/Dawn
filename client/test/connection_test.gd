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
