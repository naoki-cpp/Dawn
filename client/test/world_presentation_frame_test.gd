extends GdUnitTestSuite

const CameraControllerScript = preload("res://scripts/camera_controller.gd")
const WarpTunnelScript = preload("res://scripts/warp_tunnel_effect.gd")
const WorldPresentationScript = preload("res://scripts/world_presentation.gd")
const ProminencesScript = preload("res://scripts/solar_prominences.gd")
const BackgroundScript = preload("res://scripts/sky_background.gd")


func test_warp_and_stellar_effects_use_the_final_camera_in_same_frame() -> void:
	var fixture := _fixture()
	var camera: Camera3D = fixture.camera
	var material: ShaderMaterial = fixture.material
	await get_tree().process_frame
	var previous_basis := camera.global_basis
	camera.set("_orbit_quat", Quaternion(Vector3.UP, PI * 0.5))
	camera.set("_zoom_distance", 900.0)
	await get_tree().process_frame

	var presented_basis: Basis = material.get_shader_parameter("camera_basis")
	assert_bool(camera.global_basis.is_equal_approx(previous_basis)).is_false()
	assert_bool(presented_basis.is_equal_approx(camera.global_basis)).is_true()
	assert_float(material.get_shader_parameter("camera_fov") as float) \
		.is_equal_approx(camera.fov, 0.0001)
	assert_float(camera.fov).is_greater(60.0)
	var prominences: TrackingProminences = fixture.prominences
	assert_float(prominences.presented_fov).is_equal_approx(camera.fov, 0.0001)
	assert_bool(prominences.presented_basis.is_equal_approx(camera.global_basis)).is_true()
	assert_vector(prominences.presented_position).is_equal_approx(camera.global_position, Vector3.ONE * 0.001)
	assert_vector((fixture.background as SkyBackground).global_position).is_equal_approx(
		camera.global_position, Vector3.ONE * 0.001)


func test_follow_reads_current_ship_transform_and_advances_camera_once() -> void:
	var fixture := _fixture()
	var ship: TestShip = fixture.ship
	var camera: TrackingCamera = fixture.camera
	await get_tree().process_frame
	var updates := camera.updates
	var previous_ship_position := ship.global_position
	ship.moving = true
	await get_tree().process_frame
	assert_int(camera.updates - updates).is_equal(1)
	assert_bool(ship.global_position.is_equal_approx(previous_ship_position)).is_false()
	assert_vector(camera.global_position - ship.global_position).is_equal_approx(
		camera._orbit_offset(), Vector3.ONE * 0.01)


func test_origin_rebase_preserves_follow_offset_and_reaches_effects_same_frame() -> void:
	var fixture := _fixture()
	var camera: TrackingCamera = fixture.camera
	var ship: TestShip = fixture.ship
	var world: FakeWorld = fixture.world
	await get_tree().process_frame
	var offset := camera.global_position - ship.global_position
	var shift := Vector3(10000.0, 2000.0, -50000.0)
	world.shift = shift
	ship.absolute_position = PackedFloat64Array([shift.x, shift.y, shift.z])
	world.rebase_next = true
	await get_tree().process_frame
	assert_vector(ship.global_position).is_equal(shift)
	assert_vector(camera.global_position - ship.global_position).is_equal_approx(offset, Vector3.ONE * 0.01)
	var prominences: TrackingProminences = fixture.prominences
	assert_vector(prominences.presented_position).is_equal_approx(camera.global_position, Vector3.ONE * 0.001)
	assert_vector((fixture.background as SkyBackground).global_position).is_equal_approx(
		camera.global_position, Vector3.ONE * 0.001)
	var material: ShaderMaterial = fixture.material
	assert_bool((material.get_shader_parameter("camera_basis") as Basis).is_equal_approx(camera.global_basis)).is_true()


func test_offline_presentation_updates_camera_without_replacing_authored_fov() -> void:
	var scene := auto_free(Node3D.new()) as Node3D
	add_child(scene)
	var camera := TrackingCamera.new()
	scene.add_child(camera)
	camera.fov = 75.0
	var presentation := WorldPresentationScript.new()
	presentation._camera = camera
	scene.add_child(presentation)
	await get_tree().process_frame
	var updates := camera.updates
	await get_tree().process_frame
	assert_int(camera.updates - updates).is_equal(1)
	assert_float(camera.fov).is_equal(75.0)


func _fixture() -> Dictionary:
	var scene := auto_free(Node3D.new()) as Node3D
	add_child(scene)
	var ship := TestShip.new()
	scene.add_child(ship)
	var camera := TrackingCamera.new()
	scene.add_child(camera)
	camera.call("set_target", ship)

	var canvas := CanvasLayer.new()
	scene.add_child(canvas)
	var effect: ColorRect = WarpTunnelScript.new()
	var material := ShaderMaterial.new()
	material.shader = load("res://shaders/warp_tunnel.gdshader")
	effect.material = material
	canvas.add_child(effect)

	var presentation: WorldPresentation = WorldPresentationScript.new()
	presentation._camera = camera
	presentation._warp_tunnel = effect
	var world := FakeWorld.new()
	presentation._world = world
	presentation._session = FakeSession.new()
	presentation._ships = {7: ship}
	presentation._warp_tunnel_amount = 0.5
	var star := CelestialBodyRecord.new()
	star.kind = "Star"
	star.position = PackedFloat64Array([0.0, 0.0, 1.0e12])
	star.radius = 696000000.0
	presentation._bodies = [star]
	var sky := ShaderMaterial.new()
	sky.shader = load("res://shaders/space_sky.gdshader")
	presentation._sky_mat = sky
	var background := BackgroundScript.new()
	scene.add_child(background)
	background.build(sky)
	background.request_background(7, Vector2i(8, 4), 0)
	presentation._background = background
	var prominences := TrackingProminences.new()
	scene.add_child(prominences)
	presentation._prominences = prominences
	scene.add_child(presentation)
	return {"camera": camera, "ship": ship, "material": material,
		"prominences": prominences, "world": world, "background": background}


class TrackingCamera:
	extends CameraControllerScript
	var updates := 0

	func update_view(delta: float) -> void:
		updates += 1
		super.update_view(delta)


class TrackingProminences:
	extends ProminencesScript
	var presented_fov := 0.0
	var presented_basis := Basis.IDENTITY
	var presented_position := Vector3.ZERO

	func update_view(camera: Camera3D, direction: Vector3, radius: float) -> void:
		presented_fov = camera.fov
		presented_basis = camera.global_basis
		presented_position = camera.global_position
		super.update_view(camera, direction, radius)


class TestShip:
	extends Node3D
	var moving := false
	var absolute_position := PackedFloat64Array([0.0, 0.0, 0.0])

	func _process(_delta: float) -> void:
		if moving:
			position += Vector3(100.0, 20.0, -10.0)

	func apply_origin_rebase(origin: PackedFloat64Array) -> void:
		position += Vector3(origin[0], origin[1], origin[2])

	func server_position() -> PackedFloat64Array:
		return absolute_position

	func get_speed_godot() -> float:
		return 3000.0

	func get_velocity_godot() -> Vector3:
		return Vector3.FORWARD * 3000.0


class FakeSession:
	extends RefCounted

	func player_ship_id() -> int:
		return 7


class FakeWorld:
	extends RefCounted
	var rebase_next := false
	var shift := Vector3.ZERO

	func should_rebase_components(_x: float, _y: float, _z: float) -> bool:
		return rebase_next

	func rebase_to_components(_x: float, _y: float, _z: float) -> Vector3:
		rebase_next = false
		return shift
