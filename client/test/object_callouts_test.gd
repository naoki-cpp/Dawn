extends GdUnitTestSuite

const Callouts := preload("res://scripts/object_callouts.gd")


func test_labels_avoid_each_other_and_stay_inside_viewport() -> void:
	var viewport_size := Vector2(640, 360)
	var anchor := Vector2(610, 20)
	var first := Callouts.place_label(anchor, Vector2(160, 22), viewport_size, [])
	var second := Callouts.place_label(anchor, Vector2(160, 22), viewport_size, [{"rect": first}])
	assert_bool(Rect2(Vector2.ZERO, viewport_size).encloses(first)).is_true()
	assert_bool(Rect2(Vector2.ZERO, viewport_size).encloses(second)).is_true()
	assert_bool(first.intersects(second)).is_false()
	assert_bool(first.has_point(anchor)).is_false()


func test_overlay_tracks_visible_targets_without_intercepting_input() -> void:
	var scene := Node3D.new()
	get_tree().root.add_child(scene)
	var world := Node3D.new()
	world.name = "World"
	scene.add_child(world)
	var camera := Camera3D.new()
	world.add_child(camera)
	camera.make_current()
	var ships := Node3D.new()
	ships.name = "Ships"
	world.add_child(ships)
	var ship := Node3D.new()
	ships.add_child(ship)
	ship.position = Vector3(0, 0, -100)
	ship.set_meta("callout_text", "Magpie #1")
	var hud := CanvasLayer.new()
	scene.add_child(hud)
	var overlay := Callouts.new()
	hud.add_child(overlay)
	overlay.size = Vector2(1280, 720)
	overlay._process(0.0)
	assert_int(overlay.mouse_filter).is_equal(Control.MOUSE_FILTER_IGNORE)
	assert_int(overlay._labels.size()).is_equal(1)
	ship.position.z = 100
	overlay._process(0.0)
	assert_int(overlay._labels.size()).is_equal(0)
	ship.position.z = -100
	ship.hide()
	overlay._process(0.0)
	assert_int(overlay._labels.size()).is_equal(0)
	scene.free()
