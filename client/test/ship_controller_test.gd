extends GdUnitTestSuite

const ShipController = preload("res://scripts/ship_controller.gd")


func test_motion_correction_keeps_the_rendered_transform_for_smoothing() -> void:
	var ship := ShipController.new()
	add_child(ship)
	ship.initialize(1, Vector3.ZERO)
	ship.set_as_player()

	ship.reconcile_motion(Vector3(10.0, 0.0, 0.0), Vector3(1.0, 0.0, 0.0), 1)

	assert_vector(ship.global_position).is_equal(Vector3.ZERO)
	ship.free()


func test_warp_visual_cap_does_not_switch_back_to_prediction_during_deceleration() -> void:
	var ship := ShipController.new()
	add_child(ship)
	ship.initialize(1, Vector3.ZERO)
	ship.set_as_player()

	## 30,000 server units/tick becomes 3,000 Godot units/tick, above the
	## visual cap. The predictor must use constant-velocity mode for this path.
	ship.set_velocity(Vector3(30_000.0, 0.0, 0.0))
	ship._process(0.1)
	assert_float(ship.position.x).is_equal_approx(2_000.0, 0.001)

	## The server can report a sub-cap velocity while the committed warp is
	## decelerating. The render path must remain capped until PositionSnap,
	## rather than exposing the predictor's absolute position in one frame.
	ship.set_velocity(Vector3(10_000.0, 0.0, 0.0))
	ship._process(0.01)
	assert_float(ship.position.x).is_equal_approx(2_200.0, 0.001)

	## PositionSnap/reset is the only transition back to local prediction.
	ship.reset_motion(Vector3(7.0, 0.0, 0.0), Vector3.ZERO, 12)
	ship.set_velocity(Vector3(10.0, 0.0, 0.0))
	ship._process(0.1)
	assert_float(ship.position.x).is_equal_approx(8.0, 0.001)
	ship.free()


func test_attaching_to_an_already_warping_ship_keeps_dead_reckoning() -> void:
	var ship := ShipController.new()
	add_child(ship)
	ship.initialize(1, Vector3.ZERO)
	ship.set_velocity(Vector3(30_000.0, 0.0, 0.0))
	ship.set_as_player()
	ship._process(0.1)

	ship.set_velocity(Vector3(10_000.0, 0.0, 0.0))
	ship._process(0.01)
	assert_float(ship.position.x).is_equal_approx(2_200.0, 0.001)
	ship.free()
