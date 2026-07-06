## world_session_test.gd
##
## Tests for the client WorldSession seam. These exercise live-world state
## ingestion without loading main.tscn or opening a WebSocket.
extends GdUnitTestSuite

const WorldSession = preload("res://scripts/world_session.gd")
const ModuleRow = preload("res://scripts/module_row.gd")

var _session


func before_test() -> void:
	_session = WorldSession.new()


func after_test() -> void:
	_session = null


func test_ingest_navigation_normalizes_server_vectors() -> void:
	_session.ingest_navigation({
		"system_name": "Alpha",
		"systems": [{"id": 2, "name": "Beta"}],
		"jump_gates": [{
			"gate_id": 7,
			"position": {"x": 10.0, "y": 20.0, "z": 30.0},
			"activation_radius": 1000.0,
			"to_system_name": "Beta",
		}],
		"stations": [{
			"station_id": 5,
			"name": "Forge Station",
			"position": {"x": 11.0, "y": 21.0, "z": 31.0},
			"docking_radius": 5000.0,
		}],
		"celestial_bodies": [{
			"id": 9,
			"kind": "Star",
			"name": "Sun",
			"position": {"x": 1.0, "y": 2.0, "z": 3.0},
			"radius": 42.0,
			"spectral_type": 0.5,
		}],
	})

	assert_str(_session.current_system_name).is_equal("Alpha")
	assert_str(_session.system_names[2]).is_equal("Beta")
	assert_vector((_session.gates[0] as Dictionary)["position"]).is_equal(Vector3(10.0, 20.0, 30.0))
	assert_vector((_session.stations[0] as Dictionary)["position"]).is_equal(Vector3(11.0, 21.0, 31.0))
	assert_vector((_session.bodies[0] as Dictionary)["position"]).is_equal(Vector3(1.0, 2.0, 3.0))


func test_register_ship_promotes_connection_ship_to_player_state() -> void:
	var ship := Node3D.new()
	var result: Dictionary = _session.register_ship(11, ship, {
		"ship_id": 11,
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
	}, 11)

	assert_bool(result["became_player"]).is_true()
	assert_int(_session.player_ship_id).is_equal(11)
	assert_str(_session.player_ship_type_name).is_equal("Magpie")
	assert_float(_session.player_shield).is_equal_approx(80.0, 0.001)
	assert_float(_session.cap_current).is_equal_approx(55.0, 0.001)
	ship.free()


func test_hp_event_updates_player_and_preserves_maxima() -> void:
	var ship := Node3D.new()
	_session.register_ship(11, ship, {
		"ship_id": 11,
		"current_shield": 80.0,
		"current_armor": 70.0,
		"current_hull": 60.0,
		"max_shield": 100.0,
		"max_armor": 90.0,
		"max_hull": 80.0,
	}, 11)

	_session.apply_hp_event({
		"ship_id": 11,
		"current_shield": 40.0,
		"current_armor": 30.0,
		"current_hull": 20.0,
	})

	var hp: Dictionary = _session.ship_hp[11] as Dictionary
	assert_float(hp["shield"]).is_equal_approx(40.0, 0.001)
	assert_float(hp["max_shield"]).is_equal_approx(100.0, 0.001)
	assert_float(_session.player_hull).is_equal_approx(20.0, 0.001)
	ship.free()


func test_lock_events_only_change_player_locks() -> void:
	_session.player_ship_id = 1

	assert_bool(_session.apply_target_locked(2, 99)).is_false()
	assert_int(_session.player_lock_target).is_equal(-1)

	assert_bool(_session.apply_target_locked(1, 99)).is_true()
	assert_int(_session.player_lock_target).is_equal(99)

	assert_bool(_session.apply_lock_lost(1, 99)).is_true()
	assert_int(_session.player_lock_target).is_equal(-1)


