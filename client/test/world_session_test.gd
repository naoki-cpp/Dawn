## world_session_test.gd
##
## Tests for the Rust-owned client WorldSession seam. Initial world data is
## applied through a typed ServerMessageOutcome rather than a Dictionary.
extends GdUnitTestSuite

var _session
const AU_M: float = 1.495978707e11


class InitialStateTarget:
	extends RefCounted
	var presentation: InitialStatePresentation

	func _accept_initial_state(state: InitialStatePresentation) -> void:
		presentation = state


func before_test() -> void:
	_session = WorldSession.new()


func after_test() -> void:
	_session = null


func _apply_initial_state(connection_ship_id: int = 11) -> InitialStatePresentation:
	var outcome: ServerMessageOutcome = ServerMessageDecoder.new().test_outcome("InitialState")
	var target := InitialStateTarget.new()
	var loadout := PlayerLoadout.new()
	assert_bool(outcome.dispatch(target, _session, loadout, connection_ship_id)).is_true()
	return target.presentation


func test_typed_initial_state_preserves_absolute_f64_navigation_positions() -> void:
	var presentation := _apply_initial_state()
	assert_int(presentation.ships.size()).is_equal(2)
	assert_str(_session.current_system_name()).is_equal("Alpha")
	assert_str((_session.system_names() as Dictionary)[2]).is_equal("Beta")
	var gate_pos: PackedFloat64Array = (_session.gates()[0] as GateRecord).position
	var station_pos: PackedFloat64Array = (_session.stations()[0] as StationRecord).position
	var body_pos: PackedFloat64Array = (_session.bodies()[0] as CelestialBodyRecord).position
	assert_float(gate_pos[0]).is_equal_approx(5.0 * AU_M + 10.0, 0.001)
	assert_float(station_pos[0]).is_equal_approx(5.0 * AU_M + 10.0, 0.001)
	assert_float(body_pos[0]).is_equal_approx(5.0 * AU_M + 10.0, 0.001)
	assert_float(gate_pos[1]).is_equal_approx(20.0, 0.001)
	assert_float(station_pos[2]).is_equal_approx(30.0, 0.001)


func test_typed_initial_state_promotes_connection_ship_to_player_state() -> void:
	_apply_initial_state(11)
	assert_int(_session.player_ship_id()).is_equal(11)
	assert_str(_session.player_ship_type_name()).is_equal("Magpie")
	assert_float(_session.player_health().shield).is_equal_approx(70.0, 0.001)
	assert_float(_session.capacitor_status().current).is_equal_approx(40.0, 0.001)


func test_hp_event_updates_player_and_preserves_maxima() -> void:
	_apply_initial_state(11)
	_session.apply_health_event(11, 40.0, 30.0, 20.0)
	var hp: ShipHealth = _session.ship_health(11)
	assert_float(hp.shield).is_equal_approx(40.0, 0.001)
	assert_float(hp.max_shield).is_equal_approx(100.0, 0.001)
	assert_float(_session.player_health().hull).is_equal_approx(20.0, 0.001)


func test_lock_events_only_change_player_locks() -> void:
	_session.set_player_ship_id(1)
	assert_bool(_session.apply_target_locked(2, 99)).is_false()
	assert_int(_session.player_lock_target()).is_equal(-1)
	assert_bool(_session.apply_target_locked(1, 99)).is_true()
	assert_int(_session.player_lock_target()).is_equal(99)
	assert_bool(_session.apply_lock_lost(1, 99)).is_true()
	assert_int(_session.player_lock_target()).is_equal(-1)


func test_remove_ship_with_clear_lock_false_preserves_the_lock_target() -> void:
	_apply_initial_state(-1)
	_session.set_player_ship_id(1)
	_session.apply_target_locked(1, 11)
	var result: bool = _session.remove_ship(11, false)
	assert_bool(result).is_true()
	assert_int(_session.player_lock_target()).is_equal(11)
	assert_bool(_session.has_ship(11)).is_false()


