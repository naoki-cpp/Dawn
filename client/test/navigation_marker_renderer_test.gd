## navigation_marker_renderer_test.gd
##
## Unit tests for navigation_marker_renderer.gd's spectral colour and gate/
## body marker construction. Marker tests check structure (child count,
## local position, meta tags, label text) rather than global_position, so
## they don't need the markers added to a live scene tree.
extends GdUnitTestSuite

const __source: String = "res://scripts/navigation_marker_renderer.gd"


## Mirrors main.gd's f64 component adapter at world_scale=0.1.
func _to_godot_pos(p: PackedFloat64Array) -> Vector3:
	return Vector3(p[0], p[1], -p[2]) * 0.1

func _position(x: float, y: float, z: float) -> PackedFloat64Array:
	return PackedFloat64Array([x, y, z])


# -- spectral_color ---------------------------------------------------------------

func test_spectral_color_at_t_zero_is_coolest_blue() -> void:
	var c: Color = NavigationMarkerRenderer.spectral_color(0.0)
	assert_vector(Vector3(c.r, c.g, c.b)).is_equal_approx(Vector3(0.55, 0.65, 1.00), Vector3(0.0001, 0.0001, 0.0001))


func test_spectral_color_at_t_one_is_warmest_red() -> void:
	var c: Color = NavigationMarkerRenderer.spectral_color(1.0)
	assert_vector(Vector3(c.r, c.g, c.b)).is_equal_approx(Vector3(1.00, 0.40, 0.18), Vector3(0.0001, 0.0001, 0.0001))


func test_spectral_color_is_continuous_across_the_010_segment_boundary() -> void:
	var just_below: Color = NavigationMarkerRenderer.spectral_color(0.0999)
	var at_boundary: Color = NavigationMarkerRenderer.spectral_color(0.10)
	assert_vector(Vector3(just_below.r, just_below.g, just_below.b)) \
		.is_equal_approx(Vector3(at_boundary.r, at_boundary.g, at_boundary.b), Vector3(0.002, 0.002, 0.002))


# -- clear_children ---------------------------------------------------------------

func test_clear_children_removes_all_existing_children() -> void:
	var root: Node = auto_free(Node.new())
	root.add_child(Node.new())
	root.add_child(Node.new())

	NavigationMarkerRenderer.clear_children(root)
	await get_tree().process_frame  ## queue_free() is deferred to end of frame

	assert_int(root.get_child_count()).is_equal(0)


# -- spawn_gate_markers -------------------------------------------------------------

func test_spawn_gate_markers_builds_one_marker_per_gate_with_position_and_label() -> void:
	var gates_root: Node3D = auto_free(Node3D.new())
	var gates: Array = [
		{"gate_id": 5, "position": _position(10.0, 0.0, 20.0), "activation_radius": 2000.0, "to_system_name": "Beta"},
	]

	NavigationMarkerRenderer.spawn_gate_markers(gates_root, gates, 0.1, _to_godot_pos)

	assert_int(gates_root.get_child_count()).is_equal(1)
	var marker: Node3D = gates_root.get_child(0) as Node3D
	assert_vector(marker.position).is_equal_approx(Vector3(1.0, 0.0, -2.0), Vector3(0.0001, 0.0001, 0.0001))

	var label: Label3D = marker.get_child(1) as Label3D
	assert_str(label.text).is_equal("Gate #5 -> Beta")


func test_spawn_gate_markers_builds_one_marker_per_array_entry() -> void:
	var gates_root: Node3D = auto_free(Node3D.new())
	var gates: Array = [
		{"gate_id": 0, "position": _position(0.0, 0.0, 0.0), "activation_radius": 2000.0, "to_system_name": "Alpha"},
		{"gate_id": 1, "position": _position(100.0, 0.0, 0.0), "activation_radius": 1500.0, "to_system_name": "Gamma"},
	]

	NavigationMarkerRenderer.spawn_gate_markers(gates_root, gates, 0.1, _to_godot_pos)

	assert_int(gates_root.get_child_count()).is_equal(2)


# -- spawn_body_markers -------------------------------------------------------------

func test_spawn_body_markers_skips_stars_and_only_tags_planet_markers() -> void:
	## Stars get no marker: the sky shader draws the local star as a
	## direction-based disc (main.gd's _update_sun_direction), and layering a
	## finite-distance mesh on top caused a visible parallax mismatch as the
	## ship moved. See the doc comment on spawn_body_markers().
	var bodies_root: Node3D = auto_free(Node3D.new())
	var bodies: Array = [
		{"body_id": 1, "kind": "Star", "name": "Helios", "position": _position(0.0, 0.0, 0.0), "radius": 1000.0, "spectral_type": 0.5},
		{"body_id": 2, "kind": "Planet", "name": "Forge", "position": _position(500.0, 0.0, 0.0), "radius": 200.0, "spectral_type": 0.0},
	]

	NavigationMarkerRenderer.spawn_body_markers(bodies_root, bodies, 0.1, _to_godot_pos)

	assert_int(bodies_root.get_child_count()).is_equal(1)

	var planet_marker: Node3D = bodies_root.get_child(0) as Node3D
	assert_int(planet_marker.get_meta("body_id") as int).is_equal(2)
	assert_str(planet_marker.get_meta("body_kind") as String).is_equal("Planet")


