extends GdUnitTestSuite


func before_test() -> void:
	assert_object(RenderingServer.get_rendering_device()) \
		.override_failure_message("Solar compositor tests require Forward+ or Mobile with a working Vulkan/RenderingDevice backend.") \
		.is_not_null()


func test_unresolved_solar_disc_remains_visible_without_lens_flare() -> void:
	var viewport: SubViewport = auto_free(SubViewport.new())
	viewport.size = Vector2i(321, 241)
	viewport.own_world_3d = true
	viewport.render_target_update_mode = SubViewport.UPDATE_ALWAYS
	add_child(viewport)
	var camera := Camera3D.new()
	camera.fov = 90.0
	viewport.add_child(camera)
	camera.current = true
	var material := ShaderMaterial.new()
	material.shader = load("res://shaders/space_sky.gdshader")
	material.set_shader_parameter("ambient_color", Vector3.ZERO)
	material.set_shader_parameter("display_radiance_scale", 0.0)
	material.set_shader_parameter("extinction_scale", 0.0)
	material.set_shader_parameter("sun_direction", Vector3(0.0004, 0.0002, -1.0).normalized())
	material.set_shader_parameter("sun_active", 1.0)
	material.set_shader_parameter("sun_radiance", preload("res://scripts/stellar_source.gd").SURFACE_RADIANCE)
	material.set_shader_parameter("sun_surface_color_scale", 1.0 / preload("res://scripts/stellar_source.gd").SURFACE_RADIANCE)
	var environment := WorldEnvironment.new()
	environment.environment = Environment.new()
	environment.environment.background_mode = Environment.BG_SKY
	environment.environment.sky = Sky.new()
	environment.environment.sky.process_mode = Sky.PROCESS_MODE_REALTIME
	environment.environment.sky.sky_material = material
	viewport.add_child(environment)
	var previous := 2.0
	for radius: float in [0.00465, 0.000465, 0.0001]:
		material.set_shader_parameter("sun_angular_radius", radius)
		var image := await _capture_image(viewport)
		var light := image.get_pixel(160, 120).get_luminance()
		assert_float(light).is_greater(0.0001)
		assert_float(light).is_less(previous)
		previous = light
	material.set_shader_parameter("sun_active", 0.0)
	var dark := await _capture_image(viewport)
	assert_float(dark.get_pixel(160, 120).get_luminance()).is_less(0.0001)


func test_effect_dispatch_is_local_and_clipped_to_the_viewport() -> void:
	var effect: GDScript = load("res://addons/lens_effects/lens_flare_compositor_effect.gd")
	var size := Vector2i(1280, 720)
	var region: Rect2i = effect.effect_region(size, Vector2(0.5, 0.5), 0.06)
	assert_bool(Rect2i(Vector2i.ZERO, size).encloses(region)).is_true()
	assert_float(float(region.get_area())).is_less(size.x * size.y * 0.25)
	var edge: Rect2i = effect.effect_region(size, Vector2(0.001, 0.001), 0.32)
	assert_vector(edge.position).is_equal(Vector2i.ZERO)
	assert_bool(Rect2i(Vector2i.ZERO, size).encloses(edge)).is_true()
	var rays: Rect2i = effect.effect_region(size, Vector2(0.5, 0.5), 0.06, 0.0028, 0.028, 0.3)
	assert_bool(rays.encloses(effect.effect_region(size, Vector2(0.5, 0.5), 0.06))).is_true()
	assert_bool(rays.has_point(Vector2i(640, 550))).is_true()
	assert_bool(rays.has_point(Vector2i(640, 600))).is_false()


func _capture_image(viewport: SubViewport) -> Image:
	for frame in 5:
		await get_tree().process_frame
	RenderingServer.force_draw(false)
	# GdUnit's CLI can advance process frames without drawing. Render a second
	# frame after the first submission so scene/depth updates have been consumed.
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
	effect.Rays_Strength = 0.0
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
	return {"viewport": viewport, "camera": camera, "effect": effect, "blocker": blocker, "environment": environment.environment}


