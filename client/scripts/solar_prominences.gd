extends Node3D
## Camera-relative presentation proxy; vertices are in stellar-radius units.

const SECTIONS := 48
const STRIPS := 8
const LAYERS := 3

func _init() -> void:
	name = "SolarProminences"
	visible = false
	var material := ShaderMaterial.new()
	material.shader = preload("res://shaders/solar_prominence.gdshader")
	material.set_shader_parameter("shape_mask", _build_shape_mask())
	material.set_shader_parameter("layer_one", _flow_texture(6197, 0.035))
	material.set_shader_parameter("layer_two", _flow_texture(1741, 0.06))
	var instance := MeshInstance3D.new()
	instance.mesh = _build_mesh()
	instance.material_override = material
	instance.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	add_child(instance)


func update_view(camera: Camera3D, direction: Vector3, angular_radius: float) -> void:
	visible = camera != null and angular_radius > 0.0 and angular_radius < PI * 0.5
	if not visible:
		return
	var distance := minf(camera.far * 0.4, 300_000.0)
	global_position = camera.global_position + direction * distance
	scale = Vector3.ONE * distance * sin(angular_radius)
	var material := (get_child(0) as MeshInstance3D).material_override as ShaderMaterial
	material.set_shader_parameter("sun_direction", direction)
	material.set_shader_parameter("sun_angular_radius", angular_radius)
	# Measure angular detail on the view plane, including zoom and aspect ratio.
	# Projecting the actual star centre would be singular when it is side-on.
	var centre := camera.global_position - camera.global_basis.z
	var tangent := camera.global_basis.y
	var radius_pixels := camera.unproject_position(centre + tangent * tan(angular_radius)).distance_to(
		camera.unproject_position(centre))
	material.set_shader_parameter("detail_visibility", smoothstep(6.0, 24.0, radius_pixels))
	visible = radius_pixels > 6.0


static func _flow_texture(seed_value: int, frequency: float) -> NoiseTexture2D:
	var noise := FastNoiseLite.new()
	noise.seed = seed_value
	noise.frequency = frequency
	noise.fractal_octaves = 4
	var texture := NoiseTexture2D.new()
	texture.width = 512
	texture.height = 256
	texture.seamless = true
	texture.noise = noise
	return texture


static func _build_shape_mask() -> ImageTexture:
	# Authored envelope: torn edges and uneven knots, independent of scrolling detail.
	var noise := FastNoiseLite.new()
	noise.seed = 3407
	noise.frequency = 0.05
	var image := Image.create(256, 128, false, Image.FORMAT_L8)
	for y in range(image.get_height()):
		var v := float(y) / (image.get_height() - 1) * 2.0 - 1.0
		for x in range(image.get_width()):
			var t := float(x) / (image.get_width() - 1)
			var bend := 0.12 * sin(t * 15.0) + noise.get_noise_2d(x * 0.4, y) * 0.18
			var width := 0.38 + 0.28 * sin(PI * t) + 0.1 * sin(t * 27.0)
			var edge := 1.0 - smoothstep(width * 0.45, width, absf(v - bend))
			var feet := smoothstep(0.0, 0.035, t) * (1.0 - smoothstep(0.965, 1.0, t))
			var knot := 0.5 + 0.5 * exp(-pow((t - 0.68) / 0.17, 2.0))
			var holes := smoothstep(-0.65, 0.15, noise.get_noise_2d(x, y * 1.7))
			var density := edge * feet * knot * holes
			image.set_pixel(x, y, Color(density, density, density, 1.0))
	image.generate_mipmaps()
	return ImageTexture.create_from_image(image)


static func _build_mesh() -> ArrayMesh:
	var vertices := PackedVector3Array()
	var uv := PackedVector2Array()
	var colors := PackedColorArray()
	var indices := PackedInt32Array()
	for arch in range(6):
		var seed := fmod(float(arch) * 0.618034 + 0.37, 1.0)
		var azimuth := float(arch) * 2.39996 + 0.45
		var latitude := 0.55 + seed * 0.25 if arch % 2 == 0 else -0.08 - seed * 0.15
		var equator := sqrt(1.0 - latitude * latitude)
		var normal := Vector3(cos(azimuth) * equator, sin(azimuth) * equator, latitude)
		# The curtain lies in a surface-tangent/radial plane, with shallow depth layers.
		var tangent := Vector3(0.0, 0.0, 1.0).cross(normal).normalized()
		var depth := normal.cross(tangent).normalized()
		for layer in range(LAYERS):
			var base := vertices.size()
			for section in range(SECTIONS + 1):
				var t := float(section) / SECTIONS
				var arc := lerpf(-0.075, 0.075, pow(t, 1.12))
				var radial := normal * cos(arc) + tangent * sin(arc)
				var lift := sin(PI * t)
				var height := (0.12 + seed * 0.06) * (1.0 - layer * 0.09)
				for strip in range(STRIPS + 1):
					var v := float(strip) / STRIPS * 2.0 - 1.0
					var width := 0.035 * lift * (1.0 + 0.18 * sin(t * 21.0 + seed * TAU))
					vertices.append(radial * (1.0 + lift * height + v * width)
						+ depth * (layer - 1) * 0.009 * lift)
					uv.append(Vector2(t, v))
					colors.append(Color(1.0 if arch == 0 else 0.09, seed, float(layer) / LAYERS, 1.0))
			for section in range(SECTIONS):
				for strip in range(STRIPS):
					var a := base + section * (STRIPS + 1) + strip
					indices.append_array(PackedInt32Array([a, a + 1, a + STRIPS + 1,
						a + 1, a + STRIPS + 2, a + STRIPS + 1]))
	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = vertices
	arrays[Mesh.ARRAY_TEX_UV] = uv
	arrays[Mesh.ARRAY_COLOR] = colors
	arrays[Mesh.ARRAY_INDEX] = indices
	var mesh := ArrayMesh.new()
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	return mesh
