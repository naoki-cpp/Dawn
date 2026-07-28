## main_test.gd
##
## Unit tests for the pure-math helpers still in main.gd: coordinate
## conversion and module-deactivation bookkeeping. These are the only parts of main.gd
## testable without a running scene tree -- HUD construction and input
## routing depend on @onready scene paths and need the Godot editor to
## verify (see docs/architecture/architecture-review/client.md C-1). Picking math lives
## in ship_picking.gd (client/test/ship_picking_test.gd); marker spawning
## and spectral colour live in navigation_marker_renderer.gd
## (client/test/navigation_marker_renderer_test.gd).
extends GdUnitTestSuite

const __source: String = "res://scripts/main.gd"
const InventoryRow = preload("res://scripts/inventory_row.gd")
const HudManager = preload("res://scripts/hud_manager.gd")
const AU_M: float = 1.495978707e11

## dawn_core::StatDelta (client-side, ADR-0039) requires every field --
## unlike the client's former hand-copied mirror, it has no per-field
## `#[serde(default)]`, so a JSON fixture built for
## `PlayerLoadout.apply_payload` (test/debug-only) can no longer omit fields.
const FULL_ZERO_STAT_DELTA: Dictionary = {
	"weapon_damage_add": 0.0,
	"weapon_range_add": 0.0,
	"falloff_range_add": 0.0,
	"tracking_speed_add": 0.0,
	"speed_multiplier": 1.0,
	"mass_add": 0.0,
	"max_shield_add": 0.0,
	"max_armor_add": 0.0,
	"max_hull_add": 0.0,
	"weapon_cooldown_add": 0,
	"lock_time_add": 0,
	"max_locks_add": 0,
	"cap_max_add": 0.0,
	"cap_recharge_add": 0.0,
	"tackle_range_add": 0.0,
	"repair_amount": 0.0,
	"repair_range_add": 0.0,
}

var _main: Node


class FakeShip:
	extends Node3D

	var velocity_calls: Array[Vector3] = []
	var velocity_tick_calls: Array[int] = []
	var thrust_calls: Array[Vector3] = []
	var set_as_player_calls: int = 0
	var clear_as_player_calls: int = 0
	var reconcile_calls: Array[Dictionary] = []
	var dock_calls: Array[Dictionary] = []
	var undock_calls: Array[Dictionary] = []
	var server_position_value := PackedFloat64Array([0.0, 0.0, 0.0])

	func set_velocity(v: Vector3, tick: int = 0) -> bool:
		velocity_calls.append(v)
		velocity_tick_calls.append(tick)
		return true

	## WorldPresentation.attach_player_ship() calls this via ship.call(...);
	## a real ship (ship_controller.gd) sets up player-only visuals here.
	func set_as_player() -> void:
		set_as_player_calls += 1

	## WorldPresentation.attach_player_ship() calls this on the previously
	## piloted ship (if any) via ship.call(...) when a different ship
	## becomes the active one; a real ship tears down player-only visuals.
	func clear_as_player() -> void:
		clear_as_player_calls += 1

	func set_thrust_direction(v: Vector3) -> void:
		thrust_calls.append(v)

	func set_braking() -> void:
		thrust_calls.append(Vector3.ZERO)

	func reset_motion(p: PackedFloat64Array, v: Vector3, _tick: int) -> void:
		server_position_value = p
		velocity_calls.append(v)
		thrust_calls.append(Vector3.ZERO)

	func dock_motion(p: PackedFloat64Array, _tick: int) -> bool:
		server_position_value = p
		velocity_calls.append(Vector3.ZERO)
		thrust_calls.append(Vector3.ZERO)
		dock_calls.append({"position": p, "tick": _tick})
		return true

	func undock_motion(p: PackedFloat64Array, v: Vector3, _tick: int) -> bool:
		server_position_value = p
		velocity_calls.append(v)
		thrust_calls.append(Vector3.ZERO)
		undock_calls.append({"position": p, "velocity": v, "tick": _tick})
		return true

	func reconcile_motion(p: PackedFloat64Array, v: Vector3, _tick: int) -> void:
		server_position_value = p
		velocity_calls.append(v)
		reconcile_calls.append({"position": p, "velocity": v, "tick": _tick})

	func server_position() -> PackedFloat64Array:
		return server_position_value


