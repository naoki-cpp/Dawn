## world_presentation.gd
##
## Client-side world presentation policy for one play session. This module owns
## the visual side effects that remain after state lives in WorldSession and
## interaction policy lives in WorldInteraction: floating-origin rebases,
## navigation-marker placement, sky/sun updates, warp-tunnel intensity, and
## player-ship presentation setup.
class_name WorldPresentation
extends RefCounted

const NavigationMarkerRendererScript = preload("res://scripts/navigation_marker_renderer.gd")

const NAV_MARKER_CLAMP_DISTANCE : float = 30_000.0
const WARP_TUNNEL_THRESHOLD : float = 2_000.0
const WARP_TUNNEL_FADE_RATE : float = 3.0
const WARP_TUNNEL_FOV_BOOST : float = 15.0
const SUN_EFFECTIVE_DISTANCE : float = 50_000_000.0
const SUN_FAR_DIRECTION : Vector3 = Vector3(0.62, 0.31, 0.72)

var _world_scale: float = 0.1
var _world: RefCounted = null
var _camera: Camera3D = null
var _warp_tunnel: ColorRect = null
var _gates_root: Node3D = null
var _bodies_root: Node3D = null
var _sky_mat: ShaderMaterial = null
var _player_material: StandardMaterial3D = null
var _tactical_overlay: Node3D = null
## The ship currently wearing the player material/indicators, so
## attach_player_ship() can revert it when the active ship switches to a
## different one (ADR-0037 SelectActiveShip/Disembark/Assemble) instead of
## leaving a stale player-colored ship behind.
var _player_ship: Node3D = null
var _camera_base_fov: float = 60.0
var _warp_tunnel_amount: float = 0.0


func build(
	parent: Node,
	camera: Camera3D,
	warp_tunnel: ColorRect,
	gates_root: Node3D,
	bodies_root: Node3D,
	world: RefCounted,
	world_scale: float
) -> void:
	_camera = camera
	_warp_tunnel = warp_tunnel
	_gates_root = gates_root
	_bodies_root = bodies_root
	_world = world
	_world_scale = world_scale
	if _camera != null:
		_camera_base_fov = _camera.fov
	_build_player_material()
	_setup_space_environment(parent)


func refresh(delta: float, player_ship_id: int, ships: Dictionary, bodies: Array) -> void:
	_maybe_rebase_origin(player_ship_id, ships)
	_update_position_markers(_bodies_root, "nav_pos", player_ship_id, ships)
	_update_position_markers(_gates_root, "nav_pos", player_ship_id, ships)
	_update_sun_direction(player_ship_id, ships, bodies)
	_update_warp_tunnel_effect(delta, player_ship_id, ships)


func respawn_navigation_markers(
	gates: Array,
	bodies: Array,
	stations: Array,
	server_to_godot_pos: Callable,
	clear_navigation_selection: Callable
) -> void:
	if _gates_root != null:
		NavigationMarkerRendererScript.spawn_gate_markers(_gates_root, gates, _world_scale, server_to_godot_pos)
	if _bodies_root == null:
		return
	clear_navigation_selection.call()
	NavigationMarkerRendererScript.spawn_body_markers(_bodies_root, bodies, _world_scale, server_to_godot_pos)
	NavigationMarkerRendererScript.spawn_station_markers(_bodies_root, stations, _world_scale, server_to_godot_pos)


func apply_origin_rebase(new_origin: Vector3, keep_player_fixed: bool, player_ship_id: int, ships: Dictionary) -> void:
	if _world == null:
		return
	var shift: Vector3 = _world.rebase_to(new_origin)
	for id: int in ships:
		if keep_player_fixed and id == player_ship_id:
			continue
		(ships[id] as Node3D).position += shift
	if not keep_player_fixed and _camera != null:
		_camera.global_position += shift
		_camera.call("on_origin_rebased", shift)