func test_remove_ship_with_clear_lock_false_preserves_the_lock_target() -> void:
	## AoI leave (ADR-0019): the ship is still alive and still Locked
	## server-side (Lock has no distance-based expiry, lock.rs). Clearing
	## player_lock_target here would desync from the server -- a fresh
	## LockOnCommand the player sends afterward is silently ignored
	## server-side (already has_target()==true), so the lock would never
	## visibly complete again even once the target is back in view.
	var ship := Node3D.new()
	_session.register_ship(42, ship, {"ship_id": 42}, -1)
	_session.player_lock_target = 42

	var result: Dictionary = _session.remove_ship(42, false)

	assert_bool(result["removed"] as bool).is_true()
	assert_int(_session.player_lock_target).is_equal(42)
	assert_bool(_session.ships.has(42)).is_false()
	ship.free()


func test_remove_ship_with_clear_lock_true_clears_the_lock_target() -> void:
	var ship := Node3D.new()
	_session.register_ship(42, ship, {"ship_id": 42}, -1)
	_session.player_lock_target = 42

	var result: Dictionary = _session.remove_ship(42)

	assert_bool(result["removed"] as bool).is_true()
	assert_int(_session.player_lock_target).is_equal(-1)
	ship.free()


func test_client_ticks_advance_capacitor_without_server_events() -> void:
	var ship := Node3D.new()
	_session.register_ship(11, ship, {
		"ship_id": 11,
		"cap_max": 100.0,
		"cap_recharge_per_tick": 5.0,
	}, 11)
	var modules: Array[ModuleRow] = [ModuleRow.from_json({
		"slot": "High", "index": 0, "module_id": 1, "name": "Gun", "kind": "Weapon",
		"is_active_module": true,
		"is_active": true,
		"cap_cost_per_cycle": 20.0,
		"cycle_time_ticks": 10,
		"stat_delta": {},
	})]

	_session.advance_client_ticks(1, modules)

	assert_int(_session.current_tick).is_equal(1)
	assert_float(_session.cap_current).is_equal_approx(80.0, 0.001)
	assert_int(modules[0].cycle_remaining).is_equal(10)
	ship.free()


func test_destroying_opponent_reports_victory_candidate() -> void:
	var ship := Node3D.new()
	_session.register_ship(22, ship, {"ship_id": 22, "is_player": true}, 11)

	var result: Dictionary = _session.destroy_ship(22)

	assert_bool(result["destroyed"]).is_true()
	assert_bool(result["destroyed_opponent"]).is_true()
	assert_bool(_session.has_ship(22)).is_false()
	ship.free()


func test_dock_event_updates_player_dock_status() -> void:
	_session.player_ship_id = 7

	assert_bool(_session.apply_dock_event(7, 3, "Forge Station", 12)).is_true()

	var status: Dictionary = _session.dock_status()
	assert_bool(status["is_docked"] as bool).is_true()
	assert_int(status["docked_station_id"] as int).is_equal(3)
	assert_str(status["docked_station_name"] as String).is_equal("Forge Station")
	assert_int(status["latest_dock_state_tick"] as int).is_equal(12)


func test_undock_event_clears_player_dock_status() -> void:
	_session.player_ship_id = 7
	_session.apply_dock_event(7, 3, "Forge Station", 12)

	assert_bool(_session.apply_undock_event(7, 13)).is_true()

	var status: Dictionary = _session.dock_status()
	assert_bool(status["is_docked"] as bool).is_false()
	assert_int(status["docked_station_id"] as int).is_equal(-1)
	assert_str(status["docked_station_name"] as String).is_equal("")
	assert_int(status["latest_dock_state_tick"] as int).is_equal(13)


func test_older_fitting_dock_context_is_ignored_after_newer_undock() -> void:
	_session.player_ship_id = 7
	_session.apply_undock_event(7, 20)

	assert_bool(_session.apply_dock_fitting(3, "Forge Station", 19)).is_false()

	var status: Dictionary = _session.dock_status()
	assert_bool(status["is_docked"] as bool).is_false()
	assert_int(status["docked_station_id"] as int).is_equal(-1)
