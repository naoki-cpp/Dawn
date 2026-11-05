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
const StarfieldScript = preload("res://scripts/starfield.gd")

const DISTANT_BODY_LABEL_OFFSET : float = 800.0
const WARP_TUNNEL_THRESHOLD : float = 2_000.0
const WARP_TUNNEL_FADE_IN_RATE : float = 3.0
const WARP_TUNNEL_FADE_OUT_RATE : float = 0.5
const WARP_TUNNEL_MAX_VISUAL_DELTA : float = 0.1
const WARP_TUNNEL_FOV_BOOST : float = 7.0
const SUN_MIN_RENDER_ANGULAR_RADIUS : float = 0.0001
## Resolution of the baked background panorama (ADR-0055). The nebula is
## low-frequency, so this is far above what it can resolve; the star layer is
## separate geometry and is unaffected by it.
const NEBULA_PANORAMA_SIZE : Vector2i = Vector2i(2048, 1024)
## RenderingServer.sky_bake_panorama() reads the sky the renderer has already
## processed. Two consequences: the bake cannot happen in the frame the Sky is
## created, and a shader uniform set this frame is not visible to it either --
## which is why arming and baking are separate steps below.
const NEBULA_BAKE_ARM_FRAME : int = 2
const NEBULA_BAKE_FRAME : int = 5
## Aerial perspective: distant objects wash toward the background colour, which
## is a depth cue no amount of star tuning provides. Distances are the ones
## EVE's Tr2PPFogEffect uses (2 km / 120 km), in server metres, converted with
## the same render scale every other transform uses.
const FOG_BEGIN_METRES : float = 2_000.0
const FOG_END_METRES : float = 120_000.0

var _world: RefCounted = null
var _camera: Camera3D = null
var _warp_tunnel: ColorRect = null
var _gates_root: Node3D = null
var _bodies_root: Node3D = null
var _sky_mat: ShaderMaterial = null
var _sun_flare: CompositorEffect = null
var _starfield_root: Node3D = null
var _starfield_mat: ShaderMaterial = null
var _sky: Sky = null
var _nebula_bake_frames: int = 0
var _nebula_bake_done: bool = false
var _directional_light: DirectionalLight3D = null
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
	directional_light: DirectionalLight3D = null
) -> void:
	_camera = camera
	_warp_tunnel = warp_tunnel
	_gates_root = gates_root
	_bodies_root = bodies_root
	_world = world
	_directional_light = directional_light
	if _camera != null:
		_camera_base_fov = _camera.fov
	_build_player_material()
	_setup_space_environment(parent)


func _render_scale() -> float:
	return _world.call("render_scale") as float


func refresh(delta: float, player_ship_id: int, ships: Dictionary, bodies: Array) -> void:
	_maybe_rebase_origin(player_ship_id, ships)
	_update_starfield()
	_maybe_bake_nebula()
	_update_position_markers(_bodies_root, "nav_pos", player_ship_id, ships)
	_update_position_markers(_gates_root, "nav_pos", player_ship_id, ships)
	_update_sun_direction(player_ship_id, ships, bodies)
	_update_warp_tunnel_effect(delta, player_ship_id, ships)


func respawn_navigation_markers(
	gates: Array,
	bodies: Array,
	stations: Array,
	clear_navigation_selection: Callable
) -> void:
	if _gates_root != null:
		NavigationMarkerRendererScript.spawn_gate_markers(
			_gates_root, gates, _render_scale(), Callable(_world, "to_godot_components"))
	if _bodies_root == null:
		return
	clear_navigation_selection.call()
	NavigationMarkerRendererScript.spawn_body_markers(
		_bodies_root, bodies, _render_scale(), Callable(_world, "to_godot_components"))
	NavigationMarkerRendererScript.spawn_station_markers(
		_bodies_root, stations, _render_scale(), Callable(_world, "to_godot_components"))


