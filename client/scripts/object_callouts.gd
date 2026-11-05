extends Control

const FONT_SIZE := 13
const MAX_LABELS := 64
var _labels: Array[Dictionary] = []


func _ready() -> void:
	mouse_filter = Control.MOUSE_FILTER_IGNORE
	process_priority = 20


func _process(_delta: float) -> void:
	var camera := get_viewport().get_camera_3d()
	_labels.clear()
	if camera != null:
		for path: String in ["../../World/Bodies", "../../World/Gates", "../../World/Ships"]:
			var root := get_node_or_null(path)
			if root == null:
				continue
			for child: Node in root.get_children():
				var target := child as Node3D
				if target == null or target.is_queued_for_deletion() or not target.is_visible_in_tree():
					continue
				if not target.has_meta("callout_text") or camera.is_position_behind(target.global_position):
					continue
				var anchor := camera.unproject_position(target.global_position)
				if not Rect2(Vector2.ZERO, size).has_point(anchor) or _labels.size() >= MAX_LABELS:
					continue
				var text := str(target.get_meta("callout_text"))
				if ThemeDB.fallback_font.get_string_size(text, HORIZONTAL_ALIGNMENT_LEFT, -1, FONT_SIZE).x > 180.0:
					while not text.is_empty() and ThemeDB.fallback_font.get_string_size(text + "...", HORIZONTAL_ALIGNMENT_LEFT, -1, FONT_SIZE).x > 180.0:
						text = text.left(-1)
					text += "..."
				var width := minf(ThemeDB.fallback_font.get_string_size(text, HORIZONTAL_ALIGNMENT_LEFT, -1, FONT_SIZE).x, 180.0)
				var rect := place_label(anchor, Vector2(width + 8.0, 22.0), size, _labels)
				if rect.size == Vector2.ZERO:
					continue
				_labels.append({"anchor": anchor, "rect": rect, "text": text,
					"color": target.get_meta("callout_color", Color(0.8, 0.9, 1.0, 0.85))})
	queue_redraw()


static func place_label(anchor: Vector2, extent: Vector2, viewport_size: Vector2, occupied: Array[Dictionary]) -> Rect2:
	var right := anchor.x + 28.0 + extent.x < viewport_size.x - 8.0
	var x := anchor.x + 28.0 if right else anchor.x - 28.0 - extent.x
	for offset: float in [0.0, -28.0, 28.0, -56.0, 56.0, -84.0, 84.0]:
		var rect := Rect2(Vector2(x, anchor.y - 36.0 + offset), extent)
		if not Rect2(Vector2(8.0, 8.0), viewport_size - Vector2(16.0, 16.0)).encloses(rect):
			continue
		var overlaps := false
		for other: Dictionary in occupied:
			if rect.grow(4.0).intersects(other.rect):
				overlaps = true
				break
		if not overlaps:
			return rect
	return Rect2()


func _draw() -> void:
	var font := ThemeDB.fallback_font
	for entry: Dictionary in _labels:
		var anchor: Vector2 = entry.anchor
		var rect: Rect2 = entry.rect
		var color: Color = entry.color
		var right := rect.position.x > anchor.x
		var elbow := Vector2(rect.position.x if right else rect.end.x, rect.end.y)
		var start := anchor + (elbow - anchor).normalized() * 7.0
		draw_line(start, elbow, color * Color(1, 1, 1, 0.6), 1.0, true)
		draw_line(elbow, Vector2(rect.end.x if right else rect.position.x, elbow.y), color * Color(1, 1, 1, 0.6), 1.0, true)
		var baseline := rect.position + Vector2(4.0, 16.0)
		draw_string_outline(font, baseline, entry.text, HORIZONTAL_ALIGNMENT_LEFT, rect.size.x - 8.0, FONT_SIZE, 3, Color(0.01, 0.02, 0.03, 0.9))
		draw_string(font, baseline, entry.text, HORIZONTAL_ALIGNMENT_LEFT, rect.size.x - 8.0, FONT_SIZE, color)
