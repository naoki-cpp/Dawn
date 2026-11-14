extends GdUnitTestSuite

const CameraControllerScript = preload("res://scripts/camera_controller.gd")
const WarpTunnelScript = preload("res://scripts/warp_tunnel_effect.gd")
const WorldPresentationScript = preload("res://scripts/world_presentation.gd")


func test_warp_effect_uses_camera_pose_after_camera_process_in_same_frame() -> void:
	var scene := auto_free(Node3D.new()) as Node3D
	add_child(scene)
	var ship := TestShip.new()
	scene.add_child(ship)
	var camera: Camera3D = CameraControllerScript.new()
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
	presentation._world = FakeWorld.new()
	presentation._session = FakeSession.new()
	presentation._ships = {7: ship}
	presentation._warp_tunnel_amount = 0.5
	scene.add_child(presentation)

	await get_tree().process_frame
	var previous_basis := camera.global_basis
	camera.set("_orbit_quat", Quaternion(Vector3.UP, PI * 0.5))
	await get_tree().process_frame

	var presented_basis: Basis = material.get_shader_parameter("camera_basis")
	assert_bool(camera.global_basis.is_equal_approx(previous_basis)).is_false()
	assert_bool(presented_basis.is_equal_approx(camera.global_basis)).is_true()
	assert_float(material.get_shader_parameter("camera_fov") as float) \
		.is_equal_approx(camera.fov, 0.0001)
	assert_float(camera.fov).is_greater(60.0)


class TestShip:
	extends Node3D

	func server_position() -> PackedFloat64Array:
		return PackedFloat64Array([0.0, 0.0, 0.0])

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

	func should_rebase_components(_x: float, _y: float, _z: float) -> bool:
		return false
