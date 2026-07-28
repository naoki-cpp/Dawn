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

const PositionComponents = preload("res://scripts/position_components.gd")

const GATE_RING_INNER_RATIO    : float = 0.85
const GATE_LABEL_HEIGHT_RATIO  : float = 0.3
const PLANET_VISUAL_RADIUS_RATIO: float = 0.5
const STATION_VISUAL_RADIUS     : float = 350.0
const STATION_RING_INNER_RATIO  : float = 0.92
const STATION_LABEL_HEIGHT_RATIO: float = 1.8

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


## Frees every child of `root`. Shared by gate/body marker respawning, which
## both rebuild their marker set from scratch on each call.
static func clear_children(root: Node) -> void:
	for child: Node in root.get_children():
		child.queue_free()


## Returns a linear-light RGB colour for a blackbody spectral type [0..1].
## Mirrors spectral_color() in space_sky.gdshader. Used by both star marker
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


## Spawns a visual marker for every Jump Gate in the player's current Star
## System (ADR-0009). Re-run on Star System change to swap markers.
## `to_godot_components` converts an absolute f64 position into Godot world
## space after subtracting the floating origin (main.gd's component adapter).
static func spawn_gate_markers(gates_root: Node3D, gates: Array, world_scale: float, to_godot_components: Callable) -> void:
	clear_children(gates_root)

	for entry: Variant in gates:
		var g: GateRecord = entry as GateRecord
		var gate_pos := PositionComponents.from_value(g.position)
		var radius  : float   = g.activation_radius

		var marker: Node3D = Node3D.new()
		marker.position = to_godot_components.call(gate_pos) as Vector3
		marker.set_meta("gate_id",  g.gate_id)
		marker.set_meta("gate_pos", gate_pos)  ## server coords, kept for per-frame clamping (main.gd)
		marker.set_meta("nav_pos", gate_pos)
		gates_root.add_child(marker)

		var ring: MeshInstance3D = MeshInstance3D.new()
		var torus: TorusMesh = TorusMesh.new()
		torus.inner_radius = radius * world_scale * GATE_RING_INNER_RATIO
		torus.outer_radius = radius * world_scale
		var mat: StandardMaterial3D = StandardMaterial3D.new()
		mat.albedo_color    = Color(0.2, 0.8, 1.0, 0.35)
		mat.emission_enabled = true
		mat.emission        = Color(0.2, 0.8, 1.0)
		mat.emission_energy_multiplier = 1.5
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
		label.modulate         = Color(0.2, 0.8, 1.0)
		marker.add_child(label)


