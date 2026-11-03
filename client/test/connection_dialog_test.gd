extends GdUnitTestSuite

const Dialog = preload("res://scripts/connection_dialog.gd")

const Connection = preload("res://test/fixtures/connection_socket.gd")


func test_dialog_waits_for_welcome_and_clears_secret_field() -> void:
	var connection: Node = auto_free(Connection.new())
	var dialog: CanvasLayer = auto_free(Dialog.new())
	add_child(dialog)
	dialog.build(connection)
	dialog._endpoint.text = "wss://dawn.example"
	dialog._token.text = "a".repeat(64)
	dialog._submit()
	assert_bool(dialog.is_blocking()).is_true()
	assert_str(dialog._token.text).is_empty()
	assert_bool(dialog._connect.disabled).is_true()
	connection.open_socket()
	assert_bool(dialog.is_blocking()).is_true()
	connection._accept_welcome(1, 2, PackedByteArray([1, 2]))
	assert_bool(dialog.is_blocking()).is_false()
	connection.close_socket()
	assert_bool(dialog.is_blocking()).is_true()
	dialog._cancel_connection()
	assert_str(connection._invite_token).is_empty()
	assert_bool(dialog._connect.disabled).is_false()


func test_invalid_configuration_keeps_fields_editable() -> void:
	var connection: Node = auto_free(Connection.new())
	var dialog: CanvasLayer = auto_free(Dialog.new())
	add_child(dialog)
	dialog.build(connection)
	dialog._endpoint.text = "invalid"
	dialog._submit()
	assert_bool(dialog._endpoint.editable).is_true()
	assert_bool(dialog._connect.disabled).is_false()
	assert_str(dialog._status.text).is_not_empty()


func test_timeout_returns_to_editable_inputs_and_blocks_gameplay() -> void:
	var connection: Node = auto_free(Connection.new())
	var dialog: CanvasLayer = auto_free(Dialog.new())
	add_child(dialog)
	dialog.build(connection)
	dialog._endpoint.text = "wss://dawn.example"
	dialog._token.text = "a".repeat(64)
	dialog._submit()
	connection.open_socket()
	connection._process(90.0)
	assert_bool(dialog.is_blocking()).is_true()
	assert_bool(dialog._endpoint.editable).is_true()
	assert_bool(dialog._connect.disabled).is_false()
	assert_bool(dialog._cancel.disabled).is_true()
	assert_str(dialog._token.text).is_empty()
	assert_str(connection._invite_token).is_empty()
	assert_str(dialog._status.text).contains("timed out")
