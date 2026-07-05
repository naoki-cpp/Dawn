## main_test.gd
##
## Unit tests for the pure-math helpers still in main.gd: coordinate
## conversion and module-deactivation bookkeeping. These are the only parts of main.gd
## testable without a running scene tree -- HUD construction and input
## routing depend on @onready scene paths and need the Godot editor to
## verify (see docs/architecture/architecture-review-client.md C-1). Picking math lives
## in ship_picking.gd (client/test/ship_picking_test.gd); marker spawning
## and spectral colour live in navigation_marker_renderer.gd
## (client/test/navigation_marker_renderer_test.gd).
extends GdUnitTestSuite

const __source: String = "res://scripts/main.gd"

var _main: Node


class FakeShip:
	extends Node3D

	var velocity_calls: Array[Vector3] = []
	var thrust_calls: Array[Vector3] = []

	func set_velocity(v: Vector3) -> void:
		velocity_calls.append(v)

	func set_thrust_direction(v: Vector3) -> void:
		thrust_calls.append(v)


class FakeConnection:
	extends Node

	var activate_calls: Array[Dictionary] = []
	var deactivate_calls: Array[Dictionary] = []

	func send_activate_module(ship_id: int, module_id: int, slot: String, target_ship_id: int = -1) -> void:
		activate_calls.append({
			"ship_id": ship_id,
			"module_id": module_id,
			"slot": slot,
			"target_ship_id": target_ship_id,
		})

	func send_deactivate_module(ship_id: int, module_id: int, slot: String) -> void:
		deactivate_calls.append({"ship_id": ship_id, "module_id": module_id, "slot": slot})


func before_test() -> void:
	## .new() without adding to the scene tree never triggers _ready(), so
	## the @onready scene-path vars stay null -- fine, since none of the
	## functions under test touch them.
	_main = load(__source).new()


func after_test() -> void:
	_main.free()


func _module_fixture(module_id: int, slot: String, active: bool) -> Dictionary:
	return {
		"module_id": module_id,
		"slot": slot,
		"is_active": active,
		"is_active_module": true,
		"forced_reason": "",
	}


# -- _server_to_godot_pos ------------------------------------------------------

func test_server_to_godot_pos_flips_z_and_scales() -> void:
	var result: Vector3 = _main._server_to_godot_pos(Vector3(100.0, 20.0, 300.0))
	assert_vector(result).is_equal_approx(Vector3(10.0, 2.0, -30.0), Vector3(0.0001, 0.0001, 0.0001))


# -- _handle_position_snap ----------------------------------------------------

func test_player_position_snap_clears_residual_warp_motion() -> void:
	var ship := FakeShip.new()
	_main.add_child(ship)
	ship.global_position = Vector3(10.0, 0.0, 0.0)
	_main._ships = {1: ship}
	_main._player_ship_id = 1

	_main._handle_position_snap({
		"ship_id": 1,
		"position": {"x": 1_000_000.0, "y": 0.0, "z": 0.0},
	})

	assert_vector(ship.velocity_calls.back()).is_equal(Vector3.ZERO)
	assert_vector(ship.thrust_calls.back()).is_equal(Vector3.ZERO)
	ship.free()


func test_observed_ship_position_snap_clears_residual_warp_motion() -> void:
	var ship := FakeShip.new()
	_main.add_child(ship)
	_main._ships = {2: ship}
	_main._player_ship_id = 1

	_main._handle_position_snap({
		"ship_id": 2,
		"position": {"x": 100.0, "y": 20.0, "z": 300.0},
	})

	assert_vector(ship.position).is_equal_approx(Vector3(10.0, 2.0, -30.0), Vector3(0.0001, 0.0001, 0.0001))
	assert_vector(ship.velocity_calls.back()).is_equal(Vector3.ZERO)
	assert_vector(ship.thrust_calls.back()).is_equal(Vector3.ZERO)
	ship.free()


func test_ship_docked_event_clears_residual_motion() -> void:
	var ship := FakeShip.new()
	_main.add_child(ship)
	ship.global_position = Vector3(99.0, 99.0, 99.0)
	_main._ships = {2: ship}
	_main._stations = [{
		"station_id": 0,
		"name": "Forge Station",
		"position": Vector3(100.0, 20.0, 300.0),
	}]

	_main._handle_ship_docked({
		"ship_id": 2,
		"station_id": 0,
		"tick": 12,
	})

	assert_vector(ship.global_position).is_equal_approx(
		Vector3(10.0, 2.0, -30.0),
		Vector3(0.0001, 0.0001, 0.0001)
	)
	assert_vector(ship.velocity_calls.back()).is_equal(Vector3.ZERO)
	assert_vector(ship.thrust_calls.back()).is_equal(Vector3.ZERO)
	ship.free()


func test_player_ship_undocked_event_clears_docked_station_state() -> void:
	_main._player_ship_id = 2
	_main._nearby_station_id = -1
	_main._session.player_ship_id = 2
	_main._session.apply_dock_fitting(0, "Forge Station", 12)

	_main._handle_ship_undocked({
		"ship_id": 2,
		"station_id": 0,
		"tick": 13,
	})

	assert_int(_main._nearby_station_id).is_equal(0)
	var status: Dictionary = _main._session.dock_status()
	assert_int(status["docked_station_id"] as int).is_equal(-1)
	assert_str(status["docked_station_name"] as String).is_equal("")