func test_saturated_region_emerges_from_source_energy_after_tonemapping() -> void:
	var scene := _scene()
	var viewport: SubViewport = scene.viewport
	var effect: CompositorEffect = scene.effect
	var environment: Environment = scene.environment
	(scene.blocker as MeshInstance3D).hide()
	environment.tonemap_mode = Environment.TONE_MAPPER_FILMIC
	environment.tonemap_white = 6.0
	# Deliberately overexpose: this checks saturation, not the production preset.
	environment.tonemap_exposure = 1.1
	var counts: Array[int] = []
	for radius: float in [0.002, 0.008]:
		effect.set_sun(Vector2(0.5, 0.5), Color(1, 1, 1, 0.4), 1.0, 1.0, radius)
		var image := await _capture_image(viewport)
		var count := 0
		for x in range(140, 181):
			var color := image.get_pixel(x, 120)
			if minf(color.r, minf(color.g, color.b)) > 0.9:
				count += 1
		counts.append(count)
	assert_int(counts[1]).is_greater(counts[0])


## Real GPU coverage: source visibility, reverse-Z occlusion, MSAA resolve and
## non-workgroup-aligned viewport sizes. Projection itself has separate tests.
func test_flare_obeys_depth_and_source_visibility() -> void:
	# Vulkan guarantees 4x framebuffer samples; 2x is optional on software drivers.
	for msaa: int in [Viewport.MSAA_DISABLED, Viewport.MSAA_4X]:
		var scene := _scene()
		var viewport: SubViewport = scene.viewport
		var effect: CompositorEffect = scene.effect
		var blocker: MeshInstance3D = scene.blocker
		viewport.msaa_3d = msaa
		blocker.hide()
		effect.set_sun(Vector2(0.5, 0.5), Color(1, 1, 1, 40), 1.0, 1.0, 0.004)
		var visible_light := await _capture(viewport)
		assert_float(visible_light).is_greater(1.0)
		blocker.show()
		assert_float(await _capture(viewport)) \
			.override_failure_message("Solar source must be occluded with MSAA mode %d" % msaa) \
			.is_less(visible_light * 0.01)
		blocker.hide()
		effect.set_sun(Vector2(0.5, 0.5), Color(1, 1, 1, 40), -1.0, 1.0, 0.004)
		assert_float(await _capture(viewport)).is_less(visible_light * 0.01)
		effect.set_sun(Vector2(1.1, 0.5), Color(1, 1, 1, 40), 1.0, 1.0, 0.004)
		assert_float(await _capture(viewport)).is_less(visible_light * 0.01)


func test_optics_form_a_horizontal_streak_without_a_broad_round_ghost() -> void:
	var scene := _scene()
	var viewport: SubViewport = scene.viewport
	var effect: CompositorEffect = scene.effect
	(scene.blocker as MeshInstance3D).hide()
	effect.set_sun(Vector2(0.5, 0.5), Color(1, 1, 1, 40), 1.0, 1.0, 0.004)
	var image := await _capture_image(viewport)
	var horizontal := image.get_pixel(204, 120).get_luminance()
	assert_float(horizontal).is_greater(0.001)
	assert_float(image.get_pixel(160, 164).get_luminance()).is_less(horizontal * 0.5)
	var inner_glare := image.get_pixel(175, 135).get_luminance()
	assert_float(inner_glare).is_greater(0.001)


func test_streak_follows_source_without_reflections_and_can_be_disabled() -> void:
	var scene := _scene()
	var viewport: SubViewport = scene.viewport
	var effect: CompositorEffect = scene.effect
	(scene.blocker as MeshInstance3D).hide()
	effect.set_sun(Vector2(0.75, 0.3), Color(1, 1, 1, 40), 1.0, 1.0, 0.004)
	var first := await _capture_image(viewport)
	assert_float(first.get_pixel(240, 72).get_luminance()).is_greater(0.01)
	assert_float(first.get_pixel(116, 147).get_luminance()).is_less(0.0001)
	effect.set_sun(Vector2(0.55, 0.6), Color(1, 1, 1, 40), 1.0, 1.0, 0.004)
	var moved := await _capture_image(viewport)
	assert_float(moved.get_pixel(176, 144).get_luminance()).is_greater(0.01)
	assert_float(moved.get_pixel(240, 72).get_luminance()).is_less(0.0001)
	effect.Streak_Fraction = 0.0
	effect.Glare_Fraction = 0.0
	assert_float(await _capture(viewport)).is_less(0.0001)