class FakeConnection:
	extends Node

	var activate_calls: Array[Dictionary] = []
	var deactivate_calls: Array[Dictionary] = []

	func send_activate_module(module_id: int, slot: String, target_ship_id: int = -1) -> void:
		activate_calls.append({
			"module_id": module_id,
			"slot": slot,
			"target_ship_id": target_ship_id,
		})

	func send_deactivate_module(module_id: int, slot: String) -> void:
		deactivate_calls.append({"module_id": module_id, "slot": slot})

	var disassemble_calls: Array[Dictionary] = []
	var build_calls: Array[Dictionary] = []

	func send_disassemble_ship_command(p_ship_id: int, p_station_id: int) -> void:
		disassemble_calls.append({"ship_id": p_ship_id, "station_id": p_station_id})

	func send_build_packaged_ship_command(p_ship_id: int, p_station_id: int, p_ship_type_id: int) -> void:
		build_calls.append({
			"ship_id": p_ship_id, "station_id": p_station_id, "ship_type_id": p_ship_type_id,
		})

	var unfit_calls: Array[Dictionary] = []

	func send_unfit_module_command(p_ship_id: int, p_module_id: int, p_slot: String) -> void:
		unfit_calls.append({"ship_id": p_ship_id, "module_id": p_module_id, "slot": p_slot})

	var fit_calls: Array[Dictionary] = []

	func send_fit_module_command(p_ship_id: int, p_module_id: int, p_slot: String) -> void:
		fit_calls.append({"ship_id": p_ship_id, "module_id": p_module_id, "slot": p_slot})

	var reorder_calls: Array[Dictionary] = []

	func send_reorder_fitted_module_command(
		p_ship_id: int, p_slot: String, p_from_index: int, p_to_index: int
	) -> void:
		reorder_calls.append({
			"ship_id": p_ship_id, "slot": p_slot, "from_index": p_from_index, "to_index": p_to_index,
		})

	var transfer_to_station_calls: Array[Dictionary] = []
	var transfer_from_station_calls: Array[Dictionary] = []

	func send_transfer_to_station_command(
		p_ship_id: int, p_station_id: int, p_item_type: String, p_module_id: int = 0,
		p_ship_type_id: int = 0
	) -> void:
		transfer_to_station_calls.append({
			"ship_id": p_ship_id, "station_id": p_station_id, "item_type": p_item_type,
			"module_id": p_module_id, "ship_type_id": p_ship_type_id,
		})

	func send_transfer_from_station_command(
		p_ship_id: int, p_station_id: int, p_item_type: String, p_module_id: int = 0,
		p_ship_type_id: int = 0
	) -> void:
		transfer_from_station_calls.append({
			"ship_id": p_ship_id, "station_id": p_station_id, "item_type": p_item_type,
			"module_id": p_module_id, "ship_type_id": p_ship_type_id,
		})


func before_test() -> void:
	## .new() without adding to the scene tree never triggers _ready(), so
	## the @onready scene-path vars stay null -- fine, since none of the
	## functions under test touch them.
	_main = load(__source).new()
	_main._interaction = load("res://scripts/world_interaction.gd").new()
	_main._loadout = PlayerLoadout.new()


func after_test() -> void:
	_main.free()


func _module_fixture(module_id: int, slot: String, active: bool) -> Dictionary:
	return {
		"slot": slot,
		"index": 0,
		"module_id": module_id,
		"name": "Test Module",
		"kind": "",
		"is_active": active,
		"is_active_module": true,
		"cap_cost_per_cycle": 0.0,
		"cycle_time_ticks": 10,
		"stat_delta": FULL_ZERO_STAT_DELTA,
	}


func _set_loadout_modules(modules: Array) -> void:
	_main._loadout.apply_payload(JSON.stringify({"modules": modules}))


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

	assert_array(ship.server_position_value).contains_exactly([100.0, 20.0, 300.0])
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
		"position": PackedFloat64Array([5.0 * AU_M + 10.0, 20.0, 300.0]),
	}]
	_main._world.rebase_to_components(5.0 * AU_M, 0.0, 0.0)

	_main._handle_ship_docked({
		"ship_id": 2,
		"station_id": 0,
		"tick": 12,
	})

	## FakeShip is not attached to a live SceneTree here, so `position` is the
	## stable seam for verifying the dock snap.
	assert_array(ship.server_position_value).contains_exactly([
		5.0 * AU_M + 10.0, 20.0, 300.0,
	])
	assert_vector(ship.velocity_calls.back()).is_equal(Vector3.ZERO)
	assert_vector(ship.thrust_calls.back()).is_equal(Vector3.ZERO)
	ship.free()


