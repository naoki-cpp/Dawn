extends GdUnitTestSuite

const Dialog = preload("res://scripts/connection_dialog.gd")

class FakeConnection extends Node:
	signal session_ready
	signal connection_changed(connected: bool)
	signal connection_failed
	const SERVER_URL := "ws://127.0.0.1:7878"
	var accepted := true
	var submitted_token := ""

	func start_connection(_url: String, token: String) -> bool:
		submitted_token = token
		return accepted

	func cancel_connection() -> void:
		submitted_token = ""
		connection_changed.emit(false)


func test_dialog_waits_for_welcome_and_clears_secret_field() -> void:
	var connection: Node = auto_free(FakeConnection.new())
	var dialog: CanvasLayer = auto_free(Dialog.new())
	add_child(dialog)
	dialog.build(connection)
	dialog._token.text = "a".repeat(64)
	dialog._submit()
	assert_bool(dialog.is_blocking()).is_true()
	assert_str(dialog._token.text).is_empty()
	assert_bool(dialog._connect.disabled).is_true()
	connection.connection_changed.emit(true)
	assert_bool(dialog.is_blocking()).is_true()
	connection.session_ready.emit()
	assert_bool(dialog.is_blocking()).is_false()
	connection.connection_changed.emit(false)
	assert_bool(dialog.is_blocking()).is_true()
	dialog._cancel_connection()
	assert_str(connection.submitted_token).is_empty()
	assert_bool(dialog._connect.disabled).is_false()


func test_invalid_configuration_keeps_fields_editable() -> void:
	var connection: Node = auto_free(FakeConnection.new())
	connection.accepted = false
	var dialog: CanvasLayer = auto_free(Dialog.new())
	add_child(dialog)
	dialog.build(connection)
	dialog._submit()
	assert_bool(dialog._endpoint.editable).is_true()
	assert_bool(dialog._connect.disabled).is_false()
	assert_str(dialog._status.text).is_not_empty()
