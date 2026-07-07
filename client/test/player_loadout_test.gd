## player_loadout_test.gd
##
## Tests for the PlayerLoadout client seam. These keep the raw wire Dictionary
## shape out of main.gd/HudManager tests.
extends GdUnitTestSuite

const PlayerLoadoutScript = preload("res://scripts/player_loadout.gd")
const ModuleRow = preload("res://scripts/module_row.gd")
const ItemRow = preload("res://scripts/item_row.gd")


func test_apply_payload_adds_client_runtime_fields() -> void:
	var loadout := PlayerLoadoutScript.new()
	loadout.apply_payload({
		"modules": [{
			"slot": "High",
			"index": 0,
			"module_id": 7,
			"name": "Railgun",
			"kind": "Weapon",
			"is_active": true,
			"is_active_module": true,
			"cap_cost_per_cycle": 5.0,
			"cycle_time_ticks": 3,
			"stat_delta": {
				"weapon_range_add": 1200.0, "falloff_range_add": 300.0,
				"tackle_range_add": 20000.0, "repair_range_add": 15000.0,
			},
		}],
		"inventory": [
			{"item_type": "Module", "module_id": 8, "ship_type_id": 0, "name": "Afterburner", "kind": "Propulsion", "slot": "Mid", "count": 2},
			{"item_type": "ScrapMetal", "module_id": 0, "ship_type_id": 0, "name": "Scrap Metal", "kind": "", "slot": "", "count": 3},
		],
		"station_inventory": [
			{"item_type": "PackagedShip", "module_id": 0, "ship_type_id": 7, "name": "Magpie", "kind": "", "slot": "", "count": 1},
		],
		"tick": 12,
		"docked_station_id": 4,
		"docked_station_name": "Forge Station",
	})

	var snapshot: Dictionary = loadout.hud_snapshot()
	var module: ModuleRow = (snapshot["modules"] as Array)[0]
	assert_int(loadout.tick()).is_equal(12)
	assert_int(module.module_id).is_equal(7)
	assert_str(module.forced_reason).is_equal("")
	assert_int(module.cycle_remaining).is_equal(0)
	assert_int(((snapshot["inventory"] as Array)[0] as ItemRow).module_id).is_equal(8)
	assert_str(((snapshot["inventory"] as Array)[1] as ItemRow).item_type).is_equal("ScrapMetal")
	assert_int(((snapshot["inventory"] as Array)[1] as ItemRow).count).is_equal(3)
	assert_int(((snapshot["station_inventory"] as Array)[0] as ItemRow).ship_type_id).is_equal(7)
	var dock_status: Dictionary = loadout.dock_status()
	assert_int(dock_status["docked_station_id"] as int).is_equal(4)
	assert_str(dock_status["docked_station_name"] as String).is_equal("Forge Station")
	assert_float(module.stat_delta["tackle_range_add"]).is_equal_approx(20000.0, 0.001)
	assert_float(module.stat_delta["repair_range_add"]).is_equal_approx(15000.0, 0.001)


func test_apply_payload_keeps_undocked_station_context_at_negative_one() -> void:
	var loadout := PlayerLoadoutScript.new()
	loadout.apply_payload({
		"modules": [],
		"inventory": [],
		"station_inventory": [],
		"tick": 13,
		"docked_station_id": null,
		"docked_station_name": null,
	})

	var dock_status: Dictionary = loadout.dock_status()
	assert_int(dock_status["docked_station_id"] as int).is_equal(-1)
	assert_str(dock_status["docked_station_name"] as String).is_equal("")


func test_active_ship_id_reflects_the_wire_value() -> void:
	var loadout := PlayerLoadoutScript.new()
	loadout.apply_payload({
		"modules": [], "inventory": [], "station_inventory": [],
		"tick": 1, "docked_station_id": null, "docked_station_name": null,
		"active_ship_id": 42,
	})
	assert_int(loadout.active_ship_id()).is_equal(42)


func test_active_ship_id_is_negative_one_when_null_on_the_wire() -> void:
	var loadout := PlayerLoadoutScript.new()
	loadout.apply_payload({
		"modules": [], "inventory": [], "station_inventory": [],
		"tick": 1, "docked_station_id": null, "docked_station_name": null,
		"active_ship_id": null,
	})
	assert_int(loadout.active_ship_id()).is_equal(-1)


