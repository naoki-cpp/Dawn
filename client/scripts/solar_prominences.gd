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


static func _build_mesh() -> ArrayMesh:
	var vertices := PackedVector3Array()
	var uv := PackedVector2Array()
	var colors := PackedColorArray()
	var indices := PackedInt32Array()
	for arch in range(2):
		var prominence_seed := 0.37 if arch == 0 else 0.79
		var azimuth := 0.45 if arch == 0 else 2.76
		var latitude := 0.64 if arch == 0 else -0.42
		var equator := sqrt(1.0 - latitude * latitude)
		var normal := Vector3(cos(azimuth) * equator, sin(azimuth) * equator, latitude)
		# The curtain lies in a surface-tangent/radial plane, with shallow depth layers.
		var tangent := Vector3(0.0, 0.0, 1.0).cross(normal).normalized()
		var depth := normal.cross(tangent).normalized()
		for layer in range(LAYERS):
			var base := vertices.size()
			for section in range(SECTIONS + 1):
				var t := float(section) / SECTIONS
				var arc := lerpf(-0.17, 0.17, pow(t, 1.12)) + 0.035 * sin(t * TAU * 1.2 + prominence_seed * TAU) * sin(PI * t)
				var radial := normal * cos(arc) + tangent * sin(arc)
				var lift := pow(sin(PI * t), 0.86)
				var height := (0.34 if arch == 0 else 0.17) * (1.0 - layer * 0.08)
				for strip in range(STRIPS + 1):
					var v := float(strip) / STRIPS * 2.0 - 1.0
					var width_scale := 0.085 if arch == 0 else 0.05
					var width := width_scale * lift * (1.0 + 0.16 * sin(t * 17.0 + prominence_seed * TAU)
						+ 0.08 * sin(t * 31.0 + prominence_seed * 5.0))
					vertices.append(radial * (1.0 + lift * height + v * width)
						+ depth * (float(layer) - 1.0) * 0.022 * lift)
					uv.append(Vector2(t, v))
					var arch_opacity := 0.82 if arch == 0 else 0.07
					var layer_opacity := 1.0 if layer == 0 else (0.38 if layer == 1 else 0.16)
					colors.append(Color(arch_opacity * layer_opacity, prominence_seed, float(layer) / LAYERS, 1.0))
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
