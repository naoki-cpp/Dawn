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


func test_module_toggle_uses_a_typed_intent_record() -> void:
	var loadout := PlayerLoadout.new()
	assert_bool(loadout.has_method("hud_snapshot")).is_false()
	assert_bool(loadout.has_method("dock_status")).is_false()
	assert_bool(loadout.has_method("weapon_ranges")).is_false()
	assert_bool(loadout.has_method("toggle_at")).is_true()
	assert_bool(loadout.toggle_at(0).is_none()).is_true()

	var module := ModuleRow.test_fixture(
		"High", 0, 42, "Test Laser", "Weapon", false, true, 10.0, 5)
	assert_bool(loadout.test_fixture(
		0, [module], -1, "", -1, [])
	).is_true()

	var intent: ModuleActivationIntent = loadout.toggle_at(0)
	assert_bool(intent.is_none()).is_false()
	assert_int(intent.module_id()).is_equal(42)
	assert_str(intent.slot()).is_equal("High")
	assert_bool(intent.is_active()).is_false()
	assert_bool(intent.requires_target()).is_true()
	assert_bool(intent.has_effective_range()).is_true()
	assert_float(intent.effective_range()).is_equal_approx(0.0, 0.0001)
	assert_bool(loadout.toggle_at(1).is_none()).is_true()