func test_owned_ships_reflects_the_wire_roster() -> void:
	var loadout := PlayerLoadoutScript.new()
	loadout.apply_payload({
		"modules": [], "inventory": [], "station_inventory": [],
		"tick": 1, "docked_station_id": null, "docked_station_name": null,
		"active_ship_id": 1,
		"owned_ships": [
			{"ship_id": 1, "ship_type_id": 7, "ship_type_name": "Magpie", "docked_station_id": 0, "is_active": true},
			{"ship_id": 2, "ship_type_id": 7, "ship_type_name": "Magpie", "docked_station_id": 0, "is_active": false},
		],
	})
	var ships: Array = loadout.owned_ships()
	assert_int(ships.size()).is_equal(2)
	assert_bool((ships[0] as Dictionary)["is_active"] as bool).is_true()
	assert_bool((ships[1] as Dictionary)["is_active"] as bool).is_false()


## Minimal but complete module row -- callers override only the keys the
## test cares about, so every fixture stays valid against ModuleRow's
## required-key schema without repeating the boilerplate everywhere.
func _module_json(overrides: Dictionary) -> Dictionary:
	var base: Dictionary = {
		"slot": "High", "index": 0, "module_id": 1, "name": "Test Module", "kind": "Weapon",
		"is_active": false, "is_active_module": true,
		"cap_cost_per_cycle": 0.0, "cycle_time_ticks": 10,
		"stat_delta": {},
	}
	for key: String in overrides:
		base[key] = overrides[key]
	return base


func test_weapon_ranges_sum_active_weapon_modules_only() -> void:
	var loadout := PlayerLoadoutScript.new()
	loadout.apply_payload({"modules": [
		_module_json({"module_id": 1, "kind": "Weapon", "is_active": true, "stat_delta": {"weapon_range_add": 1000.0, "falloff_range_add": 250.0}}),
		_module_json({"module_id": 2, "kind": "Weapon", "is_active": false, "stat_delta": {"weapon_range_add": 9999.0, "falloff_range_add": 9999.0}}),
		_module_json({"module_id": 3, "kind": "Propulsion", "is_active": true, "stat_delta": {"weapon_range_add": 9999.0, "falloff_range_add": 9999.0}}),
	]})
	var ranges: Dictionary = loadout.weapon_ranges()
	assert_float(ranges["optimal"]).is_equal_approx(1000.0, 0.001)
	assert_float(ranges["falloff"]).is_equal_approx(250.0, 0.001)


func test_effective_range_for_activation_includes_the_modules_own_contribution() -> void:
	## The module being activated isn't active yet, so it wouldn't otherwise
	## be counted -- effective_range_for_activation must include it anyway,
	## mirroring the server's tentative-apply-then-check (commands.rs).
	var loadout := PlayerLoadoutScript.new()
	loadout.apply_payload({"modules": [
		_module_json({"module_id": 1, "kind": "Weapon", "is_active": false,
			"stat_delta": {"weapon_range_add": 3000.0, "falloff_range_add": 2000.0}}),
	]})
	var effective_range: float = loadout.effective_range_for_activation("Weapon", 1)
	assert_float(effective_range).is_equal_approx(5000.0, 0.001)


func test_effective_range_for_activation_sums_other_already_active_modules_of_the_same_family() -> void:
	var loadout := PlayerLoadoutScript.new()
	loadout.apply_payload({"modules": [
		_module_json({"module_id": 1, "kind": "Weapon", "is_active": false,
			"stat_delta": {"weapon_range_add": 3000.0, "falloff_range_add": 2000.0}}),
		_module_json({"module_id": 2, "kind": "Weapon", "is_active": true,
			"stat_delta": {"weapon_range_add": 1000.0, "falloff_range_add": 500.0}}),
		## Inactive Weapon that isn't the one being activated must not count.
		_module_json({"module_id": 3, "kind": "Weapon", "is_active": false,
			"stat_delta": {"weapon_range_add": 9999.0, "falloff_range_add": 9999.0}}),
	]})
	var effective_range: float = loadout.effective_range_for_activation("Weapon", 1)
	assert_float(effective_range).is_equal_approx(6500.0, 0.001)