func test_ship_docked_event_stops_ship_when_station_map_is_not_ready() -> void:
	var ship := FakeShip.new()
	_main.add_child(ship)
	ship.server_position_value = PackedFloat64Array([9.0, 8.0, 7.0])
	_main._ships = {2: ship}

	_main._handle_ship_docked({
		"ship_id": 2,
		"station_id": 99,
		"tick": 12,
	})

	assert_int(ship.dock_calls.size()).is_equal(1)
	assert_array(ship.dock_calls[0]["position"]).contains_exactly([9.0, 8.0, 7.0])
	assert_vector(ship.velocity_calls.back()).is_equal(Vector3.ZERO)
	ship.free()


func test_stale_player_undock_event_does_not_leave_docked_state() -> void:
	var ship := FakeShip.new()
	_main.add_child(ship)
	_main._ships = {2: ship}
	_main._player_ship_id = 2
	_main._session.set_player_ship_id(2)
	_main._session.apply_dock_fitting(0, "Forge Station", 12)

	_main._handle_ship_undocked({
		"ship_id": 2,
		"station_id": 0,
		"tick": 11,
	})

	assert_int(ship.undock_calls.size()).is_equal(0)
	assert_int(_main._session.dock_status()["docked_station_id"] as int).is_equal(0)
	ship.free()


func test_player_loadout_refresh_preserves_docked_motion_state() -> void:
	var ship := FakeShip.new()
	_main.add_child(ship)
	_main._ships = {2: ship}
	_main._player_ship_id = 2
	_main._session.set_player_ship_id(2)
	_main._loadout.apply_payload(JSON.stringify({
		"tick": 12,
		"active_ship_id": 2,
		"docked_station_id": 0,
		"docked_station_name": "Forge Station",
	}))

	_main._apply_loadout_side_effects()

	assert_int(ship.dock_calls.size()).is_equal(1)
	assert_int(ship.undock_calls.size()).is_equal(0)
	ship.free()


func test_au_navigation_proximity_uses_unquantized_positions() -> void:
	var ship := FakeShip.new()
	_main.add_child(ship)
	_main._ships = {1: ship}
	_main._player_ship_id = 1
	_main._world.rebase_to_components(5.0 * AU_M, 0.0, 0.0)
	_main._gates = [{
		"gate_id": 7,
		"position": PackedFloat64Array([5.0 * AU_M + 10.0, 0.0, 0.0]),
		"activation_radius": 20.0,
	}]
	_main._stations = [{
		"station_id": 5,
		"position": PackedFloat64Array([5.0 * AU_M + 15.0, 0.0, 0.0]),
		"docking_radius": 20.0,
	}]

	_main._update_gate_proximity()
	_main._update_station_proximity()

	assert_int(_main._nearby_gate_id).is_equal(7)
	assert_array(_main._nearby_station_ids).contains_exactly([5])
	ship.free()


func test_motion_correction_reconciles_the_active_ship() -> void:
	var ship := FakeShip.new()
	_main.add_child(ship)
	_main._ships = {1: ship}
	_main._player_ship_id = 1

	_main._handle_motion_correction({
		"ship_id": 1,
		"position": {"x": 100.0, "y": 20.0, "z": 300.0},
		"velocity": {"dx": 4.0, "dy": 5.0, "dz": -6.0},
		"tick": 42,
	})

	assert_int(ship.reconcile_calls.size()).is_equal(1)
	var motion_call: Dictionary = ship.reconcile_calls.back()
	assert_array(motion_call["position"]).contains_exactly([100.0, 20.0, 300.0])
	assert_vector(motion_call["velocity"]).is_equal(Vector3(4.0, 5.0, -6.0))
	assert_int(motion_call["tick"]).is_equal(42)
	ship.free()


func test_velocity_changed_passes_the_authority_tick_to_the_ship() -> void:
	var ship := FakeShip.new()
	_main.add_child(ship)
	_main._ships = {1: ship}

	_main._handle_velocity_changed({
		"ship_id": 1,
		"velocity": {"dx": 4.0, "dy": 5.0, "dz": -6.0},
		"tick": 42,
	})

	assert_array(ship.velocity_tick_calls).contains_exactly([42])
	ship.free()


func test_motion_correction_preserves_small_motion_near_a_true_au_origin() -> void:
	var ship := FakeShip.new()
	_main.add_child(ship)
	_main._ships = {1: ship}
	_main._player_ship_id = 1
	_main._world.rebase_to_components(5.0 * 1.495978707e11, 0.0, 0.0)

	_main._handle_motion_correction({
		"ship_id": 1,
		"position": {"x": 5.0 * 1.495978707e11 + 10.0, "y": 0.0, "z": 0.0},
		"velocity": {"dx": 4.0, "dy": 0.0, "dz": 0.0},
		"tick": 43,
	})

	var motion_call: Dictionary = ship.reconcile_calls.back()
	assert_array(motion_call["position"]).contains_exactly([
		5.0 * AU_M + 10.0, 0.0, 0.0,
	])
	ship.free()