func apply_origin_rebase(
	new_origin: PackedFloat64Array,
	keep_player_fixed: bool,
	player_ship_id: int,
	ships: Dictionary
) -> void:
	apply_origin_rebase_components(new_origin, keep_player_fixed, player_ship_id, ships)


func apply_origin_rebase_components(
	new_origin: PackedFloat64Array,
	keep_player_fixed: bool,
	player_ship_id: int,
	ships: Dictionary
) -> void:
	if _world == null:
		return
	var shift: Vector3 = _world.call(
		"rebase_to_components", new_origin[0], new_origin[1], new_origin[2]) as Vector3
	for id: int in ships:
		var ship := ships[id] as Node3D
		if ship.has_method("apply_origin_rebase"):
			## Every track must adopt the same absolute origin, including the
			## player track. The new render frame keeps the player fixed when the
			## caller selected keep_player_fixed.
			ship.call("apply_origin_rebase", new_origin)
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
	if _player_ship != ship:
		detach_player_ship()
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


## Revert the tracked player ship's material/velocity-thrust indicators and
## forget it, without attaching a replacement -- the caller has no active
## ship at all (ADR-0037 Disembark, or the active ship despawned). Idempotent
## when there's no tracked ship, or it's already been freed. Camera framing
## is left untouched: staying on the last-piloted ship's position is a
## reasonable default for "no ship to fly" rather than snapping away.
func detach_player_ship() -> void:
	if _player_ship != null and is_instance_valid(_player_ship):
		_clear_player_material(_player_ship)
		_player_ship.call("clear_as_player")
	_player_ship = null


func update_tactical_overlay_ranges(weapon_range: float, weapon_falloff: float) -> void:
	if _tactical_overlay == null:
		return
	_tactical_overlay.call("set_ranges", weapon_range * _render_scale(), weapon_falloff * _render_scale())


func toggle_tactical_overlay() -> void:
	if _tactical_overlay != null:
		_tactical_overlay.call("toggle_visible")


static func next_warp_tunnel_amount(
	current: float,
	speed_godot: float,
	delta: float,
	threshold: float = WARP_TUNNEL_THRESHOLD,
	fade_in_rate: float = WARP_TUNNEL_FADE_IN_RATE,
	fade_out_rate: float = WARP_TUNNEL_FADE_OUT_RATE
) -> float:
	var target := 1.0 if speed_godot > threshold else 0.0
	if target > current:
		return lerpf(current, target, clampf(delta * fade_in_rate, 0.0, 1.0))
	var visual_delta := clampf(delta, 0.0, WARP_TUNNEL_MAX_VISUAL_DELTA)
	return move_toward(current, target, visual_delta * fade_out_rate)


## Direction from the star toward the illuminated scene. `sun_direction` is
## the opposite direction (observer -> star), so the light ray must be
## reversed before orienting DirectionalLight3D.
static func star_light_ray(sun_direction: Vector3) -> Vector3:
	if sun_direction.length_squared() <= 0.000001:
		return Vector3.FORWARD
	return -sun_direction.normalized()


static func light_up_axis(light_ray: Vector3) -> Vector3:
	return Vector3.RIGHT if absf(light_ray.dot(Vector3.UP)) > 0.95 else Vector3.UP


static func _find_star(bodies: Array) -> CelestialBodyRecord:
	for entry: Variant in bodies:
		var body: CelestialBodyRecord = entry as CelestialBodyRecord
		if body.kind == "Star":
			return body
	return null


