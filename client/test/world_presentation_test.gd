## world_presentation_test.gd
##
## Tests for the client WorldPresentation seam. These focus on the pure
## presentation policy now extracted from main.gd: marker clamping, warp-tunnel
## easing, and sun-direction derivation from star data.
extends GdUnitTestSuite

const WorldPresentationScript = preload("res://scripts/world_presentation.gd")
const StarfieldScript = preload("res://scripts/starfield.gd")
const GalacticModelScript = preload("res://scripts/galactic_model.gd")


func test_projection_adapter_rejects_invalid_coordinate_shapes() -> void:
	assert_object(CelestialProjection.sun(PackedFloat64Array(), _position(100.0, 0.0, 0.0), 1.0)).is_null()
	assert_object(CelestialProjection.marker(_position(0.0, 0.0, 0.0), PackedFloat64Array([1.0]),
		Vector3.ZERO, null, 0.1, false, false, 0.0)).is_null()


func test_sun_flare_rejects_rear_and_offscreen_directions() -> void:
	var camera: Camera3D = auto_free(Camera3D.new())
	add_child(camera)
	assert_dict(WorldPresentationScript.project_sun_flare(camera, Vector3.BACK, 0.005)).is_empty()
	assert_dict(WorldPresentationScript.project_sun_flare(camera, Vector3(10, 0, -1), 0.005)).is_empty()
	assert_dict(WorldPresentationScript.project_sun_flare(camera, Vector3.ZERO, 0.005)).is_empty()
	assert_dict(WorldPresentationScript.project_sun_flare(camera, Vector3.FORWARD, 0.0)).is_empty()
	var source := WorldPresentationScript.project_sun_flare(camera, Vector3.FORWARD, 0.005)
	assert_vector(source.position as Vector2).is_equal_approx(Vector2(0.5, 0.5), Vector2(0.001, 0.001))
	assert_float(source.visibility as float).is_equal(1.0)


func test_close_resolved_disc_keeps_the_same_camera_energy_partition() -> void:
	var root: Node3D = auto_free(Node3D.new())
	add_child(root)
	var camera := Camera3D.new()
	root.add_child(camera)
	var presentation := WorldPresentationScript.new()
	presentation._camera = camera
	presentation._world = FakeWorld.new()
	presentation._setup_space_environment(root)
	presentation._update_sun_flare(Vector3.FORWARD, Vector3.ONE, 0.005)
	assert_float(presentation._sky_mat.get_shader_parameter("sun_direct_fraction")).is_less(1.0)
	presentation._update_sun_flare(Vector3.FORWARD, Vector3.ONE, 0.7)
	assert_float(presentation._sky_mat.get_shader_parameter("sun_direct_fraction")).is_equal_approx(presentation._sun_flare.direct_fraction(), 0.0001)
	assert_bool(presentation._sun_flare.enabled).is_true()


func test_sun_flare_tracks_direction_and_apparent_radius_not_absolute_distance() -> void:
	var camera: Camera3D = auto_free(Camera3D.new())
	add_child(camera)
	var first := WorldPresentationScript.project_sun_flare(camera, Vector3.FORWARD, 0.005)
	var moved := WorldPresentationScript.project_sun_flare(camera, Vector3(0.2, 0, -1), 0.01)
	assert_float((moved.position as Vector2).x).is_greater((first.position as Vector2).x)
	assert_float(moved.radius as float).is_greater(first.radius as float)
	camera.rotation.y = PI
	assert_dict(WorldPresentationScript.project_sun_flare(camera, Vector3.FORWARD, 0.005)).is_empty()


func test_resolved_stellar_disc_expands_the_same_source_footprint() -> void:
	var camera: Camera3D = auto_free(Camera3D.new())
	add_child(camera)
	var distant := WorldPresentationScript.project_sun_flare(camera, Vector3.FORWARD, 0.005)
	var nearby := WorldPresentationScript.project_sun_flare(camera, Vector3.FORWARD, asin(1.0 / 1.5))
	assert_float(distant.visibility).is_equal(1.0)
	assert_float(nearby.visibility).is_equal(1.0)
	assert_float(nearby.radius).is_greater(12.0)

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


