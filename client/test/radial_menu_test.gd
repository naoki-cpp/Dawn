extends GdUnitTestSuite

const RadialMenuScript = preload("res://scripts/radial_menu.gd")


func test_release_in_a_wedge_selects_its_action_and_center_cancels() -> void:
	var menu: Control = auto_free(RadialMenuScript.new())
	add_child(menu)
	var entries: Array[Dictionary] = [{
		"action": 1,
		"label": "Approach",
		"icon": "approach",
		"angle": -PI / 2.0,
	}]
	var center := Vector2(300.0, 300.0)
	menu.call("open_menu", center, "SHIP #12", entries)

	assert_int(menu.mouse_filter).is_equal(Control.MOUSE_FILTER_STOP)
	assert_int(menu.call("selected_action", center + Vector2.UP * 90.0)).is_equal(1)
	assert_str(menu.call("hovered_label")).is_equal("")
	menu.call("update_pointer", center + Vector2.UP * 90.0)
	assert_str(menu.call("hovered_label")).is_equal("Approach")
	assert_int(menu.call("selected_action", center)).is_equal(0)
	assert_int(menu.call("selected_action", center + Vector2.RIGHT * 180.0)).is_equal(0)

	menu.call("close_menu")
	assert_int(menu.mouse_filter).is_equal(Control.MOUSE_FILTER_IGNORE)


func test_shared_wedge_boundary_is_not_resolved_by_entry_order() -> void:
	var menu: Control = auto_free(RadialMenuScript.new())
	add_child(menu)
	var clockwise := {
		"action": 1,
		"label": "Approach",
		"icon": "approach",
		"angle": 0.0,
	}
	var counterclockwise := {
		"action": 2,
		"label": "Orbit",
		"icon": "orbit",
		"angle": PI / 4.0,
	}
	var center := Vector2(300.0, 300.0)
	var boundary_point := center + Vector2.from_angle(PI / 8.0) * 90.0
	var entries: Array[Dictionary] = [clockwise, counterclockwise]
	var reversed_entries: Array[Dictionary] = [counterclockwise, clockwise]

	menu.call("open_menu", center, "TARGET", entries)
	var forward_action: int = menu.call("selected_action", boundary_point)
	menu.call("open_menu", center, "TARGET", reversed_entries)
	var reversed_action: int = menu.call("selected_action", boundary_point)

	assert_int(reversed_action).is_equal(forward_action)
	assert_int(forward_action).is_equal(0)


func test_pointer_selection_matches_the_visible_wedge_width() -> void:
	var menu: Control = auto_free(RadialMenuScript.new())
	add_child(menu)
	var entries: Array[Dictionary] = [
		{"action": 1, "label": "Approach", "icon": "approach", "angle": 0.0},
		{"action": 2, "label": "Orbit", "icon": "orbit", "angle": PI / 4.0},
	]
	var center := Vector2(300.0, 300.0)
	menu.call("open_menu", center, "TARGET", entries)

	assert_int(menu.call(
		"selected_action", center + Vector2.from_angle(PI / 8.0 - 0.001) * 90.0
	)).is_equal(1)
	assert_int(menu.call(
		"selected_action", center + Vector2.from_angle(PI / 8.0 + 0.001) * 90.0
	)).is_equal(2)
