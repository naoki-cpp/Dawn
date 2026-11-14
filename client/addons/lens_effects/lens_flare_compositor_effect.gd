@tool
extends BaseCompositorEffect
class_name LensFlareEffect

const Source = preload("res://scripts/stellar_source.gd")

## Fractions partition the camera response; widths describe the camera, not the star.
@export_range(0.0, 1.0, 0.01) var Streak_Fraction := 0.08
@export_range(0.02, 0.3, 0.001) var Streak_Length := 0.06
@export_range(0.0005, 0.01, 0.0001) var Streak_Width := 0.002
@export_range(0.0, 1.0, 0.01) var Glare_Fraction := 0.4
@export_range(0.002, 0.03, 0.001) var Glare_Width := 0.008
@export_range(1.0, 3.0, 0.01) var Squeeze := 1.8
## Neutral transmission preserves the source colour; optional coating only attenuates it.
@export var Lens_Tint := Color.WHITE
@export_group("Internal Reflections")
@export_range(0.0, 1.0, 0.001) var Ghost_Fraction := 0.0
@export_range(0.003, 0.05, 0.001) var Ghost_Width := 0.018
@export_range(0.0, 1.0, 0.001) var Halo_Fraction := 0.0
@export_range(0.03, 0.25, 0.001) var Halo_Radius := 0.09
@export_group("Occlusion Light Shafts")
@export_range(0.0, 1.0, 0.01) var Rays_Strength := 0.12
@export_range(0.05, 0.5, 0.01) var Rays_Radius := 0.3
@export_range(8, 96, 1) var Rays_Samples := 48
@export_range(0.8, 1.0, 0.001) var Rays_Decay := 0.98
@export_range(0.2, 1.0, 0.01) var Rays_Density := 0.96
@export_range(0.0, 4.0, 0.01) var Rays_Shadow_Weight := 2.0

var _sun_mutex := Mutex.new()
var _pending_source: Array = []
var _pending_is_direction := false
var _source_is_direction := false
var _sun_direction := Vector3.ZERO
var _angular_radius := 0.0
var _source := Vector4(0.5, 0.5, 0.0, 1.0)
var _source_color := Color.WHITE
var _radius := 0.0
var _shader: RID
var _pipeline: RID
var _uniform_buffer: RID


## Source radius is the unclamped projected radius / viewport height.
## Occlusion radius remains a separate depth-sampling footprint in pixels.
func set_sun(position: Vector2, color: Color, visibility: float, occlusion_radius: float, projected_radius: float) -> void:
	_sun_mutex.lock()
	_pending_source = [position, color, visibility, occlusion_radius, projected_radius]
	_pending_is_direction = false
	_sun_mutex.unlock()


## The direction is projected with each view's render-frame camera matrices.
func set_sun_direction(direction: Vector3, color: Color, angular_radius: float) -> void:
	_sun_mutex.lock()
	_pending_source = [direction, color, angular_radius]
	_pending_is_direction = true
	_sun_mutex.unlock()


func direct_fraction() -> float:
	return maxf(0.0, 1.0 - _glare_fraction() - _streak_fraction() - _reflection_fractions().x - _reflection_fractions().y)


func _reflection_fractions() -> Vector2:
	var remaining := maxf(0.0, 1.0 - _glare_fraction() - _streak_fraction())
	var ghost := clampf(Ghost_Fraction, 0.0, remaining)
	return Vector2(ghost, clampf(Halo_Fraction, 0.0, remaining - ghost))


func _glare_fraction() -> float:
	return clampf(Glare_Fraction, 0.0, 1.0)


func _streak_fraction() -> float:
	return clampf(Streak_Fraction, 0.0, 1.0 - _glare_fraction())


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
	var snapshot := _pending_source
	var is_direction := _pending_is_direction
	_pending_source = []
	_sun_mutex.unlock()
	if not snapshot.is_empty():
		_source_is_direction = is_direction
		_source_color = snapshot[1]
		if is_direction:
			_sun_direction = snapshot[0]
			_angular_radius = snapshot[2]
		else:
			var position: Vector2 = snapshot[0]
			_source = Vector4(position.x, position.y, snapshot[2], snapshot[3])
			_radius = snapshot[4] if is_finite(snapshot[4]) and snapshot[4] > 0.0 else 0.0
	_update_uniform_buffer()


func _update_uniform_buffer() -> void:
	var tint := Color(clampf(Lens_Tint.r, 0.0, 1.0), clampf(Lens_Tint.g, 0.0, 1.0), clampf(Lens_Tint.b, 0.0, 1.0))
	var reflections := _reflection_fractions()
	var data: Array = [_source_color, tint, _source,
		Vector4(_length(), _width(), _streak_fraction(), _glare_fraction()),
		Vector4(_glare_width(), Source.variance(_radius), Source.lens_flux(_radius), clampf(Squeeze, 1.0, 3.0)),
		Vector4(_rays_radius(), clampf(Rays_Strength, 0.0, 1.0), clampi(Rays_Samples, 8, 96), 0.0),
		Vector4(clampf(Rays_Decay, 0.8, 1.0), clampf(Rays_Density, 0.2, 1.0), clampf(Rays_Shadow_Weight, 0.0, 4.0), 0.0),
		Vector4(reflections.x, clampf(Ghost_Width, 0.003, 0.05), reflections.y, clampf(Halo_Radius, 0.03, 0.25))]
	if not _uniform_buffer.is_valid():
		_uniform_buffer = create_uniform_buffer(data)
	else:
		update_uniform_buffer(_uniform_buffer, data)


func _length() -> float:
	return clampf(Streak_Length, 0.02, 0.3) * clampf(Squeeze, 1.0, 3.0) / 1.8