func test_space_background_uses_the_shared_galactic_model() -> void:
	var presentation := WorldPresentationScript.new()
	var root: Node3D = auto_free(Node3D.new()) as Node3D
	add_child(root)
	presentation._world = FakeWorld.new()
	presentation._setup_space_environment(root)

	var world_environment: WorldEnvironment = root.get_child(0) as WorldEnvironment
	var sky_material: ShaderMaterial = world_environment.environment.sky.sky_material as ShaderMaterial
	assert_float(sky_material.get_shader_parameter("display_radiance_scale") as float) \
		.is_equal_approx(GalacticModelScript.DISPLAY_RADIANCE_SCALE, 0.0001)
	assert_float(sky_material.get_shader_parameter("extinction_scale") as float) \
		.is_equal_approx(1.0, 0.0001)
	assert_float(sky_material.get_shader_parameter("dust_scale_length_kpc") as float) \
		.is_equal_approx(GalacticModelScript.DUST_RADIAL_SCALE_KPC, 0.0001)
	assert_float(sky_material.get_shader_parameter("dust_scale_height_kpc") as float) \
		.is_equal_approx(GalacticModelScript.DUST_SCALE_HEIGHT_KPC, 0.0001)
	assert_float(sky_material.get_shader_parameter("thin_disk_scale_height_kpc") as float) \
		.is_equal_approx(GalacticModelScript.THIN_DISK_SCALE_HEIGHT_KPC, 0.0001)
	assert_float(sky_material.get_shader_parameter("bulge_density_normalization") as float) \
		.is_equal_approx(GalacticModelScript.bulge_density_normalization(), 0.0001)
	assert_vector(sky_material.get_shader_parameter("dust_turbulence_octave_weights") as Vector3) \
		.is_equal_approx(GalacticModelScript.DUST_TURBULENCE_OCTAVE_WEIGHTS, Vector3.ONE * 0.0001)
	assert_float(sky_material.get_shader_parameter("dust_turbulence_normalization") as float) \
		.is_equal_approx(GalacticModelScript.dust_turbulence_normalization(), 0.0001)
	assert_float(sky_material.get_shader_parameter("dust_projected_turbulence_normalization") as float) \
		.is_equal_approx(GalacticModelScript.dust_turbulence_normalization(), 0.0001)
	assert_float(sky_material.get_shader_parameter("dust_volume_log_density_sigma") as float) \
		.is_equal_approx(GalacticModelScript.dust_volume_log_density_sigma(), 0.0001)
	assert_float(sky_material.get_shader_parameter("dust_projected_log_density_sigma") as float) \
		.is_equal_approx(GalacticModelScript.dust_projected_log_density_sigma(), 0.0001)
	assert_vector(sky_material.get_shader_parameter("dust_cloud_angular_scales_radians") as Vector3) \
		.is_equal_approx(GalacticModelScript.DUST_CLOUD_ANGULAR_SCALES_RADIANS, Vector3.ONE * 0.0001)
	assert_vector(sky_material.get_shader_parameter("stellar_mottle_weights") as Vector2) \
		.is_equal_approx(GalacticModelScript.STELLAR_MOTTLE_WEIGHTS, Vector2.ONE * 0.0001)
	assert_vector(sky_material.get_shader_parameter("stellar_mottle_angular_scales_radians") as Vector2) \
		.is_equal_approx(GalacticModelScript.STELLAR_MOTTLE_ANGULAR_SCALES_RADIANS, Vector2.ONE * 0.0001)
	assert_float(sky_material.get_shader_parameter("stellar_mottle_noise_normalization") as float) \
		.is_equal_approx(GalacticModelScript.stellar_mottle_noise_normalization(), 0.0001)


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
	assert_float(presentation._starfield_mat.get_shader_parameter(
		"core_sigma_pixels") as float) \
		.is_equal_approx(StarfieldScript.POINT_CORE_SIGMA_PIXELS, 0.0001)
	assert_float(presentation._starfield_mat.get_shader_parameter(
		"halo_sigma_pixels") as float) \
		.is_equal_approx(StarfieldScript.POINT_HALO_SIGMA_PIXELS, 0.0001)
	assert_float(presentation._starfield_mat.get_shader_parameter(
		"halo_peak_ratio") as float) \
		.is_equal_approx(StarfieldScript.POINT_HALO_PEAK_RATIO, 0.0001)
	var star_uniforms: Array[String] = []
	for uniform: Dictionary in presentation._starfield_mat.shader.get_shader_uniform_list():
		star_uniforms.append(uniform["name"] as String)
	for uniform_name: String in [
		"point_spread_pixels",
		"core_sigma_pixels",
		"halo_sigma_pixels",
		"halo_peak_ratio",
	]:
		assert_bool(star_uniforms.has(uniform_name)) \
			.override_failure_message("starfield shader declares no uniform " + uniform_name) \
			.is_true()
	assert_float(presentation._starfield_mat.get_shader_parameter(
		"extinction_scale") as float).is_equal_approx(1.0, 0.0001)
	assert_float(presentation._starfield_mat.get_shader_parameter(
		"dust_projected_log_density_sigma") as float) \
		.is_equal_approx(GalacticModelScript.dust_projected_log_density_sigma(), 0.0001)

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
		"display_radiance_scale",
		"extinction_scale",
		"galactic_sun_position_kpc",
		"galactic_center_radius_kpc",
		"thin_disk_radial_scale_kpc",
		"thick_disk_radial_scale_kpc",
		"thin_disk_scale_height_kpc",
		"thick_disk_scale_height_kpc",
		"thick_disk_local_weight",
		"bulge_angle_radians",
		"bulge_sigma_axes_kpc",
		"bulge_density_normalization",
		"dust_scale_length_kpc",
		"dust_scale_height_kpc",
		"local_av_extinction_mag_per_kpc",
		"local_v_luminosity_density_lsun_pc3",
		"dust_turbulence_octave_weights",
		"dust_turbulence_normalization",
		"dust_volume_log_density_sigma",
		"dust_projected_log_density_sigma",
		"dust_cloud_angular_scales_radians",
		"dust_projected_turbulence_normalization",
		"stellar_mottle_weights",
		"stellar_mottle_angular_scales_radians",
		"stellar_mottle_noise_normalization",
		"magnitude_to_optical_depth",
		"rgb_extinction_over_av",
		"ambient_color",
		"nebula_panorama",
		"use_baked_nebula",
		"bake_pass",
		"sun_direction",
		"sun_active",
		"sun_color",
		"sun_angular_radius",
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
	var station_visual: MeshInstance3D = station_marker.get_child(0) as MeshInstance3D
	var station_mesh: SphereMesh = station_visual.mesh as SphereMesh
	assert_float(station_mesh.radius).is_equal_approx(NavigationMarkerRenderer.STATION_VISUAL_RADIUS * 0.25, 0.0001)