func test_player_ship_undocked_event_clears_docked_station_state() -> void:
	_main._player_ship_id = 2
	_main._nearby_station_ids = [] as Array[int]
	_main._session.set_player_ship_id(2)
	_main._session.apply_dock_fitting(0, "Forge Station", 12)

	_main._handle_ship_undocked({
		"ship_id": 2,
		"station_id": 0,
		"tick": 13,
	})

	assert_int(_main._nearby_station_ids[0]).is_equal(0)
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
	_set_loadout_modules([_module_fixture(5, "Mid", false)])

	_main._on_module_activated(1, 5, "Mid")

	var mod_dict: ModuleRow = _main._loadout.modules()[0]
	assert_bool(mod_dict.is_active).is_true()
	assert_str(mod_dict.forced_reason).is_equal("")


func test_module_toggle_marks_module_active_before_server_echo() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_set_loadout_modules([_module_fixture(5, "Mid", false)])

	_main._toggle_module_by_index(0)

	var mod_dict: ModuleRow = _main._loadout.modules()[0]
	assert_bool(mod_dict.is_active).is_true()
	assert_str(mod_dict.forced_reason).is_equal("")
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
	_set_loadout_modules([{
		"slot": "High", "index": 0, "module_id": 5, "name": "Test Module", "kind": "Weapon",
		"is_active": false, "is_active_module": true,
		"cap_cost_per_cycle": 0.0, "cycle_time_ticks": 10, "stat_delta": FULL_ZERO_STAT_DELTA,
	}])
	## Fresh _main has no player lock target.

	_main._toggle_module_by_index(0)

	var mod_dict: ModuleRow = _main._loadout.modules()[0]
	assert_bool(mod_dict.is_active).is_false()
	assert_int(connection.activate_calls.size()).is_equal(0)
	connection.free()


func test_module_toggle_of_a_targeted_kind_against_a_locked_but_out_of_aoi_target_is_refused_client_side() -> void:
	## Lock survives AoI leave (ADR-0019, WorldSession state
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
	_main._session.set_player_ship_id(1)
	_main._session.apply_target_locked(1, 99)
	_main._ships = {} # target 99 is not in AoI; player ship 1 isn't either.
	_set_loadout_modules([{
		"slot": "High", "index": 0, "module_id": 5, "name": "Test Module", "kind": "Weapon",
		"is_active": false, "is_active_module": true,
		"cap_cost_per_cycle": 0.0, "cycle_time_ticks": 10, "stat_delta": FULL_ZERO_STAT_DELTA,
	}])

	_main._toggle_module_by_index(0)

	var mod_dict: ModuleRow = _main._loadout.modules()[0]
	assert_bool(mod_dict.is_active).is_false()
	assert_int(connection.activate_calls.size()).is_equal(0)
	connection.free()


func test_module_toggle_marks_module_inactive_before_server_echo() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_set_loadout_modules([_module_fixture(5, "High", true)])

	_main._toggle_module_by_index(0)

	var mod_dict: ModuleRow = _main._loadout.modules()[0]
	assert_bool(mod_dict.is_active).is_false()
	assert_str(mod_dict.forced_reason).is_equal("")
	assert_int(connection.deactivate_calls.size()).is_equal(1)
	connection.free()


func test_module_deactivated_with_no_reason_is_a_plain_off() -> void:
	_main._player_ship_id = 1
	_set_loadout_modules([_module_fixture(5, "High", true)])

	_main._on_module_deactivated(1, 5, "High", "")

	var mod_dict: ModuleRow = _main._loadout.modules()[0]
	assert_bool(mod_dict.is_active).is_false()
	assert_str(mod_dict.forced_reason).is_equal("")


func test_module_deactivated_with_cap_reason_flags_forced_reason() -> void:
	_main._player_ship_id = 1
	_set_loadout_modules([_module_fixture(5, "High", true)])

	_main._on_module_deactivated(1, 5, "High", "cap")

	var mod_dict: ModuleRow = _main._loadout.modules()[0]
	assert_str(mod_dict.forced_reason).is_equal("cap")


