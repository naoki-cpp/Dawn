## player_loadout_test.gd
##
## Tests for the PlayerLoadout client seam. These keep the raw wire Dictionary
## shape out of main.gd/HudManager tests.
extends GdUnitTestSuite

const PlayerLoadoutScript = preload("res://scripts/player_loadout.gd")


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
			{"item_type": "Module", "module_id": 8, "name": "Afterburner", "kind": "Propulsion", "slot": "Mid", "count": 2},
			{"item_type": "ScrapMetal", "name": "Scrap Metal", "count": 3},
		],
		"station_inventory": [
			{"item_type": "PackagedShip", "ship_type_id": 7, "name": "Magpie", "count": 1},
		],
		"tick": 12,
		"docked_station_id": 4,
		"docked_station_name": "Forge Station",
	})

	var snapshot: Dictionary = loadout.hud_snapshot()
	var module: Dictionary = (snapshot["modules"] as Array)[0] as Dictionary
	assert_int(loadout.tick()).is_equal(12)
	assert_int(module["module_id"]).is_equal(7)
	assert_str(module["forced_reason"]).is_equal("")
	assert_int(module["cycle_remaining"]).is_equal(0)
	assert_int(((snapshot["inventory"] as Array)[0] as Dictionary)["module_id"]).is_equal(8)
	assert_str(((snapshot["inventory"] as Array)[1] as Dictionary)["item_type"]).is_equal("ScrapMetal")
	assert_int(((snapshot["inventory"] as Array)[1] as Dictionary)["count"]).is_equal(3)
	assert_int(((snapshot["station_inventory"] as Array)[0] as Dictionary)["ship_type_id"]).is_equal(7)
	var dock_status: Dictionary = loadout.dock_status()
	assert_int(dock_status["docked_station_id"] as int).is_equal(4)
	assert_str(dock_status["docked_station_name"] as String).is_equal("Forge Station")
	var stat_delta: Dictionary = module["stat_delta"] as Dictionary
	assert_float(stat_delta["tackle_range_add"]).is_equal_approx(20000.0, 0.001)
	assert_float(stat_delta["repair_range_add"]).is_equal_approx(15000.0, 0.001)


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


func test_weapon_ranges_sum_active_weapon_modules_only() -> void:
	var loadout := PlayerLoadoutScript.new()
	loadout.apply_payload({"modules": [
		{"kind": "Weapon", "is_active": true, "is_active_module": true, "stat_delta": {"weapon_range_add": 1000.0, "falloff_range_add": 250.0}},
		{"kind": "Weapon", "is_active": false, "is_active_module": true, "stat_delta": {"weapon_range_add": 9999.0, "falloff_range_add": 9999.0}},
		{"kind": "Propulsion", "is_active": true, "is_active_module": true, "stat_delta": {"weapon_range_add": 9999.0, "falloff_range_add": 9999.0}},
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
		{"module_id": 1, "kind": "Weapon", "is_active": false, "is_active_module": true,
			"stat_delta": {"weapon_range_add": 3000.0, "falloff_range_add": 2000.0}},
	]})
	var effective_range: float = loadout.effective_range_for_activation("Weapon", 1)
	assert_float(effective_range).is_equal_approx(5000.0, 0.001)


func test_effective_range_for_activation_sums_other_already_active_modules_of_the_same_family() -> void:
	var loadout := PlayerLoadoutScript.new()
	loadout.apply_payload({"modules": [
		{"module_id": 1, "kind": "Weapon", "is_active": false,
			"stat_delta": {"weapon_range_add": 3000.0, "falloff_range_add": 2000.0}},
		{"module_id": 2, "kind": "Weapon", "is_active": true,
			"stat_delta": {"weapon_range_add": 1000.0, "falloff_range_add": 500.0}},
		## Inactive Weapon that isn't the one being activated must not count.
		{"module_id": 3, "kind": "Weapon", "is_active": false,
			"stat_delta": {"weapon_range_add": 9999.0, "falloff_range_add": 9999.0}},
	]})
	var effective_range: float = loadout.effective_range_for_activation("Weapon", 1)
	assert_float(effective_range).is_equal_approx(6500.0, 0.001)


func test_effective_range_for_activation_uses_tackle_range_for_tackle_kind() -> void:
	var loadout := PlayerLoadoutScript.new()
	loadout.apply_payload({"modules": [
		{"module_id": 5, "kind": "Tackle", "is_active": false,
			"stat_delta": {"tackle_range_add": 20000.0}},
	]})
	var effective_range: float = loadout.effective_range_for_activation("Tackle", 5)
	assert_float(effective_range).is_equal_approx(20000.0, 0.001)


func test_effective_range_for_activation_uses_repair_range_for_remote_repair_kinds() -> void:
	var loadout := PlayerLoadoutScript.new()
	loadout.apply_payload({"modules": [
		{"module_id": 6, "kind": "RemoteShieldBooster", "is_active": false,
			"stat_delta": {"repair_range_add": 15000.0}},
	]})
	var effective_range: float = loadout.effective_range_for_activation("RemoteShieldBooster", 6)
	assert_float(effective_range).is_equal_approx(15000.0, 0.001)


func test_effective_range_for_activation_returns_negative_one_for_non_range_gated_kinds() -> void:
	var loadout := PlayerLoadoutScript.new()
	loadout.apply_payload({"modules": [{"module_id": 9, "kind": "ShieldBooster", "is_active": false, "stat_delta": {}}]})
	assert_float(loadout.effective_range_for_activation("ShieldBooster", 9)).is_less(0.0)


func test_set_module_activation_resets_cycle_and_marks_forced_reason() -> void:
	var loadout := PlayerLoadoutScript.new()
	loadout.apply_payload({"modules": [{"module_id": 4, "is_active": true, "is_active_module": true, "cycle_remaining": 5, "forced_reason": ""}]})
	loadout.apply_module_activation(4, false, "cap")
	var module: Dictionary = (loadout.modules()[0] as Dictionary)
	assert_bool(module["is_active"]).is_false()
	assert_int(module["cycle_remaining"]).is_equal(0)
	assert_str(module["forced_reason"]).is_equal("cap")


func test_active_module_toggle_at_skips_passive_modules() -> void:
	var loadout := PlayerLoadoutScript.new()
	loadout.apply_payload({"modules": [
		{"module_id": 1, "slot": "Low", "is_active_module": false, "is_active": true},
		{"module_id": 2, "slot": "High", "kind": "Weapon", "is_active_module": true, "is_active": false},
	]})
	var toggle: Dictionary = loadout.toggle_at(0)
	assert_int(toggle["module_id"]).is_equal(2)
	assert_str(toggle["slot"]).is_equal("High")
	assert_bool(toggle["is_active"]).is_false()


func test_simulate_capacitor_ticks_starts_and_counts_down_cycles() -> void:
	var loadout := PlayerLoadoutScript.new()
	loadout.apply_payload({"modules": [{
		"is_active_module": true,
		"is_active": true,
		"cap_cost_per_cycle": 20.0,
		"cycle_time_ticks": 3,
		"cycle_remaining": 0,
	}]})
	var cap: float = loadout.simulate_capacitor_ticks(50.0, 100.0, 10.0, 2)
	assert_float(cap).is_equal_approx(50.0, 0.001)
	assert_int((loadout.modules()[0] as Dictionary)["cycle_remaining"]).is_equal(2)