func test_spawn_body_markers_uses_the_body_name_as_the_label_text() -> void:
	var bodies_root: Node3D = auto_free(Node3D.new())
	var bodies: Array = [
		{"body_id": 2, "kind": "Planet", "name": "Forge", "position": _position(500.0, 0.0, 0.0), "radius": 200.0, "spectral_type": 0.0},
	]

	NavigationMarkerRenderer.spawn_body_markers(bodies_root, bodies, 0.1, _to_godot_pos)

	var marker: Node3D = bodies_root.get_child(0) as Node3D
	var label: Label3D = marker.get_child(1) as Label3D  ## index 0 = mesh, index 1 = label
	assert_str(label.text).is_equal("Forge")


func test_spawn_body_markers_produces_no_markers_when_only_a_star_is_present() -> void:
	var bodies_root: Node3D = auto_free(Node3D.new())
	var bodies: Array = [
		{"body_id": 1, "kind": "Star", "name": "Helios", "position": _position(0.0, 0.0, 0.0), "radius": 1000.0, "spectral_type": 0.5},
	]

	NavigationMarkerRenderer.spawn_body_markers(bodies_root, bodies, 0.1, _to_godot_pos)

	assert_int(bodies_root.get_child_count()).is_equal(0)


func test_spawn_body_markers_adds_a_fixed_size_selection_reticle_to_each_planet() -> void:
	## Pairs with ShipPicking.pick_body_at's screen-space picking: a planet
	## should stay equally easy to click regardless of distance, which needs
	## a reticle that renders at a constant screen size (fixed_size).
	var bodies_root: Node3D = auto_free(Node3D.new())
	var bodies: Array = [
		{"body_id": 2, "kind": "Planet", "name": "Forge", "position": _position(500.0, 0.0, 0.0), "radius": 200.0, "spectral_type": 0.0},
	]

	NavigationMarkerRenderer.spawn_body_markers(bodies_root, bodies, 0.1, _to_godot_pos)

	var marker: Node3D = bodies_root.get_child(0) as Node3D
	var reticle: Sprite3D = marker.get_child(2) as Sprite3D  ## index 0=mesh, 1=label, 2=reticle
	assert_object(reticle).is_not_null()
	assert_bool(reticle.fixed_size).is_true()
	assert_object(reticle.texture).is_not_null()


# -- spawn_station_markers ----------------------------------------------------------

func test_spawn_station_markers_builds_one_marker_per_station_with_label_and_ring() -> void:
	var bodies_root: Node3D = auto_free(Node3D.new())
	var stations: Array = [
		{"station_id": 3, "name": "Forge Station", "position": _position(100.0, 0.0, 200.0), "docking_radius": 16000.0},
	]

	NavigationMarkerRenderer.spawn_station_markers(bodies_root, stations, 0.1, _to_godot_pos)

	assert_int(bodies_root.get_child_count()).is_equal(1)
	var marker: Node3D = bodies_root.get_child(0) as Node3D
	assert_int(marker.get_meta("station_id") as int).is_equal(3)
	assert_vector(marker.position).is_equal_approx(Vector3(10.0, 0.0, -20.0), Vector3(0.0001, 0.0001, 0.0001))
	var label: Label3D = marker.get_child(2) as Label3D  ## 0=mesh,1=ring,2=label
	assert_str(label.text).is_equal("Forge Station")


func test_spawn_station_markers_appends_after_existing_body_markers() -> void:
	var bodies_root: Node3D = auto_free(Node3D.new())
	NavigationMarkerRenderer.spawn_body_markers(bodies_root, [
		{"body_id": 2, "kind": "Planet", "name": "Forge", "position": _position(500.0, 0.0, 0.0), "radius": 200.0, "spectral_type": 0.0},
	], 0.1, _to_godot_pos)

	NavigationMarkerRenderer.spawn_station_markers(bodies_root, [
		{"station_id": 3, "name": "Forge Station", "position": _position(100.0, 0.0, 200.0), "docking_radius": 16000.0},
	], 0.1, _to_godot_pos)

	assert_int(bodies_root.get_child_count()).is_equal(2)
