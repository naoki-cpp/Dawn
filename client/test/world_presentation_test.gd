## world_presentation_test.gd
##
## Tests for the client WorldPresentation seam. These focus on the pure
## presentation policy now extracted from main.gd: marker clamping, warp-tunnel
## easing, and sun-direction derivation from star data.
extends GdUnitTestSuite

const WorldPresentation = preload("res://scripts/world_presentation.gd")

func _position(x: float, y: float, z: float) -> PackedFloat64Array:
	return PackedFloat64Array([x, y, z])


class FakeWorld:
	extends RefCounted

	var rebase_shift := Vector3.ZERO
	var render_scale_value: float = 0.25

	func render_scale() -> float:
		return render_scale_value

	func rebase_to_components(_x: float, _y: float, _z: float) -> Vector3:
		return rebase_shift

	func to_godot_components(x: float, y: float, z: float) -> Vector3:
		return Vector3(x, y, -z) * render_scale_value

	func to_server_components(position: Vector3) -> PackedFloat64Array:
		return PackedFloat64Array([
			position.x / render_scale_value,
			position.y / render_scale_value,
			-position.z / render_scale_value,
		])

	func dir_to_godot(direction: Vector3) -> Vector3:
		return Vector3(direction.x, direction.y, -direction.z)


class FakeShip:
	extends Node3D

	var motion_rebase_calls: Array[PackedFloat64Array] = []
	var server_position_value := PackedFloat64Array([0.0, 0.0, 0.0])
	var world_presentation_position_value := PackedFloat64Array([0.0, 0.0, 0.0])

	func rebase_motion(new_origin: PackedFloat64Array) -> void:
		motion_rebase_calls.append(new_origin)

	func apply_origin_rebase(new_origin: PackedFloat64Array) -> void:
		rebase_motion(new_origin)

	func server_position() -> PackedFloat64Array:
		return server_position_value

	func world_presentation_position() -> PackedFloat64Array:
		return world_presentation_position_value


func test_render_scale_is_queried_from_world_space_authority() -> void:
	var presentation := WorldPresentation.new()
	var world := FakeWorld.new()
	world.render_scale_value = 0.25
	presentation._world = world

	assert_float(presentation._render_scale()).is_equal_approx(0.25, 0.0001)


func test_render_scale_authority_controls_spawned_navigation_geometry() -> void:
	var presentation := WorldPresentation.new()
	var world := FakeWorld.new()
	world.render_scale_value = 0.25
	presentation._world = world

	var gates_root: Node3D = auto_free(Node3D.new())
	var bodies_root: Node3D = auto_free(Node3D.new())
	presentation._gates_root = gates_root
	presentation._bodies_root = bodies_root
	presentation.respawn_navigation_markers(
		[_gate(7, _position(4.0, 0.0, 8.0), 40.0, "Beta")],
		[_body(2, "Planet", "Forge", _position(12.0, 0.0, 16.0), 20.0, 0.0)],
		[_station(3, "Forge Station", _position(20.0, 0.0, 24.0), 80.0)],
		func() -> void:
			pass)

	var gate_marker: Node3D = gates_root.get_child(0) as Node3D
	var gate_ring: MeshInstance3D = gate_marker.get_child(0) as MeshInstance3D
	var gate_mesh: TorusMesh = gate_ring.mesh as TorusMesh
	assert_float(gate_mesh.outer_radius).is_equal_approx(10.0, 0.0001)

	var planet_marker: Node3D = bodies_root.get_child(0) as Node3D
	var planet_visual: MeshInstance3D = planet_marker.get_child(0) as MeshInstance3D
	var planet_mesh: SphereMesh = planet_visual.mesh as SphereMesh
	assert_float(planet_mesh.radius).is_equal_approx(2.5, 0.0001)

	var station_marker: Node3D = bodies_root.get_child(1) as Node3D
	var station_ring: MeshInstance3D = station_marker.get_child(1) as MeshInstance3D
	var station_mesh: TorusMesh = station_ring.mesh as TorusMesh
	assert_float(station_mesh.outer_radius).is_equal_approx(20.0, 0.0001)


func test_clamped_marker_position_leaves_nearby_marker_unchanged() -> void:
	var player := Vector3.ZERO
	var marker := Vector3(100.0, 0.0, 0.0)

	var result: Vector3 = WorldPresentation.clamped_marker_position(player, marker, 500.0)

	assert_vector(result).is_equal(marker)


func test_clamped_marker_position_pulls_far_marker_back_to_clamp_radius() -> void:
	var player := Vector3.ZERO
	var marker := Vector3(1000.0, 0.0, 0.0)

	var result: Vector3 = WorldPresentation.clamped_marker_position(player, marker, 250.0)

	assert_vector(result).is_equal(Vector3(250.0, 0.0, 0.0))


