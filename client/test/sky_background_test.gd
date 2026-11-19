extends GdUnitTestSuite

const BackgroundScript = preload("res://scripts/sky_background.gd")


func _fixture() -> SkyBackground:
	var background: SkyBackground = auto_free(BackgroundScript.new())
	add_child(background)
	var material := ShaderMaterial.new()
	material.shader = load("res://shaders/space_sky.gdshader")
	background.build(material)
	background.request_background(7, Vector2i(16, 8), 16)
	return background


func _wait_ready(background: SkyBackground, camera: Camera3D) -> void:
	var deadline := Time.get_ticks_msec() + 15000
	while not background.is_background_ready() and Time.get_ticks_msec() < deadline:
		background.update_frame(camera)
		await get_tree().process_frame
	assert_bool(background.is_background_ready()).is_true()


func test_generated_panorama_and_stars_are_installed_together() -> void:
	var background := _fixture()
	var camera := auto_free(Camera3D.new()) as Camera3D
	add_child(camera)
	await _wait_ready(background, camera)
	var stars := background.get_node("Stars") as MultiMeshInstance3D
	assert_int(stars.multimesh.instance_count).is_equal(16)
	assert_bool(background._sky_material.get_shader_parameter("background_ready") as bool).is_true()
	var texture := background._sky_material.get_shader_parameter("background_panorama") as Texture2D
	assert_vector(Vector2(texture.get_size())).is_equal(Vector2(16, 8))
	assert_float((stars.material_override as ShaderMaterial).get_shader_parameter(
		"viewport_pixels").x).is_greater(0.0)


func test_translation_changes_neither_star_directions_nor_sky_orientation() -> void:
	var background := _fixture()
	var camera := auto_free(Camera3D.new()) as Camera3D
	add_child(camera)
	await _wait_ready(background, camera)
	var stars := background.get_node("Stars") as MultiMeshInstance3D
	var offset := stars.multimesh.get_instance_transform(0).origin
	var before := (stars.to_global(offset) - camera.global_position).normalized()
	camera.global_position = Vector3(1000000.0, -40000.0, 1200.0)
	camera.rotate_y(0.4)
	background.update_frame(camera)
	var after := (stars.to_global(offset) - camera.global_position).normalized()
	assert_vector(after).is_equal_approx(before, Vector3.ONE * 0.0001)
	assert_vector(background.global_basis.get_euler()).is_equal_approx(Vector3.ZERO, Vector3.ONE * 0.0001)


func test_invalid_replacement_retains_completed_background() -> void:
	var background := _fixture()
	await _wait_ready(background, null)
	var material := background._sky_material
	var previous: Texture2D = material.get_shader_parameter("background_panorama")
	background.request_background(10, Vector2i(-1, 8), 16)
	background.update_frame(null)
	assert_object(material.get_shader_parameter("background_panorama")).is_same(previous)
	assert_bool(background.is_background_ready()).is_true()


func test_local_stellar_disc_stays_independent_of_background_generation() -> void:
	var background := _fixture()
	background._sky_material.set_shader_parameter("sun_active", 1.0)
	background._sky_material.set_shader_parameter("sun_direction", Vector3.RIGHT)
	await _wait_ready(background, null)
	assert_float(background._sky_material.get_shader_parameter("sun_active") as float).is_equal(1.0)
	assert_vector(background._sky_material.get_shader_parameter("sun_direction") as Vector3).is_equal(Vector3.RIGHT)