func test_module_deactivated_with_range_reason_flags_forced_reason() -> void:
	_main._player_ship_id = 1
	_set_loadout_modules([_module_fixture(5, "High", true)])

	_main._on_module_deactivated(1, 5, "High", "range")

	var mod_dict: ModuleRow = _main._loadout.modules()[0]
	assert_str(mod_dict.forced_reason).is_equal("range")


## Regression: switching active ship via the SHIPS roster (SelectActiveShip,
## ADR-0037) used to only update _player_ship_id and session state --
## bookkeeping the camera never reads. WorldPresentation.attach_player_ship()
## (which retargets the camera, applies the player material, and re-attaches
## the tactical overlay) was never called for this path, so the camera
## silently kept following the old ship.
func test_switching_active_ship_to_a_known_ship_reattaches_the_camera() -> void:
	var camera: Camera3D = auto_free(load("res://scripts/camera_controller.gd").new())
	add_child(camera)
	_main._camera = camera
	_main._presentation._camera = camera
	_main._hud_surface.build(
		auto_free(Node.new()), auto_free(CanvasLayer.new()), auto_free(Label.new()))

	var ship_a: FakeShip = auto_free(FakeShip.new())
	add_child(ship_a)
	var ship_b: FakeShip = auto_free(FakeShip.new())
	add_child(ship_b)
	ship_b.global_position = Vector3(100.0, 0.0, 0.0)

	_main._ships = {1: ship_a, 2: ship_b}
	_main._session.register_ship(1, {
		"is_player": true,
		"ship_type_name": "Magpie",
		"current_shield": 80.0,
		"current_armor": 70.0,
		"current_hull": 60.0,
		"max_shield": 100.0,
		"max_armor": 90.0,
		"max_hull": 80.0,
		"cap_max": 55.0,
		"cap_recharge_per_tick": 3.0,
	}, 1)
	_main._session.register_ship(2, {
		"is_player": false,
		"ship_type_name": "Venture",
		"current_shield": 210.0,
		"current_armor": 160.0,
		"current_hull": 110.0,
		"max_shield": 250.0,
		"max_armor": 180.0,
		"max_hull": 120.0,
		"cap_max": 80.0,
		"cap_recharge_per_tick": 4.0,
	}, 1)
	_main._session.set_player_ship_id(1)
	## Route through the real attach path (not a bare camera.set_target())
	## so WorldPresentation._player_ship is seeded correctly -- otherwise the
	## "revert the old ship" assertions below would trivially pass even
	## without the fix, since there'd be no previous ship to revert.
	_main._set_as_player_ship(1, ship_a)

	_main._loadout.apply_payload(JSON.stringify({"active_ship_id": 2}))
	_main._apply_loadout_side_effects()

	assert_int(_main._player_ship_id).is_equal(2)
	var snapshot: Dictionary = _main._session.snapshot()
	assert_int(snapshot.player_ship_id).is_equal(2)
	assert_str(snapshot.player_ship_type_name).is_equal("Venture")
	assert_float(snapshot.player_shield).is_equal_approx(210.0, 0.001)
	assert_float(snapshot.player_max_shield).is_equal_approx(250.0, 0.001)
	assert_float(snapshot.cap_current).is_equal_approx(80.0, 0.001)
	assert_float(snapshot.cap_max).is_equal_approx(80.0, 0.001)
	assert_float(snapshot.cap_recharge).is_equal_approx(4.0, 0.001)
	assert_int(ship_b.set_as_player_calls).is_equal(1)
	assert_object(camera._target_node).is_equal(ship_b)
	## Regression: the old ship used to stay permanently player-colored
	## (and kept drawing frozen velocity/thrust indicators) after switching.
	assert_int(ship_a.clear_as_player_calls).is_equal(1)


## The client has never rendered ship 3 (never entered AoI), so there is no
## Node3D to attach the camera to -- this case is left alone (needs to spawn
## the ship first, docs/architecture/ownership.md §8), not a regression.
func test_switching_active_ship_to_an_unknown_ship_leaves_the_camera_alone() -> void:
	var camera: Camera3D = auto_free(load("res://scripts/camera_controller.gd").new())
	add_child(camera)
	_main._camera = camera
	_main._presentation._camera = camera
	_main._hud_surface.build(
		auto_free(Node.new()), auto_free(CanvasLayer.new()), auto_free(Label.new()))

	var ship_a: FakeShip = auto_free(FakeShip.new())
	add_child(ship_a)

	_main._ships = {1: ship_a}
	_main._session.set_player_ship_id(1)
	_main._player_ship_id = 1
	camera.set_target(ship_a)

	_main._loadout.apply_payload(JSON.stringify({"active_ship_id": 3}))
	_main._apply_loadout_side_effects()

	assert_int(_main._player_ship_id).is_equal(1)
	assert_object(camera._target_node).is_equal(ship_a)


