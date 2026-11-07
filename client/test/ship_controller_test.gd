extends GdUnitTestSuite

const ShipController = preload("res://scripts/ship_controller.gd")


func test_motion_correction_keeps_the_rendered_transform_for_smoothing() -> void:
	var ship := ShipController.new()
	add_child(ship)
	ship.initialize(1, PackedFloat64Array([0.0, 0.0, 0.0]), PackedFloat64Array([0.0, 0.0, 0.0]))
	ship.set_as_player()

	ship.reconcile_motion(PackedFloat64Array([10.0, 0.0, 0.0]), Vector3(1.0, 0.0, 0.0), 1)

	assert_vector(ship.global_position).is_equal(Vector3.ZERO)
	ship.free()


func test_rust_visibility_is_applied_to_the_ship_node() -> void:
	var ship := ShipController.new()
	add_child(ship)
	ship.initialize(1, PackedFloat64Array([0.0, 0.0, 0.0]), PackedFloat64Array([0.0, 0.0, 0.0]))
	ship.set_velocity(Vector3(30_000.0, 0.0, 0.0))
	ship._process(0.1)
	assert_bool(ship.visible).is_false()
	ship.set_as_player()
	ship._process(0.1)
	assert_bool(ship.visible).is_true()
	ship.clear_as_player()
	ship._process(0.1)
	assert_bool(ship.visible).is_false()
	ship.free()


func test_warp_exposes_continuous_world_position_for_celestial_parallax() -> void:
	var ship := ShipController.new()
	add_child(ship)
	ship.initialize(1, PackedFloat64Array([0.0, 0.0, 0.0]), PackedFloat64Array([0.0, 0.0, 0.0]))
	ship.set_as_player()

	ship.set_velocity(Vector3(30_000.0, 0.0, 0.0))
	ship._process(0.05)

	var capped_position: PackedFloat64Array = ship.server_position()
	var world_position: PackedFloat64Array = ship.world_presentation_position()
	assert_float(capped_position[0]).is_equal_approx(10_000.0, 0.001)
	assert_float(world_position[0]).is_equal_approx(15_000.0, 0.001)
	ship.free()
