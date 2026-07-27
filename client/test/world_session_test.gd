## world_session_test.gd
##
## Tests for the client WorldSession seam. These exercise live-world state
## ingestion without loading main.tscn or opening a WebSocket.
extends GdUnitTestSuite

var _session
const AU_M: float = 1.495978707e11


func before_test() -> void:
	_session = WorldSession.new()


func after_test() -> void:
	_session = null


func test_ingest_navigation_preserves_absolute_f64_positions() -> void:
	_session.ingest_navigation({
		"system_name": "Alpha",
		"systems": [{"id": 2, "name": "Beta"}],
		"jump_gates": [{
			"gate_id": 7,
			"position": {"x": 5.0 * AU_M + 10.0, "y": 20.0, "z": 30.0},
			"activation_radius": 1000.0,
			"to_system_name": "Beta",
		}],
		"stations": [{
			"station_id": 5,
			"name": "Forge Station",
			"position": {"x": 5.0 * AU_M + 20.0, "y": 21.0, "z": 31.0},
			"docking_radius": 5000.0,
		}],
		"celestial_bodies": [{
			"id": 9,
			"kind": "Star",
			"name": "Sun",
			"position": {"x": 5.0 * AU_M + 30.0, "y": 2.0, "z": 3.0},
			"radius": 42.0,
			"spectral_type": 0.5,
		}],
	})
	var snapshot: Dictionary = _session.snapshot()

	assert_str(snapshot.current_system_name).is_equal("Alpha")
	assert_str((snapshot.system_names as Dictionary)[2]).is_equal("Beta")
	var gate_pos: PackedFloat64Array = (snapshot.gates[0] as Dictionary)["position"]
	var station_pos: PackedFloat64Array = (snapshot.stations[0] as Dictionary)["position"]
	var body_pos: PackedFloat64Array = (snapshot.bodies[0] as Dictionary)["position"]
	assert_float(gate_pos[0]).is_equal_approx(5.0 * AU_M + 10.0, 0.001)
	assert_float(station_pos[0]).is_equal_approx(5.0 * AU_M + 20.0, 0.001)
	assert_float(body_pos[0]).is_equal_approx(5.0 * AU_M + 30.0, 0.001)
	assert_float(gate_pos[1]).is_equal_approx(20.0, 0.001)
	assert_float(station_pos[2]).is_equal_approx(31.0, 0.001)


func test_register_ship_promotes_connection_ship_to_player_state() -> void:
	var result: Dictionary = _session.register_ship(11, JSON.stringify({
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
	}), 11)
	var snapshot: Dictionary = _session.snapshot()

	assert_bool(result["became_player"]).is_true()
	assert_int(snapshot.player_ship_id).is_equal(11)
	assert_str(snapshot.player_ship_type_name).is_equal("Magpie")
	assert_float(snapshot.player_shield).is_equal_approx(80.0, 0.001)
	assert_float(snapshot.cap_current).is_equal_approx(55.0, 0.001)


func test_hp_event_updates_player_and_preserves_maxima() -> void:
	_session.register_ship(11, JSON.stringify({
		"ship_id": 11,
		"current_shield": 80.0,
		"current_armor": 70.0,
		"current_hull": 60.0,
		"max_shield": 100.0,
		"max_armor": 90.0,
		"max_hull": 80.0,
	}), 11)

	_session.apply_health_event(11, 40.0, 30.0, 20.0)
	var snapshot: Dictionary = _session.snapshot()

	var hp: Dictionary = (snapshot.ship_hp as Dictionary)[11] as Dictionary
	assert_float(hp["shield"]).is_equal_approx(40.0, 0.001)
	assert_float(hp["max_shield"]).is_equal_approx(100.0, 0.001)
	assert_float(snapshot.player_hull).is_equal_approx(20.0, 0.001)


func test_lock_events_only_change_player_locks() -> void:
	_session.set_player_ship_id(1)

	var snapshot: Dictionary = _session.snapshot()

	assert_bool(_session.apply_target_locked(2, 99)).is_false()
	assert_int(snapshot.player_lock_target).is_equal(-1)

	assert_bool(_session.apply_target_locked(1, 99)).is_true()
	assert_int(_session.snapshot().player_lock_target).is_equal(99)

	assert_bool(_session.apply_lock_lost(1, 99)).is_true()
	assert_int(_session.snapshot().player_lock_target).is_equal(-1)


func test_remove_ship_with_clear_lock_false_preserves_the_lock_target() -> void:
	## AoI leave (ADR-0019): the ship is still alive and still Locked
	## server-side (Lock has no distance-based expiry, lock.rs). Clearing
	## player_lock_target here would desync from the server -- a fresh
	## LockOnCommand the player sends afterward is silently ignored
	## server-side (already has_target()==true), so the lock would never
	## visibly complete again even once the target is back in view.
	_session.register_ship(42, JSON.stringify({}), -1)
	_session.set_player_ship_id(1)
	_session.apply_target_locked(1, 42)

	var result: Dictionary = _session.remove_ship(42, false)

	assert_bool(result["removed"] as bool).is_true()
	assert_int(_session.snapshot().player_lock_target).is_equal(42)
	assert_bool(_session.has_ship(42)).is_false()


