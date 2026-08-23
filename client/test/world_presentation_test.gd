## world_presentation_test.gd
##
## Tests for the client WorldPresentation seam. These focus on the pure
## presentation policy now extracted from main.gd: marker clamping, warp-tunnel
## easing, and sun-direction derivation from star data.
extends GdUnitTestSuite

const WorldPresentationScript = preload("res://scripts/world_presentation.gd")
const StarfieldScript = preload("res://scripts/starfield.gd")

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

	func distance_components(first: PackedFloat64Array, second: PackedFloat64Array) -> float:
		var dx: float = first[0] - second[0]
		var dy: float = first[1] - second[1]
		var dz: float = first[2] - second[2]
		return sqrt(dx * dx + dy * dy + dz * dz)

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
	var presentation := WorldPresentationScript.new()
	var world := FakeWorld.new()
	world.render_scale_value = 0.25
	presentation._world = world

	assert_float(presentation._render_scale()).is_equal_approx(0.25, 0.0001)


func test_space_background_uses_a_warm_nebula_palette() -> void:
	var presentation := WorldPresentationScript.new()
	var root: Node3D = auto_free(Node3D.new()) as Node3D
	add_child(root)
	presentation._world = FakeWorld.new()
	presentation._setup_space_environment(root)

	var world_environment: WorldEnvironment = root.get_child(0) as WorldEnvironment
	var sky_material: ShaderMaterial = world_environment.environment.sky.sky_material as ShaderMaterial
	var primary: Color = sky_material.get_shader_parameter("nebula_primary_color") as Color
	var secondary: Color = sky_material.get_shader_parameter("nebula_secondary_color") as Color
	var highlight: Color = sky_material.get_shader_parameter("nebula_highlight_color") as Color

	assert_float(sky_material.get_shader_parameter("nebula_strength") as float) \
		.is_equal_approx(0.72, 0.0001)
	assert_float(primary.r).is_equal_approx(1.00, 0.0001)
	assert_float(primary.g).is_equal_approx(0.16, 0.0001)
	assert_float(secondary.r).is_equal_approx(0.72, 0.0001)
	assert_float(secondary.g).is_equal_approx(0.24, 0.0001)
	assert_float(highlight.g).is_equal_approx(0.46, 0.0001)


func test_starfield_is_built_and_pinned_to_the_camera() -> void:
	var presentation := WorldPresentationScript.new()
	var root: Node3D = auto_free(Node3D.new()) as Node3D
	add_child(root)
	var camera: Camera3D = auto_free(Camera3D.new()) as Camera3D
	root.add_child(camera)
	presentation._camera = camera
	presentation._setup_starfield(root)

	var starfield: Node3D = root.get_node("Starfield") as Node3D
	var instance: MultiMeshInstance3D = starfield.get_child(0) as MultiMeshInstance3D
	assert_int(instance.multimesh.instance_count).is_greater(1000)

	# Stars are at infinity: the shell tracks where the camera is but never how
	# it is turned, so the field cannot parallax as the ship crosses AU.
	camera.global_position = Vector3(1200.0, -340.0, 90.0)
	camera.rotate_y(0.8)
	presentation._update_starfield()

	assert_vector(starfield.global_position).is_equal_approx(
		Vector3(1200.0, -340.0, 90.0), Vector3.ONE * 0.001)
	assert_vector(starfield.global_basis.get_euler()).is_equal_approx(
		Vector3.ZERO, Vector3.ONE * 0.001)


func test_nebula_bake_is_deferred_and_degrades_to_the_procedural_sky() -> void:
	var presentation := WorldPresentationScript.new()
	var root: Node3D = auto_free(Node3D.new()) as Node3D
	add_child(root)
	presentation._world = FakeWorld.new()
	presentation._setup_space_environment(root)
	var sky_material: ShaderMaterial = presentation._sky_mat

	# RenderingServer.sky_bake_panorama() reads a sky the renderer has already
	# processed, so the bake cannot run in the frame the Sky is created.
	presentation._maybe_bake_nebula()
	assert_bool(presentation._nebula_bake_done).is_false()
	assert_object(sky_material.get_shader_parameter("nebula_panorama")).is_null()

	for _tick: int in range(WorldPresentationScript.NEBULA_BAKE_FRAME + 2):
		presentation._maybe_bake_nebula()
	assert_bool(presentation._nebula_bake_done).is_true()

	# The headless dummy renderer returns no image. That is a supported outcome:
	# the shader keeps its procedural path so the sky is never left blank.
	var baked: Variant = sky_material.get_shader_parameter("use_baked_nebula")
	if sky_material.get_shader_parameter("nebula_panorama") == null:
		assert_bool(baked == true).is_false()


