## Owns distant background resources, asynchronous replacement and angular stars.
## Galaxy generation is CPU-only Rust; this node only creates Godot resources.
class_name SkyBackground
extends Node3D

const DEFAULT_SEED: int = 0x5DA0
const PANORAMA_SIZE := Vector2i(512, 256)
const STAR_COUNT: int = 2000
const SHELL_RADIUS: float = 10000.0

var _generator := GalaxyBackgroundGenerator.new()
var _sky_material: ShaderMaterial
var _stars := MultiMeshInstance3D.new()
var _star_material := ShaderMaterial.new()
var _ready_background := false


func build(sky_material: ShaderMaterial) -> void:
	_sky_material = sky_material
	name = "SkyBackground"
	_star_material.shader = preload("res://shaders/star_sprite.gdshader")
	_stars.name = "Stars"
	_stars.material_override = _star_material
	_stars.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	_stars.custom_aabb = AABB(Vector3.ONE * -SHELL_RADIUS, Vector3.ONE * SHELL_RADIUS * 2.0)
	add_child(_stars)
	request_background()


func request_background(background_seed: int = DEFAULT_SEED,
		size: Vector2i = PANORAMA_SIZE, star_count: int = STAR_COUNT) -> void:
	_generator.request(background_seed, size.x, size.y, star_count)


func update_frame(camera: Camera3D) -> void:
	var completed: GalaxyBackgroundResult = _generator.poll()
	if completed != null:
		_apply_background(completed)
	var error := _generator.take_error()
	if not error.is_empty():
		push_warning("[SkyBackground] " + error)
	if camera == null:
		return
	# Direction-only stars must not acquire artificial parallax from orbit/zoom.
	global_position = camera.global_position
	_stars.scale = Vector3.ONE * minf(1.0, camera.far * 0.5 / SHELL_RADIUS)
	_star_material.set_shader_parameter("viewport_pixels", camera.get_viewport().get_visible_rect().size)


func set_stellar_disc(direction: Vector3, angular_radius: float) -> void:
	_star_material.set_shader_parameter("sun_direction", direction)
	_star_material.set_shader_parameter("sun_angular_radius", angular_radius)


func is_background_ready() -> bool:
	return _ready_background


func set_layer_visibility(continuum: bool, stars: bool) -> void:
	_sky_material.set_shader_parameter("background_visible", continuum)
	_stars.visible = stars


func _apply_background(completed: GalaxyBackgroundResult) -> void:
	var image := completed.panorama()
	if image == null or image.is_empty():
		push_warning("[SkyBackground] generation returned an empty panorama")
		return
	var directions := completed.star_directions()
	var colors := completed.star_colors()
	var fluxes := completed.star_fluxes()
	var mesh := MultiMesh.new()
	mesh.transform_format = MultiMesh.TRANSFORM_3D
	mesh.use_colors = true
	mesh.use_custom_data = true
	var quad := QuadMesh.new()
	quad.size = Vector2.ONE
	mesh.mesh = quad
	mesh.instance_count = directions.size()
	for index: int in directions.size():
		mesh.set_instance_transform(index, Transform3D(Basis.IDENTITY, directions[index] * SHELL_RADIUS))
		mesh.set_instance_color(index, colors[index])
		mesh.set_instance_custom_data(index, Color(fluxes[index], 0.0, 0.0, 0.0))
	# Both representations become visible together, never from different requests.
	_sky_material.set_shader_parameter("background_panorama", ImageTexture.create_from_image(image))
	_sky_material.set_shader_parameter("background_ready", true)
	_stars.multimesh = mesh
	_ready_background = true


func _exit_tree() -> void:
	_generator.cancel()
