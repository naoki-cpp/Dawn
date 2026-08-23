## navigation_marker_renderer.gd
##
## Builds the visual markers for Jump Gates and celestial bodies, extracted
## from main.gd (architecture-review/client.md C-1, second slice after
## ShipPicking). Stateless static helpers -- callers supply the scene root
## to populate and the live navigation data; main.gd keeps ownership of
## that data and any state that isn't purely about rendering (e.g.
## _selected_body_id).
class_name NavigationMarkerRenderer
extends RefCounted

const GATE_RING_INNER_RATIO    : float = 0.93
const GATE_LABEL_HEIGHT_RATIO  : float = 0.3
const STATION_VISUAL_RADIUS     : float = 350.0
const STATION_RING_INNER_RATIO  : float = 0.96
const STATION_LABEL_HEIGHT_RATIO: float = 1.8
const GATE_COLOR: Color = Color(0.18, 0.86, 1.0, 0.88)
const PLANET_COLOR: Color = Color(0.62, 0.84, 1.0, 0.82)
const STATION_COLOR: Color = Color(1.0, 0.72, 0.24, 0.90)
## SpriteBase3D.pixel_size is the 3D width of one texture pixel, not a
## screen-pixel count. With the 128px bracket texture this gives a 0.0256
## world-unit base width before fixed-size projection, about one tenth of the
## previous 0.256 width.
const BRACKET_PIXEL_SIZE: float = 0.0002

## Selection reticle: a fixed-screen-size ring billboard so every planet is
## equally easy to click regardless of distance (pairs with
## ShipPicking.pick_body_at's screen-space picking). Built via
## BillboardRing, shared with ship_controller.gd's lock-on ring -- both are
## the same kind of "this is selected/selectable" indicator and should
## behave the same way. Sized smaller than the smallest existing HUD elements
## (hud_manager.gd's conn_dot is 8x8px, module-slot font is 9px) since this
## marker sits in 3D space rather than screen space and otherwise reads as
## too large against the ship/planet geometry around it.
const RETICLE_PIXEL_SIZE : float = 0.0015
const RETICLE_COLOR      : Color = Color(1.0, 1.0, 1.0, 0.85)
const BRACKET_SCRIPT := preload("res://scripts/billboard_bracket.gd")
const PLANET_SHADER := preload("res://shaders/planet_surface.gdshader")


## Frees every child of `root`. Shared by gate/body marker respawning, which
## both rebuild their marker set from scratch on each call.
static func clear_children(root: Node) -> void:
	for child: Node in root.get_children():
		child.queue_free()


## Returns a linear-light RGB colour for a blackbody spectral type [0..1].
## The single owner of the spectral-type colour table (Ballesteros 2012).
## ADR-0054 removed the shader's duplicate. Used by star marker
## materials and main.gd's sun-direction shader update.
static func spectral_color(t: float) -> Color:
	var r: float; var g: float; var b: float
	if t < 0.10:
		r = lerp(0.55, 0.65, t / 0.10);        g = lerp(0.65, 0.76, t / 0.10);        b = 1.00
	elif t < 0.25:
		r = lerp(0.65, 0.88, (t-0.10)/0.15);   g = lerp(0.76, 0.93, (t-0.10)/0.15);   b = 1.00
	elif t < 0.40:
		r = lerp(0.88, 1.00, (t-0.25)/0.15);   g = lerp(0.93, 0.99, (t-0.25)/0.15);   b = lerp(1.00, 0.94, (t-0.25)/0.15)
	elif t < 0.55:
		r = 1.00;                               g = lerp(0.99, 0.95, (t-0.40)/0.15);   b = lerp(0.94, 0.82, (t-0.40)/0.15)
	elif t < 0.68:
		r = 1.00;                               g = lerp(0.95, 0.85, (t-0.55)/0.13);   b = lerp(0.82, 0.58, (t-0.55)/0.13)
	elif t < 0.83:
		r = 1.00;                               g = lerp(0.85, 0.64, (t-0.68)/0.15);   b = lerp(0.58, 0.32, (t-0.68)/0.15)
	else:
		r = 1.00;                               g = lerp(0.64, 0.40, (t-0.83)/0.17);   b = lerp(0.32, 0.18, (t-0.83)/0.17)
	return Color(r, g, b)


