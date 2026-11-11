## Fixed, offline art-review scene. Uses production sky, optics and ship assets.
extends Node3D

const GalacticModelScript = preload("res://scripts/galactic_model.gd")

var _presentation := WorldPresentation.new()
var _capture := false
var _capture_layers := false


func _ready() -> void:
	_capture_layers = OS.get_cmdline_user_args().has("--layers")
	var args := OS.get_cmdline_user_args()
	_capture = _capture_layers or args.has("--capture") or args.has("--sweep") or args.has("--close")
	get_window().size = Vector2i(1280, 720)
	var camera := Camera3D.new()
	camera.near = 10.0
	camera.far = 5000000.0
	camera.fov = 60.0
	add_child(camera)
	camera.make_current()
	_presentation._camera = camera
	_presentation._world = WorldSpace.new()
	_presentation._setup_space_environment(self)
	var direction := Vector3(0.0, 0.12, -1.0).normalized()
	var angular_radius := 0.32 if OS.get_cmdline_user_args().has("--close") else 0.00465
	_presentation.update_stellar_view(direction, Vector3(1, 0.95, 0.8), angular_radius)
	var sunlight := DirectionalLight3D.new()
	sunlight.light_color = Color(1, 0.95, 0.8)
	sunlight.light_energy = 1.8
	sunlight.shadow_enabled = true
	add_child(sunlight)
	sunlight.look_at(-direction, Vector3.UP)
	var noise := FastNoiseLite.new()
	noise.seed = 531
	noise.frequency = 2.0
	noise.fractal_octaves = 4
	var texture_noise := FastNoiseLite.new()
	texture_noise.seed = 531
	texture_noise.frequency = 0.035
	var surface := NoiseTexture2D.new()
	surface.width = 256
	surface.height = 256
	surface.seamless = true
	surface.noise = texture_noise
	var normal := NoiseTexture2D.new()
	normal.width = 256
	normal.height = 256
	normal.seamless = true
	normal.as_normal_map = true
	normal.bump_strength = 3.0
	normal.noise = texture_noise
	var material := StandardMaterial3D.new()
	material.albedo_color = Color(0.34, 0.30, 0.26)
	material.albedo_texture = surface
	material.roughness = 0.88
	material.normal_enabled = true
	material.normal_texture = normal
	material.normal_scale = 0.65
	var rock_mesh := _rock_mesh(noise)
	for point: Vector3 in [Vector3(0.47, 0.39, 34), Vector3(0.535, 0.36, 39), Vector3(0.56, 0.47, 45), Vector3(0.43, 0.52, 31), Vector3(0.34, 0.31, 57), Vector3(0.67, 0.28, 64)]:
		var rock := MeshInstance3D.new()
		rock.mesh = rock_mesh
		rock.visible = not OS.get_cmdline_user_args().has("--close")
		rock.material_override = material
		rock.position = camera.project_position(Vector2(point.x, point.y) * Vector2(get_window().size), 800.0)
		rock.scale = Vector3(1.0, 1.3, 0.8) * point.z
		rock.rotation = Vector3(point.x * 3.0, point.y * 6.0, point.x * 12.0)
		add_child(rock)
	var ship := preload("res://scenes/ship.tscn").instantiate() as Node3D
	ship.position = Vector3(-100, -160, -850)
	ship.rotation = Vector3(0.25, 0.5, -0.35)
	add_child(ship)
	if _capture:
		_capture_image([surface, normal], sunlight)


func _rock_mesh(noise: FastNoiseLite) -> ArrayMesh:
	var sphere := SphereMesh.new()
	sphere.radius = 1.0
	sphere.height = 2.0
	sphere.radial_segments = 48
	sphere.rings = 24
	var arrays := sphere.get_mesh_arrays()
	var positions: PackedVector3Array = arrays[Mesh.ARRAY_VERTEX]
	var uvs: PackedVector2Array = arrays[Mesh.ARRAY_TEX_UV]
	var indices: PackedInt32Array = arrays[Mesh.ARRAY_INDEX]
	var builder := SurfaceTool.new()
	builder.begin(Mesh.PRIMITIVE_TRIANGLES)
	for index in indices:
		var point := positions[index]
		builder.set_uv(uvs[index])
		builder.set_smooth_group(0)
		builder.add_vertex(point * (1.0 + noise.get_noise_3dv(point) * 0.23))
	builder.index()
	builder.generate_normals()
	return builder.commit()