func test_next_warp_tunnel_amount_eases_toward_one_above_threshold() -> void:
	var amount: float = WorldPresentation.next_warp_tunnel_amount(0.0, 3000.0, 0.1, 2000.0, 3.0)

	assert_float(amount).is_equal_approx(0.3, 0.0001)


func test_warp_arrival_keeps_the_tunnel_visible_during_the_first_tenth_second() -> void:
	var amount: float = WorldPresentation.next_warp_tunnel_amount(1.0, 0.0, 0.1)

	assert_float(amount).is_equal_approx(0.95, 0.0001)


func test_warp_arrival_frame_hitch_cannot_clear_the_tunnel_in_one_step() -> void:
	var amount: float = WorldPresentation.next_warp_tunnel_amount(1.0, 0.0, 0.5)

	assert_float(amount).is_equal_approx(0.95, 0.0001)


func test_sun_state_returns_inactive_when_no_star_exists() -> void:
	var state: Dictionary = WorldPresentation.sun_state([
		_body(2, "Planet", "Forge", _position(100.0, 0.0, 0.0), 200.0, 0.1),
	], _position(0.0, 0.0, 0.0), func(diff: Vector3) -> Vector3:
		return diff
	)

	assert_bool(state.get("active", true) as bool).is_false()


func test_sun_state_returns_direction_and_color_from_star_data() -> void:
	var state: Dictionary = WorldPresentation.sun_state([
		_body(1, "Star", "Helios", _position(0.0, 0.0, 0.0), 1000.0, 0.0),
	], _position(0.0, 0.0, 0.0), func(diff: Vector3) -> Vector3:
		return diff
	)

	assert_bool(state.get("active", false) as bool).is_true()
	assert_vector(state.get("direction", Vector3.ZERO) as Vector3) \
		.is_equal_approx(WorldPresentation.SUN_FAR_DIRECTION.normalized(), Vector3(0.0001, 0.0001, 0.0001))
	assert_vector(state.get("color", Vector3.ZERO) as Vector3) \
		.is_equal_approx(Vector3(0.55, 0.65, 1.00), Vector3(0.0001, 0.0001, 0.0001))


func test_sun_direction_uses_world_position_while_warp_render_position_is_fixed() -> void:
	var presentation := WorldPresentation.new()
	var world := FakeWorld.new()
	presentation._world = world
	var sky_material := ShaderMaterial.new()
	sky_material.shader = load("res://shaders/space_sky.gdshader") as Shader
	presentation._sky_mat = sky_material

	var ship := auto_free(FakeShip.new()) as FakeShip
	add_child(ship)
	ship.position = Vector3.ZERO
	ship.server_position_value = _position(0.0, 0.0, 0.0)
	ship.world_presentation_position_value = _position(25_000_000.0, 0.0, 0.0)
	var bodies: Array = [
		_body(1, "Star", "Helios", _position(0.0, 0.0, 0.0), 1000.0, 0.6),
	]
	presentation._update_sun_direction(7, {7: ship}, bodies)

	var expected: Dictionary = WorldPresentation.sun_state(
		bodies,
		ship.world_presentation_position_value,
		Callable(world, "dir_to_godot"))
	assert_vector(sky_material.get_shader_parameter("sun_direction") as Vector3) \
		.is_equal_approx(
			expected.get("direction", Vector3.ZERO) as Vector3,
			Vector3(0.0001, 0.0001, 0.0001))


func test_origin_rebase_moves_ship_and_motion_track_together() -> void:
	var presentation := WorldPresentation.new()
	var world := FakeWorld.new()
	world.rebase_shift = Vector3(100.0, 2.0, -3.0)
	presentation._world = world

	var ship := FakeShip.new()
	presentation.apply_origin_rebase(
		PackedFloat64Array([100.0, 2.0, -3.0]), false, -1, {7: ship})

	assert_array(ship.motion_rebase_calls[0]).contains_exactly([100.0, 2.0, -3.0])
	assert_array(ship.motion_rebase_calls).contains_exactly([
		PackedFloat64Array([100.0, 2.0, -3.0]),
	])
	ship.free()


## Typed fixture builders matching the navigation records returned by WorldSession.
func _gate(gate_id: int, pos: PackedFloat64Array, activation_radius: float, to_system_name: String) -> GateRecord:
	var gate := GateRecord.new()
	gate.gate_id = gate_id
	gate.position = pos
	gate.activation_radius = activation_radius
	gate.to_system_name = to_system_name
	return gate


func _station(station_id: int, name: String, pos: PackedFloat64Array, docking_radius: float) -> StationRecord:
	var station := StationRecord.new()
	station.station_id = station_id
	station.name = name
	station.position = pos
	station.docking_radius = docking_radius
	return station


func _body(body_id: int, kind: String, name: String, pos: PackedFloat64Array, radius: float, spectral_type: float) -> CelestialBodyRecord:
	var b := CelestialBodyRecord.new()
	b.body_id = body_id
	b.kind = kind
	b.name = name
	b.position = pos
	b.radius = radius
	b.spectral_type = spectral_type
	return b