## Deterministic presentation profile. The server remains authoritative for
## body kind, radius, and position; these values only describe how a generic
## planet surface is rendered when no texture asset is available.
static func planet_profile(body_id: int, body_name: String) -> Dictionary:
	var profiles := {
		"Forge": {"base": Color(0.48, 0.28, 0.16), "accent": Color(0.12, 0.07, 0.04), "ice": Color(0.86, 0.84, 0.72), "ocean": 0.52, "ice_amount": 0.08},
		"Meridian": {"base": Color(0.10, 0.30, 0.48), "accent": Color(0.03, 0.08, 0.16), "ice": Color(0.74, 0.88, 0.98), "ocean": 0.40, "ice_amount": 0.22},
		"Haven": {"base": Color(0.48, 0.22, 0.12), "accent": Color(0.16, 0.05, 0.025), "ice": Color(0.92, 0.78, 0.60), "ocean": 0.66, "ice_amount": 0.04},
		"Bastion": {"base": Color(0.30, 0.34, 0.38), "accent": Color(0.07, 0.08, 0.10), "ice": Color(0.72, 0.80, 0.88), "ocean": 0.78, "ice_amount": 0.10},
	}
	var profile: Dictionary = profiles.get(body_name, {})
	if profile.is_empty():
		var hue := fmod(float(abs(body_id) * 37 % 360) / 360.0, 1.0)
		profile = {
			"base": Color.from_hsv(hue, 0.42, 0.62),
			"accent": Color.from_hsv(hue, 0.58, 0.20),
			"ice": Color(0.78, 0.86, 0.94),
			"ocean": 0.50,
			"ice_amount": 0.12,
		}
	profile["seed"] = float(abs(body_id) + 1) * 0.731
	return profile


static func planet_material(body_id: int, body_name: String) -> ShaderMaterial:
	var profile := planet_profile(body_id, body_name)
	var material := ShaderMaterial.new()
	material.shader = PLANET_SHADER
	material.set_shader_parameter("base_color", profile.base)
	material.set_shader_parameter("accent_color", profile.accent)
	material.set_shader_parameter("ice_color", profile.ice)
	material.set_shader_parameter("ocean_level", profile.ocean)
	material.set_shader_parameter("ice_amount", profile.ice_amount)
	material.set_shader_parameter("seed", profile.seed)
	return material


## Spawns a visual marker for every Jump Gate in the player's current Star
## System (ADR-0009). Re-run on Star System change to swap markers.
## `to_godot_components` converts an absolute f64 position into Godot world
## space after subtracting the floating origin (main.gd's component adapter).
static func spawn_gate_markers(gates_root: Node3D, gates: Array, world_scale: float, to_godot_components: Callable) -> void:
	clear_children(gates_root)

	for entry: Variant in gates:
		var g: GateRecord = entry as GateRecord
		var gate_pos: PackedFloat64Array = g.position
		var radius  : float   = g.activation_radius

		var marker: Node3D = Node3D.new()
		marker.position = to_godot_components.call(gate_pos[0], gate_pos[1], gate_pos[2]) as Vector3
		marker.set_meta("gate_id",  g.gate_id)
		marker.set_meta("gate_pos", gate_pos)  ## server coords, kept for per-frame clamping (main.gd)
		marker.set_meta("nav_pos", gate_pos)
		marker.set_meta("nav_pick_radius_px", BRACKET_SCRIPT.PICK_RADIUS_PX)
		gates_root.add_child(marker)

		var ring: MeshInstance3D = MeshInstance3D.new()
		var torus: TorusMesh = TorusMesh.new()
		torus.inner_radius = radius * world_scale * GATE_RING_INNER_RATIO
		torus.outer_radius = radius * world_scale
		var mat: StandardMaterial3D = StandardMaterial3D.new()
		mat.albedo_color    = GATE_COLOR
		mat.emission_enabled = true
		mat.emission        = GATE_COLOR
		mat.emission_energy_multiplier = 1.2
		mat.transparency    = BaseMaterial3D.TRANSPARENCY_ALPHA
		mat.shading_mode    = BaseMaterial3D.SHADING_MODE_UNSHADED
		ring.mesh     = torus
		ring.material_override = mat
		ring.rotation_degrees = Vector3(90.0, 0.0, 0.0)
		marker.add_child(ring)

		var label: Label3D = Label3D.new()
		label.text             = "Gate #%d -> %s" % [g.gate_id, g.to_system_name]
		label.position         = Vector3(0.0, radius * world_scale * GATE_LABEL_HEIGHT_RATIO, 0.0)
		label.billboard        = BaseMaterial3D.BILLBOARD_ENABLED
		label.no_depth_test    = true
		label.modulate         = GATE_COLOR
		label.outline_size     = 8
		label.outline_modulate = Color(0.01, 0.03, 0.05, 0.92)
		marker.add_child(label)
		marker.add_child(BRACKET_SCRIPT.build(GATE_COLOR, BRACKET_PIXEL_SIZE))


