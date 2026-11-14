## camera_controller_test.gd
##
## Regression tests for orbit-camera drag input.
extends GdUnitTestSuite

const CameraController = preload("res://scripts/camera_controller.gd")


func test_warp_translation_does_not_change_camera_distance_or_view_angle() -> void:
	for fps: int in [30, 60, 144]:
		var target: Node3D = auto_free(Node3D.new())
		add_child(target)
		var camera: Camera3D = auto_free(CameraController.new())
		add_child(camera)
		camera.set_process(false)
		camera.set_target(target)
		var expected: Vector3 = camera._orbit_offset()
		var max_error := 0.0
		var delta := 1.0 / float(fps)
		for frame: int in range(fps * 3):
			var speed := 20_000.0 * sin(PI * float(frame) / float(fps * 3))
			target.position += Vector3.FORWARD * speed * delta
			camera._process(delta)
			max_error = maxf(max_error,
				(camera.global_position - target.global_position - expected).length())
		assert_float(max_error).is_less(0.02)


func test_orbit_and_zoom_easing_are_frame_rate_independent() -> void:
	var reference := Vector3.ZERO
	for fps: int in [30, 60, 144]:
		var target: Node3D = auto_free(Node3D.new())
		add_child(target)
		var camera: Camera3D = auto_free(CameraController.new())
		add_child(camera)
		camera.set_process(false)
		camera.set_target(target)
		var before := camera.global_position
		camera._apply_orbit(150.0, 50.0)
		camera._zoom_distance = 900.0
		camera._process(1.0 / float(fps))
		assert_float(camera.global_position.distance_to(before)).is_greater(0.0)
		assert_float(camera.global_position.distance_to(camera._orbit_offset())).is_greater(1.0)
		for _frame: int in range(fps - 1):
			camera._process(1.0 / float(fps))
		if fps == 30:
			reference = camera.global_position
		else:
			assert_vector(camera.global_position).is_equal_approx(reference, Vector3.ONE * 0.02)


func test_origin_rebase_preserves_camera_offset_while_orbit_is_easing() -> void:
	var target: Node3D = auto_free(Node3D.new())
	add_child(target)
	var camera: Camera3D = auto_free(CameraController.new())
	add_child(camera)
	camera.set_process(false)
	camera.set_target(target)
	camera._apply_orbit(120.0, 40.0)
	camera._process(0.1)
	var offset := camera.global_position - target.global_position
	var view := camera.global_basis
	var shift := Vector3(10_000.0, 2_000.0, -50_000.0)
	target.global_position += shift
	camera.global_position += shift
	camera.on_origin_rebased(shift)
	camera._process(0.0)
	assert_vector(camera.global_position - target.global_position).is_equal_approx(
		offset, Vector3.ONE * 0.01)
	assert_vector(camera.global_basis.z).is_equal_approx(view.z, Vector3.ONE * 0.001)


func test_camera_runs_after_ship_presentation() -> void:
	var camera: Camera3D = auto_free(CameraController.new())
	add_child(camera)

	assert_int(camera.process_priority).is_equal(10)


func test_wheel_zoom_reaches_extended_distance_and_respects_limits() -> void:
	var target: Node3D = auto_free(Node3D.new())
	add_child(target)
	var camera: Camera3D = auto_free(CameraController.new())
	add_child(camera)
	camera.set_process(false)
	camera.set_target(target)

	for _notch: int in range(40):
		camera._unhandled_input(_wheel_event(MOUSE_BUTTON_WHEEL_DOWN))
	assert_float(camera._zoom_distance).is_greater(10_000.0)
	assert_float(camera._zoom_distance).is_less_equal(camera.zoom_max)

	for _notch: int in range(80):
		camera._unhandled_input(_wheel_event(MOUSE_BUTTON_WHEEL_DOWN))
	assert_float(camera._zoom_distance).is_equal(camera.zoom_max)

	for _notch: int in range(160):
		camera._unhandled_input(_wheel_event(MOUSE_BUTTON_WHEEL_UP))
	assert_float(camera._zoom_distance).is_equal(camera.zoom_min)


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


func _wheel_event(button: MouseButton) -> InputEventMouseButton:
	var event := InputEventMouseButton.new()
	event.button_index = button
	event.pressed = true
	return event
