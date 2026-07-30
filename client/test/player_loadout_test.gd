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
	assert_bool(loadout.apply_payload(JSON.stringify({
		"docked_station_id": 4,
		"docked_station_name": "Haven",
		"owned_ships": [{
			"ship_id": 9,
			"ship_type_id": null,
			"ship_type_name": null,
			"docked_station_id": null,
			"is_active": false,
		}],
	}))).is_true()
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


func test_dictionary_read_projections_are_removed_but_toggle_intent_remains_closed() -> void:
	var loadout := PlayerLoadout.new()
	assert_bool(loadout.has_method("hud_snapshot")).is_false()
	assert_bool(loadout.has_method("dock_status")).is_false()
	assert_bool(loadout.has_method("weapon_ranges")).is_false()
	assert_bool(loadout.has_method("toggle_at")).is_true()
	assert_dict(loadout.toggle_at(0)).is_empty()