func test_star_bracket_tracks_the_sky_direction_and_remains_pickable() -> void:
	var presentation := WorldPresentationScript.new()
	presentation._world = FakeWorld.new()
	var camera: Camera3D = auto_free(Camera3D.new())
	add_child(camera)
	camera.position = Vector3(120.0, 40.0, 300.0)
	camera.far = 100_000.0
	presentation._camera = camera
	var root: Node3D = auto_free(Node3D.new())
	add_child(root)
	NavigationMarkerRenderer.spawn_body_markers(root,
		[_body(1, "Star", "Helios", _position(0.0, 0.0, 1.0e12), 696_000_000.0, 0.5)],
		0.25, presentation._world.to_godot_components)
	var ship: FakeShip = auto_free(FakeShip.new())
	add_child(ship)
	for distance: float in [1.0e12, 1_044_000_000.0]:
		ship.world_presentation_position_value = _position(0.0, 0.0, 1.0e12 - distance)
		presentation._update_position_markers(root, "nav_pos", 7, {7: ship})
		var marker := root.get_child(0) as Node3D
		assert_vector((marker.global_position - camera.global_position).normalized()).is_equal_approx(
			Vector3.FORWARD, Vector3.ONE * 0.0001)
		assert_int(ShipPicking.pick_body_at(camera, camera.unproject_position(marker.global_position), root)).is_equal(1)


