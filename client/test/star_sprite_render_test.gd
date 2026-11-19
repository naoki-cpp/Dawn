extends GdUnitTestSuite


func test_angular_flux_renders_a_bounded_point_under_vulkan_projection() -> void:
	assert_object(RenderingServer.get_rendering_device()).is_not_null()
	var viewport: SubViewport = auto_free(SubViewport.new())
	viewport.size = Vector2i(129, 129)
	viewport.own_world_3d = true
	viewport.render_target_update_mode = SubViewport.UPDATE_ALWAYS
	add_child(viewport)
	var environment := WorldEnvironment.new()
	environment.environment = Environment.new()
	environment.environment.background_mode = Environment.BG_COLOR
	environment.environment.background_color = Color.BLACK
	environment.environment.tonemap_mode = Environment.TONE_MAPPER_LINEAR
	viewport.add_child(environment)
	var camera := Camera3D.new()
	viewport.add_child(camera)
	camera.current = true
	var material := ShaderMaterial.new()
	material.shader = load("res://shaders/star_sprite.gdshader")
	material.set_shader_parameter("viewport_pixels", Vector2(viewport.size))
	var stars := MultiMeshInstance3D.new()
	stars.material_override = material
	stars.multimesh = MultiMesh.new()
	stars.multimesh.transform_format = MultiMesh.TRANSFORM_3D
	stars.multimesh.use_colors = true
	stars.multimesh.use_custom_data = true
	stars.multimesh.mesh = QuadMesh.new()
	stars.multimesh.instance_count = 1
	stars.multimesh.set_instance_transform(0, Transform3D(Basis.IDENTITY, Vector3(0, 0, -100)))
	stars.multimesh.set_instance_color(0, Color.WHITE)
	stars.multimesh.set_instance_custom_data(0, Color(0.000001, 0, 0, 0))
	viewport.add_child(stars)
	for field_of_view: float in [60.0, 90.0]:
		camera.fov = field_of_view
		var image := await _capture_image(viewport)
		var peak := image.get_pixel(64, 64).get_luminance()
		assert_float(peak).is_greater(0.001)
		assert_float(peak).is_less(0.2)
		assert_float(image.get_pixel(0, 0).get_luminance()).is_less(0.001)
	# The scene occluder is farther away than the star's construction shell.
	# Angular background stars must still remain behind it.
	var occluder := MeshInstance3D.new()
	var sphere := SphereMesh.new()
	sphere.radius = 15.0
	sphere.height = 30.0
	occluder.mesh = sphere
	occluder.position = Vector3(0, 0, -200)
	var opaque := StandardMaterial3D.new()
	opaque.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	opaque.albedo_color = Color.BLACK
	occluder.material_override = opaque
	viewport.add_child(occluder)
	var blocked := await _capture_image(viewport)
	assert_float(blocked.get_pixel(64, 64).get_luminance()).is_less(0.001)


func _capture_image(viewport: SubViewport) -> Image:
	for _frame: int in range(5):
		await get_tree().process_frame
	RenderingServer.force_draw(false)
	await get_tree().process_frame
	RenderingServer.force_draw(false)
	return viewport.get_texture().get_image()


func test_panorama_filter_does_not_blend_opposite_poles() -> void:
	assert_object(RenderingServer.get_rendering_device()).is_not_null()
	var viewport: SubViewport = auto_free(SubViewport.new())
	viewport.size = Vector2i(129, 129)
	viewport.own_world_3d = true
	viewport.render_target_update_mode = SubViewport.UPDATE_ALWAYS
	add_child(viewport)
	var panorama := Image.create(8, 4, false, Image.FORMAT_RGBF)
	panorama.fill(Color.BLACK)
	for x: int in range(8):
		panorama.set_pixel(x, 0, Color.RED)
		panorama.set_pixel(x, 3, Color.BLUE)
	var material := ShaderMaterial.new()
	material.shader = load("res://shaders/space_sky.gdshader")
	material.set_shader_parameter("ambient_color", Vector3.ZERO)
	material.set_shader_parameter("background_ready", true)
	material.set_shader_parameter("background_panorama", ImageTexture.create_from_image(panorama))
	var environment := WorldEnvironment.new()
	environment.environment = Environment.new()
	environment.environment.background_mode = Environment.BG_SKY
	environment.environment.sky = Sky.new()
	environment.environment.sky.process_mode = Sky.PROCESS_MODE_REALTIME
	environment.environment.sky.sky_material = material
	viewport.add_child(environment)
	var camera := Camera3D.new()
	viewport.add_child(camera)
	camera.current = true
	for direction: Vector3 in [Vector3.UP, Vector3.DOWN]:
		camera.look_at(direction, Vector3.FORWARD)
		var image := await _capture_image(viewport)
		var pole := image.get_pixel(64, 64)
		if direction == Vector3.UP:
			assert_float(pole.r).is_greater(0.3)
			assert_float(pole.b).is_less(0.01)
		else:
			assert_float(pole.b).is_greater(0.3)
			assert_float(pole.r).is_less(0.01)
