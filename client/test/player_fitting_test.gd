## player_fitting_test.gd
##
## Tests for the PlayerFitting client seam. These keep the raw wire Dictionary
## shape out of main.gd/HudManager tests.
extends GdUnitTestSuite

const PlayerFitting = preload("res://scripts/player_fitting.gd")


func test_normalize_payload_adds_client_runtime_fields() -> void:
	var fitting: Dictionary = PlayerFitting.normalize_payload({
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
			"stat_delta": {"weapon_range_add": 1200.0, "falloff_range_add": 300.0},
		}],
		"inventory": [{"module_id": 8, "name": "Afterburner", "kind": "Propulsion", "slot": "Mid"}],
	})

	var module: Dictionary = (fitting["modules"] as Array)[0] as Dictionary
	assert_int(module["module_id"]).is_equal(7)
	assert_str(module["forced_reason"]).is_equal("")
	assert_int(module["cycle_remaining"]).is_equal(0)
	assert_int(((fitting["inventory"] as Array)[0] as Dictionary)["module_id"]).is_equal(8)


func test_weapon_ranges_sum_active_weapon_modules_only() -> void:
	var modules: Array = [
		{"kind": "Weapon", "is_active": true, "stat_delta": {"weapon_range_add": 1000.0, "falloff_range_add": 250.0}},
		{"kind": "Weapon", "is_active": false, "stat_delta": {"weapon_range_add": 9999.0, "falloff_range_add": 9999.0}},
		{"kind": "Propulsion", "is_active": true, "stat_delta": {"weapon_range_add": 9999.0, "falloff_range_add": 9999.0}},
	]
	var ranges: Dictionary = PlayerFitting.weapon_ranges(modules)
	assert_float(ranges["optimal"]).is_equal_approx(1000.0, 0.001)
	assert_float(ranges["falloff"]).is_equal_approx(250.0, 0.001)


func test_set_module_activation_resets_cycle_and_marks_forced_reason() -> void:
	var modules: Array = [{"module_id": 4, "is_active": true, "cycle_remaining": 5, "forced_reason": ""}]
	PlayerFitting.set_module_activation(modules, 4, false, "cap")
	var module: Dictionary = modules[0] as Dictionary
	assert_bool(module["is_active"]).is_false()
	assert_int(module["cycle_remaining"]).is_equal(0)
	assert_str(module["forced_reason"]).is_equal("cap")


func test_active_module_toggle_at_skips_passive_modules() -> void:
	var modules: Array = [
		{"module_id": 1, "slot": "Low", "is_active_module": false, "is_active": true},
		{"module_id": 2, "slot": "High", "is_active_module": true, "is_active": false},
	]
	var toggle: Dictionary = PlayerFitting.active_module_toggle_at(modules, 0)
	assert_int(toggle["module_id"]).is_equal(2)
	assert_str(toggle["slot"]).is_equal("High")
	assert_bool(toggle["is_active"]).is_false()


func test_simulate_capacitor_ticks_starts_and_counts_down_cycles() -> void:
	var modules: Array = [{
		"is_active_module": true,
		"is_active": true,
		"cap_cost_per_cycle": 20.0,
		"cycle_time_ticks": 3,
		"cycle_remaining": 0,
	}]
	var cap: float = PlayerFitting.simulate_capacitor_ticks(modules, 50.0, 100.0, 10.0, 2)
	assert_float(cap).is_equal_approx(50.0, 0.001)
	assert_int((modules[0] as Dictionary)["cycle_remaining"]).is_equal(2)
