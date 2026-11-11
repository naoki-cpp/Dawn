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