func test_remove_ship_with_clear_lock_true_clears_the_lock_target() -> void:
	_session.register_ship(42, JSON.stringify({}), -1)
	_session.set_player_ship_id(1)
	_session.apply_target_locked(1, 42)

	var result: Dictionary = _session.remove_ship(42, true)

	assert_bool(result["removed"] as bool).is_true()
	assert_int(_session.snapshot().player_lock_target).is_equal(-1)


func test_client_ticks_advance_capacitor_without_server_events() -> void:
	_session.register_ship(11, JSON.stringify({
		"ship_id": 11,
		"cap_max": 100.0,
		"cap_recharge_per_tick": 5.0,
	}), 11)
	var loadout := PlayerLoadout.new()
	loadout.apply_payload(JSON.stringify({
		"modules": [{
			"slot": "High", "index": 0, "module_id": 1, "name": "Gun", "kind": "Weapon",
			"is_active_module": true,
			"is_active": true,
			"cap_cost_per_cycle": 20.0,
			"cycle_time_ticks": 10,
			"stat_delta": {},
		}],
	}))

	_session.advance_client_ticks(1, loadout)

	var snapshot: Dictionary = _session.snapshot()
	var modules: Array = loadout.modules()
	assert_int(snapshot.current_tick).is_equal(1)
	assert_float(snapshot.cap_current).is_equal_approx(80.0, 0.001)
	assert_int((modules[0] as ModuleRow).cycle_remaining).is_equal(10)


func test_destroying_opponent_reports_victory_candidate() -> void:
	_session.register_ship(22, JSON.stringify({"ship_id": 22, "is_player": true}), 11)

	var result: Dictionary = _session.destroy_ship(22)

	assert_bool(result["destroyed"]).is_true()
	assert_bool(result["destroyed_opponent"]).is_true()
	assert_bool(_session.has_ship(22)).is_false()


func test_dock_event_updates_player_dock_status() -> void:
	_session.set_player_ship_id(7)

	assert_bool(_session.apply_dock_event(7, 3, "Forge Station", 12)).is_true()

	var status: Dictionary = _session.dock_status()
	assert_bool(status["is_docked"] as bool).is_true()
	assert_int(status["docked_station_id"] as int).is_equal(3)
	assert_str(status["docked_station_name"] as String).is_equal("Forge Station")
	assert_int(status["latest_dock_state_tick"] as int).is_equal(12)


func test_undock_event_clears_player_dock_status() -> void:
	_session.set_player_ship_id(7)
	_session.apply_dock_event(7, 3, "Forge Station", 12)

	assert_bool(_session.apply_undock_event(7, 13)).is_true()

	var status: Dictionary = _session.dock_status()
	assert_bool(status["is_docked"] as bool).is_false()
	assert_int(status["docked_station_id"] as int).is_equal(-1)
	assert_str(status["docked_station_name"] as String).is_equal("")
	assert_int(status["latest_dock_state_tick"] as int).is_equal(13)


func test_older_fitting_dock_context_is_ignored_after_newer_undock() -> void:
	_session.set_player_ship_id(7)
	_session.apply_undock_event(7, 20)

	assert_bool(_session.apply_dock_fitting(3, "Forge Station", 19)).is_false()

	var status: Dictionary = _session.dock_status()
	assert_bool(status["is_docked"] as bool).is_false()
	assert_int(status["docked_station_id"] as int).is_equal(-1)


func test_dock_event_with_station_id_zero_is_treated_as_docked() -> void:
	# station_id 0 is a real station (the first one in a Sector) -- is_docked
	# must key off the tick guard / >= 0 comparison, not station_id truthiness.
	_session.set_player_ship_id(7)

	assert_bool(_session.apply_dock_event(7, 0, "Forge Station", 12)).is_true()

	var status: Dictionary = _session.dock_status()
	assert_bool(status["is_docked"] as bool).is_true()
	assert_int(status["docked_station_id"] as int).is_equal(0)


func test_stale_undock_event_does_not_revert_a_newer_dock_fitting_context() -> void:
	# The reverse direction of the race already covered above: a PlayerLoadout
	# fitting can also arrive *before* a delayed ShipUndocked event for an
	# already-superseded tick. The stale event must not revert the newer
	# dock context established by apply_dock_fitting.
	_session.set_player_ship_id(7)
	_session.apply_dock_fitting(5, "Forge Station", 20)

	assert_bool(_session.apply_undock_event(7, 15)).is_false()

	var status: Dictionary = _session.dock_status()
	assert_bool(status["is_docked"] as bool).is_true()
	assert_int(status["docked_station_id"] as int).is_equal(5)
	assert_str(status["docked_station_name"] as String).is_equal("Forge Station")
