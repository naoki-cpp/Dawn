extends Node3D
## Tactical overlay — EVE-style range rings drawn in the XZ plane.
##
## Attach to a Node3D child of the player ship node (or update position each frame).
## Call set_ranges() whenever fitting changes.

const SEGMENTS := 128

var _optimal_range  : float = 0.0
var _falloff_range  : float = 0.0   ## full radius = optimal + falloff
var _visible_overlay: bool  = true

## Mesh instances for the two rings.
var _mesh_optimal : MeshInstance3D
var _mesh_falloff : MeshInstance3D


func _ready() -> void:
	_mesh_optimal = _make_ring_mesh_instance()
	_mesh_falloff = _make_ring_mesh_instance()
	add_child(_mesh_optimal)
	add_child(_mesh_falloff)
	_redraw()


func set_ranges(optimal: float, falloff: float) -> void:
	_optimal_range = optimal
	_falloff_range = falloff
	_redraw()


func set_overlay_visible(v: bool) -> void:
	_visible_overlay = v
	visible = v


func toggle_visible() -> void:
	set_overlay_visible(not _visible_overlay)


# ── private ──────────────────────────────────────────────────────────────────

func _make_ring_mesh_instance() -> MeshInstance3D:
	var mi := MeshInstance3D.new()
	mi.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	return mi


func _redraw() -> void:
	_draw_ring(_mesh_optimal, _optimal_range,
		Color(0.2, 0.9, 0.2, 0.7))   ## green — within optimal: no range penalty
	_draw_ring(_mesh_falloff, _optimal_range + _falloff_range,
		Color(0.9, 0.5, 0.1, 0.5))   ## orange — at this radius hit_chance = 0.5


func _draw_ring(mi: MeshInstance3D, radius: float, color: Color) -> void:
	if radius <= 0.0:
		mi.mesh = null
		return

	var im := ImmediateMesh.new()
	im.surface_begin(Mesh.PRIMITIVE_LINE_STRIP)
	im.surface_set_color(color)

	for i in range(SEGMENTS + 1):
		var angle := (float(i) / float(SEGMENTS)) * TAU
		im.surface_add_vertex(Vector3(cos(angle) * radius, 0.0, sin(angle) * radius))

	im.surface_end()

	mi.mesh = im

	## Unshaded material so the ring is always visible regardless of lighting.
	var mat := StandardMaterial3D.new()
	mat.shading_mode      = BaseMaterial3D.SHADING_MODE_UNSHADED
	mat.vertex_color_use_as_albedo = true
	mat.transparency      = BaseMaterial3D.TRANSPARENCY_ALPHA
	mat.albedo_color      = color
	mat.flags_no_depth_test = false
	mi.material_override  = mat