func test_warp_destination_approaches_before_the_arrival_snap() -> void:
	var presentation := WorldPresentationScript.new()
	presentation._world = FakeWorld.new()
	var root: Node3D = auto_free(Node3D.new())
	add_child(root)
	var marker := Node3D.new()
	root.add_child(marker)
	marker.set_meta("nav_pos", _position(1.0e12, 0.0, 0.0))
	var ship: FakeShip = auto_free(FakeShip.new())
	add_child(ship)
	ship.position = Vector3(100.0, 0.0, 0.0)
	# The mesh's bounded track is far behind the observer during warp.
	ship.server_position_value = _position(400.0, 0.0, 0.0)
	ship.world_presentation_position_value = _position(1.0e12 - 40_000.0, 0.0, 0.0)
	presentation._update_position_markers(root, "nav_pos", 7, {7: ship})
	assert_vector(marker.global_position - ship.global_position).is_equal_approx(
		Vector3(10_000.0, 0.0, 0.0), Vector3.ONE * 0.01)
	ship.world_presentation_position_value = _position(1.0e12 - 4_000.0, 0.0, 0.0)
	presentation._update_position_markers(root, "nav_pos", 7, {7: ship})
	assert_vector(marker.global_position - ship.global_position).is_equal_approx(
		Vector3(1_000.0, 0.0, 0.0), Vector3.ONE * 0.01)


func test_next_warp_tunnel_amount_eases_toward_one_above_threshold() -> void:
	var amount: float = WorldPresentationScript.next_warp_tunnel_amount(0.0, 3000.0, 0.1, 2000.0, 3.0)

	assert_float(amount).is_equal_approx(0.3, 0.0001)


func test_warp_arrival_keeps_the_tunnel_visible_during_the_first_tenth_second() -> void:
	var amount: float = WorldPresentationScript.next_warp_tunnel_amount(1.0, 0.0, 0.1)

	assert_float(amount).is_equal_approx(0.95, 0.0001)


func test_warp_arrival_frame_hitch_cannot_clear_the_tunnel_in_one_step() -> void:
	var amount: float = WorldPresentationScript.next_warp_tunnel_amount(1.0, 0.0, 0.5)

	assert_float(amount).is_equal_approx(0.95, 0.0001)


func test_warp_effect_preserves_world_travel_direction_when_camera_turns() -> void:
	var effect: ColorRect = auto_free(load("res://scripts/warp_tunnel_effect.gd").new())
	var shader_material := ShaderMaterial.new()
	shader_material.shader = load("res://shaders/warp_tunnel.gdshader")
	effect.material = shader_material
	for angle in [0.0, PI * 0.5, PI]:
		var view := Basis(Vector3.UP, angle)
		effect.set_motion(Vector3.FORWARD * 3000.0, view, 67.0)
		assert_vector(shader_material.get_shader_parameter("travel_direction") as Vector3).is_equal(Vector3.FORWARD)
		var actual_basis: Basis = shader_material.get_shader_parameter("camera_basis")
		assert_bool(actual_basis.is_equal_approx(view)).is_true()
		assert_float(shader_material.get_shader_parameter("camera_fov") as float).is_equal(67.0)


func test_warp_effect_retains_arrival_axis_but_keeps_camera_live() -> void:
	var effect: ColorRect = auto_free(load("res://scripts/warp_tunnel_effect.gd").new())
	var shader_material := ShaderMaterial.new()
	shader_material.shader = load("res://shaders/warp_tunnel.gdshader")
	effect.material = shader_material
	effect.set_motion(Vector3.RIGHT * 3000.0, Basis.IDENTITY, 67.0)
	var view := Basis(Vector3.UP, PI)
	effect.set_motion(Vector3.ZERO, view, 60.0)
	assert_vector(shader_material.get_shader_parameter("travel_direction") as Vector3).is_equal(Vector3.RIGHT)
	var actual_basis: Basis = shader_material.get_shader_parameter("camera_basis")
	assert_bool(actual_basis.is_equal_approx(view)).is_true()


func test_sun_state_returns_inactive_when_no_star_exists() -> void:
	var sky := _present_sun([
		_body(2, "Planet", "Forge", _position(100.0, 0.0, 0.0), 200.0, 0.1),
	], _position(0.0, 0.0, 0.0))
	assert_float(sky.get_shader_parameter("sun_active") as float).is_equal(0.0)


func test_sun_state_returns_direction_and_color_from_star_data() -> void:
	var sky := _present_sun([
		_body(1, "Star", "Helios", _position(0.0, 0.0, 0.0), 1000.0, 0.0),
	], _position(5000.0, 0.0, 0.0))
	assert_float(sky.get_shader_parameter("sun_active") as float).is_equal(1.0)
	assert_vector(sky.get_shader_parameter("sun_direction") as Vector3) \
		.is_equal_approx(Vector3(-1.0, 0.0, 0.0), Vector3(0.0001, 0.0001, 0.0001))
	assert_vector(sky.get_shader_parameter("sun_color") as Vector3) \
		.is_equal_approx(Vector3(0.55, 0.65, 1.00), Vector3(0.0001, 0.0001, 0.0001))


