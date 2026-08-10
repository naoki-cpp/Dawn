## Unit tests for the typed keyboard-intent policy.
extends GdUnitTestSuite


const NO_SHIP: int = -1


func _decode(
	keycode: Key,
	player_ship_id: int,
	selection: ClientSelection,
	nearby_gate_id: int = -1,
	nearby_station_id: int = -1,
	docked_station_id: int = -1
) -> ClientIntent:
	return InputDecoder.decode_key(
		keycode,
		player_ship_id,
		selection,
		nearby_gate_id,
		nearby_station_id,
		docked_station_id)


func test_f_keys_return_typed_module_intents() -> void:
	var intent := _decode(KEY_F1, NO_SHIP, ClientSelection.none())
	assert_bool(intent.is_toggle_module()).is_true()
	assert_int(intent.module_index()).is_equal(0)

	intent = _decode(KEY_F8, 1, ClientSelection.none())
	assert_bool(intent.is_toggle_module()).is_true()
	assert_int(intent.module_index()).is_equal(7)


func test_stop_requires_a_player_ship() -> void:
	assert_bool(_decode(KEY_S, 1, ClientSelection.none()).is_stop()).is_true()
	assert_bool(_decode(KEY_S, NO_SHIP, ClientSelection.none()).is_none()).is_true()


func test_jump_prefers_selected_gate_then_nearby_gate() -> void:
	var intent := _decode(KEY_J, 1, ClientSelection.gate(5), 9)
	assert_bool(intent.is_jump()).is_true()
	assert_int(intent.gate_id()).is_equal(5)

	intent = _decode(KEY_J, 1, ClientSelection.none(), 9)
	assert_bool(intent.is_jump()).is_true()
	assert_int(intent.gate_id()).is_equal(9)

	assert_bool(_decode(KEY_J, 1, ClientSelection.none()).is_none()).is_true()


func test_approach_returns_a_typed_target_intent() -> void:
	var intent := _decode(KEY_A, 1, ClientSelection.gate(5))
	assert_bool(intent.is_approach_gate()).is_true()
	assert_int(intent.gate_id()).is_equal(5)

	intent = _decode(KEY_A, 1, ClientSelection.ship(7))
	assert_bool(intent.is_approach_ship()).is_true()
	assert_int(intent.ship_id()).is_equal(7)

	assert_bool(_decode(KEY_A, 1, ClientSelection.none()).is_none()).is_true()


func test_warp_and_orbit_keep_navigation_targets_typed() -> void:
	var intent := _decode(KEY_W, 1, ClientSelection.gate(5))
	assert_bool(intent.is_warp_to_gate()).is_true()
	assert_int(intent.gate_id()).is_equal(5)

	intent = _decode(KEY_W, 1, ClientSelection.body(3))
	assert_bool(intent.is_warp_to_body()).is_true()
	assert_int(intent.body_id()).is_equal(3)

	intent = _decode(KEY_O, 1, ClientSelection.gate(5))
	assert_bool(intent.is_orbit_gate()).is_true()
	assert_int(intent.gate_id()).is_equal(5)

	intent = _decode(KEY_O, 1, ClientSelection.ship(7))
	assert_bool(intent.is_orbit_ship()).is_true()
	assert_int(intent.ship_id()).is_equal(7)


func test_keep_at_range_and_adjustment_are_typed() -> void:
	var intent := _decode(KEY_K, 1, ClientSelection.gate(5))
	assert_bool(intent.is_keep_at_range_gate()).is_true()
	assert_int(intent.gate_id()).is_equal(5)

	intent = _decode(KEY_K, 1, ClientSelection.ship(7))
	assert_bool(intent.is_keep_at_range_ship()).is_true()
	assert_int(intent.ship_id()).is_equal(7)

	intent = _decode(KEY_BRACKETLEFT, 1, ClientSelection.none())
	assert_bool(intent.is_adjust_keep_at_range()).is_true()
	assert_float(intent.delta_km()).is_equal(-1.0)

	intent = _decode(KEY_BRACKETRIGHT, 1, ClientSelection.none())
	assert_float(intent.delta_km()).is_equal(1.0)


func test_panel_and_overlay_intents_do_not_require_dictionary_tags() -> void:
	assert_bool(_decode(KEY_I, NO_SHIP, ClientSelection.none()).is_toggle_inventory_panel()).is_true()
	assert_bool(_decode(KEY_M, 1, ClientSelection.none(), -1, -1, 3).is_toggle_market_panel()).is_true()
	assert_bool(_decode(KEY_M, 1, ClientSelection.none()).is_none()).is_true()
	assert_bool(_decode(KEY_TAB, NO_SHIP, ClientSelection.none()).is_toggle_tactical_overlay()).is_true()


func test_station_intents_carry_only_their_station_identity() -> void:
	var intent := _decode(KEY_D, 1, ClientSelection.none(), -1, 3)
	assert_bool(intent.is_dock()).is_true()
	assert_int(intent.station_id()).is_equal(3)

	assert_bool(_decode(KEY_U, 1, ClientSelection.none(), -1, -1, 3).is_undock()).is_true()
	intent = _decode(KEY_B, 1, ClientSelection.none(), -1, -1, 3)
	assert_bool(intent.is_build_packaged_ship()).is_true()
	assert_int(intent.station_id()).is_equal(3)

	intent = _decode(KEY_Y, 1, ClientSelection.none(), -1, -1, 3)
	assert_bool(intent.is_disassemble_ship()).is_true()
	assert_int(intent.station_id()).is_equal(3)
	assert_bool(_decode(KEY_X, 1, ClientSelection.none(), -1, -1, 3).is_disembark()).is_true()
	assert_bool(_decode(KEY_X, 1, ClientSelection.none()).is_none()).is_true()


func test_unmapped_key_returns_the_typed_none_intent() -> void:
	assert_bool(_decode(KEY_Z, 1, ClientSelection.none()).is_none()).is_true()
