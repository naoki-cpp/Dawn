@tool
extends BaseCompositorEffect
class_name LensFlareEffect

## Camera-space optical response, independent of the stellar disc radius.
@export_range(0.0, 4.0, 0.01) var Intensity := 0.28
@export_range(0.02, 0.3, 0.001) var Streak_Length := 0.09
@export_range(0.0005, 0.01, 0.0001) var Streak_Width := 0.0028
@export_range(0.0, 2.0, 0.01) var Glare_Strength := 0.7
@export_range(0.01, 0.1, 0.001) var Glare_Radius := 0.028
@export_range(1.0, 3.0, 0.01) var Squeeze := 1.8
@export var Lens_Tint := Color(1.0, 0.68, 0.32, 1.0)
@export_group("Occlusion Light Shafts")
@export_range(0.0, 1.0, 0.01) var Rays_Strength := 0.12
@export_range(0.05, 0.5, 0.01) var Rays_Radius := 0.3
@export_range(8, 96, 1) var Rays_Samples := 48
@export_range(0.8, 1.0, 0.001) var Rays_Decay := 0.98
@export_range(0.2, 1.0, 0.01) var Rays_Density := 0.96
@export_range(0.0, 4.0, 0.01) var Rays_Shadow_Weight := 2.0

var _sun_mutex := Mutex.new()
var _pending_sun: Array = []
var _source := Vector4(0.5, 0.5, 0.0, 1.0)
var _source_color := Color.WHITE
var _optical_scale := 1.0
var _shader: RID
var _pipeline: RID
var _uniform_buffer: RID


## Publish one coherent source snapshot to the rendering thread.
func set_sun(position: Vector2, color: Color, visibility: float, radius: float, optical_scale: float = 1.0) -> void:
	_sun_mutex.lock()
	_pending_sun = [position, color, visibility, radius, optical_scale]
	_sun_mutex.unlock()


func _initialize_resource() -> void:
	effect_callback_type = EFFECT_CALLBACK_TYPE_POST_TRANSPARENT
	access_resolved_color = true
	access_resolved_depth = true


func _initialize_render() -> void:
	_shader = create_shader("res://addons/lens_effects/lens_flares.glsl")
	if _shader.is_valid():
		_pipeline = create_pipeline(_shader)


func _render_setup() -> void:
	_sun_mutex.lock()
	var snapshot := _pending_sun
	_pending_sun = []
	_sun_mutex.unlock()
	if not snapshot.is_empty():
		var position: Vector2 = snapshot[0]
		_source = Vector4(position.x, position.y, snapshot[2], snapshot[3])
		_source_color = snapshot[1]
		_optical_scale = clampf(snapshot[4], 0.1, 1.5) if is_finite(snapshot[4]) else 1.0
	var tint := Color(_source_color.r * Lens_Tint.r, _source_color.g * Lens_Tint.g,
		_source_color.b * Lens_Tint.b, _source_color.a)
	var data: Array = [tint, _source,
		Vector4(_length(), _width(), maxf(Intensity, 0.0) * _optical_scale / clampf(Squeeze / 1.8, 0.5, 2.0), 0.0),
		Vector4(_glare_radius(), maxf(Glare_Strength, 0.0) * _optical_scale, 0.0, 0.0),
		Vector4(_rays_radius(), clampf(Rays_Strength, 0.0, 1.0) * _optical_scale, clampi(Rays_Samples, 8, 96), 0.0),
		Vector4(clampf(Rays_Decay, 0.8, 1.0), clampf(Rays_Density, 0.2, 1.0), clampf(Rays_Shadow_Weight, 0.0, 4.0), 0.0)]
	if not _uniform_buffer.is_valid():
		_uniform_buffer = create_uniform_buffer(data)
	else:
		update_uniform_buffer(_uniform_buffer, data)


func _length() -> float:
	return clampf(Streak_Length, 0.02, 0.3) * clampf(Squeeze, 1.0, 3.0) / 1.8 * _optical_scale


func _width() -> float:
	return maxf(clampf(Streak_Width, 0.0005, 0.01), 0.75 / maxf(render_size.y, 1.0))


func _glare_radius() -> float:
	return clampf(Glare_Radius, 0.01, 0.1) * _optical_scale


func _rays_radius() -> float:
	return clampf(Rays_Radius, 0.05, 0.5) * _optical_scale if Rays_Strength > 0.0 else 0.0


func _render_view(view: int) -> void:
	if not _pipeline.is_valid() or not _uniform_buffer.is_valid() or _source.z <= 0.0:
		return
	var region := effect_region(render_size, Vector2(_source.x, _source.y), _length(), _width(), _glare_radius(), _rays_radius())
	if region.size.x <= 0 or region.size.y <= 0:
		return
	_workgroups = Vector3i(ceili(region.size.x / 16.0), ceili(region.size.y / 16.0), 1)
	run_compute_shader("Solar camera optics", _shader, _pipeline, [
		[get_color_image_uniform(view), get_depth_sampler_uniform(view, nearest_sampler)],
		[get_uniform_buffer_uniform(_uniform_buffer, 0)]],
		PackedFloat32Array([region.position.x, region.position.y, region.size.x, region.size.y]))


## Finite optical support plus the independently bounded shaft radius.
static func effect_region(size: Vector2i, source: Vector2, length: float, width: float = 0.0028, glare_radius: float = 0.028, rays_radius: float = 0.0) -> Rect2i:
	if size.x <= 0 or size.y <= 0 or not source.is_finite() or source.x < 0.0 or source.x >= 1.0 or source.y < 0.0 or source.y >= 1.0:
		return Rect2i()
	var center := source * Vector2(size)
	var extent := Vector2(length * 6.0, maxf(width, 0.75 / size.y) * 1.8 * 6.0).max(Vector2.ONE * glare_radius * 6.0) * size.y
	extent = extent.max(Vector2.ONE * rays_radius * size.y)
	var start := Vector2i((center - extent).floor()).clamp(Vector2i.ZERO, size)
	var end := Vector2i((center + extent).ceil()).clamp(Vector2i.ZERO, size)
	return Rect2i(start, end - start)