func test_sun_state_is_inactive_inside_the_photosphere() -> void:
	var sky := _present_sun([
		_body(1, "Star", "Helios", _position(0.0, 0.0, 0.0), 1000.0, 0.6),
	], _position(0.0, 0.0, 0.0))
	assert_float(sky.get_shader_parameter("sun_active") as float).is_equal(0.0)


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

	var expected := CelestialProjection.sun(ship.world_presentation_position_value, bodies[0].position, bodies[0].radius)
	assert_vector(sky_material.get_shader_parameter("sun_direction") as Vector3) \
		.is_equal_approx(
			expected.direction,
			Vector3(0.0001, 0.0001, 0.0001))
	assert_float(sky_material.get_shader_parameter("sun_angular_radius") as float) \
		.is_equal_approx(
			maxf(expected.angular_radius, WorldPresentationScript.SUN_MIN_RENDER_ANGULAR_RADIUS),
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


func test_distant_planet_remains_visible_at_its_true_angular_size() -> void:
	var presentation := WorldPresentationScript.new()
	var world := FakeWorld.new()
	world.render_scale_value = 0.1
	presentation._world = world
	var camera: Camera3D = auto_free(Camera3D.new())
	add_child(camera)
	camera.far = 100_000.0
	presentation._camera = camera
	var bodies_root: Node3D = auto_free(Node3D.new())
	add_child(bodies_root)
	presentation._bodies_root = bodies_root
	presentation.respawn_navigation_markers([], [
		_body(2, "Planet", "Forge", _position(0.0, 0.0, 1.0e9), 6_400_000.0, 0.0)
	], [], func() -> void: pass)
	var ship: FakeShip = auto_free(FakeShip.new())
	add_child(ship)
	var marker: Node3D = bodies_root.get_child(0)
	var mesh: MeshInstance3D = marker.get_meta("physical_body_mesh")
	for distance: float in [1.0e9, 1.0e8, 1.0e7]:
		ship.world_presentation_position_value = _position(0.0, 0.0, 1.0e9 - distance)
		presentation._update_position_markers(bodies_root, "nav_pos", 7, {7: ship})
		assert_bool(mesh.visible).is_true()
		var radius: float = (mesh.mesh as SphereMesh).radius * mesh.scale.x
		var rendered_distance := marker.global_position.distance_to(camera.global_position)
		assert_float(radius / rendered_distance).is_equal_approx(6_400_000.0 / distance, 0.00001)
		assert_float(rendered_distance + radius).is_less(camera.far)


func test_origin_rebase_moves_ship_and_motion_track_together() -> void:
	var presentation := WorldPresentationScript.new()
	var world := FakeWorld.new()
	world.rebase_shift = Vector3(100.0, 2.0, -3.0)
	presentation._world = world

	var camera: Camera3D = auto_free(load("res://scripts/camera_controller.gd").new())
	add_child(camera)
	camera.set_process(false)
	presentation._camera = camera
	for keep_fixed: bool in [false, true]:
		var ship := FakeShip.new()
		camera.position = Vector3(10, 20, 30)
		camera._target_pos = Vector3.ZERO
		presentation.apply_origin_rebase_components(
			PackedFloat64Array([100.0, 2.0, -3.0]), keep_fixed, {7: ship})
		assert_array(ship.motion_rebase_calls).contains_exactly([
			PackedFloat64Array([100.0, 2.0, -3.0]),
		])
		var shift := Vector3.ZERO if keep_fixed else world.rebase_shift
		assert_vector(camera.global_position).is_equal(Vector3(10, 20, 30) + shift)
		assert_vector(camera._target_pos).is_equal(shift)
		ship.free()


func _present_sun(bodies: Array, observer: PackedFloat64Array) -> ShaderMaterial:
	var presentation := WorldPresentationScript.new()
	presentation._world = FakeWorld.new()
	var sky := ShaderMaterial.new()
	sky.shader = load("res://shaders/space_sky.gdshader")
	presentation._sky_mat = sky
	var ship: FakeShip = auto_free(FakeShip.new())
	add_child(ship)
	ship.world_presentation_position_value = observer
	presentation._update_sun_direction(7, {7: ship}, bodies)
	return sky


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