func attach_player_ship(ship: Node3D, weapon_range: float, weapon_falloff: float) -> void:
	if ship == null:
		return
	## Revert the previously-piloted ship (if any, and if it isn't the same
	## one) instead of leaving it with the player material/velocity-thrust
	## indicators forever -- regression: switching active ship left the old
	## ship permanently player-colored.
	if _player_ship != null and is_instance_valid(_player_ship) and _player_ship != ship:
		_clear_player_material(_player_ship)
		_player_ship.call("clear_as_player")
	_player_ship = ship
	_apply_player_material(ship)
	ship.call("set_as_player")
	if _camera != null:
		_camera.call("set_target", ship)
	if _tactical_overlay != null:
		_tactical_overlay.queue_free()
	var overlay_script: GDScript = load("res://scripts/tactical_overlay.gd") as GDScript
	if overlay_script != null:
		_tactical_overlay = Node3D.new()
		_tactical_overlay.set_script(overlay_script)
		ship.add_child(_tactical_overlay)
		update_tactical_overlay_ranges(weapon_range, weapon_falloff)


func update_tactical_overlay_ranges(weapon_range: float, weapon_falloff: float) -> void:
	if _tactical_overlay == null:
		return
	_tactical_overlay.call("set_ranges", weapon_range * _world_scale, weapon_falloff * _world_scale)


func toggle_tactical_overlay() -> void:
	if _tactical_overlay != null:
		_tactical_overlay.call("toggle_visible")


static func clamped_marker_position(
	player_godot: Vector3,
	marker_godot: Vector3,
	clamp_distance: float = NAV_MARKER_CLAMP_DISTANCE
) -> Vector3:
	var delta: Vector3 = marker_godot - player_godot
	var dist: float = delta.length()
	if dist <= clamp_distance or dist <= 0.0:
		return marker_godot
	return player_godot + delta / dist * clamp_distance


static func next_warp_tunnel_amount(
	current: float,
	speed_godot: float,
	delta: float,
	threshold: float = WARP_TUNNEL_THRESHOLD,
	fade_rate: float = WARP_TUNNEL_FADE_RATE
) -> float:
	var target := 1.0 if speed_godot > threshold else 0.0
	return lerpf(current, target, clampf(delta * fade_rate, 0.0, 1.0))


static func sun_state(
	bodies: Array,
	player_server: Vector3,
	dir_to_godot: Callable
) -> Dictionary:
	var star: Dictionary = _find_star(bodies)
	if star.is_empty():
		return {"active": false}
	var star_pos: Vector3 = star.get("position", Vector3.ZERO) as Vector3
	var effective_star_pos: Vector3 = star_pos + SUN_FAR_DIRECTION.normalized() * SUN_EFFECTIVE_DISTANCE
	var diff: Vector3 = effective_star_pos - player_server
	if diff.length_squared() < 1.0:
		return {"active": false}
	var godot_dir: Vector3 = (dir_to_godot.call(diff) as Vector3).normalized()
	var spec: float = star.get("spectral_type", 0.60) as float
	var sun_col: Color = NavigationMarkerRendererScript.spectral_color(spec)
	return {
		"active": true,
		"direction": godot_dir,
		"color": Vector3(sun_col.r, sun_col.g, sun_col.b),
	}


static func _find_star(bodies: Array) -> Dictionary:
	for entry: Variant in bodies:
		var body: Dictionary = entry as Dictionary
		if (body.get("kind", "") as String) == "Star":
			return body
	return {}


func _update_position_markers(root: Node3D, meta_key: String, player_ship_id: int, ships: Dictionary) -> void:
	if root == null or player_ship_id < 0 or not ships.has(player_ship_id):
		return
	var player_godot: Vector3 = (ships[player_ship_id] as Node3D).global_position
	for child: Node in root.get_children():
		var marker: Node3D = child as Node3D
		if marker == null or not marker.has_meta(meta_key):
			continue
		var marker_godot: Vector3 = _server_to_godot_pos(marker.get_meta(meta_key) as Vector3)
		marker.global_position = clamped_marker_position(player_godot, marker_godot)


func _update_warp_tunnel_effect(delta: float, player_ship_id: int, ships: Dictionary) -> void:
	var speed_godot := 0.0
	if player_ship_id >= 0 and ships.has(player_ship_id):
		speed_godot = (ships[player_ship_id] as Node3D).call("get_speed_godot") as float
	_warp_tunnel_amount = next_warp_tunnel_amount(_warp_tunnel_amount, speed_godot, delta)
	if _warp_tunnel != null:
		_warp_tunnel.call("set_intensity", _warp_tunnel_amount)
	if _camera != null:
		_camera.fov = _camera_base_fov + WARP_TUNNEL_FOV_BOOST * _warp_tunnel_amount