func test_ray_occlusion_weight_controls_shadows_without_dimming_clear_side() -> void:
	var scene := _scene()
	var viewport: SubViewport = scene.viewport
	var effect: CompositorEffect = scene.effect
	var blocker: MeshInstance3D = scene.blocker
	effect.Streak_Fraction = 0.0
	effect.Glare_Fraction = 0.0
	effect.Rays_Strength = 0.4
	effect.Rays_Radius = 0.4
	(blocker.mesh as QuadMesh).size = Vector2(0.15, 0.25)
	blocker.position = (scene.camera as Camera3D).project_position(Vector2(184.5, 120.5), 2.0)
	effect.Rays_Shadow_Weight = 0.0
	effect.set_sun(Vector2(0.5, 0.5), Color.WHITE, 1.0, 1.0, 0.004)
	var unmasked := await _capture_image(viewport)
	effect.Rays_Shadow_Weight = 2.0
	var masked := await _capture_image(viewport)
	assert_float(masked.get_pixel(204, 120).get_luminance()).is_less(unmasked.get_pixel(204, 120).get_luminance() * 0.8)
	assert_float(masked.get_pixel(116, 120).get_luminance()).is_equal_approx(unmasked.get_pixel(116, 120).get_luminance(), 0.001)
	effect.Rays_Samples = 96
	var refined := await _capture_image(viewport)
	assert_float(refined.get_pixel(116, 120).get_luminance()).is_equal_approx(masked.get_pixel(116, 120).get_luminance(), 0.001)


func test_streak_fades_outward_and_squeeze_extends_horizontal_response() -> void:
	var scene := _scene()
	var viewport: SubViewport = scene.viewport
	var effect: CompositorEffect = scene.effect
	(scene.blocker as MeshInstance3D).hide()
	effect.Squeeze = 1.0
	effect.Glare_Fraction = 0.0
	effect.set_sun(Vector2(0.5, 0.5), Color(1, 1, 1, 4), 1.0, 1.0, 0.004)
	var narrow := await _capture_image(viewport)
	assert_float(narrow.get_pixel(164, 120).get_luminance()).is_greater(narrow.get_pixel(180, 120).get_luminance())
	assert_float(narrow.get_pixel(180, 120).get_luminance()).is_greater(narrow.get_pixel(204, 120).get_luminance())
	effect.Squeeze = 2.4
	var wide := await _capture_image(viewport)
	assert_float(wide.get_pixel(220, 120).get_luminance()).is_greater(narrow.get_pixel(220, 120).get_luminance() * 1.5)
	assert_float(wide.get_pixel(160, 140).get_luminance()).is_less(0.0001)


func test_lens_weights_partition_camera_response_and_reject_overallocation() -> void:
	var lens := load("res://resources/solar_anamorphic_lens.tres").duplicate(true)
	assert_float(lens.direct_fraction() + lens.Glare_Fraction + lens.Streak_Fraction + lens.Ghost_Fraction + lens.Halo_Fraction).is_equal_approx(1.0, 0.00001)
	lens.Glare_Fraction = 2.0
	lens.Streak_Fraction = 2.0
	assert_float(lens.direct_fraction()).is_equal(0.0)


func _linear_energy(image: Image) -> float:
	var energy := 0.0
	for y in image.get_height():
		for x in image.get_width():
			energy += image.get_pixel(x, y).srgb_to_linear().get_luminance()
	return energy / (image.get_height() * image.get_height())


func test_psf_energy_follows_camera_response_not_squeeze_or_resolution() -> void:
	var scene := _scene()
	var viewport: SubViewport = scene.viewport
	var effect: CompositorEffect = scene.effect
	(scene.blocker as MeshInstance3D).hide()
	# Dim input keeps readback below clipping; compare scene-linear camera response,
	# not brightness after display saturation. Fractions/tint remain unchanged.
	var color := Color(1.0, 1.0, 1.0, 0.04)
	effect.set_sun(Vector2(0.5, 0.5), color, 1.0, 1.0, 0.002)
	var small := _linear_energy(await _capture_image(viewport))
	effect.set_sun(Vector2(0.5, 0.5), color, 1.0, 1.0, 0.004)
	var larger := _linear_energy(await _capture_image(viewport))
	var source := preload("res://scripts/stellar_source.gd")
	var expected_ratio: float = source.lens_flux(0.004) / source.lens_flux(0.002)
	assert_float(larger / small).is_equal_approx(expected_ratio, 0.15)
	var coating: Color = effect.Lens_Tint
	var transmitted: float = effect.Glare_Fraction * (0.7 + 0.3 * coating.get_luminance()) \
		+ effect.Streak_Fraction * coating.get_luminance()
	var expected: float = source.lens_flux(0.004) * color.a * transmitted
	assert_float(larger / expected).is_equal_approx(1.0, 0.15)
	effect.Squeeze = 2.6
	var stretched := _linear_energy(await _capture_image(viewport))
	assert_float(stretched / larger).is_equal_approx(1.0, 0.12)
	viewport.size = Vector2i(642, 482)
	var resized := _linear_energy(await _capture_image(viewport))
	assert_float(resized / stretched).is_equal_approx(1.0, 0.12)


