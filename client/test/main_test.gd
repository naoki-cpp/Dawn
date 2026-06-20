## main_test.gd
##
## Unit tests for the pure-math helpers still in main.gd: coordinate
## conversion and warp-snap-pos core. These are the only parts of main.gd
## testable without a running scene tree -- HUD construction and input
## routing depend on @onready scene paths and need the Godot editor to
## verify (see docs/architecture-review-client.md C-1). Picking math lives
## in ship_picking.gd (client/test/ship_picking_test.gd); marker spawning
## and spectral colour live in navigation_marker_renderer.gd
## (client/test/navigation_marker_renderer_test.gd).
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


# -- _on_module_deactivated (manual OFF vs capacitor-forced OFF) ---------------------
#
# ModuleDeactivated carries no reason field (ADR-0006), so the client must
# track its own DeactivateModuleCommand sends to tell "player turned it off"
# apart from "capacitor forced it off" (regression: toggling a module on
# then off was showing CAP! instead of OFF).

func test_module_deactivated_after_a_manual_toggle_does_not_flag_cap_forced_off() -> void:
	_main._player_ship_id = 1
	_main._player_modules = [{"module_id": 5, "is_active": true, "cap_forced_off": false}]
	_main._pending_manual_deactivations = {5: true}  ## set by _toggle_module_by_index

	_main._on_module_deactivated(1, 5, "High")

	var mod_dict: Dictionary = _main._player_modules[0]
	assert_bool(mod_dict["is_active"] as bool).is_false()
	assert_bool(mod_dict["cap_forced_off"] as bool).is_false()


func test_module_deactivated_without_a_pending_manual_request_flags_cap_forced_off() -> void:
	_main._player_ship_id = 1
	_main._player_modules = [{"module_id": 5, "is_active": true, "cap_forced_off": false}]
	## No entry in _pending_manual_deactivations -- the server deactivated it unprompted.

	_main._on_module_deactivated(1, 5, "High")

	var mod_dict: Dictionary = _main._player_modules[0]
	assert_bool(mod_dict["cap_forced_off"] as bool).is_true()


func test_module_deactivated_clears_the_pending_flag_so_it_does_not_leak_to_the_next_event() -> void:
	_main._player_ship_id = 1
	_main._player_modules = [{"module_id": 5, "is_active": true, "cap_forced_off": false}]
	_main._pending_manual_deactivations = {5: true}

	_main._on_module_deactivated(1, 5, "High")

	assert_bool(_main._pending_manual_deactivations.has(5)).is_false()
