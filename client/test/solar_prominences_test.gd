extends GdUnitTestSuite

const Prominences = preload("res://scripts/solar_prominences.gd")

func test_curtain_feet_remain_on_the_stellar_surface() -> void:
	var arrays := Prominences._build_mesh().surface_get_arrays(0)
	var vertices: PackedVector3Array = arrays[Mesh.ARRAY_VERTEX]
	var uv: PackedVector2Array = arrays[Mesh.ARRAY_TEX_UV]
	var all_finite := true
	var feet_on_surface := true
	var foot_count := 0
	for i in range(vertices.size()):
		all_finite = all_finite and vertices[i].is_finite()
		if uv[i].x == 0.0 or uv[i].x == 1.0:
			feet_on_surface = feet_on_surface and absf(vertices[i].length() - 1.0) < 0.00001
			foot_count += 1
	assert_bool(all_finite).is_true()
	assert_bool(feet_on_surface).is_true()
	assert_int(foot_count).is_greater(0)
	assert_int((arrays[Mesh.ARRAY_INDEX] as PackedInt32Array).size() % 3).is_equal(0)


func test_proxy_preserves_angular_radius_and_follows_the_camera_without_rotating_feet() -> void:
	var camera: Camera3D = auto_free(Camera3D.new())
	var prominence: Node3D = auto_free(Prominences.new())
	add_child(camera)
	add_child(prominence)
	camera.far = 5_000_000.0
	for angle: float in [0.04, asin(1.0 / 1.5)]:
		camera.position += Vector3(100.0, 20.0, 0.0)
		prominence.update_view(camera, Vector3.FORWARD, angle)
		var distance := camera.global_position.distance_to(prominence.global_position)
		assert_bool(prominence.visible).is_true()
		assert_float(prominence.scale.x / distance).is_equal_approx(sin(angle), 0.00001)
		assert_vector(prominence.rotation).is_equal(Vector3.ZERO)
	prominence.update_view(camera, Vector3.FORWARD, 0.0001)
	assert_bool(prominence.visible).is_false()


func test_shape_mask_has_transparent_edges_and_nonuniform_material() -> void:
	var image := Prominences._build_shape_mask().get_image()
	assert_float(image.get_pixel(0, 64).r).is_equal(0.0)
	assert_float(image.get_pixel(128, 0).r).is_equal(0.0)
	assert_float(image.get_pixel(128, 127).r).is_equal(0.0)
	var minimum := 1.0
	var maximum := 0.0
	for y in range(32, 96):
		for x in range(32, 224):
			var value := image.get_pixel(x, y).r
			minimum = minf(minimum, value)
			maximum = maxf(maximum, value)
	assert_float(maximum).is_greater(0.4)
	assert_float(maximum - minimum).is_greater(0.3)


func test_unresolved_prominences_become_visible_when_the_camera_zooms_in() -> void:
	var camera: Camera3D = auto_free(Camera3D.new())
	var prominence: Node3D = auto_free(Prominences.new())
	add_child(camera)
	add_child(prominence)
	camera.fov = 120.0
	prominence.update_view(camera, Vector3.FORWARD, 0.01)
	assert_bool(prominence.visible).is_false()
	camera.fov = 10.0
	prominence.update_view(camera, Vector3.FORWARD, 0.01)
	assert_bool(prominence.visible).is_true()
	prominence.update_view(camera, Vector3.RIGHT, 0.01)
	assert_bool(prominence.global_position.is_finite()).is_true()