## Spawn visual nodes for celestial bodies in the current star system
## (planets only, ADR-0025 §5 superseded for stars -- see note below).
## Re-called on system change. `to_godot_components` converts an absolute f64
## position into Godot world space after origin subtraction.
##
## Stars get no marker/mesh here: the sky shader (space_sky.gdshader) already
## draws the local star as a direction-based disc/corona/glow. WorldPresentation
## updates that direction from the ship's continuous world-presentation position so it
## keeps celestial parallax during warp. A finite-distance mesh duplicated the
## star with a separate projection and created a visible seam. Keeping one sky
## representation removes that mismatch, at the cost of the star no longer
## being a clickable warp target (planets are unaffected).
static func spawn_body_markers(bodies_root: Node3D, bodies: Array, world_scale: float, to_godot_components: Callable) -> void:
	clear_children(bodies_root)

	for entry: Variant in bodies:
		var b: CelestialBodyRecord = entry as CelestialBodyRecord
		if b.kind == "Star":
			continue

		var b_id    : int     = b.body_id
		var kind    : String  = b.kind
		var name_str: String  = b.name
		var b_pos: PackedFloat64Array = b.position
		var radius  : float   = b.radius

		var godot_pos: Vector3 = to_godot_components.call(b_pos[0], b_pos[1], b_pos[2]) as Vector3

		var marker: Node3D = Node3D.new()
		marker.position = godot_pos
		marker.set_meta("body_id",   b_id)
		marker.set_meta("body_kind", kind)
		marker.set_meta("body_pos",  b_pos)  ## server coords, kept for sun direction
		marker.set_meta("nav_pos",   b_pos)
		marker.set_meta("preserve_physical_position", true)
		marker.set_meta("physical_extent", radius)
		marker.set_meta("nav_pick_radius_px", BRACKET_SCRIPT.PICK_RADIUS_PX)
		bodies_root.add_child(marker)

		## Visual sphere. Physical metres are converted once through WorldSpace;
		## the renderer does not apply a second gameplay or camera-size policy.
		var mesh_inst: MeshInstance3D = MeshInstance3D.new()
		var sphere: SphereMesh = SphereMesh.new()
		sphere.radius = radius * world_scale
		sphere.height = sphere.radius * 2.0
		mesh_inst.material_override = planet_material(b_id, name_str)
		mesh_inst.mesh = sphere
		marker.add_child(mesh_inst)
		marker.set_meta("physical_body_mesh", mesh_inst)

		## Name label.
		var label: Label3D = Label3D.new()
		label.text        = name_str
		label.position    = Vector3(0.0, sphere.radius * 1.4, 0.0)
		label.billboard   = BaseMaterial3D.BILLBOARD_ENABLED
		label.no_depth_test = true
		label.modulate    = PLANET_COLOR
		label.outline_size = 8
		label.outline_modulate = Color(0.01, 0.02, 0.04, 0.90)
		marker.add_child(label)
		marker.set_meta("body_label", label)

		## Selection reticle: always the same screen size, so the planet stays
		## easy to click regardless of distance (pairs with
		## ShipPicking.pick_body_at's screen-space picking).
		marker.add_child(BillboardRing.build(RETICLE_COLOR, RETICLE_PIXEL_SIZE))
		marker.add_child(BRACKET_SCRIPT.build(PLANET_COLOR, BRACKET_PIXEL_SIZE))


