## player_loadout_test.gd
## GDExtension-boundary contract tests for PlayerLoadout read accessors.
extends GdUnitTestSuite

const __source: String = "PlayerLoadout"


func test_empty_state_preserves_explicit_scalar_sentinels() -> void:
	var loadout := PlayerLoadout.new()
	assert_int(loadout.docked_station_id()).is_equal(-1)
	assert_str(loadout.docked_station_name()).is_equal("")
	assert_bool(loadout.is_docked()).is_false()
	assert_float(loadout.weapon_optimal_range()).is_equal_approx(0.0, 0.0001)
	assert_float(loadout.weapon_falloff_range()).is_equal_approx(0.0, 0.0001)


func test_module_fixture_rejects_an_invalid_kind() -> void:
	var module := ModuleRow.test_fixture(
		"High", 0, 42, "Future Module", "SomeFutureModuleKind", false, true, 0.0, 5)
	assert_object(module).is_null()


func test_owned_ships_cross_the_boundary_as_typed_rows() -> void:
	var loadout := PlayerLoadout.new()
	var modules: Array[ModuleRow] = []
	var owned_ships: Array[OwnedShipRow] = [
		OwnedShipRow.test_fixture(9, -1, "", -1, false),
	]
	assert_bool(loadout.test_fixture(
		0, modules, 4, "Haven", -1, owned_ships
	)).is_true()
	var rows: Array = loadout.owned_ships()
	assert_int(rows.size()).is_equal(1)
	assert_bool(rows[0] is OwnedShipRow).is_true()
	var row: OwnedShipRow = rows[0]
	assert_int(row.ship_id).is_equal(9)
	assert_int(row.ship_type_id).is_equal(-1)
	assert_str(row.ship_type_name).is_equal("")
	assert_int(row.docked_station_id).is_equal(-1)
	assert_bool(row.is_active).is_false()
	assert_int(loadout.docked_station_id()).is_equal(4)
	assert_str(loadout.docked_station_name()).is_equal("Haven")
	assert_bool(loadout.is_docked()).is_true()


func test_module_toggle_returns_a_typed_activation_decision() -> void:
	var loadout := PlayerLoadout.new()
	assert_bool(loadout.has_method("hud_snapshot")).is_false()
	assert_bool(loadout.has_method("dock_status")).is_false()
	assert_bool(loadout.has_method("weapon_ranges")).is_false()
	assert_bool(loadout.has_method("resolve_activation")).is_true()

	var module := ModuleRow.test_fixture(
		"High", 0, 42, "Test Laser", "Weapon", false, true, 10.0, 5)
	assert_bool(loadout.test_fixture(
		0, [module], -1, "", -1, [])
	).is_true()

	var decision: ModuleActivationDecision = loadout.resolve_activation(0, true, 77, true, 0.0)
	assert_str(decision.action()).is_equal("activate")
	assert_int(decision.module_id()).is_equal(42)
	assert_str(decision.slot()).is_equal("High")
	assert_int(decision.target_ship_id()).is_equal(77)
	assert_str(decision.rejection_reason()).is_equal("")
	assert_str(loadout.resolve_activation(0, true, -1, false, 0.0).rejection_reason()).is_equal(
		"no_locked_target"
	)
	assert_str(loadout.resolve_activation(1, true, -1, false, 0.0).action()).is_equal("noop")
