## input_decoder_test.gd
##
## Unit tests for input_decoder.gd's keyboard-shortcut decision logic.
## Pure function of (keycode, selection state) -> action dict, no scene
## tree or network needed.
extends GdUnitTestSuite

const __source: String = "res://scripts/input_decoder.gd"

const NO_SELECTION: int = -1


# -- F1-F8 -> toggle_module ---------------------------------------------------------

func test_f_keys_map_to_toggle_module_with_zero_based_index() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_F1, NO_SELECTION, -1, -1, -1, -1)
	assert_str(action.kind as String).is_equal("toggle_module")
	assert_int(action.module_index as int).is_equal(0)


func test_f8_maps_to_module_index_seven() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_F8, 1, -1, -1, -1, -1)
	assert_int(action.module_index as int).is_equal(7)


func test_f_keys_work_even_without_a_player_ship() -> void:
	## Module toggling has no _player_ship_id gate in the original logic.
	var action: Dictionary = InputDecoder.decode_key(KEY_F3, NO_SELECTION, -1, -1, -1, -1)
	assert_str(action.kind as String).is_equal("toggle_module")
	assert_int(action.module_index as int).is_equal(2)


# -- S key -> stop --------------------------------------------------------------------

func test_s_key_stops_when_a_player_ship_exists() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_S, 1, -1, -1, -1, -1)
	assert_str(action.kind as String).is_equal("stop")


func test_s_key_does_nothing_without_a_player_ship() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_S, NO_SELECTION, -1, -1, -1, -1)
	assert_str(action.kind as String).is_equal("none")


# -- J key -> jump ----------------------------------------------------------------------

func test_j_key_jumps_to_the_selected_gate_over_the_nearby_one() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_J, 1, 5, -1, -1, 9)
	assert_str(action.kind as String).is_equal("jump")
	assert_int(action.gate_id as int).is_equal(5)


func test_j_key_falls_back_to_the_nearby_gate_when_none_is_selected() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_J, 1, NO_SELECTION, -1, -1, 9)
	assert_str(action.kind as String).is_equal("jump")
	assert_int(action.gate_id as int).is_equal(9)


func test_j_key_does_nothing_when_no_gate_is_selected_or_nearby() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_J, 1, NO_SELECTION, -1, -1, NO_SELECTION)
	assert_str(action.kind as String).is_equal("none")


# -- A key -> approach ------------------------------------------------------------------

func test_a_key_approaches_the_selected_gate_over_a_selected_ship() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_A, 1, 5, 7, -1, -1)
	assert_str(action.kind as String).is_equal("approach_gate")
	assert_int(action.gate_id as int).is_equal(5)


func test_a_key_approaches_the_selected_ship_when_no_gate_is_selected() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_A, 1, NO_SELECTION, 7, -1, -1)
	assert_str(action.kind as String).is_equal("approach_ship")
	assert_int(action.ship_id as int).is_equal(7)


func test_a_key_does_nothing_without_a_selection() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_A, 1, NO_SELECTION, NO_SELECTION, -1, -1)
	assert_str(action.kind as String).is_equal("none")


# -- W key -> warp ----------------------------------------------------------------------

func test_w_key_warps_to_the_selected_gate_over_a_selected_body() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_W, 1, 5, -1, 3, -1)
	assert_str(action.kind as String).is_equal("warp_to_gate")
	assert_int(action.gate_id as int).is_equal(5)


func test_w_key_warps_to_the_selected_body_when_no_gate_is_selected() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_W, 1, NO_SELECTION, -1, 3, -1)
	assert_str(action.kind as String).is_equal("warp_to_body")
	assert_int(action.body_id as int).is_equal(3)


# -- O key -> orbit -----------------------------------------------------------------------

func test_o_key_orbits_the_selected_gate_over_a_selected_ship() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_O, 1, 5, 7, -1, -1)
	assert_str(action.kind as String).is_equal("orbit_gate")
	assert_int(action.gate_id as int).is_equal(5)


func test_o_key_orbits_the_selected_ship_when_no_gate_is_selected() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_O, 1, NO_SELECTION, 7, -1, -1)
	assert_str(action.kind as String).is_equal("orbit_ship")
	assert_int(action.ship_id as int).is_equal(7)


