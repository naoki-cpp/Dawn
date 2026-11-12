extends Control

const INNER_RADIUS: float = 34.0
const OUTER_RADIUS: float = 136.0
const ICON_RADIUS: float = 92.0
const HALF_WEDGE: float = PI / 8.0
const WEDGE_STEPS: int = 8
const ICON_LINE_WIDTH: float = 1.8
const TITLE_FONT_SIZE: int = 12
const ACTION_FONT_SIZE: int = 14

var _origin: Vector2 = Vector2.ZERO
var _title: String = ""
var _entries: Array[Dictionary] = []
var _hovered_action: int = 0


func _ready() -> void:
	set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	mouse_filter = Control.MOUSE_FILTER_IGNORE
	visible = false


func open_menu(screen_position: Vector2, title: String, entries: Array[Dictionary]) -> void:
	_origin = screen_position
	var viewport_size := get_viewport_rect().size
	if viewport_size.x > OUTER_RADIUS * 2.0:
		_origin.x = clampf(_origin.x, OUTER_RADIUS, viewport_size.x - OUTER_RADIUS)
	var vertical_margin := OUTER_RADIUS + TITLE_FONT_SIZE + 8.0
	if viewport_size.y > vertical_margin * 2.0:
		_origin.y = clampf(_origin.y, vertical_margin, viewport_size.y - OUTER_RADIUS)
	_title = title
	_entries = entries.duplicate(true)
	_hovered_action = 0
	mouse_filter = Control.MOUSE_FILTER_STOP
	visible = true
	queue_redraw()


func update_pointer(screen_position: Vector2) -> void:
	var next_action := _action_at(screen_position)
	if next_action != _hovered_action:
		_hovered_action = next_action
		queue_redraw()


func selected_action(screen_position: Vector2) -> int:
	return _action_at(screen_position)


func hovered_label() -> String:
	for entry: Dictionary in _entries:
		if int(entry.action) == _hovered_action:
			return String(entry.label)
	return ""


func close_menu() -> void:
	visible = false
	mouse_filter = Control.MOUSE_FILTER_IGNORE
	_entries.clear()
	_hovered_action = 0
	queue_redraw()


func _action_at(screen_position: Vector2) -> int:
	if not visible:
		return 0
	var offset: Vector2 = screen_position - _origin
	if offset.length() < INNER_RADIUS or offset.length() > OUTER_RADIUS:
		return 0
	var pointer_angle := offset.angle()
	var winner_action := 0
	var closest_difference := HALF_WEDGE
	for entry: Dictionary in _entries:
		var difference := absf(wrapf(pointer_angle - float(entry.angle), -PI, PI))
		if difference > HALF_WEDGE:
			continue
		if difference < closest_difference and not is_equal_approx(difference, closest_difference):
			closest_difference = difference
			winner_action = int(entry.action)
		elif is_equal_approx(difference, closest_difference):
			winner_action = 0
	return winner_action


func _draw() -> void:
	if not visible:
		return
	var font: Font = ThemeDB.fallback_font
	for entry: Dictionary in _entries:
		var center_angle: float = float(entry.angle)
		var start_angle := center_angle - HALF_WEDGE
		var end_angle := center_angle + HALF_WEDGE
		var points := PackedVector2Array()
		points.append(_origin + Vector2.from_angle(start_angle) * INNER_RADIUS)
		for step: int in range(WEDGE_STEPS + 1):
			var angle := lerpf(start_angle, end_angle, float(step) / float(WEDGE_STEPS))
			points.append(_origin + Vector2.from_angle(angle) * OUTER_RADIUS)
		points.append(_origin + Vector2.from_angle(end_angle) * INNER_RADIUS)
		for step: int in range(WEDGE_STEPS, -1, -1):
			var angle := lerpf(start_angle, end_angle, float(step) / float(WEDGE_STEPS))
			points.append(_origin + Vector2.from_angle(angle) * INNER_RADIUS)

		var active: bool = int(entry.action) == _hovered_action
		var color := Color(0.035, 0.055, 0.07, 0.94)
		var icon_color := Color(0.76, 0.86, 0.84, 0.96)
		if active:
			color = Color(0.18, 0.46, 0.48, 0.97)
			icon_color = Color(1.0, 1.0, 0.94, 1.0)
		draw_colored_polygon(points, color)
		var outline := points.duplicate()
		outline.append(points[0])
		draw_polyline(outline, Color(0.66, 0.78, 0.76, 0.7), 1.0, true)

		var icon_position := _origin + Vector2.from_angle(center_angle) * ICON_RADIUS
		_draw_entry_icon(String(entry.get("icon", "")), icon_position, icon_color)

	draw_arc(_origin, OUTER_RADIUS, 0.0, TAU, 72, Color(0.83, 0.9, 0.86, 0.8), 1.5, true)
	draw_circle(_origin, INNER_RADIUS, Color(0.025, 0.035, 0.045, 0.98))
	draw_arc(_origin, INNER_RADIUS, 0.0, TAU, 48, Color(0.66, 0.78, 0.76, 0.8), 1.0, true)
	_draw_centered_text(
		font, _title, _origin + Vector2(0.0, -OUTER_RADIUS - 10.0),
		TITLE_FONT_SIZE, Color(0.84, 0.9, 0.88))
	_draw_action_label(font, hovered_label())