func test_nebula_bake_refuses_to_paint_the_local_star_into_the_background() -> void:
	var presentation := WorldPresentationScript.new()
	var root: Node3D = auto_free(Node3D.new()) as Node3D
	add_child(root)
	presentation._world = FakeWorld.new()
	presentation._setup_space_environment(root)

	# The star moves with the ship, so a background baked while it is lit would
	# carry a painted sun forever.
	presentation._sky_mat.set_shader_parameter("sun_active", 1.0)
	for _tick: int in range(WorldPresentationScript.NEBULA_BAKE_FRAME + 2):
		presentation._maybe_bake_nebula()

	assert_bool(presentation._nebula_bake_done).is_true()
	assert_object(presentation._sky_mat.get_shader_parameter("nebula_panorama")).is_null()


func test_distant_objects_wash_toward_the_background_but_close_combat_does_not() -> void:
	var presentation := WorldPresentationScript.new()
	var world := FakeWorld.new()
	world.render_scale_value = 0.1
	presentation._world = world
	var root: Node3D = auto_free(Node3D.new()) as Node3D
	add_child(root)
	presentation._setup_space_environment(root)
	var environment: Environment = (root.get_child(0) as WorldEnvironment).environment

	assert_bool(environment.fog_enabled).is_true()
	assert_int(environment.fog_mode).is_equal(Environment.FOG_MODE_DEPTH)

	# Aerial perspective takes the haze colour from the sky, which is what ties a
	# distant object to the nebula behind it rather than to a flat constant.
	assert_float(environment.fog_aerial_perspective).is_greater(0.5)
	# Fogging the sky would wash the background into itself.
	assert_float(environment.fog_sky_affect).is_equal_approx(0.0, 0.0001)

	# Combat happens within a few km. Haze must start well beyond that, or every
	# engagement would be fought through fog.
	var combat_range_metres: float = 500.0
	assert_float(environment.fog_depth_begin) 		.is_greater(combat_range_metres * world.render_scale_value)
	assert_float(environment.fog_depth_end).is_greater(environment.fog_depth_begin)


func test_starfield_opts_out_of_fog() -> void:
	# The sprite shell sits at SHELL_RADIUS, hundreds of times past the fog's end
	# distance, so without an explicit opt-out every star renders as fog colour.
	# Godot exposes no way to read a shader's render_mode, so this checks the
	# source; the alternative is no coverage of a whole-sky failure.
	var shader: Shader = load("res://shaders/star_sprite.gdshader") as Shader
	assert_str(shader.code).contains("fog_disabled")
	assert_float(StarfieldScript.SHELL_RADIUS).is_greater(
		WorldPresentationScript.FOG_END_METRES)


func test_sky_material_only_sets_uniforms_the_shader_declares() -> void:
	var presentation := WorldPresentationScript.new()
	var root: Node3D = auto_free(Node3D.new()) as Node3D
	add_child(root)
	presentation._world = FakeWorld.new()
	presentation._setup_space_environment(root)

	var world_environment: WorldEnvironment = root.get_child(0) as WorldEnvironment
	var sky_material: ShaderMaterial = world_environment.environment.sky.sky_material as ShaderMaterial
	var declared: Array[String] = []
	for uniform: Dictionary in sky_material.shader.get_shader_uniform_list():
		declared.append(uniform["name"] as String)

	# set_shader_parameter() accepts unknown names silently, so a uniform that
	# is renamed or dropped in the shader would otherwise take a presentation
	# setting with it and report nothing. An empty list also catches a shader
	# that failed to parse at all.
	for uniform_name: String in [
		"nebula_strength",
		"milkyway_strength",
		"milkyway_core_color",
		"milkyway_outer_color",
		"nebula_primary_color",
		"nebula_secondary_color",
		"nebula_highlight_color",
		"ambient_color",
		"nebula_panorama",
		"use_baked_nebula",
		"bake_pass",
		"sun_direction",
		"sun_active",
		"sun_color",
		"sun_angular_radius",
		"sun_flare_right",
		"sun_flare_up",
	]:
		assert_bool(declared.has(uniform_name)) 			.override_failure_message("sky shader declares no uniform " + uniform_name) 			.is_true()


func test_render_scale_authority_controls_spawned_navigation_geometry() -> void:
	var presentation := WorldPresentationScript.new()
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
	assert_float(planet_mesh.radius).is_equal_approx(5.0, 0.0001)

	var station_marker: Node3D = bodies_root.get_child(1) as Node3D
	var station_ring: MeshInstance3D = station_marker.get_child(1) as MeshInstance3D
	var station_mesh: TorusMesh = station_ring.mesh as TorusMesh
	assert_float(station_mesh.outer_radius).is_equal_approx(20.0, 0.0001)