func _update_position_markers(root: Node3D, meta_key: String, player_ship_id: int, ships: Dictionary) -> void:
	if root == null or player_ship_id < 0 or not ships.has(player_ship_id):
		return
	var player_ship: Node3D = ships[player_ship_id] as Node3D
	var player_godot: Vector3 = player_ship.global_position
	var player_server: PackedFloat64Array = _ship_server_position(player_ship)
	for child: Node in root.get_children():
		var marker: Node3D = child as Node3D
		if marker == null or not marker.has_meta(meta_key):
			continue
		var marker_server := marker.get_meta(meta_key) as PackedFloat64Array
		var projected := CelestialProjection.marker(
			player_server, marker_server, player_godot, _camera, _render_scale(),
			bool(marker.get_meta("preserve_physical_position", false)),
			marker.has_meta("physical_body_mesh"), float(marker.get_meta("physical_extent", 0.0)))
		if projected == null:
			continue
		marker.global_position = projected.position
		_update_body_lod(marker, projected.body_scale)


func _ship_server_position(ship: Node3D) -> PackedFloat64Array:
	if ship.has_method("world_presentation_position"):
		return ship.call("world_presentation_position") as PackedFloat64Array
	if ship.has_method("server_position"):
		return ship.call("server_position") as PackedFloat64Array
	return _world.to_server_components(ship.global_position)


func _update_body_lod(marker: Node3D, body_scale: float) -> void:
	if not marker.has_meta("physical_body_mesh") and not marker.has_meta("body_label"):
		return
	var body_mesh: MeshInstance3D = marker.get_meta("physical_body_mesh") as MeshInstance3D
	if body_mesh != null:
		body_mesh.scale = Vector3.ONE * body_scale
		body_mesh.visible = true
	var body_label: Label3D = marker.get_meta("body_label") as Label3D
	if body_label != null:
		var label_offset := DISTANT_BODY_LABEL_OFFSET
		if body_mesh != null:
			var sphere: SphereMesh = body_mesh.mesh as SphereMesh
			label_offset = sphere.radius * body_scale * 1.4
		body_label.position = Vector3(0.0, label_offset, 0.0)


func _update_warp_tunnel_effect(delta: float, player_ship_id: int, ships: Dictionary) -> void:
	var speed_godot := 0.0
	var velocity := Vector3.ZERO
	if player_ship_id >= 0 and ships.has(player_ship_id):
		var player_ship: Node3D = ships[player_ship_id] as Node3D
		speed_godot = player_ship.call("get_speed_godot") as float
		if _camera != null and player_ship.has_method("get_velocity_godot"):
			velocity = player_ship.call("get_velocity_godot") as Vector3
	_warp_tunnel_amount = next_warp_tunnel_amount(_warp_tunnel_amount, speed_godot, delta)
	if _warp_tunnel != null:
		_warp_tunnel.call("set_intensity", _warp_tunnel_amount)
	if _camera != null:
		_camera.fov = _camera_base_fov + WARP_TUNNEL_FOV_BOOST * _warp_tunnel_amount
		if _warp_tunnel != null:
			_warp_tunnel.call("set_motion", velocity, _camera.global_basis, _camera.fov)


func _maybe_rebase_origin(player_ship_id: int, ships: Dictionary) -> void:
	if _world == null or player_ship_id < 0 or not ships.has(player_ship_id):
		return
	var player_ship := ships[player_ship_id] as Node3D
	if not player_ship.has_method("server_position"):
		return
	var player_server: PackedFloat64Array = player_ship.call("server_position") as PackedFloat64Array
	if not _world.should_rebase_components(player_server[0], player_server[1], player_server[2]):
		return
	apply_origin_rebase_components(player_server, false, player_ship_id, ships)


