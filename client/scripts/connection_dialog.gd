extends CanvasLayer

signal blocking_changed(blocking: bool)

var _connection: Node
var _panel: ColorRect
var _endpoint: LineEdit
var _token: LineEdit
var _status: Label
var _connect: Button
var _cancel: Button
var _busy := false


func build(connection: Node) -> void:
	_connection = connection
	layer = 20
	_panel = ColorRect.new()
	_panel.color = Color(0.015, 0.02, 0.025, 0.94)
	_panel.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	add_child(_panel)
	var center := CenterContainer.new()
	center.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	_panel.add_child(center)
	var margin := MarginContainer.new()
	for side: String in ["left", "top", "right", "bottom"]:
		margin.add_theme_constant_override("margin_" + side, 20)
	center.add_child(margin)
	var fields := VBoxContainer.new()
	fields.add_theme_constant_override("separation", 12)
	margin.add_child(fields)
	var title := Label.new()
	title.text = "DAWN"
	title.add_theme_font_size_override("font_size", 28)
	fields.add_child(title)
	_endpoint = _field(fields, "Server", "wss://server.example.com")
	_endpoint.text = OS.get_environment("DAWN_SERVER_URL")
	if _endpoint.text.is_empty():
		_endpoint.text = connection.SERVER_URL
	_token = _field(fields, "Invite code", "")
	_token.secret = true
	_token.text = OS.get_environment("DAWN_INVITE_TOKEN")
	_token.text_submitted.connect(func(_text: String) -> void: _submit())
	_status = Label.new()
	_status.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	_status.custom_minimum_size.y = 48
	fields.add_child(_status)
	_connect = Button.new()
	_connect.text = "Connect"
	_connect.pressed.connect(_submit)
	fields.add_child(_connect)
	_cancel = Button.new()
	_cancel.text = "Cancel"
	_cancel.disabled = true
	_cancel.pressed.connect(_cancel_connection)
	fields.add_child(_cancel)
	connection.session_ready.connect(_on_ready)
	connection.connection_changed.connect(_on_connection_changed)
	connection.connection_failed.connect(_on_failed)
	get_viewport().size_changed.connect(_resize.bind(fields))
	_resize(fields)
	blocking_changed.emit(true)
	_endpoint.grab_focus()


func _field(parent: Control, caption: String, placeholder: String) -> LineEdit:
	var label := Label.new()
	label.text = caption
	parent.add_child(label)
	var field := LineEdit.new()
	field.placeholder_text = placeholder
	parent.add_child(field)
	return field


func _resize(fields: Control) -> void:
	fields.custom_minimum_size.x = minf(400.0, maxf(0.0, get_viewport().get_visible_rect().size.x - 40.0))


func is_blocking() -> bool:
	return _panel != null and _panel.visible


func _set_busy(busy: bool) -> void:
	_busy = busy
	_endpoint.editable = not busy
	_token.editable = not busy
	_connect.disabled = busy
	_cancel.disabled = not busy


func _submit() -> void:
	if _busy:
		return
	if not _connection.start_connection(_endpoint.text.strip_edges(), _token.text):
		_status.text = "Check the server URL and invite code."
		return
	_token.clear()
	_set_busy(true)
	_status.text = "Connecting..."


func _cancel_connection() -> void:
	_connection.cancel_connection()
	_set_busy(false)
	_status.text = "Connection cancelled."


func _on_ready() -> void:
	_panel.hide()
	blocking_changed.emit(false)


func _on_connection_changed(connected: bool) -> void:
	if connected or not _busy:
		return
	_panel.show()
	_status.text = "Reconnecting..."
	blocking_changed.emit(true)


func _on_failed() -> void:
	_set_busy(false)
	_status.text = "Connection timed out. Check the server and invite code."