func test_projected_source_radius_is_resolution_independent_and_follows_fov() -> void:
	var scene := _scene()
	var viewport: SubViewport = scene.viewport
	var camera: Camera3D = scene.camera
	var presentation := preload("res://scripts/world_presentation.gd")
	var first := presentation.project_sun_flare(camera, Vector3.FORWARD, 0.00465)
	viewport.size = Vector2i(642, 482)
	var resized := presentation.project_sun_flare(camera, Vector3.FORWARD, 0.00465)
	assert_float(resized.projected_radius / first.projected_radius).is_equal_approx(1.0, 0.01)
	camera.fov = 35.0
	var zoomed := presentation.project_sun_flare(camera, Vector3.FORWARD, 0.00465)
	assert_float(zoomed.projected_radius).is_greater(resized.projected_radius)


func test_camera_lens_preset_instances_keep_independent_settings() -> void:
	var preset := load("res://resources/solar_anamorphic_lens.tres")
	var first := preset.duplicate(true)
	var second := preset.duplicate(true)
	first.Squeeze = 2.4
	first.Streak_Fraction = 0.0
	first.enabled = true
	assert_float(second.Squeeze).is_equal(1.8)
	assert_float(second.Streak_Fraction).is_greater(0.0)
	assert_bool(second.enabled).is_false()


func test_source_size_does_not_change_the_depth_occlusion_footprint() -> void:
	var scene := _scene()
	var viewport: SubViewport = scene.viewport
	var effect: CompositorEffect = scene.effect
	var blocker: MeshInstance3D = scene.blocker
	(blocker.mesh as QuadMesh).size = Vector2(0.08, 0.08)
	for source_radius: float in [0.002, 0.008]:
		blocker.hide()
		effect.set_sun(Vector2(0.5, 0.5), Color(1, 1, 1, 40), 1.0, 1.0, source_radius)
		var visible_light := await _capture(viewport)
		assert_float(visible_light).is_greater(0.1)
		blocker.show()
		assert_float(await _capture(viewport)).is_less(visible_light * 0.01)


func test_flare_survives_repeated_viewport_resize() -> void:
	var scene := _scene()
	var viewport: SubViewport = scene.viewport
	var effect: CompositorEffect = scene.effect
	(scene.blocker as MeshInstance3D).hide()
	for size: Vector2i in [Vector2i(321, 241), Vector2i(481, 319), Vector2i(321, 241)]:
		viewport.size = size
		effect.set_sun(Vector2(0.5, 0.5), Color(1, 1, 1, 40), 1.0, 1.0, 0.004)
		var image := await _capture_image(viewport)
		assert_vector(image.get_size()).is_equal(size)
		var visible_light := await _capture(viewport)
		assert_float(visible_light).is_greater(1.0)
		effect.set_sun(Vector2(0.5, 0.5), Color(1, 1, 1, 40), -1.0, 1.0, 0.004)
		assert_float(await _capture(viewport)).is_less(visible_light * 0.01)


func test_lens_artifact_can_overlay_foreground_without_casting_world_shadows() -> void:
	var scene := _scene()
	var viewport: SubViewport = scene.viewport
	var effect: CompositorEffect = scene.effect
	var blocker: MeshInstance3D = scene.blocker
	var camera: Camera3D = scene.camera
	(blocker.mesh as QuadMesh).size = Vector2(0.15, 0.25)
	blocker.position = camera.project_position(Vector2(204.5, 120.5), 2.0)
	effect.set_sun(Vector2(0.5, 0.5), Color(1, 1, 1, 40), 1.0, 1.0, 0.004)
	blocker.hide()
	var clear := await _capture_image(viewport)
	blocker.show()
	var foreground := await _capture_image(viewport)
	assert_float(clear.get_pixel(204, 120).get_luminance()).is_greater(0.001)
	assert_float(foreground.get_pixel(204, 120).get_luminance()).is_equal_approx(clear.get_pixel(204, 120).get_luminance(), 0.001)
	assert_float(foreground.get_pixel(0, 0).get_luminance()).is_less(0.0001)