## Regression: Disembark (active_ship_id -> -1) used to set _player_ship_id
## directly with no call into WorldPresentation at all, so the disembarked
## ship kept the player material and _is_player = true forever -- the same
## desync class as the camera bug above, just on the "no active ship" branch
## instead of the "switch to a different ship" branch.
func test_disembarking_reverts_the_old_ships_player_material() -> void:
	var camera: Camera3D = auto_free(load("res://scripts/camera_controller.gd").new())
	add_child(camera)
	_main._camera = camera
	_main._presentation._camera = camera
	_main._hud_surface.build(
		auto_free(Node.new()), auto_free(CanvasLayer.new()), auto_free(Label.new()))

	var ship_a: FakeShip = auto_free(FakeShip.new())
	add_child(ship_a)

	_main._ships = {1: ship_a}
	_main._session.set_player_ship_id(1)
	_main._set_as_player_ship(1, ship_a)

	_main._loadout.apply_payload(JSON.stringify({"active_ship_id": null}))
	_main._apply_loadout_side_effects()

	assert_int(_main._player_ship_id).is_equal(-1)
	assert_int(_main._session.snapshot().player_ship_id).is_equal(-1)
	assert_int(ship_a.clear_as_player_calls).is_equal(1)


# -- Phase 9B: Disassemble/Build inventory-row buttons -------------------------

func test_disassemble_row_click_sends_disassemble_command_for_the_docked_ship() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_main._session.apply_dock_fitting(3, "Forge Station", 12)

	var row: InventoryRow = InventoryRow.for_item(null, 0, "", InventoryRow.ACTION_DISASSEMBLE)
	_main._handle_inventory_row_click(row)

	assert_int(connection.disassemble_calls.size()).is_equal(1)
	assert_int(connection.disassemble_calls[0]["ship_id"] as int).is_equal(1)
	assert_int(connection.disassemble_calls[0]["station_id"] as int).is_equal(3)
	connection.free()


func test_disassemble_row_click_is_a_no_op_when_not_docked() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_main._session.apply_dock_fitting(-1, "", 0)

	var row: InventoryRow = InventoryRow.for_item(null, 0, "", InventoryRow.ACTION_DISASSEMBLE)
	_main._handle_inventory_row_click(row)

	assert_int(connection.disassemble_calls.size()).is_equal(0)
	connection.free()


func test_build_ship_type_row_click_sends_the_picked_ship_type_not_the_hardcoded_default() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_main._session.apply_dock_fitting(3, "Forge Station", 12)

	var row: InventoryRow = InventoryRow.for_item(
		null, 0, "", InventoryRow.ACTION_BUILD_SHIP_TYPE, 42)
	_main._handle_inventory_row_click(row)

	assert_int(connection.build_calls.size()).is_equal(1)
	assert_int(connection.build_calls[0]["ship_id"] as int).is_equal(1)
	assert_int(connection.build_calls[0]["station_id"] as int).is_equal(3)
	assert_int(connection.build_calls[0]["ship_type_id"] as int).is_equal(42)
	connection.free()


func test_unfit_all_row_click_sends_one_unfit_command_per_fitted_module() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_main._session.apply_dock_fitting(3, "Forge Station", 12)
	_set_loadout_modules([
		_module_fixture(1, "High", false),
		_module_fixture(2, "Low", false),
	])

	var row: InventoryRow = InventoryRow.for_item(null, 0, "", InventoryRow.ACTION_UNFIT_ALL)
	_main._handle_inventory_row_click(row)

	assert_int(connection.unfit_calls.size()).is_equal(2)
	assert_int(connection.unfit_calls[0]["module_id"] as int).is_equal(1)
	assert_str(connection.unfit_calls[0]["slot"] as String).is_equal("High")
	assert_int(connection.unfit_calls[1]["module_id"] as int).is_equal(2)
	assert_str(connection.unfit_calls[1]["slot"] as String).is_equal("Low")
	connection.free()


func test_unfit_all_row_click_is_a_no_op_when_no_module_is_fitted() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1

	var row: InventoryRow = InventoryRow.for_item(null, 0, "", InventoryRow.ACTION_UNFIT_ALL)
	_main._handle_inventory_row_click(row)

	assert_int(connection.unfit_calls.size()).is_equal(0)
	connection.free()


# -- Drag-and-drop dispatch matrix --------------------------------------------