func _draw_entry_icon(icon: String, center: Vector2, color: Color) -> void:
	match icon:
		"approach":
			draw_line(center + Vector2(-9.0, 0.0), center + Vector2(1.0, 0.0), color, ICON_LINE_WIDTH, true)
			draw_line(center + Vector2(1.0, 0.0), center + Vector2(-3.0, -4.0), color, ICON_LINE_WIDTH, true)
			draw_line(center + Vector2(1.0, 0.0), center + Vector2(-3.0, 4.0), color, ICON_LINE_WIDTH, true)
			draw_circle(center + Vector2(6.0, 0.0), 2.5, color, false, ICON_LINE_WIDTH, true)
		"warp":
			draw_line(center + Vector2(-9.0, -5.0), center + Vector2(-3.0, -5.0), color, ICON_LINE_WIDTH, true)
			draw_line(center + Vector2(-9.0, 5.0), center + Vector2(-3.0, 5.0), color, ICON_LINE_WIDTH, true)
			draw_line(center + Vector2(-8.0, 0.0), center + Vector2(7.0, 0.0), color, ICON_LINE_WIDTH, true)
			draw_line(center + Vector2(7.0, 0.0), center + Vector2(2.0, -4.0), color, ICON_LINE_WIDTH, true)
			draw_line(center + Vector2(7.0, 0.0), center + Vector2(2.0, 4.0), color, ICON_LINE_WIDTH, true)
		"orbit":
			var start_angle := deg_to_rad(35.0)
			var end_angle := deg_to_rad(305.0)
			draw_arc(center, 7.0, start_angle, end_angle, 48, color, ICON_LINE_WIDTH, true)
			draw_circle(center, 2.0, color)
			var tip := center + Vector2.from_angle(end_angle) * 7.0
			var tangent := Vector2(-sin(end_angle), cos(end_angle))
			var normal := Vector2.from_angle(end_angle)
			draw_line(tip, tip - tangent * 4.0 + normal * 2.5, color, ICON_LINE_WIDTH, true)
			draw_line(tip, tip - tangent * 4.0 - normal * 2.5, color, ICON_LINE_WIDTH, true)
		"range":
			draw_arc(center, 8.0, 0.0, TAU, 48, color, ICON_LINE_WIDTH, true)
			draw_circle(center, 2.0, color)
			draw_line(center + Vector2(2.0, 0.0), center + Vector2(8.0, 0.0), color, ICON_LINE_WIDTH, true)
			draw_line(center + Vector2(8.0, 0.0), center + Vector2(5.0, -2.5), color, ICON_LINE_WIDTH, true)
			draw_line(center + Vector2(8.0, 0.0), center + Vector2(5.0, 2.5), color, ICON_LINE_WIDTH, true)
		"lock":
			for sign_x: float in [-1.0, 1.0]:
				for sign_y: float in [-1.0, 1.0]:
					var corner := center + Vector2(sign_x * 8.0, sign_y * 8.0)
					draw_line(corner, corner + Vector2(-sign_x * 4.0, 0.0), color, ICON_LINE_WIDTH, true)
					draw_line(corner, corner + Vector2(0.0, -sign_y * 4.0), color, ICON_LINE_WIDTH, true)
			draw_circle(center, 2.0, color)
		"dock":
			draw_line(center + Vector2(-8.0, -7.0), center + Vector2(-8.0, 7.0), color, ICON_LINE_WIDTH, true)
			draw_line(center + Vector2(8.0, -7.0), center + Vector2(8.0, 7.0), color, ICON_LINE_WIDTH, true)
			draw_line(center + Vector2(-8.0, 7.0), center + Vector2(8.0, 7.0), color, ICON_LINE_WIDTH, true)
			draw_line(center + Vector2(0.0, -8.0), center + Vector2(0.0, 2.0), color, ICON_LINE_WIDTH, true)
			draw_line(center + Vector2(0.0, 2.0), center + Vector2(-3.0, -1.0), color, ICON_LINE_WIDTH, true)
			draw_line(center + Vector2(0.0, 2.0), center + Vector2(3.0, -1.0), color, ICON_LINE_WIDTH, true)
		"jump":
			draw_line(center + Vector2(3.0, -8.0), center + Vector2(8.0, -8.0), color, ICON_LINE_WIDTH, true)
			draw_line(center + Vector2(8.0, -8.0), center + Vector2(8.0, 8.0), color, ICON_LINE_WIDTH, true)
			draw_line(center + Vector2(8.0, 8.0), center + Vector2(3.0, 8.0), color, ICON_LINE_WIDTH, true)
			draw_line(center + Vector2(-8.0, 0.0), center + Vector2(6.0, 0.0), color, ICON_LINE_WIDTH, true)
			draw_line(center + Vector2(6.0, 0.0), center + Vector2(2.0, -3.5), color, ICON_LINE_WIDTH, true)
			draw_line(center + Vector2(6.0, 0.0), center + Vector2(2.0, 3.5), color, ICON_LINE_WIDTH, true)


func _draw_action_label(font: Font, label: String) -> void:
	if label.is_empty():
		return
	var color := Color(0.96, 0.98, 0.96)
	if label == "Keep at range":
		_draw_centered_text(font, "Keep at", _origin + Vector2(0.0, -6.0), ACTION_FONT_SIZE, color)
		_draw_centered_text(font, "range", _origin + Vector2(0.0, 7.0), ACTION_FONT_SIZE, color)
	else:
		_draw_centered_text(font, label, _origin, ACTION_FONT_SIZE, color)


func _draw_centered_text(font: Font, text: String, center: Vector2, font_size: int, color: Color) -> void:
	var text_size := font.get_string_size(text, HORIZONTAL_ALIGNMENT_LEFT, -1, font_size)
	var baseline_y := center.y + (font.get_ascent(font_size) - font.get_descent(font_size)) * 0.5
	draw_string(
		font, Vector2(center.x - text_size.x * 0.5, baseline_y), text,
		HORIZONTAL_ALIGNMENT_LEFT, -1.0, font_size, color)