func test_light_shafts_follow_occluders_without_brightening_foreground() -> void:
	var scene := _scene()
	var viewport: SubViewport = scene.viewport
	var effect: CompositorEffect = scene.effect
	var blocker: MeshInstance3D = scene.blocker
	var camera: Camera3D = scene.camera
	effect.Streak_Fraction = 0.0
	effect.Glare_Fraction = 0.0
	effect.Rays_Strength = 0.4
	effect.Rays_Radius = 0.4
	(blocker.mesh as QuadMesh).size = Vector2(0.15, 0.25)
	blocker.position = camera.project_position(Vector2(184.5, 120.5), 2.0)
	for msaa: int in [Viewport.MSAA_DISABLED, Viewport.MSAA_4X]:
		viewport.msaa_3d = msaa
		effect.set_sun(Vector2(0.5, 0.5), Color.WHITE, 1.0, 1.0, 0.004)
		blocker.hide()
		var clear := await _capture_image(viewport)
		blocker.show()
		var shadow := await _capture_image(viewport)
		var light := clear.get_pixel(204, 120).get_luminance()
		assert_float(light).is_greater(0.01)
		assert_float(shadow.get_pixel(204, 120).get_luminance()).is_less(light * 0.8)
		assert_float(shadow.get_pixel(116, 120).get_luminance()).is_greater(clear.get_pixel(116, 120).get_luminance() * 0.95)
		assert_float(shadow.get_pixel(184, 120).get_luminance()).is_less(0.0001)
		assert_float(shadow.get_pixel(0, 0).get_luminance()).is_less(0.0001)
	blocker.hide()
	for source: Vector2 in [Vector2(-0.1, 0.5), Vector2(1.1, 0.5)]:
		effect.set_sun(source, Color.WHITE, 1.0, 1.0, 0.004)
		assert_float(await _capture(viewport)).is_less(0.0001)
	effect.set_sun(Vector2(0.5, 0.5), Color.WHITE, -1.0, 1.0, 0.004)
	assert_float(await _capture(viewport)).is_less(0.0001)


func test_resolved_disc_glare_has_a_flat_interior_instead_of_a_central_peak() -> void:
	var scene := _scene()
	var effect: CompositorEffect = scene.effect
	(scene.blocker as MeshInstance3D).hide()
	effect.Streak_Fraction = 0.0
	effect.Glare_Fraction = 1.0
	effect.set_sun(Vector2(0.5, 0.5), Color(1, 1, 1, 0.01), 1.0, 1.0, 0.2)
	var image := await _capture_image(scene.viewport)
	var centre := image.get_pixel(160, 120).srgb_to_linear().get_luminance()
	var interior := image.get_pixel(190, 120).srgb_to_linear().get_luminance()
	assert_float(centre).is_greater(0.001)
	assert_float(interior / centre).is_greater(0.7)


func test_production_lens_preserves_source_colour_without_a_fixed_blue_cast() -> void:
	var scene := _scene()
	var effect: CompositorEffect = scene.effect
	(scene.blocker as MeshInstance3D).hide()
	var preset := load("res://resources/solar_anamorphic_lens.tres")
	assert_bool(preset.Lens_Tint == Color.WHITE).is_true()
	effect.Lens_Tint = preset.Lens_Tint
	effect.Ghost_Fraction = 0.1
	effect.Halo_Fraction = 0.1
	for colour: Color in [Color(1, 0, 0, 40), Color(0, 0, 1, 40)]:
		effect.set_sun(Vector2(0.5, 0.5), colour, 1.0, 1.0, 0.004)
		var image := await _capture_image(scene.viewport)
		var pixel := image.get_pixel(180, 120)
		if colour.r > 0.0:
			assert_float(pixel.r).is_greater(0.001)
			assert_float(pixel.b).is_less(0.0001)
		else:
			assert_float(pixel.b).is_greater(0.001)
			assert_float(pixel.r).is_less(0.0001)


func test_resolved_source_remains_visible_when_only_its_centre_is_blocked() -> void:
	var scene := _scene()
	var viewport: SubViewport = scene.viewport
	var effect: CompositorEffect = scene.effect
	var blocker: MeshInstance3D = scene.blocker
	(blocker.mesh as QuadMesh).size = Vector2(0.4, 0.4)
	effect.set_sun(Vector2(0.5, 0.5), Color(1, 1, 1, 0.4), 1.0, 50.0, 50.0 / 241.0)
	blocker.hide()
	var clear := await _capture(viewport)
	blocker.show()
	var partial := await _capture(viewport)
	assert_float(clear).is_greater(0.001)
	assert_float(partial).is_greater(clear * 0.3)
	assert_float(partial).is_less(clear * 0.99)