func test_o_key_does_nothing_without_a_selection() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_O, 1, NO_SELECTION, NO_SELECTION, -1, -1)
	assert_str(action.kind as String).is_equal("none")


# -- K key -> keep at range -----------------------------------------------------------------

func test_k_key_keeps_range_from_the_selected_gate_over_a_selected_ship() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_K, 1, 5, 7, -1, -1)
	assert_str(action.kind as String).is_equal("keep_at_range_gate")
	assert_int(action.gate_id as int).is_equal(5)


func test_k_key_keeps_range_from_the_selected_ship_when_no_gate_is_selected() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_K, 1, NO_SELECTION, 7, -1, -1)
	assert_str(action.kind as String).is_equal("keep_at_range_ship")
	assert_int(action.ship_id as int).is_equal(7)


func test_k_key_does_nothing_without_a_selection() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_K, 1, NO_SELECTION, NO_SELECTION, -1, -1)
	assert_str(action.kind as String).is_equal("none")


# -- [ / ] keys -> adjust keep-at-range distance ----------------------------------------

func test_bracket_left_decreases_keep_at_range_distance() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_BRACKETLEFT, 1, -1, -1, -1, -1)
	assert_str(action.kind as String).is_equal("adjust_keep_at_range")
	assert_float(action.delta_km as float).is_equal(-1.0)


func test_bracket_right_increases_keep_at_range_distance() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_BRACKETRIGHT, 1, -1, -1, -1, -1)
	assert_str(action.kind as String).is_equal("adjust_keep_at_range")
	assert_float(action.delta_km as float).is_equal(1.0)


func test_bracket_keys_do_nothing_without_a_player_ship() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_BRACKETLEFT, NO_SELECTION, -1, -1, -1, -1)
	assert_str(action.kind as String).is_equal("none")


# -- I key -> toggle inventory panel -----------------------------------------------------

func test_i_key_toggles_the_inventory_panel() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_I, 1, -1, -1, -1, -1)
	assert_str(action.kind as String).is_equal("toggle_inventory_panel")


func test_i_key_does_nothing_without_a_player_ship() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_I, NO_SELECTION, -1, -1, -1, -1)
	assert_str(action.kind as String).is_equal("none")


# -- D/U/B/Y keys -> station actions ---------------------------------------------------

func test_d_key_docks_at_the_nearby_station_when_not_already_docked() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_D, 1, -1, -1, -1, -1, 3, -1)
	assert_str(action.kind as String).is_equal("dock")
	assert_int(action.station_id as int).is_equal(3)


func test_u_key_undocks_when_currently_docked() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_U, 1, -1, -1, -1, -1, -1, 3)
	assert_str(action.kind as String).is_equal("undock")


func test_b_key_builds_a_packaged_ship_when_currently_docked() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_B, 1, -1, -1, -1, -1, -1, 3)
	assert_str(action.kind as String).is_equal("build_packaged_ship")
	assert_int(action.station_id as int).is_equal(3)


func test_y_key_disassembles_the_current_ship_when_docked() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_Y, 1, -1, -1, -1, -1, -1, 3)
	assert_str(action.kind as String).is_equal("disassemble_ship")
	assert_int(action.station_id as int).is_equal(3)


func test_x_key_disembarks_when_currently_docked() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_X, 1, -1, -1, -1, -1, -1, 3)
	assert_str(action.kind as String).is_equal("disembark")


func test_x_key_does_nothing_when_not_docked() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_X, 1, -1, -1, -1, -1, -1, -1)
	assert_str(action.kind as String).is_equal("none")


# -- Tab key -> tactical overlay ---------------------------------------------------------

func test_tab_key_toggles_the_tactical_overlay_even_without_a_player_ship() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_TAB, NO_SELECTION, -1, -1, -1, -1)
	assert_str(action.kind as String).is_equal("toggle_tactical_overlay")


# -- Unmapped keys -----------------------------------------------------------------------

func test_unmapped_key_does_nothing() -> void:
	var action: Dictionary = InputDecoder.decode_key(KEY_Z, 1, -1, -1, -1, -1)
	assert_str(action.kind as String).is_equal("none")