func _maybe_rebase_origin(player_ship_id: int, ships: Dictionary) -> void:
	if _world == null or player_ship_id < 0 or not ships.has(player_ship_id):
		return
	var player_server: Vector3 = _world.to_server((ships[player_ship_id] as Node3D).global_position)
	if not _world.should_rebase(player_server):
		return
	apply_origin_rebase(player_server, false, player_ship_id, ships)


func _update_sun_direction(player_ship_id: int, ships: Dictionary, bodies: Array) -> void:
	if _sky_mat == null or player_ship_id < 0 or not ships.has(player_ship_id) or _world == null:
		return
	var ship_node: Node3D = ships[player_ship_id] as Node3D
	var player_server: Vector3 = _world.to_server(ship_node.global_position)
	var state: Dictionary = sun_state(bodies, player_server, Callable(_world, "dir_to_godot"))
	if not (state.get("active", false) as bool):
		_sky_mat.set_shader_parameter("sun_active", 0.0)
		return
	_sky_mat.set_shader_parameter("sun_direction", state.get("direction", Vector3.FORWARD) as Vector3)
	_sky_mat.set_shader_parameter("sun_active", 1.0)
	_sky_mat.set_shader_parameter("sun_color", state.get("color", Vector3.ONE) as Vector3)


func _server_to_godot_pos(p: Vector3) -> Vector3:
	return _world.to_godot(p)


func _setup_space_environment(parent: Node) -> void:
	var shader := load("res://shaders/space_sky.gdshader") as Shader
	if shader == null:
		push_warning("[WorldPresentation] space_sky.gdshader not found")
		return

	var sky_mat := ShaderMaterial.new()
	sky_mat.shader = shader
	_sky_mat = sky_mat
	sky_mat.set_shader_parameter("star_threshold", 0.960)
	sky_mat.set_shader_parameter("star_brightness", 3.5)
	sky_mat.set_shader_parameter("nebula_strength", 0.40)
	sky_mat.set_shader_parameter("milkyway_strength", 0.12)
	sky_mat.set_shader_parameter("milkyway_color", Color(0.48, 0.58, 0.90))
	sky_mat.set_shader_parameter("ambient_color", Color(0.004, 0.003, 0.010))

	var sky := Sky.new()
	sky.sky_material = sky_mat
	sky.process_mode = Sky.PROCESS_MODE_REALTIME
	sky.radiance_size = Sky.RADIANCE_SIZE_256

	var env := Environment.new()
	env.background_mode = Environment.BG_SKY
	env.sky = sky
	env.ambient_light_source = Environment.AMBIENT_SOURCE_SKY
	env.ambient_light_energy = 0.03
	env.tonemap_mode = Environment.TONE_MAPPER_FILMIC
	env.tonemap_exposure = 1.0
	env.tonemap_white = 6.0
	env.glow_enabled = true
	env.glow_normalized = false
	env.glow_intensity = 0.8
	env.glow_bloom = 0.10
	env.glow_blend_mode = Environment.GLOW_BLEND_MODE_SOFTLIGHT
	env.glow_hdr_threshold = 2.0
	env.glow_hdr_scale = 1.0

	var world_env := WorldEnvironment.new()
	world_env.environment = env
	parent.add_child(world_env)


func _build_player_material() -> void:
	_player_material = StandardMaterial3D.new()
	_player_material.albedo_color = Color(1.0, 0.5, 0.1, 1)
	_player_material.metallic = 0.9
	_player_material.roughness = 0.2
	_player_material.emission_enabled = true
	_player_material.emission = Color(1.0, 0.3, 0.0, 1)
	_player_material.emission_energy_multiplier = 1.5


func _apply_player_material(ship: Node3D) -> void:
	var hull: MeshInstance3D = ship.get_node_or_null("Hull") as MeshInstance3D
	if hull != null:
		hull.set_surface_override_material(0, _player_material)


## Undo _apply_player_material(): clearing the override lets the Hull mesh's
## own material (whatever a non-piloted ship normally uses) show again.
func _clear_player_material(ship: Node3D) -> void:
	var hull: MeshInstance3D = ship.get_node_or_null("Hull") as MeshInstance3D
	if hull != null:
		hull.set_surface_override_material(0, null)
