## world_session_test.gd
##
## Tests for the public Rust-owned WorldSession seam. Server-driven state is
## created only by dispatching typed ServerMessageOutcome fixtures; detailed
## transition rules are tested directly against WorldSessionState in Rust.
extends GdUnitTestSuite

var _session
const AU_M: float = 1.495978707e11


class TypedOutcomeTarget:
	extends RefCounted
	var presentation: InitialStatePresentation
	var dock_accepted: bool = false

	func _on_initial_state(state: InitialStatePresentation) -> void:
		presentation = state

	func _handle_ship_docked(
		_ship_id: int, _station_id: int, _tick: int, session_accepted: bool
	) -> void:
		dock_accepted = session_accepted


func before_test() -> void:
	_session = WorldSession.new()


func after_test() -> void:
	_session = null


func _dispatch(kind: String, connection_ship_id: int = 11) -> TypedOutcomeTarget:
	var outcome: ServerMessageOutcome = ServerMessageDecoder.new().test_outcome(kind)
	var target := TypedOutcomeTarget.new()
	var loadout := PlayerLoadout.new()
	assert_object(outcome).is_not_null()
	assert_bool(outcome.dispatch(
		target, target, _session, loadout, connection_ship_id
	)).is_true()
	return target


func _apply_initial_state(connection_ship_id: int = 11) -> InitialStatePresentation:
	return _dispatch("InitialState", connection_ship_id).presentation


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
	assert_int(_session.ship_count()).is_equal(2)
	assert_bool(_session.has_ship(22)).is_true()


func test_ship_health_accessor_returns_typed_health_or_null() -> void:
	_apply_initial_state(11)
	var health: ShipHealth = _session.ship_health(22)

	assert_float(health.shield).is_equal_approx(210.0, 0.001)
	assert_float(health.max_shield).is_equal_approx(250.0, 0.001)
	assert_float(health.hull).is_equal_approx(110.0, 0.001)
	assert_object(_session.ship_health(99)).is_null()


func test_typed_dock_event_uses_the_production_state_application_path() -> void:
	_apply_initial_state(11)
	var target := _dispatch("ShipDocked", 11)

	assert_bool(target.dock_accepted).is_true()
	assert_bool(_session.is_docked()).is_true()
	assert_int(_session.docked_station_id()).is_equal(5)
	assert_str(_session.docked_station_name()).is_equal("Forge Station")
	assert_int(_session.latest_dock_state_tick()).is_equal(12)


func test_reset_clears_server_derived_state_and_restores_defaults() -> void:
	_apply_initial_state(11)
	_dispatch("ShipDocked", 11)
	_session.advance_client_ticks(3, PlayerLoadout.new())
	assert_bool(_session.is_docked()).is_true()
	assert_int(_session.current_tick()).is_equal(3)

	_session.reset()

	assert_int(_session.ship_count()).is_equal(0)
	assert_int(_session.player_ship_id()).is_equal(-1)
	assert_bool(_session.is_docked()).is_false()
	assert_int(_session.current_tick()).is_equal(0)
	assert_str(_session.current_system_name()).is_equal("Unknown")


func test_client_ticks_advance_capacitor_without_server_events() -> void:
	_apply_initial_state(11)
	var loadout := PlayerLoadout.new()
	var module := ModuleRow.test_fixture(
		"High", 0, 1, "Gun", "Weapon", true, true, 20.0, 10
	)
	var fixture_modules: Array[ModuleRow] = [module]
	var owned_ships: Array[OwnedShipRow] = []
	assert_bool(loadout.test_fixture(
		0, fixture_modules, -1, "", 11, owned_ships
	)).is_true()

	_session.advance_client_ticks(1, loadout)

	var modules: Array = loadout.modules()
	assert_int(_session.current_tick()).is_equal(1)
	assert_float(_session.capacitor_status().current).is_equal_approx(20.0, 0.001)
	assert_int((modules[0] as ModuleRow).cycle_remaining).is_equal(10)