## Spawn visual nodes for celestial bodies in the current star system
## (planets only, ADR-0025 §5 superseded for stars -- see note below).
## Re-called on system change. `to_godot_components` converts an absolute f64
## position into Godot world space after origin subtraction.
##
## Stars get no marker/mesh here: the sky shader (space_sky.gdshader) already
## draws the local star as a direction-based disc/corona/glow (main.gd's
## _update_sun_direction), which is correct for something effectively at
## infinite distance for skybox purposes. Layering a *finite-distance* 3D
## mesh sphere on top of that direction-only disc caused a visible mismatch:
## the mesh has real parallax as the ship moves/orbits, the skybox disc does
## not, so the two drifted apart depending on viewing angle. Keeping only the
## skybox representation removes the duplicate and the seam, at the cost of
## the star no longer being a clickable warp target (planets are unaffected).
static func spawn_body_markers(bodies_root: Node3D, bodies: Array, world_scale: float, to_godot_components: Callable) -> void:
	clear_children(bodies_root)

	for entry: Variant in bodies:
		var b: CelestialBodyRecord = entry as CelestialBodyRecord
		if b.kind == "Star":
			continue

		var b_id    : int     = b.body_id
		var kind    : String  = b.kind
		var name_str: String  = b.name
		var b_pos   := PositionComponents.from_value(b.position)
		var radius  : float   = b.radius

		var godot_pos: Vector3 = to_godot_components.call(b_pos) as Vector3

		var marker: Node3D = Node3D.new()
		marker.position = godot_pos
		marker.set_meta("body_id",   b_id)
		marker.set_meta("body_kind", kind)
		marker.set_meta("body_pos",  b_pos)  ## server coords, kept for sun direction
		marker.set_meta("nav_pos",   b_pos)
		bodies_root.add_child(marker)

		## Visual sphere. Planets: solid matte sphere, visual radius = 8% of
		## logical radius.
		var mesh_inst: MeshInstance3D = MeshInstance3D.new()
		var sphere: SphereMesh = SphereMesh.new()
		sphere.radius = radius * world_scale * PLANET_VISUAL_RADIUS_RATIO
		sphere.height = sphere.radius * 2.0
		var mat: StandardMaterial3D = StandardMaterial3D.new()
		mat.albedo_color = Color(0.45, 0.50, 0.60)
		mat.roughness    = 0.85
		mat.metallic     = 0.1
		mesh_inst.material_override = mat
		mesh_inst.mesh = sphere
		marker.add_child(mesh_inst)

		## Name label.
		var label: Label3D = Label3D.new()
		label.text        = name_str
		label.position    = Vector3(0.0, sphere.radius * 1.4, 0.0)
		label.billboard   = BaseMaterial3D.BILLBOARD_ENABLED
		label.no_depth_test = true
		label.modulate    = Color(0.7, 0.8, 1.0)
		marker.add_child(label)

		## Selection reticle: always the same screen size, so the planet stays
		## easy to click regardless of distance (pairs with
		## ShipPicking.pick_body_at's screen-space picking).
		marker.add_child(BillboardRing.build(RETICLE_COLOR, RETICLE_PIXEL_SIZE))


## Appends visual markers for NPC stations in the current star system.
## Stations share the bodies root because they live in the same local spatial
## context as planets and should clamp the same way at true AU distances.
static func spawn_station_markers(bodies_root: Node3D, stations: Array, world_scale: float, to_godot_components: Callable) -> void:
	for entry: Variant in stations:
		var station: StationRecord = entry as StationRecord
		var station_id: int = station.station_id
		var name_str: String = station.name
		var station_pos := PositionComponents.from_value(station.position)
		var docking_radius: float = station.docking_radius
		var visual_radius: float = STATION_VISUAL_RADIUS * world_scale

		var marker: Node3D = Node3D.new()
		marker.position = to_godot_components.call(station_pos) as Vector3
		marker.set_meta("station_id", station_id)
		marker.set_meta("station_pos", station_pos)
		marker.set_meta("nav_pos", station_pos)
		bodies_root.add_child(marker)

		var mesh_inst: MeshInstance3D = MeshInstance3D.new()
		var sphere: SphereMesh = SphereMesh.new()
		sphere.radius = visual_radius
		sphere.height = visual_radius * 2.0
		var station_mat: StandardMaterial3D = StandardMaterial3D.new()
		station_mat.albedo_color = Color(0.85, 0.72, 0.30)
		station_mat.emission_enabled = true
		station_mat.emission = Color(0.85, 0.72, 0.30)
		station_mat.emission_energy_multiplier = 1.0
		station_mat.roughness = 0.35
		mesh_inst.material_override = station_mat
		mesh_inst.mesh = sphere
		marker.add_child(mesh_inst)

		var ring: MeshInstance3D = MeshInstance3D.new()
		var torus: TorusMesh = TorusMesh.new()
		torus.inner_radius = docking_radius * world_scale * STATION_RING_INNER_RATIO
		torus.outer_radius = docking_radius * world_scale
		var ring_mat: StandardMaterial3D = StandardMaterial3D.new()
		ring_mat.albedo_color = Color(0.95, 0.78, 0.22, 0.30)
		ring_mat.emission_enabled = true
		ring_mat.emission = Color(0.95, 0.78, 0.22)
		ring_mat.emission_energy_multiplier = 1.2
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
		label.modulate = Color(0.95, 0.82, 0.32)
		marker.add_child(label)
