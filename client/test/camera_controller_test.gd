## camera_controller_test.gd
##
## Regression tests for orbit-camera drag input.
extends GdUnitTestSuite

const CameraController = preload("res://scripts/camera_controller.gd")


func test_horizontal_drag_rotates_the_orbit_offset_sideways() -> void:
	var camera: Camera3D = auto_free(CameraController.new())
	add_child(camera)
	var target := Node3D.new()
	auto_free(target)
	add_child(target)
	target.global_position = Vector3.ZERO
	camera.set_target(target)

	var before_offset: Vector3 = camera._orbit_offset()

	var press := InputEventMouseButton.new()
	press.button_index = MOUSE_BUTTON_LEFT
	press.pressed = true
	press.position = Vector2(100.0, 100.0)
	camera._unhandled_input(press)

	var motion := InputEventMouseMotion.new()
	motion.position = Vector2(140.0, 100.0)
	motion.relative = Vector2(40.0, 0.0)
	camera._unhandled_input(motion)

	var after_offset: Vector3 = camera._orbit_offset()
	assert_bool(camera.is_dragging()).is_true()
	assert_float(absf(after_offset.x - before_offset.x)).is_greater(1.0)
	assert_float(absf(after_offset.z - before_offset.z)).is_greater(1.0)


func test_main_input_can_seed_orbit_drag_before_unhandled_motion() -> void:
	var camera: Camera3D = auto_free(CameraController.new())
	add_child(camera)
	var target := Node3D.new()
	auto_free(target)
	add_child(target)
	target.global_position = Vector3.ZERO
	camera.set_target(target)

	var before_offset: Vector3 = camera._orbit_offset()

	camera.begin_orbit_drag(Vector2(100.0, 100.0))
	var rotated: bool = camera.update_orbit_drag(Vector2(140.0, 100.0), Vector2(40.0, 0.0))

	var after_offset: Vector3 = camera._orbit_offset()
	assert_bool(rotated).is_true()
	assert_bool(camera.is_dragging()).is_true()
	assert_float(absf(after_offset.x - before_offset.x)).is_greater(1.0)
	assert_float(absf(after_offset.z - before_offset.z)).is_greater(1.0)