func _update_sun_direction(player_ship_id: int, ships: Dictionary, bodies: Array) -> void:
	if _sky_mat == null or player_ship_id < 0 or not ships.has(player_ship_id) or _world == null:
		if _sun_flare != null:
			_sun_flare.enabled = false
		return
	var ship_node: Node3D = ships[player_ship_id] as Node3D
	var star := _find_star(bodies)
	var state: CelestialSunProjection = null
	if star != null:
		state = CelestialProjection.sun(_ship_server_position(ship_node), star.position, star.radius)
	if state == null:
		if _sun_flare != null:
			_sun_flare.enabled = false
		_sky_mat.set_shader_parameter("sun_active", 0.0)
		if _directional_light != null:
			_directional_light.light_energy = 0.0
		return
	var sun_direction := state.direction
	_sky_mat.set_shader_parameter("sun_direction", sun_direction)
	_sky_mat.set_shader_parameter("sun_active", 1.0)
	var color := NavigationMarkerRendererScript.spectral_color(star.spectral_type)
	var sun_color := Vector3(color.r, color.g, color.b)
	_sky_mat.set_shader_parameter("sun_color", sun_color)
	_sky_mat.set_shader_parameter(
		"sun_angular_radius",
		maxf(state.angular_radius, SUN_MIN_RENDER_ANGULAR_RADIUS))
	_update_sun_flare(sun_direction, sun_color, state.angular_radius)
	if _directional_light != null:
		var light_ray := star_light_ray(sun_direction)
		_directional_light.visible = true
		_directional_light.light_color = Color(sun_color.x, sun_color.y, sun_color.z)
		_directional_light.light_energy = 1.8
		var target := ship_node.global_position
		_directional_light.global_position = target - light_ray * 1_000.0
		_directional_light.look_at(target, light_up_axis(light_ray))


func _setup_space_environment(parent: Node) -> void:
	var shader := load("res://shaders/space_sky.gdshader") as Shader
	if shader == null:
		push_warning("[WorldPresentation] space_sky.gdshader not found")
		return

	var sky_mat := ShaderMaterial.new()
	sky_mat.shader = shader
	_sky_mat = sky_mat
	sky_mat.set_shader_parameter("nebula_strength", 0.18)
	sky_mat.set_shader_parameter("milkyway_strength", 0.11)
	sky_mat.set_shader_parameter("milkyway_core_color", Color(0.88, 0.80, 0.66))
	sky_mat.set_shader_parameter("milkyway_outer_color", Color(0.62, 0.68, 0.78))
	sky_mat.set_shader_parameter("nebula_primary_color", Color(0.80, 0.12, 0.18))
	sky_mat.set_shader_parameter("nebula_secondary_color", Color(0.32, 0.38, 0.48))
	sky_mat.set_shader_parameter("nebula_highlight_color", Color(0.28, 0.48, 0.52))
	sky_mat.set_shader_parameter("ambient_color", Color(0.0003, 0.0004, 0.0006))

	var sky := Sky.new()
	_sky = sky
	sky.sky_material = sky_mat
	sky.process_mode = Sky.PROCESS_MODE_REALTIME
	sky.radiance_size = Sky.RADIANCE_SIZE_256

	var env := Environment.new()
	env.background_mode = Environment.BG_SKY
	env.sky = sky
	env.ambient_light_source = Environment.AMBIENT_SOURCE_SKY
	env.ambient_light_energy = 0.03
	env.tonemap_mode = Environment.TONE_MAPPER_FILMIC
	env.tonemap_exposure = 1.1
	env.tonemap_white = 6.0
	env.glow_enabled = true
	env.glow_normalized = false
	env.glow_intensity = 0.9
	env.glow_bloom = 0.14
	env.glow_blend_mode = Environment.GLOW_BLEND_MODE_SOFTLIGHT
	env.glow_hdr_threshold = 1.8
	env.glow_hdr_scale = 1.0

	## Depth fog whose colour comes mostly from the sky, so the haze on a distant
	## object matches the nebula behind it. This is what EVE builds by hand in
	## Tr2PPFogEffect (nebulaInfluence 0.5 over three distance bands); Godot
	## exposes the same idea as fog_aerial_perspective over one curve.
	var scale: float = _render_scale()
	env.fog_enabled = true
	env.fog_mode = Environment.FOG_MODE_DEPTH
	env.fog_depth_begin = FOG_BEGIN_METRES * scale
	env.fog_depth_end = FOG_END_METRES * scale
	## Below 1.0 the haze builds early, matching EVE's mid band carrying a third
	## of the blend well before the far distance.
	env.fog_depth_curve = 0.6
	env.fog_aerial_perspective = 0.8
	env.fog_light_color = Color(0.42, 0.20, 0.12)
	env.fog_light_energy = 1.0
	## Measured against a test sphere held at constant angular size: 0.35 shifts
	## it by only 8% at 120 km, which does not read as distance at all. 0.7
	## shifts it about 27% and carries the background's warm tint with it.
	env.fog_density = 0.7
	env.fog_sun_scatter = 0.0
	## The sky already IS the background. Fogging it would wash the nebula into
	## itself and flatten the very depth this is meant to create.
	env.fog_sky_affect = 0.0

	var world_env := WorldEnvironment.new()
	world_env.environment = env
	parent.add_child(world_env)
	# Compositors require a RenderingDevice (Forward+ or Mobile), not Compatibility.
	if RenderingServer.get_rendering_device() != null:
		_sun_flare = load("res://addons/lens_effects/lens_flare_compositor_effect.gd").new()
		_sun_flare.enabled = false
		_sun_flare.Effect_Multiplier = 0.65
		_sun_flare.Anamorphic_Intensity = 16.0
		_sun_flare.Anamorphic_Stretch = 0.44
		_sun_flare.Anamorphic_Brightness = 0.65
		# Local camera optics, not atmospheric scattering throughout the scene.
		_sun_flare.Weight = 0.045
		_sun_flare.SampleCount = 24
		_sun_flare.Density = 0.98
		_sun_flare.Decay = 0.96
		var compositor := Compositor.new()
		compositor.compositor_effects = [_sun_flare]
		world_env.compositor = compositor

	_setup_starfield(parent)


