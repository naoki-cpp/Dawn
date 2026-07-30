## world_interaction_test.gd
##
## Unit tests for world_interaction.gd's selection ownership and click
## interpretation. This module is pure session state plus normalized input
## facts, so it does not need a scene tree or network.
extends GdUnitTestSuite

const __source: String = "res://scripts/world_interaction.gd"

var _interaction: RefCounted


func before_test() -> void:
	_interaction = load(__source).new()


func test_primary_click_selects_ship_over_other_candidates() -> void:
	var action: Dictionary = _interaction.interpret_primary_click(
		Vector2(100.0, 50.0),
		1.0,
		false,
		7,
		42,
		9,
		3
	)

	assert_str(action.kind as String).is_equal("selection_changed")
	assert_int(_interaction.selected_target_id()).is_equal(42)
	assert_int(_interaction.selected_gate_id()).is_equal(-1)
	assert_int(_interaction.selected_body_id()).is_equal(-1)


func test_primary_click_selects_gate_when_no_ship_is_hit() -> void:
	var action: Dictionary = _interaction.interpret_primary_click(
		Vector2(100.0, 50.0),
		1.0,
		false,
		7,
		-1,
		9,
		3
	)

	assert_str(action.kind as String).is_equal("selection_changed")
	assert_int(_interaction.selected_gate_id()).is_equal(9)
	assert_int(_interaction.selected_target_id()).is_equal(-1)


func test_primary_click_selects_body_when_it_is_the_only_hit() -> void:
	var action: Dictionary = _interaction.interpret_primary_click(
		Vector2(100.0, 50.0),
		1.0,
		false,
		7,
		-1,
		-1,
		3
	)

	assert_str(action.kind as String).is_equal("selection_changed")
	assert_int(_interaction.selected_body_id()).is_equal(3)
	assert_int(_interaction.selected_gate_id()).is_equal(-1)


func test_double_click_returns_move_intent_and_does_not_change_selection() -> void:
	_interaction.interpret_primary_click(Vector2(100.0, 50.0), 1.0, false, 7, 12, -1, -1)

	var action: Dictionary = _interaction.interpret_primary_click(
		Vector2(104.0, 53.0),
		1.2,
		false,
		7,
		33,
		-1,
		-1
	)

	assert_str(action.kind as String).is_equal("double_click_move")
	assert_int(_interaction.selected_target_id()).is_equal(12)


func test_dragging_suppresses_double_click_move_intent() -> void:
	_interaction.interpret_primary_click(Vector2(100.0, 50.0), 1.0, false, 7, -1, -1, -1)

	var action: Dictionary = _interaction.interpret_primary_click(
		Vector2(101.0, 51.0),
		1.2,
		true,
		7,
		-1,
		-1,
		-1
	)

	assert_str(action.kind as String).is_equal("none")


func test_resolve_key_action_uses_internal_selection_state() -> void:
	_interaction.interpret_primary_click(Vector2(100.0, 50.0), 1.0, false, 7, -1, 5, -1)

	var action: Dictionary = _interaction.resolve_key_action(KEY_A, 7, -1, -1, -1)

	assert_str(action.kind as String).is_equal("approach_gate")
	assert_int(action.gate_id as int).is_equal(5)


func test_clear_target_if_matches_clears_only_the_selected_ship() -> void:
	_interaction.interpret_primary_click(Vector2(100.0, 50.0), 1.0, false, 7, 42, -1, -1)

	_interaction.clear_target_if_matches(42)

	assert_int(_interaction.selected_target_id()).is_equal(-1)


func test_interpret_lock_click_returns_lock_intent_for_hit_ship() -> void:
	var action: Dictionary = _interaction.interpret_lock_click(7, 99)
	assert_str(action.kind as String).is_equal("lock_on")
	assert_int(action.ship_id as int).is_equal(99)
