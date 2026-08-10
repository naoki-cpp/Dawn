## Unit tests for typed selection ownership and world interaction intents.
extends GdUnitTestSuite


const __source: String = "res://scripts/world_interaction.gd"

var _interaction: WorldInteraction


func before_test() -> void:
	_interaction = load(__source).new()


func test_primary_click_selects_ship_over_other_candidates() -> void:
	var intent := _interaction.interpret_primary_click(
		Vector2(100.0, 50.0), 1.0, false, 7, 42, 9, 3)

	assert_bool(intent.is_selection_changed()).is_true()
	assert_int(_interaction.selected_target_id()).is_equal(42)
	assert_int(_interaction.selected_gate_id()).is_equal(-1)
	assert_int(_interaction.selected_body_id()).is_equal(-1)


func test_primary_click_selects_gate_when_no_ship_is_hit() -> void:
	var intent := _interaction.interpret_primary_click(
		Vector2(100.0, 50.0), 1.0, false, 7, -1, 9, 3)

	assert_bool(intent.is_selection_changed()).is_true()
	assert_int(_interaction.selected_gate_id()).is_equal(9)
	assert_int(_interaction.selected_target_id()).is_equal(-1)


func test_primary_click_selects_body_when_it_is_the_only_hit() -> void:
	var intent := _interaction.interpret_primary_click(
		Vector2(100.0, 50.0), 1.0, false, 7, -1, -1, 3)

	assert_bool(intent.is_selection_changed()).is_true()
	assert_int(_interaction.selected_body_id()).is_equal(3)
	assert_int(_interaction.selected_gate_id()).is_equal(-1)


func test_double_click_returns_move_intent_without_changing_typed_selection() -> void:
	_interaction.interpret_primary_click(Vector2(100.0, 50.0), 1.0, false, 7, 12, -1, -1)

	var intent := _interaction.interpret_primary_click(
		Vector2(104.0, 53.0), 1.2, false, 7, 33, -1, -1)

	assert_bool(intent.is_double_click_move()).is_true()
	assert_int(_interaction.selected_target_id()).is_equal(12)


func test_dragging_suppresses_double_click_move_intent() -> void:
	_interaction.interpret_primary_click(Vector2(100.0, 50.0), 1.0, false, 7, -1, -1, -1)

	var intent := _interaction.interpret_primary_click(
		Vector2(101.0, 51.0), 1.2, true, 7, -1, -1, -1)

	assert_bool(intent.is_none()).is_true()


func test_resolve_key_intent_uses_the_mutually_exclusive_selection() -> void:
	_interaction.interpret_primary_click(Vector2(100.0, 50.0), 1.0, false, 7, -1, 5, -1)

	var intent := _interaction.resolve_key_intent(KEY_A, 7, -1, -1, -1)

	assert_bool(intent.is_approach_gate()).is_true()
	assert_int(intent.gate_id()).is_equal(5)


func test_clear_target_if_matches_clears_only_a_selected_ship() -> void:
	_interaction.interpret_primary_click(Vector2(100.0, 50.0), 1.0, false, 7, 42, -1, -1)

	_interaction.clear_target_if_matches(42)

	assert_int(_interaction.selected_target_id()).is_equal(-1)


func test_interpret_lock_click_returns_a_typed_lock_intent() -> void:
	var intent := _interaction.interpret_lock_click(7, 99)
	assert_bool(intent.is_lock_on()).is_true()
	assert_int(intent.ship_id()).is_equal(99)


func test_invalid_typed_selection_ids_collapse_to_none() -> void:
	assert_bool(ClientSelection.ship(-1).is_none()).is_true()
	assert_bool(ClientIntent.approach_gate(-1).is_none()).is_true()
	assert_bool(ClientIntent.adjust_keep_at_range(NAN).is_none()).is_true()