func _update_sun_flare(direction: Vector3, color: Vector3, angular_radius: float) -> void:
	if _sun_flare == null:
		return
	if _camera == null:
		_sun_flare.enabled = false
		return
	var source := project_sun_flare(_camera, direction, angular_radius)
	if source.is_empty():
		_sun_flare.enabled = false
		return
	_sun_flare.set_sun(source.position, Color(color.x, color.y, color.z, 1.0),
		source.visibility, source.radius)
	_sun_flare.enabled = true


## Project a direction near the camera, never a narrowed absolute star position.
static func project_sun_flare(camera: Camera3D, direction: Vector3, angular_radius: float) -> Dictionary:
	if direction.is_zero_approx() or angular_radius <= 0.0:
		return {}
	var forward := -camera.global_basis.z.normalized()
	var ray := direction.normalized()
	if forward.dot(ray) <= 0.001:
		return {}
	var size := camera.get_viewport().get_visible_rect().size
	if size.x <= 0.0 or size.y <= 0.0:
		return {}
	var distance := maxf(camera.near * 2.0, 1.0)
	var point := camera.global_position + ray * distance
	var pixel := camera.unproject_position(point)
	var uv := pixel / size
	if uv.x <= 0.0 or uv.x >= 1.0 or uv.y <= 0.0 or uv.y >= 1.0:
		return {}
	var edge := minf(minf(uv.x, 1.0 - uv.x), minf(uv.y, 1.0 - uv.y))
	var tangent := camera.global_basis.y.slide(ray).normalized()
	var limb := point + tangent * distance * tan(angular_radius)
	var radius := camera.unproject_position(limb).distance_to(pixel)
	return {"position": uv, "visibility": smoothstep(0.0, 0.04, edge),
		"radius": clampf(radius * 0.7, 1.0, 64.0)}


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