func _capture_image(textures: Array[NoiseTexture2D], sunlight: DirectionalLight3D) -> void:
	if _capture_layers:
		await _capture_nebula_layers(sunlight)
		return
	for texture in textures:
		if texture.get_image() == null:
			await texture.changed
	for frame in 45:
		await get_tree().process_frame
	await RenderingServer.frame_post_draw
	var output := ProjectSettings.globalize_path("res://").path_join("../.tmp").simplify_path()
	DirAccess.make_dir_recursive_absolute(output)
	if OS.get_cmdline_user_args().has("--sweep"):
		for uv: Vector2 in [Vector2(0.5, 0.5), Vector2(0.3, 0.4), Vector2(0.08, 0.25)]:
			var camera: Camera3D = _presentation._camera
			var direction := camera.project_ray_normal(uv * Vector2(get_window().size))
			sunlight.look_at(-direction, Vector3.UP)
			for material in [_presentation._sky_mat, _presentation._starfield_mat]:
				material.set_shader_parameter("sun_direction", direction)
			_presentation._update_sun_flare(direction, Vector3(1, 0.95, 0.8), 0.00465)
			for frame in 8:
				await get_tree().process_frame
			await RenderingServer.frame_post_draw
			get_viewport().get_texture().get_image().save_png(output.path_join("lens-field-%02d.png" % roundi(uv.x * 100.0)))
	else:
		var filename := "stellar-close.png" if OS.get_cmdline_user_args().has("--close") else "lighting-study.png"
		get_viewport().get_texture().get_image().save_png(output.path_join(filename))
	get_tree().quit()


func _capture_nebula_layers(sunlight: DirectionalLight3D) -> void:
	var output := ProjectSettings.globalize_path("res://").path_join("../.tmp").simplify_path()
	DirAccess.make_dir_recursive_absolute(output)
	for geometry: Node in find_children("*", "GeometryInstance3D", true, false):
		(geometry as GeometryInstance3D).visible = false
	sunlight.visible = false
	if _presentation._prominences != null:
		_presentation._prominences.visible = false
	if _presentation._sun_flare != null:
		_presentation._sun_flare.enabled = false

	var sky_material: ShaderMaterial = _presentation._sky_mat
	sky_material.set_shader_parameter("sun_active", 0.0)
	sky_material.set_shader_parameter("use_baked_nebula", false)
	var star_instance := _presentation._starfield_root.get_child(0) as GeometryInstance3D
	var stages: Array[Dictionary] = [
		{"name": "01-stellar-density", "display_scale": GalacticModelScript.DISPLAY_RADIANCE_SCALE, "extinction": 0.0, "stars": false},
		{"name": "02-stellar-extinction", "display_scale": GalacticModelScript.DISPLAY_RADIANCE_SCALE, "extinction": 1.0, "stars": false},
		{"name": "03-starfield-clear", "display_scale": 0.0, "extinction": 0.0, "stars": true},
		{"name": "04-starfield-extinction", "display_scale": 0.0, "extinction": 1.0, "stars": true},
		{"name": "05-combined", "display_scale": GalacticModelScript.DISPLAY_RADIANCE_SCALE, "extinction": 1.0, "stars": true},
	]
	for stage: Dictionary in stages:
		sky_material.set_shader_parameter("display_radiance_scale", stage["display_scale"])
		sky_material.set_shader_parameter("extinction_scale", stage["extinction"])
		_presentation._starfield_mat.set_shader_parameter("extinction_scale", stage["extinction"])
		star_instance.visible = stage["stars"] as bool
		for _frame: int in range(8):
			await get_tree().process_frame
		await RenderingServer.frame_post_draw
		get_viewport().get_texture().get_image().save_png(
			output.path_join(stage["name"] as String + ".png"))
	get_tree().quit()