func test_effective_range_for_activation_uses_tackle_range_for_tackle_kind() -> void:
	var loadout := PlayerLoadoutScript.new()
	loadout.apply_payload({"modules": [
		_module_json({"module_id": 5, "kind": "Tackle", "is_active": false,
			"stat_delta": {"tackle_range_add": 20000.0}}),
	]})
	var effective_range: float = loadout.effective_range_for_activation("Tackle", 5)
	assert_float(effective_range).is_equal_approx(20000.0, 0.001)


func test_effective_range_for_activation_uses_repair_range_for_remote_repair_kinds() -> void:
	var loadout := PlayerLoadoutScript.new()
	loadout.apply_payload({"modules": [
		_module_json({"module_id": 6, "kind": "RemoteShieldBooster", "is_active": false,
			"stat_delta": {"repair_range_add": 15000.0}}),
	]})
	var effective_range: float = loadout.effective_range_for_activation("RemoteShieldBooster", 6)
	assert_float(effective_range).is_equal_approx(15000.0, 0.001)


func test_effective_range_for_activation_returns_negative_one_for_non_range_gated_kinds() -> void:
	var loadout := PlayerLoadoutScript.new()
	loadout.apply_payload({"modules": [
		_module_json({"module_id": 9, "kind": "ShieldBooster", "is_active": false, "stat_delta": {}}),
	]})
	assert_float(loadout.effective_range_for_activation("ShieldBooster", 9)).is_less(0.0)


func test_set_module_activation_resets_cycle_and_marks_forced_reason() -> void:
	var loadout := PlayerLoadoutScript.new()
	loadout.apply_payload({"modules": [
		_module_json({"module_id": 4, "is_active": true, "is_active_module": true}),
	]})
	loadout.apply_module_activation(4, false, "cap")
	var module: ModuleRow = loadout.modules()[0]
	assert_bool(module.is_active).is_false()
	assert_int(module.cycle_remaining).is_equal(0)
	assert_str(module.forced_reason).is_equal("cap")


func test_active_module_toggle_at_skips_passive_modules() -> void:
	var loadout := PlayerLoadoutScript.new()
	loadout.apply_payload({"modules": [
		_module_json({"module_id": 1, "slot": "Low", "kind": "Propulsion", "is_active_module": false, "is_active": true}),
		_module_json({"module_id": 2, "slot": "High", "kind": "Weapon", "is_active_module": true, "is_active": false}),
	]})
	var toggle: Dictionary = loadout.toggle_at(0)
	assert_int(toggle["module_id"]).is_equal(2)
	assert_str(toggle["slot"]).is_equal("High")
	assert_bool(toggle["is_active"]).is_false()


func test_simulate_capacitor_ticks_starts_and_counts_down_cycles() -> void:
	var loadout := PlayerLoadoutScript.new()
	loadout.apply_payload({"modules": [
		_module_json({"is_active_module": true, "is_active": true, "cap_cost_per_cycle": 20.0, "cycle_time_ticks": 3}),
	]})
	var cap: float = loadout.simulate_capacitor_ticks(50.0, 100.0, 10.0, 2)
	assert_float(cap).is_equal_approx(50.0, 0.001)
	assert_int((loadout.modules()[0] as ModuleRow).cycle_remaining).is_equal(2)


func test_module_row_from_json_drops_and_logs_a_row_missing_a_required_key() -> void:
	assert_object(ModuleRow.from_json(_module_json({}))).is_not_null()
	var incomplete: Dictionary = _module_json({})
	incomplete.erase("module_id")
	assert_object(ModuleRow.from_json(incomplete)).is_null()


func test_apply_payload_drops_an_invalid_module_row_instead_of_defaulting_it() -> void:
	var loadout := PlayerLoadoutScript.new()
	var incomplete: Dictionary = _module_json({})
	incomplete.erase("cap_cost_per_cycle")
	loadout.apply_payload({"modules": [incomplete]})
	assert_array(loadout.modules()).is_empty()


func test_item_row_from_json_drops_and_logs_a_row_missing_a_required_key() -> void:
	var complete: Dictionary = {
		"item_type": "Module", "module_id": 1, "ship_type_id": 0,
		"name": "Gun", "kind": "Weapon", "slot": "High", "count": 1,
	}
	assert_object(ItemRow.from_json(complete)).is_not_null()
	var incomplete: Dictionary = complete.duplicate()
	incomplete.erase("count")
	assert_object(ItemRow.from_json(incomplete)).is_null()