func test_drag_from_ship_cargo_to_fitted_sends_fit_command() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_main._session.apply_dock_fitting(3, "Forge Station", 12)

	var row: InventoryRow = InventoryRow.for_item(
		null, 5, "High", InventoryRow.ACTION_FIT, 0, "Module", 1, InventoryRow.SOURCE_SHIP_CARGO)
	_main._handle_inventory_row_drop(row, InventoryRow.SOURCE_FITTED, Vector2.ZERO)

	assert_int(connection.fit_calls.size()).is_equal(1)
	assert_int(connection.fit_calls[0]["module_id"] as int).is_equal(5)
	assert_str(connection.fit_calls[0]["slot"] as String).is_equal("High")
	connection.free()


func test_drag_from_ship_cargo_to_fitted_is_a_no_op_when_undocked() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_main._session.apply_dock_fitting(-1, "", 0)

	var row: InventoryRow = InventoryRow.for_item(
		null, 5, "High", InventoryRow.ACTION_FIT, 0, "Module", 1, InventoryRow.SOURCE_SHIP_CARGO)
	_main._handle_inventory_row_drop(row, InventoryRow.SOURCE_FITTED, Vector2.ZERO)

	assert_int(connection.fit_calls.size()).is_equal(0)
	connection.free()


func test_drag_from_fitted_to_ship_cargo_sends_unfit_command() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_main._session.apply_dock_fitting(3, "Forge Station", 12)

	var row: InventoryRow = InventoryRow.for_item(
		null, 5, "High", InventoryRow.ACTION_UNFIT, 0, "", 0, InventoryRow.SOURCE_FITTED)
	_main._handle_inventory_row_drop(row, InventoryRow.SOURCE_SHIP_CARGO, Vector2.ZERO)

	assert_int(connection.unfit_calls.size()).is_equal(1)
	assert_int(connection.unfit_calls[0]["module_id"] as int).is_equal(5)
	connection.free()


func test_drag_from_ship_cargo_to_station_sends_transfer_to_station_command() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_main._session.apply_dock_fitting(3, "Forge Station", 12)

	var row: InventoryRow = InventoryRow.for_item(
		null, 0, "", InventoryRow.ACTION_NONE, 0, "ScrapMetal", 4, InventoryRow.SOURCE_SHIP_CARGO)
	_main._handle_inventory_row_drop(row, InventoryRow.SOURCE_STATION, Vector2.ZERO)

	assert_int(connection.transfer_to_station_calls.size()).is_equal(1)
	assert_str(connection.transfer_to_station_calls[0]["item_type"] as String).is_equal("ScrapMetal")
	connection.free()


func test_drag_from_station_to_ship_cargo_sends_transfer_from_station_command() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_main._session.apply_dock_fitting(3, "Forge Station", 12)

	var row: InventoryRow = InventoryRow.for_item(
		null, 0, "", InventoryRow.ACTION_NONE, 0, "ScrapMetal", 4, InventoryRow.SOURCE_STATION)
	_main._handle_inventory_row_drop(row, InventoryRow.SOURCE_SHIP_CARGO, Vector2.ZERO)

	assert_int(connection.transfer_from_station_calls.size()).is_equal(1)
	assert_int(connection.transfer_from_station_calls[0]["station_id"] as int).is_equal(3)
	assert_str(connection.transfer_from_station_calls[0]["item_type"] as String).is_equal("ScrapMetal")
	connection.free()


func test_drag_dropped_back_onto_its_own_column_is_a_no_op() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_main._session.apply_dock_fitting(3, "Forge Station", 12)

	var row: InventoryRow = InventoryRow.for_item(
		null, 0, "", InventoryRow.ACTION_NONE, 0, "ScrapMetal", 4, InventoryRow.SOURCE_SHIP_CARGO)
	_main._handle_inventory_row_drop(row, InventoryRow.SOURCE_SHIP_CARGO, Vector2.ZERO)

	assert_int(connection.transfer_to_station_calls.size()).is_equal(0)
	assert_int(connection.transfer_from_station_calls.size()).is_equal(0)
	connection.free()