func test_clamped_marker_position_leaves_nearby_marker_unchanged() -> void:
	var player := Vector3.ZERO
	var marker := Vector3(100.0, 0.0, 0.0)

	var result: Vector3 = WorldPresentationScript.clamped_marker_position(player, marker, 500.0)

	assert_vector(result).is_equal(marker)


func test_clamped_marker_position_pulls_far_marker_back_to_clamp_radius() -> void:
	var player := Vector3.ZERO
	var marker := Vector3(1000.0, 0.0, 0.0)

	var result: Vector3 = WorldPresentationScript.clamped_marker_position(player, marker, 250.0)

	assert_vector(result).is_equal(Vector3(250.0, 0.0, 0.0))


func test_next_warp_tunnel_amount_eases_toward_one_above_threshold() -> void:
	var amount: float = WorldPresentationScript.next_warp_tunnel_amount(0.0, 3000.0, 0.1, 2000.0, 3.0)

	assert_float(amount).is_equal_approx(0.3, 0.0001)


func test_warp_arrival_keeps_the_tunnel_visible_during_the_first_tenth_second() -> void:
	var amount: float = WorldPresentationScript.next_warp_tunnel_amount(1.0, 0.0, 0.1)

	assert_float(amount).is_equal_approx(0.95, 0.0001)


func test_warp_arrival_frame_hitch_cannot_clear_the_tunnel_in_one_step() -> void:
	var amount: float = WorldPresentationScript.next_warp_tunnel_amount(1.0, 0.0, 0.5)

	assert_float(amount).is_equal_approx(0.95, 0.0001)


func test_screen_flow_direction_follows_velocity_projection() -> void:
	var direction := WorldPresentationScript.screen_flow_direction(
		Vector3.RIGHT, Vector3.RIGHT, Vector3.UP)

	assert_vector(direction).is_equal_approx(Vector2.RIGHT, Vector2(0.0001, 0.0001))


func test_screen_flow_direction_falls_back_when_velocity_points_into_view() -> void:
	var direction := WorldPresentationScript.screen_flow_direction(
		Vector3.FORWARD, Vector3.RIGHT, Vector3.UP)

	assert_vector(direction).is_equal_approx(Vector2(0.0, -1.0), Vector2(0.0001, 0.0001))


func test_screen_flow_confidence_is_full_for_sideways_velocity() -> void:
	var confidence := WorldPresentationScript.screen_flow_confidence(
		Vector3.RIGHT, Vector3.RIGHT, Vector3.UP)

	assert_float(confidence).is_equal_approx(1.0, 0.0001)


func test_screen_flow_confidence_is_zero_for_depth_only_velocity() -> void:
	var confidence := WorldPresentationScript.screen_flow_confidence(
		Vector3.FORWARD, Vector3.RIGHT, Vector3.UP)

	assert_float(confidence).is_equal_approx(0.0, 0.0001)


func test_sun_state_returns_inactive_when_no_star_exists() -> void:
	var state: Dictionary = WorldPresentationScript.sun_state([
		_body(2, "Planet", "Forge", _position(100.0, 0.0, 0.0), 200.0, 0.1),
	], _position(0.0, 0.0, 0.0), func(diff: Vector3) -> Vector3:
		return diff
	)

	assert_bool(state.get("active", true) as bool).is_false()


func test_sun_state_returns_direction_and_color_from_star_data() -> void:
	var state: Dictionary = WorldPresentationScript.sun_state([
		_body(1, "Star", "Helios", _position(0.0, 0.0, 0.0), 1000.0, 0.0),
	], _position(5000.0, 0.0, 0.0), func(diff: Vector3) -> Vector3:
		return diff
	)

	assert_bool(state.get("active", false) as bool).is_true()
	assert_vector(state.get("direction", Vector3.ZERO) as Vector3) \
		.is_equal_approx(Vector3(-1.0, 0.0, 0.0), Vector3(0.0001, 0.0001, 0.0001))
	assert_vector(state.get("color", Vector3.ZERO) as Vector3) \
		.is_equal_approx(Vector3(0.55, 0.65, 1.00), Vector3(0.0001, 0.0001, 0.0001))