func test_off_axis_reflections_preserve_energy_and_mirror_with_the_source() -> void:
	var scene := _scene()
	var viewport: SubViewport = scene.viewport
	var effect: CompositorEffect = scene.effect
	(scene.blocker as MeshInstance3D).hide()
	effect.Streak_Fraction = 0.0
	effect.Glare_Fraction = 0.0
	effect.Lens_Tint = Color.WHITE
	for component: String in ["Ghost_Fraction", "Halo_Fraction"]:
		effect.set(component, 0.3)
		effect.set_sun(Vector2(0.3, 0.4), Color(1, 1, 1, 0.4), 1.0, 1.0, 0.004)
		var left := await _capture_image(viewport)
		effect.set_sun(Vector2(0.7, 0.6), Color(1, 1, 1, 0.4), 1.0, 1.0, 0.004)
		var right := await _capture_image(viewport)
		var expected: float = preload("res://scripts/stellar_source.gd").lens_flux(0.004) * 0.4 * 0.3
		assert_float(_linear_energy(left) / expected).is_equal_approx(1.0, 0.18)
		for y in range(0, left.get_height(), 4):
			for x in range(0, left.get_width(), 4):
				assert_float(left.get_pixel(x, y).get_luminance()).is_equal_approx(
					right.get_pixel(right.get_width() - 1 - x, right.get_height() - 1 - y).get_luminance(), 0.005)
		effect.set(component, 0.0)


func test_internal_reflections_are_independent_normalized_and_occluded() -> void:
	var scene := _scene()
	var viewport: SubViewport = scene.viewport
	var effect: CompositorEffect = scene.effect
	var blocker: MeshInstance3D = scene.blocker
	blocker.hide()
	effect.Streak_Fraction = 0.0
	effect.Glare_Fraction = 0.0
	effect.Lens_Tint = Color.WHITE
	effect.set_sun(Vector2(0.5, 0.5), Color(1.0, 1.0, 1.0, 0.4), 1.0, 1.0, 0.004)
	assert_float(await _capture(viewport)).is_less(0.0001)
	for component: String in ["Ghost_Fraction", "Halo_Fraction"]:
		effect.set(component, 0.3)
		assert_float(effect.direct_fraction()).is_equal_approx(0.7, 0.0001)
		var image := await _capture_image(viewport)
		var expected: float = preload("res://scripts/stellar_source.gd").lens_flux(0.004) * 0.4 * 0.3
		assert_float(_linear_energy(image) / expected).is_equal_approx(1.0, 0.22)
		if component == "Halo_Fraction":
			assert_float(image.get_pixel(160, 120).get_luminance()).is_less(0.0001)
			assert_float(image.get_pixel(182, 120).get_luminance()).is_greater(0.001)
		blocker.show()
		assert_float(await _capture(viewport)).is_less(0.0001)
		blocker.hide()
		effect.set(component, 0.0)
	effect.Ghost_Fraction = 2.0
	effect.Halo_Fraction = 2.0
	assert_float(effect.direct_fraction()).is_equal_approx(0.0, 0.0001)


func test_radial_light_fades_progressively_without_a_bright_outer_rim() -> void:
	var scene := _scene()
	var viewport: SubViewport = scene.viewport
	var effect: CompositorEffect = scene.effect
	(scene.blocker as MeshInstance3D).hide()
	effect.Streak_Fraction = 0.0
	effect.Glare_Fraction = 0.0
	effect.Rays_Strength = 0.5
	effect.Rays_Radius = 0.4
	effect.set_sun(Vector2(0.5, 0.5), Color.WHITE, 1.0, 1.0, 0.004)
	var image := await _capture_image(viewport)
	var previous := image.get_pixel(160, 120).get_luminance()
	for offset: int in [8, 16, 24, 32, 48, 64, 80, 96]:
		var light := image.get_pixel(160, 120 + offset).get_luminance()
		assert_float(light).is_less_equal(previous)
		previous = light
	assert_float(image.get_pixel(160, 200).get_luminance()).is_less(
		image.get_pixel(160, 168).get_luminance() * 0.2)
	assert_float(image.get_pixel(160, 217).get_luminance()).is_less(0.0001)
