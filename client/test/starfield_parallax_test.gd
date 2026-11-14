## starfield_parallax_test.gd
##
## Keeps camera-relative parallax coverage isolated from render-device tests.
extends GdUnitTestSuite

const WorldPresentationScript = preload("res://scripts/world_presentation.gd")
const StarfieldScript = preload("res://scripts/starfield.gd")
const GalacticModelScript = preload("res://scripts/galactic_model.gd")


func test_starfield_depth_and_pixel_spread_are_configured() -> void:
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


func test_starfield_is_centered_on_the_player_not_the_orbiting_camera() -> void:
	var presentation := WorldPresentationScript.new()
	var root: Node3D = auto_free(Node3D.new()) as Node3D
	add_child(root)
	var camera: Camera3D = auto_free(Camera3D.new()) as Camera3D
	root.add_child(camera)
	var ship: Node3D = auto_free(Node3D.new()) as Node3D
	root.add_child(ship)
	ship.global_position = Vector3(40.0, 20.0, -10.0)
	presentation._camera = camera
	presentation._setup_starfield(root)

	var starfield: Node3D = root.get_node("Starfield") as Node3D
	camera.global_position = Vector3(1200.0, -340.0, 90.0)
	camera.rotate_y(0.8)
	presentation._update_starfield(7, {7: ship})

	assert_vector(starfield.global_position).is_equal_approx(
		ship.global_position, Vector3.ONE * 0.001)
	assert_vector(starfield.global_basis.get_euler()).is_equal_approx(
		Vector3.ZERO, Vector3.ONE * 0.001)
