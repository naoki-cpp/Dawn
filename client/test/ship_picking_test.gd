## ship_picking_test.gd
##
## Unit tests for ship_picking.gd's ray-pick math. Camera3D.project_ray_*
## needs a live Viewport, so these tests add the camera (and candidate
## Node3Ds) to the scene tree via add_child()/auto_free() -- same pattern
## as main_test.gd's warp-snap-pos tests, which caught a real bug from
## skipping this step.
extends GdUnitTestSuite

const __source: String = "res://scripts/ship_picking.gd"


func _make_camera() -> Camera3D:
	var camera: Camera3D = auto_free(Camera3D.new())
	add_child(camera)
	camera.global_position = Vector3(0.0, 0.0, 10.0)  ## default orientation looks down -Z
	return camera


func _screen_center() -> Vector2:
	return get_viewport().get_visible_rect().size / 2.0


# -- ray_point_distance ---------------------------------------------------------

func test_ray_point_distance_returns_perpendicular_distance_and_ray_parameter() -> void:
	var result: Vector2 = ShipPicking.ray_point_distance(Vector3.ZERO, Vector3(1.0, 0.0, 0.0), Vector3(5.0, 3.0, 0.0))
	assert_vector(result).is_equal_approx(Vector2(3.0, 5.0), Vector2(0.0001, 0.0001))


func test_ray_point_distance_reports_negative_t_when_point_is_behind_the_ray_origin() -> void:
	var result: Vector2 = ShipPicking.ray_point_distance(Vector3.ZERO, Vector3(1.0, 0.0, 0.0), Vector3(-5.0, 0.0, 0.0))
	assert_float(result.y).is_less(0.0)


# -- pick_ship_at -----------------------------------------------------------------

func test_pick_ship_at_returns_the_ship_directly_on_the_camera_ray() -> void:
	var camera: Camera3D = _make_camera()
	var ship: Node3D = auto_free(Node3D.new())
	add_child(ship)
	ship.global_position = Vector3.ZERO

	var picked: int = ShipPicking.pick_ship_at(camera, _screen_center(), {7: ship}, -1)
	assert_int(picked).is_equal(7)


func test_pick_ship_at_excludes_the_given_id_even_when_it_is_the_only_candidate() -> void:
	var camera: Camera3D = _make_camera()
	var ship: Node3D = auto_free(Node3D.new())
	add_child(ship)
	ship.global_position = Vector3.ZERO

	var picked: int = ShipPicking.pick_ship_at(camera, _screen_center(), {7: ship}, 7)
	assert_int(picked).is_equal(-1)


func test_pick_ship_at_returns_minus_one_when_nothing_is_within_pick_radius() -> void:
	var camera: Camera3D = _make_camera()
	var ship: Node3D = auto_free(Node3D.new())
	add_child(ship)
	ship.global_position = Vector3(5000.0, 0.0, 0.0)  ## far off the ray, outside PICK_RADIUS_SHIP

	var picked: int = ShipPicking.pick_ship_at(camera, _screen_center(), {7: ship}, -1)
	assert_int(picked).is_equal(-1)


# -- pick_gate_at -----------------------------------------------------------------

func test_pick_gate_at_returns_the_gate_whose_converted_position_is_on_the_ray() -> void:
	var camera: Camera3D = _make_camera()
	## Mirrors main.gd's _server_to_godot_pos at WORLD_SCALE=0.1: server
	## (0,0,0) -> Godot (0,0,0), right on the test camera's ray.
	var to_godot_pos: Callable = func(p: Vector3) -> Vector3:
		return Vector3(p.x, p.y, -p.z) * 0.1
	var gates: Array = [{"gate_id": 3, "position": Vector3.ZERO}]

	var picked: int = ShipPicking.pick_gate_at(camera, _screen_center(), gates, to_godot_pos)
	assert_int(picked).is_equal(3)


# -- screen_point_distance ---------------------------------------------------------

func test_screen_point_distance_is_near_zero_for_a_point_dead_ahead() -> void:
	var camera: Camera3D = _make_camera()
	var dt: Vector2 = ShipPicking.screen_point_distance(camera, _screen_center(), Vector3.ZERO)
	assert_float(dt.x).is_less(1.0)
	assert_float(dt.y).is_greater(0.0)


func test_screen_point_distance_reports_negative_front_when_point_is_behind_the_camera() -> void:
	var camera: Camera3D = _make_camera()
	## Camera is at z=10 looking down -Z; z=20 is behind it.
	var dt: Vector2 = ShipPicking.screen_point_distance(camera, _screen_center(), Vector3(0.0, 0.0, 20.0))
	assert_float(dt.y).is_less(0.0)


# -- pick_body_at -----------------------------------------------------------------

func test_pick_body_at_returns_the_body_whose_marker_is_on_screen_at_the_click() -> void:
	var camera: Camera3D = _make_camera()
	var bodies_root: Node = auto_free(Node.new())
	add_child(bodies_root)

	var marker := Node3D.new()
	marker.set_meta("body_id", 9)
	bodies_root.add_child(marker)
	marker.global_position = Vector3.ZERO

	var picked: int = ShipPicking.pick_body_at(camera, _screen_center(), bodies_root)
	assert_int(picked).is_equal(9)


func test_pick_body_at_ignores_children_without_a_body_id_meta() -> void:
	var camera: Camera3D = _make_camera()
	var bodies_root: Node = auto_free(Node.new())
	add_child(bodies_root)

	var decoy := Node3D.new()
	bodies_root.add_child(decoy)
	decoy.global_position = Vector3.ZERO  ## on screen, but has no body_id meta

	var picked: int = ShipPicking.pick_body_at(camera, _screen_center(), bodies_root)
	assert_int(picked).is_equal(-1)


func test_pick_body_at_returns_minus_one_when_marker_is_behind_the_camera() -> void:
	var camera: Camera3D = _make_camera()
	var bodies_root: Node = auto_free(Node.new())
	add_child(bodies_root)

	var marker := Node3D.new()
	marker.set_meta("body_id", 9)
	bodies_root.add_child(marker)
	marker.global_position = Vector3(0.0, 0.0, 20.0)  ## behind the camera at z=10

	var picked: int = ShipPicking.pick_body_at(camera, _screen_center(), bodies_root)
	assert_int(picked).is_equal(-1)