## Appends visual markers for NPC stations in the current star system.
## Stations share the bodies root because they live in the same local spatial
## context as planets. Their positions and docking rings remain in physical
## coordinates; only gate markers use the camera-relative distance policy.
static func spawn_station_markers(bodies_root: Node3D, stations: Array, world_scale: float, to_godot_components: Callable) -> void:
	for entry: Variant in stations:
		var station: StationRecord = entry as StationRecord
		var station_id: int = station.station_id
		var name_str: String = station.name
		var station_pos: PackedFloat64Array = station.position
		var docking_radius: float = station.docking_radius
		var visual_radius: float = STATION_VISUAL_RADIUS * world_scale

		var marker: Node3D = Node3D.new()
		marker.position = to_godot_components.call(station_pos[0], station_pos[1], station_pos[2]) as Vector3
		marker.set_meta("station_id", station_id)
		marker.set_meta("station_pos", station_pos)
		marker.set_meta("nav_pos", station_pos)
		marker.set_meta("preserve_physical_position", true)
		marker.set_meta("physical_extent", docking_radius)
		marker.set_meta("nav_pick_radius_px", BRACKET_SCRIPT.PICK_RADIUS_PX)
		bodies_root.add_child(marker)

		var mesh_inst: MeshInstance3D = MeshInstance3D.new()
		var sphere: SphereMesh = SphereMesh.new()
		sphere.radius = visual_radius
		sphere.height = visual_radius * 2.0
		var station_mat: StandardMaterial3D = StandardMaterial3D.new()
		station_mat.albedo_color = Color(0.14, 0.11, 0.06)
		station_mat.emission_enabled = true
		station_mat.emission = STATION_COLOR
		station_mat.emission_energy_multiplier = 0.65
		station_mat.roughness = 0.48
		mesh_inst.material_override = station_mat
		mesh_inst.mesh = sphere
		marker.add_child(mesh_inst)

		var ring: MeshInstance3D = MeshInstance3D.new()
		var torus: TorusMesh = TorusMesh.new()
		torus.inner_radius = docking_radius * world_scale * STATION_RING_INNER_RATIO
		torus.outer_radius = docking_radius * world_scale
		var ring_mat: StandardMaterial3D = StandardMaterial3D.new()
		ring_mat.albedo_color = STATION_COLOR
		ring_mat.emission_enabled = true
		ring_mat.emission = STATION_COLOR
		ring_mat.emission_energy_multiplier = 0.9
		ring_mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
		ring_mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
		ring.mesh = torus
		ring.material_override = ring_mat
		ring.rotation_degrees = Vector3(90.0, 0.0, 0.0)
		marker.add_child(ring)

		var label: Label3D = Label3D.new()
		label.text = name_str
		label.position = Vector3(0.0, visual_radius * STATION_LABEL_HEIGHT_RATIO, 0.0)
		label.billboard = BaseMaterial3D.BILLBOARD_ENABLED
		label.no_depth_test = true
		label.modulate = STATION_COLOR
		label.outline_size = 8
		label.outline_modulate = Color(0.06, 0.035, 0.01, 0.92)
		marker.add_child(label)
		marker.add_child(BRACKET_SCRIPT.build(STATION_COLOR, BRACKET_PIXEL_SIZE))