## Builds the ADR-0054 star layer: the named catalogue plus the seeded
## statistical field, packed into one MultiMesh of billboard sprites.
func _setup_starfield(parent: Node) -> void:
	var shader := load("res://shaders/star_sprite.gdshader") as Shader
	if shader == null:
		push_warning("[WorldPresentation] star_sprite.gdshader not found")
		return

	var stars: Array[Dictionary] = StarfieldScript.catalog_stars()
	stars.append_array(StarfieldScript.generate(
		StarfieldScript.DEFAULT_SEED, StarfieldScript.DEFAULT_COUNT))

	var material := ShaderMaterial.new()
	material.shader = shader
	material.set_shader_parameter(
		"point_spread_pixels", StarfieldScript.POINT_SPREAD_PIXELS)
	material.set_shader_parameter(
		"core_sharpness", StarfieldScript.POINT_SHARPNESS)
	_starfield_mat = material

	var instance := MultiMeshInstance3D.new()
	instance.multimesh = StarfieldScript.build_multimesh(stars)
	instance.material_override = material
	instance.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	## The shell surrounds the camera and so is never off screen. Stating the
	## bound explicitly keeps the renderer from culling it against the instance
	## bounds it would otherwise derive.
	instance.custom_aabb = AABB(
		Vector3.ONE * -StarfieldScript.SHELL_RADIUS,
		Vector3.ONE * (StarfieldScript.SHELL_RADIUS * 2.0))

	## Stars sit at infinity: the root follows the camera's translation but never
	## its rotation, so the field cannot parallax. Dawn's ships cross AU, and any
	## finite star distance would streak past them (ADR-0054 §2).
	_starfield_root = Node3D.new()
	_starfield_root.name = "Starfield"
	_starfield_root.add_child(instance)
	parent.add_child(_starfield_root)


## Keeps the star shell centred on the camera so the field reads as infinitely
## distant, and keeps the sprite shader's pixel footprint honest across
## window resizes. Called every frame from refresh().
func _update_starfield() -> void:
	if _starfield_root == null or _camera == null:
		return
	_starfield_root.global_position = _camera.global_position
	if _starfield_mat == null:
		return
	var viewport: Viewport = _camera.get_viewport()
	if viewport != null:
		_starfield_mat.set_shader_parameter(
			"viewport_pixels", viewport.get_visible_rect().size)


## Generates the nebula and Milky Way once into an equirectangular panorama and
## switches the sky over to sampling it (ADR-0055). They are direction-only
## functions, so one texture fetch replaces roughly 280 hash evaluations per
## pixel. Godot bakes the same Sky the environment already uses, which also
## feeds ambient light and reflections, so the background, the ambient and the
## ship reflections keep agreeing with each other.
##
## Runs in two steps because a uniform written this frame is not yet visible to
## the bake. Arming `bake_pass` and baking in one call captures the presentation
## vignette into the texture, and the runtime then applies it a second time.
func _maybe_bake_nebula() -> void:
	if _nebula_bake_done or _sky == null or _sky_mat == null:
		return
	_nebula_bake_frames += 1

	if _nebula_bake_frames == NEBULA_BAKE_ARM_FRAME:
		## The local star moves with the ship. Baking it would paint it onto the
		## background permanently, so if it is already lit, stay procedural.
		var sun_active: Variant = _sky_mat.get_shader_parameter("sun_active")
		if sun_active != null and (sun_active as float) > 0.0:
			_nebula_bake_done = true
			push_warning(
				"[WorldPresentation] star already lit; keeping the procedural sky")
			return
		_sky_mat.set_shader_parameter("bake_pass", true)
		return

	if _nebula_bake_frames < NEBULA_BAKE_FRAME:
		return

	_nebula_bake_done = true
	var panorama: Image = RenderingServer.sky_bake_panorama(
		_sky.get_rid(), 1.0, false, NEBULA_PANORAMA_SIZE)
	_sky_mat.set_shader_parameter("bake_pass", false)

	## Headless and dummy renderers return nothing here. That is not an error:
	## the procedural path stays in the shader precisely so this can fail.
	if panorama == null or panorama.is_empty():
		return
	_sky_mat.set_shader_parameter(
		"nebula_panorama", ImageTexture.create_from_image(panorama))
	_sky_mat.set_shader_parameter("use_baked_nebula", true)
