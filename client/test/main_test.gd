## main_test.gd
##
## Unit tests for the pure-math helpers in main.gd: coordinate conversion,
## warp-snap-pos core, and spectral colour. These are the only parts of
## main.gd testable without a running scene tree -- HUD construction, input
## routing, and marker spawning depend on @onready scene paths and Node3D
## state and need the Godot editor to verify (see
## docs/architecture-review-client.md C-1). Picking math now lives in
## ship_picking.gd; see client/test/ship_picking_test.gd.
extends GdUnitTestSuite

const __source: String = "res://scripts/main.gd"

var _main: Node


func before_test() -> void:
	## .new() without adding to the scene tree never triggers _ready(), so
	## the @onready scene-path vars stay null -- fine, since none of the
	## functions under test touch them.
	_main = load(__source).new()


func after_test() -> void:
	_main.free()


# -- _server_to_godot_pos ------------------------------------------------------

func test_server_to_godot_pos_flips_z_and_scales() -> void:
	var result: Vector3 = _main._server_to_godot_pos(Vector3(100.0, 20.0, 300.0))
	assert_vector(result).is_equal_approx(Vector3(10.0, 2.0, -30.0), Vector3(0.0001, 0.0001, 0.0001))


# -- _spectral_color ------------------------------------------------------------

func test_spectral_color_at_t_zero_is_coolest_blue() -> void:
	var c: Color = _main._spectral_color(0.0)
	assert_vector(Vector3(c.r, c.g, c.b)).is_equal_approx(Vector3(0.55, 0.65, 1.00), Vector3(0.0001, 0.0001, 0.0001))


func test_spectral_color_at_t_one_is_warmest_red() -> void:
	var c: Color = _main._spectral_color(1.0)
	assert_vector(Vector3(c.r, c.g, c.b)).is_equal_approx(Vector3(1.00, 0.40, 0.18), Vector3(0.0001, 0.0001, 0.0001))


func test_spectral_color_is_continuous_across_the_010_segment_boundary() -> void:
	var just_below: Color = _main._spectral_color(0.0999)
	var at_boundary: Color = _main._spectral_color(0.10)
	assert_vector(Vector3(just_below.r, just_below.g, just_below.b)) \
		.is_equal_approx(Vector3(at_boundary.r, at_boundary.g, at_boundary.b), Vector3(0.002, 0.002, 0.002))


# -- _compute_warp_snap_pos_core ------------------------------------------------

func test_warp_snap_pos_core_places_arrival_point_toward_the_ship_from_the_target() -> void:
	## global_position only resolves correctly once the node is inside the
	## scene tree (Node3D.get_global_transform() returns identity otherwise) --
	## add_child() puts it under this test suite, which the runner keeps in tree.
	var ship: Node3D = auto_free(Node3D.new())
	add_child(ship)
	ship.global_position = Vector3(100.0, 0.0, 0.0)  ## Godot coords; server pos = (1000,0,0) at WORLD_SCALE=0.1
	_main._ships = {1: ship}
	_main._player_ship_id = 1

	var result: Vector3 = _main._compute_warp_snap_pos_core(Vector3.ZERO, 2000.0, 0.75)
	assert_vector(result).is_equal_approx(Vector3(1500.0, 0.0, 0.0), Vector3(0.01, 0.01, 0.01))


func test_warp_snap_pos_core_falls_back_to_a_fixed_direction_when_ship_is_at_the_target() -> void:
	var ship: Node3D = auto_free(Node3D.new())
	add_child(ship)
	ship.global_position = Vector3(50.0, 0.0, 0.0)  ## server pos = (500,0,0), same as target below
	_main._ships = {1: ship}
	_main._player_ship_id = 1

	var result: Vector3 = _main._compute_warp_snap_pos_core(Vector3(500.0, 0.0, 0.0), 2000.0, 0.75)
	assert_vector(result).is_equal_approx(Vector3(-1000.0, 0.0, 0.0), Vector3(0.01, 0.01, 0.01))


func test_warp_snap_pos_core_returns_inf_when_player_ship_is_unknown() -> void:
	_main._ships = {}
	_main._player_ship_id = -1

	var result: Vector3 = _main._compute_warp_snap_pos_core(Vector3.ZERO, 2000.0, 0.75)
	assert_vector(result).is_equal(Vector3.INF)