func _width() -> float:
	return maxf(clampf(Streak_Width, 0.0005, 0.01), 0.75 / maxf(render_size.y, 1.0))


func _glare_width() -> float:
	return clampf(Glare_Width, 0.002, 0.03)


func _rays_radius() -> float:
	return clampf(Rays_Radius, 0.05, 0.5) if Rays_Strength > 0.0 else 0.0


func _render_view(view: int) -> void:
	if _source_is_direction:
		var projected := _project_direction_for_view(view)
		if projected.is_empty():
			return
		var position: Vector2 = projected.position
		_source = Vector4(position.x, position.y, projected.visibility, projected.occlusion_radius)
		_radius = projected.projected_radius
		_update_uniform_buffer()
	if not _pipeline.is_valid() or not _uniform_buffer.is_valid() or _source.z <= 0.0:
		return
	var variance := Source.variance(_radius)
	var broad := sqrt(pow(_glare_width() * 5.0, 2.0) + variance)
	var horizontal := maxf(sqrt(_length() * _length() + variance),
		sqrt(pow(_glare_width() * clampf(Squeeze, 1.0, 3.0), 2.0) + variance))
	var region := effect_region(render_size, Vector2(_source.x, _source.y), horizontal,
		sqrt(pow(_width() * 2.5, 2.0) + variance), broad, _rays_radius())
	var reflections := _reflection_fractions()
	if reflections.x > 0.0 or reflections.y > 0.0:
		# Reflection images lie on both sides of the optical axis, not just near the source.
		region = Rect2i(Vector2i.ZERO, render_size)
	if region.size.x <= 0 or region.size.y <= 0:
		return
	_workgroups = Vector3i(ceili(region.size.x / 16.0), ceili(region.size.y / 16.0), 1)
	run_compute_shader("Solar camera optics", _shader, _pipeline, [
		[get_color_image_uniform(view), get_depth_sampler_uniform(view, nearest_sampler)],
		[get_uniform_buffer_uniform(_uniform_buffer, 0)]],
		PackedFloat32Array([region.position.x, region.position.y, region.size.x, region.size.y]))


func _project_direction_for_view(view: int) -> Dictionary:
	if render_scene_data == null:
		return {}
	return project_sun_direction(_sun_direction, _angular_radius,
		render_scene_data.get_cam_transform(), render_scene_data.get_view_projection(view), render_size)


## Project a distant source and its angular radius through the render-frame view.
static func project_sun_direction(
	direction: Vector3,
	angular_radius: float,
	camera_transform: Transform3D,
	projection: Projection,
	viewport_size: Vector2i
) -> Dictionary:
	if direction.is_zero_approx() or not direction.is_finite() or not is_finite(angular_radius) \
			or angular_radius <= 0.0 or viewport_size.x <= 0 or viewport_size.y <= 0 \
			or projection.is_orthogonal():
		return {}
	var ray := direction.normalized()
	var world_up := camera_transform.basis.y.normalized()
	var tangent := world_up.slide(ray)
	if tangent.length_squared() < 0.000001:
		tangent = camera_transform.basis.x.normalized().slide(ray)
	if tangent.length_squared() < 0.000001:
		return {}
	tangent = tangent.normalized()
	var world_limb := (ray * cos(angular_radius) + tangent * sin(angular_radius)).normalized()
	var camera_basis_inverse := camera_transform.basis.inverse()
	var camera_ray := camera_basis_inverse * ray
	var camera_limb := camera_basis_inverse * world_limb
	var source_clip := projection * Vector4(camera_ray.x, camera_ray.y, camera_ray.z, 0.0)
	var limb_clip := projection * Vector4(camera_limb.x, camera_limb.y, camera_limb.z, 0.0)
	if source_clip.w <= 0.0 or limb_clip.w <= 0.0 or not source_clip.is_finite() or not limb_clip.is_finite():
		return {}
	var position := Vector2(source_clip.x, -source_clip.y) / source_clip.w * 0.5 + Vector2(0.5, 0.5)
	var limb_position := Vector2(limb_clip.x, -limb_clip.y) / limb_clip.w * 0.5 + Vector2(0.5, 0.5)
	if not position.is_finite() or not limb_position.is_finite() \
			or position.x <= 0.0 or position.x >= 1.0 or position.y <= 0.0 or position.y >= 1.0:
		return {}
	var pixel_radius := ((limb_position - position) * Vector2(viewport_size)).length()
	var edge := minf(minf(position.x, 1.0 - position.x), minf(position.y, 1.0 - position.y))
	return {"position": position, "visibility": smoothstep(0.0, 0.04, edge),
		"radius": pixel_radius, "occlusion_radius": maxf(pixel_radius, 1.0),
		"projected_radius": pixel_radius / viewport_size.y}


## Six PSF standard deviations plus independently bounded shaft support.
static func effect_region(size: Vector2i, source: Vector2, length: float, width: float = 0.002, glare_width: float = 0.04, rays_radius: float = 0.0) -> Rect2i:
	if size.x <= 0 or size.y <= 0 or not source.is_finite() or source.x < 0.0 or source.x >= 1.0 or source.y < 0.0 or source.y >= 1.0:
		return Rect2i()
	var extent := Vector2(length, maxf(width, 0.75 / size.y)).max(Vector2.ONE * glare_width) * 6.0 * size.y
	extent = extent.max(Vector2.ONE * rays_radius * size.y)
	var center := source * Vector2(size)
	var start := Vector2i((center - extent).floor()).clamp(Vector2i.ZERO, size)
	var end := Vector2i((center + extent).ceil()).clamp(Vector2i.ZERO, size)
	return Rect2i(start, end - start)