# -- _on_module_deactivated (manual OFF vs system-forced OFF) -----------------------
#
# ModuleDeactivated now carries a server-authoritative reason ("cap" | "range"
# | "", ADR-0035), so the client trusts it directly instead of guessing from
# its own DeactivateModuleCommand sends (regression this replaced: every
# forced-off used to render as CAP! even when the real cause was out-of-range).

func test_module_activated_marks_matching_player_module_active() -> void:
	_main._player_ship_id = 1
	_main._player_modules = [_module_fixture(5, "Mid", false)]

	_main._on_module_activated(1, 5, "Mid")

	var mod_dict: Dictionary = _main._player_modules[0]
	assert_bool(mod_dict["is_active"] as bool).is_true()
	assert_str(mod_dict["forced_reason"] as String).is_equal("")


func test_module_toggle_marks_module_active_before_server_echo() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_main._player_modules = [_module_fixture(5, "Mid", false)]

	_main._toggle_module_by_index(0)

	var mod_dict: Dictionary = _main._player_modules[0]
	assert_bool(mod_dict["is_active"] as bool).is_true()
	assert_str(mod_dict["forced_reason"] as String).is_equal("")
	assert_int(connection.activate_calls.size()).is_equal(1)
	connection.free()


func test_module_toggle_of_a_targeted_kind_without_a_locked_target_is_refused_client_side() -> void:
	## Sending Activate for a Weapon/Tackle/Remote-repair kind without a
	## Locked target is rejected server-side outright (ADR-0035), which
	## previously showed as an instant on-then-off flicker (optimistic
	## toggle immediately corrected by the PlayerFitting resync). The
	## client now refuses client-side instead, regardless of range.
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_main._player_modules = [{
		"module_id": 5,
		"slot": "High",
		"kind": "Weapon",
		"is_active": false,
		"is_active_module": true,
		"forced_reason": "",
	}]
	## Fresh _main has _session.player_lock_target == -1 (no Lock).

	_main._toggle_module_by_index(0)

	var mod_dict: Dictionary = _main._player_modules[0]
	assert_bool(mod_dict["is_active"] as bool).is_false()
	assert_int(connection.activate_calls.size()).is_equal(0)
	connection.free()


func test_module_toggle_of_a_targeted_kind_against_a_locked_but_out_of_aoi_target_is_refused_client_side() -> void:
	## Lock survives AoI leave (ADR-0019, world_session.gd
	## remove_ship(clear_lock=false)): the locked target can be gone from
	## _ships while player_lock_target still points at it (e.g. right after
	## the target warps away). With no node to read a position from, the
	## range guard used to be skipped entirely and the activation fell
	## through to the server -- which rejects it, producing the same
	## on-then-off flicker the visible-target range guard exists to
	## prevent. The client must refuse here too.
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_main._session.player_lock_target = 99
	_main._ships = {} # target 99 is not in AoI; player ship 1 isn't either.
	_main._player_modules = [{
		"module_id": 5,
		"slot": "High",
		"kind": "Weapon",
		"is_active": false,
		"is_active_module": true,
		"forced_reason": "",
	}]

	_main._toggle_module_by_index(0)

	var mod_dict: Dictionary = _main._player_modules[0]
	assert_bool(mod_dict["is_active"] as bool).is_false()
	assert_int(connection.activate_calls.size()).is_equal(0)
	connection.free()


func test_module_toggle_marks_module_inactive_before_server_echo() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_main._player_modules = [_module_fixture(5, "High", true)]

	_main._toggle_module_by_index(0)

	var mod_dict: Dictionary = _main._player_modules[0]
	assert_bool(mod_dict["is_active"] as bool).is_false()
	assert_str(mod_dict["forced_reason"] as String).is_equal("")
	assert_int(connection.deactivate_calls.size()).is_equal(1)
	connection.free()


func test_module_deactivated_with_no_reason_is_a_plain_off() -> void:
	_main._player_ship_id = 1
	_main._player_modules = [_module_fixture(5, "High", true)]

	_main._on_module_deactivated(1, 5, "High", "")

	var mod_dict: Dictionary = _main._player_modules[0]
	assert_bool(mod_dict["is_active"] as bool).is_false()
	assert_str(mod_dict["forced_reason"] as String).is_equal("")


func test_module_deactivated_with_cap_reason_flags_forced_reason() -> void:
	_main._player_ship_id = 1
	_main._player_modules = [_module_fixture(5, "High", true)]

	_main._on_module_deactivated(1, 5, "High", "cap")

	var mod_dict: Dictionary = _main._player_modules[0]
	assert_str(mod_dict["forced_reason"] as String).is_equal("cap")


func test_module_deactivated_with_range_reason_flags_forced_reason() -> void:
	_main._player_ship_id = 1
	_main._player_modules = [_module_fixture(5, "High", true)]

	_main._on_module_deactivated(1, 5, "High", "range")

	var mod_dict: Dictionary = _main._player_modules[0]
	assert_str(mod_dict["forced_reason"] as String).is_equal("range")