## Reordering needs the real built panel (inventory_panel_row_at() reads live
## Control rects), unlike the other drag cases above.
func test_drag_within_fitted_reorders_two_modules_of_the_same_slot_kind() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_main._session.apply_dock_fitting(3, "Forge Station", 12)
	var hud: CanvasLayer = auto_free(CanvasLayer.new())
	add_child(hud)
	_main._hud_surface.build(auto_free(Node.new()), hud, auto_free(Label.new()))

	var mid_1: Dictionary = _module_fixture(1, "Mid", false)
	var mid_2: Dictionary = _module_fixture(2, "Mid", false)
	mid_2["index"] = 1  # ModuleRow.index is the per-slot-kind position; _module_fixture defaults to 0
	_set_loadout_modules([mid_1, mid_2])
	_main._hud_surface.set_player_fitting(_main._loadout.modules(), [])
	## inventory_panel_row_at() (used by the reorder branch) short-circuits
	## to null while the panel is hidden -- it starts hidden by default.
	HudManager.toggle_inventory_panel(_main._hud_surface._inventory_panel_refs)
	await get_tree().process_frame

	var fitted_rows: Array[InventoryRow] = _main._hud_surface._inventory_panel_refs.fitted_rows
	var source_row: InventoryRow = fitted_rows[0]
	var target_row: InventoryRow = fitted_rows[1]
	var target_pos: Vector2 = (target_row.panel as Panel).get_global_rect().position + Vector2(2, 2)

	_main._handle_inventory_row_drop(source_row, InventoryRow.SOURCE_FITTED, target_pos)

	assert_int(connection.reorder_calls.size()).is_equal(1)
	assert_str(connection.reorder_calls[0]["slot"] as String).is_equal("Mid")
	assert_int(connection.reorder_calls[0]["from_index"] as int).is_equal(0)
	assert_int(connection.reorder_calls[0]["to_index"] as int).is_equal(1)
	connection.free()


func test_drag_within_fitted_across_different_slot_kinds_is_a_no_op() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_main._session.apply_dock_fitting(3, "Forge Station", 12)
	var hud: CanvasLayer = auto_free(CanvasLayer.new())
	add_child(hud)
	_main._hud_surface.build(auto_free(Node.new()), hud, auto_free(Label.new()))

	_set_loadout_modules([
		_module_fixture(1, "High", false),
		_module_fixture(2, "Mid", false),
	])
	_main._hud_surface.set_player_fitting(_main._loadout.modules(), [])
	HudManager.toggle_inventory_panel(_main._hud_surface._inventory_panel_refs)
	await get_tree().process_frame

	var fitted_rows: Array[InventoryRow] = _main._hud_surface._inventory_panel_refs.fitted_rows
	var source_row: InventoryRow = fitted_rows[0]
	var target_row: InventoryRow = fitted_rows[1]
	var target_pos: Vector2 = (target_row.panel as Panel).get_global_rect().position + Vector2(2, 2)

	_main._handle_inventory_row_drop(source_row, InventoryRow.SOURCE_FITTED, target_pos)

	assert_int(connection.reorder_calls.size()).is_equal(0)
	connection.free()


# -- Drag threshold (click vs. drop) -------------------------------------------

func test_release_within_threshold_of_press_is_treated_as_a_plain_click() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_main._session.apply_dock_fitting(3, "Forge Station", 12)

	_main._drag_row = InventoryRow.for_item(
		null, 5, "High", InventoryRow.ACTION_UNFIT, 0, "", 0, InventoryRow.SOURCE_FITTED)
	_main._drag_start_pos = Vector2(100, 100)
	_main._end_inventory_drag(Vector2(102, 101))  # well within DRAG_THRESHOLD_PX

	assert_int(connection.unfit_calls.size()).is_equal(1)
	assert_object(_main._drag_row).is_null()
	connection.free()


func test_release_past_threshold_is_treated_as_a_drop_not_a_click() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_main._session.apply_dock_fitting(3, "Forge Station", 12)
	var hud: CanvasLayer = auto_free(CanvasLayer.new())
	add_child(hud)
	_main._hud_surface.build(auto_free(Node.new()), hud, auto_free(Label.new()))
	HudManager.update_inventory_panel(_main._hud_surface._inventory_panel_refs, [], [], [], [], [])
	HudManager.toggle_inventory_panel(_main._hud_surface._inventory_panel_refs)
	await get_tree().process_frame

	var station_list: VBoxContainer = _main._hud_surface._inventory_panel_refs.station_list
	var far_pos: Vector2 = station_list.get_global_rect().position + Vector2(2, 2)

	_main._drag_row = InventoryRow.for_item(
		null, 0, "", InventoryRow.ACTION_NONE, 0, "ScrapMetal", 4, InventoryRow.SOURCE_SHIP_CARGO)
	_main._drag_start_pos = far_pos + Vector2(500, 500)  # far past DRAG_THRESHOLD_PX
	_main._end_inventory_drag(far_pos)

	assert_int(connection.transfer_to_station_calls.size()).is_equal(1)
	connection.free()
