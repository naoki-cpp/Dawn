extends GdUnitTestSuite


func test_effect_dispatch_is_local_and_clipped_to_the_viewport() -> void:
	var effect: GDScript = load("res://addons/lens_effects/lens_flare_compositor_effect.gd")
	var size := Vector2i(1280, 720)
	var region: Rect2i = effect.effect_region(size, Vector2(0.5, 0.5), 0.32)
	assert_bool(Rect2i(Vector2i.ZERO, size).encloses(region)).is_true()
	assert_float(float(region.get_area())).is_less(size.x * size.y * 0.25)
	var edge: Rect2i = effect.effect_region(size, Vector2(0.001, 0.001), 0.32)
	assert_vector(edge.position).is_equal(Vector2i.ZERO)
	assert_bool(Rect2i(Vector2i.ZERO, size).encloses(edge)).is_true()


func _capture_image(viewport: SubViewport) -> Image:
	for frame in 5:
		await get_tree().process_frame
	RenderingServer.force_draw(false)
	return viewport.get_texture().get_image()


func _capture(viewport: SubViewport) -> float:
	var image := await _capture_image(viewport)
	var luminance := 0.0
	for y in range(0, image.get_height(), 4):
		for x in range(0, image.get_width(), 4):
			luminance += image.get_pixel(x, y).get_luminance()
	return luminance


func _scene() -> Dictionary:
	var viewport: SubViewport = auto_free(SubViewport.new())
	viewport.size = Vector2i(321, 241)
	viewport.own_world_3d = true
	viewport.render_target_update_mode = SubViewport.UPDATE_ALWAYS
	add_child(viewport)
	var camera := Camera3D.new()
	viewport.add_child(camera)
	camera.current = true
	var environment := WorldEnvironment.new()
	environment.environment = Environment.new()
	environment.environment.background_mode = Environment.BG_COLOR
	environment.environment.background_color = Color.BLACK
	viewport.add_child(environment)
	var effect: CompositorEffect = load("res://addons/lens_effects/lens_flare_compositor_effect.gd").new()
	effect.Weight = 0.0
	effect.enabled = true
	environment.compositor = Compositor.new()
	environment.compositor.compositor_effects = [effect]
	var blocker := MeshInstance3D.new()
	blocker.mesh = QuadMesh.new()
	blocker.position.z = -2.0
	var material := StandardMaterial3D.new()
	material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	material.albedo_color = Color.BLACK
	blocker.material_override = material
	viewport.add_child(blocker)
	return {"viewport": viewport, "camera": camera, "effect": effect, "blocker": blocker}


## Real GPU coverage: source visibility, reverse-Z occlusion, MSAA resolve and
## non-workgroup-aligned viewport sizes. Projection itself has separate tests.
func test_flare_obeys_depth_and_source_visibility() -> void:
	var scene := _scene()
	var viewport: SubViewport = scene.viewport
	var effect: CompositorEffect = scene.effect
	var blocker: MeshInstance3D = scene.blocker
	for msaa: int in [Viewport.MSAA_DISABLED, Viewport.MSAA_2X]:
		viewport.msaa_3d = msaa
		blocker.hide()
		effect.set_sun(Vector2(0.5, 0.5), Color.WHITE, 1.0, 1.0)
		var visible_light := await _capture(viewport)
		assert_float(visible_light).is_greater(1.0)
		blocker.show()
		assert_float(await _capture(viewport)).is_less(visible_light * 0.01)
		blocker.hide()
		effect.set_sun(Vector2(0.5, 0.5), Color.WHITE, -1.0, 1.0)
		assert_float(await _capture(viewport)).is_less(visible_light * 0.01)
		effect.set_sun(Vector2(1.1, 0.5), Color.WHITE, 1.0, 1.0)
		assert_float(await _capture(viewport)).is_less(visible_light * 0.01)


func test_local_glow_casts_depth_shadows_without_lighting_foreground_or_screen_corners() -> void:
	var scene := _scene()
	var viewport: SubViewport = scene.viewport
	var effect: CompositorEffect = scene.effect
	var blocker: MeshInstance3D = scene.blocker
	var camera: Camera3D = scene.camera
	effect.Effect_Multiplier = 0.0
	effect.Weight = 0.35
	effect.SampleCount = 32
	effect.Decay = 0.96
	effect.Density = 0.98
	(blocker.mesh as QuadMesh).size = Vector2(0.15, 0.25)
	blocker.position = camera.project_position(Vector2(184.5, 120.5), 2.0)
	for msaa: int in [Viewport.MSAA_DISABLED, Viewport.MSAA_2X]:
		viewport.msaa_3d = msaa
		effect.set_sun(Vector2(0.5, 0.5), Color.WHITE, 1.0, 1.0)
		blocker.hide()
		var clear_image := await _capture_image(viewport)
		blocker.show()
		var shadow_image := await _capture_image(viewport)
		var clear_light := clear_image.get_pixel(204, 120).get_luminance()
		assert_float(clear_light).is_greater(0.01)
		assert_float(shadow_image.get_pixel(204, 120).get_luminance()).is_less(clear_light * 0.8)
		assert_float(shadow_image.get_pixel(116, 120).get_luminance()) \
			.is_greater(clear_image.get_pixel(116, 120).get_luminance() * 0.95)
		assert_float(shadow_image.get_pixel(184, 120).get_luminance()).is_less(0.001)
		assert_float(shadow_image.get_pixel(0, 0).get_luminance()).is_less(0.001)
		assert_float(shadow_image.get_pixel(160, 10).get_luminance()).is_less(0.001)