func test_sun_state_is_inactive_inside_the_photosphere() -> void:
	var state: Dictionary = WorldPresentationScript.sun_state([
		_body(1, "Star", "Helios", _position(0.0, 0.0, 0.0), 1000.0, 0.6),
	], _position(0.0, 0.0, 0.0), func(diff: Vector3) -> Vector3:
		return diff
	)

	assert_bool(state.get("active", true) as bool).is_false()
	assert_bool(state.get("invalid_position", false) as bool).is_true()


func test_sun_angular_radius_grows_as_the_observer_approaches_the_star() -> void:
	var bodies: Array = [
		_body(1, "Star", "Helios", _position(0.0, 0.0, 0.0), 1000.0, 0.6),
	]
	var far_state: Dictionary = WorldPresentationScript.sun_state(
		bodies, _position(20_000.0, 0.0, 0.0), func(diff: Vector3) -> Vector3:
			return diff)
	var near_state: Dictionary = WorldPresentationScript.sun_state(
		bodies, _position(10_000.0, 0.0, 0.0), func(diff: Vector3) -> Vector3:
			return diff)

	assert_float(near_state.get("angular_radius", 0.0) as float) \
		.is_greater(far_state.get("angular_radius", 0.0) as float)
	assert_float(near_state.get("angular_radius", 0.0) as float) \
		.is_equal_approx(asin(0.1), 0.0001)


func test_sun_angular_radius_is_not_capped_at_valid_physical_distance() -> void:
	assert_float(WorldPresentationScript.sun_angular_radius(1_000.0, 2_000.0)) \
		.is_equal_approx(asin(0.5), 0.0001)


func test_directional_light_uses_the_ray_from_star_to_ship() -> void:
	var sun_direction := Vector3(0.6, 0.0, 0.8).normalized()

	assert_vector(WorldPresentationScript.star_light_ray(sun_direction)) \
		.is_equal_approx(-sun_direction, Vector3(0.0001, 0.0001, 0.0001))
	assert_vector(WorldPresentationScript.light_up_axis(Vector3.UP)) \
		.is_equal_approx(Vector3.RIGHT, Vector3(0.0001, 0.0001, 0.0001))


func test_sun_direction_uses_world_position_while_warp_render_position_is_fixed() -> void:
	var presentation := WorldPresentationScript.new()
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

	var expected: Dictionary = WorldPresentationScript.sun_state(
		bodies,
		ship.world_presentation_position_value,
		Callable(world, "dir_to_godot"))
	assert_vector(sky_material.get_shader_parameter("sun_direction") as Vector3) \
		.is_equal_approx(
			expected.get("direction", Vector3.ZERO) as Vector3,
			Vector3(0.0001, 0.0001, 0.0001))
	assert_float(sky_material.get_shader_parameter("sun_angular_radius") as float) \
		.is_equal_approx(
			maxf(expected.get("angular_radius", 0.0) as float, WorldPresentationScript.SUN_MIN_RENDER_ANGULAR_RADIUS),
			0.0000001)


func test_physical_body_marker_keeps_its_true_render_position() -> void:
	var presentation := WorldPresentationScript.new()
	var world := FakeWorld.new()
	world.render_scale_value = 0.1
	presentation._world = world
	var bodies_root: Node3D = auto_free(Node3D.new())
	add_child(bodies_root)
	presentation._bodies_root = bodies_root
	presentation.respawn_navigation_markers(
		[],
		[_body(2, "Planet", "Forge", _position(400_000.0, 0.0, 0.0), 6_400_000.0, 0.0)],
		[],
		func() -> void:
			pass)
	var ship := auto_free(FakeShip.new()) as FakeShip
	add_child(ship)
	ship.server_position_value = _position(0.0, 0.0, 0.0)
	presentation._update_position_markers(bodies_root, "nav_pos", 7, {7: ship})
	assert_vector((bodies_root.get_child(0) as Node3D).global_position) \
		.is_equal_approx(Vector3(40_000.0, 0.0, 0.0), Vector3(0.0001, 0.0001, 0.0001))


func test_origin_rebase_moves_ship_and_motion_track_together() -> void:
	var presentation := WorldPresentationScript.new()
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


func _station(station_id: int, station_name: String, pos: PackedFloat64Array, docking_radius: float) -> StationRecord:
	var station := StationRecord.new()
	station.station_id = station_id
	station.name = station_name
	station.position = pos
	station.docking_radius = docking_radius
	return station


func _body(body_id: int, kind: String, body_name: String, pos: PackedFloat64Array, radius: float, spectral_type: float) -> CelestialBodyRecord:
	var b := CelestialBodyRecord.new()
	b.body_id = body_id
	b.kind = kind
	b.name = body_name
	b.position = pos
	b.radius = radius
	b.spectral_type = spectral_type
	return b