func test_remove_ship_with_clear_lock_true_clears_the_lock_target() -> void:
	_apply_initial_state(-1)
	_session.set_player_ship_id(1)
	_session.apply_target_locked(1, 11)
	var result: bool = _session.remove_ship(11, true)
	assert_bool(result).is_true()
	assert_int(_session.player_lock_target()).is_equal(-1)


func test_client_ticks_advance_capacitor_without_server_events() -> void:
	_apply_initial_state(11)
	var loadout := PlayerLoadout.new()
	var module := ModuleRow.test_fixture(
		"High", 0, 1, "Gun", "Weapon", true, true, 20.0, 10
	)
	var fixture_modules: Array[ModuleRow] = [module]
	var owned_ships: Array[OwnedShipRow] = []
	assert_bool(loadout.test_fixture(
		0, fixture_modules, -1, "", -1, owned_ships
	)).is_true()
	_session.advance_client_ticks(1, loadout)
	var modules: Array = loadout.modules()
	assert_int(_session.current_tick()).is_equal(1)
	assert_float(_session.capacitor_status().current).is_equal_approx(20.0, 0.001)
	assert_int((modules[0] as ModuleRow).cycle_remaining).is_equal(10)


func test_destroying_opponent_reports_victory_candidate() -> void:
	_apply_initial_state(-1)
	var result: DestructionOutcome = _session.destroy_ship(11)
	assert_bool(result.destroyed).is_true()
	assert_bool(result.destroyed_opponent).is_true()
	assert_bool(_session.has_ship(11)).is_false()


func test_dock_event_updates_player_dock_status() -> void:
	_session.set_player_ship_id(7)

	assert_bool(_session.apply_dock_event(7, 3, "Forge Station", 12)).is_true()

	assert_bool(_session.is_docked()).is_true()
	assert_int(_session.docked_station_id()).is_equal(3)
	assert_str(_session.docked_station_name()).is_equal("Forge Station")
	assert_int(_session.latest_dock_state_tick()).is_equal(12)


func test_undock_event_clears_player_dock_status() -> void:
	_session.set_player_ship_id(7)
	_session.apply_dock_event(7, 3, "Forge Station", 12)

	assert_bool(_session.apply_undock_event(7, 13)).is_true()

	assert_bool(_session.is_docked()).is_false()
	assert_int(_session.docked_station_id()).is_equal(-1)
	assert_str(_session.docked_station_name()).is_equal("")
	assert_int(_session.latest_dock_state_tick()).is_equal(13)


func test_older_fitting_dock_context_is_ignored_after_newer_undock() -> void:
	_session.set_player_ship_id(7)
	_session.apply_undock_event(7, 20)

	assert_bool(_session.apply_dock_fitting(3, "Forge Station", 19)).is_false()

	assert_bool(_session.is_docked()).is_false()
	assert_int(_session.docked_station_id()).is_equal(-1)


func test_dock_event_with_station_id_zero_is_treated_as_docked() -> void:
	# station_id 0 is a real station (the first one in a Sector) -- is_docked
	# must key off the tick guard / >= 0 comparison, not station_id truthiness.
	_session.set_player_ship_id(7)

	assert_bool(_session.apply_dock_event(7, 0, "Forge Station", 12)).is_true()

	assert_bool(_session.is_docked()).is_true()
	assert_int(_session.docked_station_id()).is_equal(0)


func test_stale_undock_event_does_not_revert_a_newer_dock_fitting_context() -> void:
	# The reverse direction of the race already covered above: a PlayerLoadout
	# fitting can also arrive *before* a delayed ShipUndocked event for an
	# already-superseded tick. The stale event must not revert the newer
	# dock context established by apply_dock_fitting.
	_session.set_player_ship_id(7)
	_session.apply_dock_fitting(5, "Forge Station", 20)

	assert_bool(_session.apply_undock_event(7, 15)).is_false()

	assert_bool(_session.is_docked()).is_true()
	assert_int(_session.docked_station_id()).is_equal(5)
	assert_str(_session.docked_station_name()).is_equal("Forge Station")
