## Boundary tests for the Godot adapter around the Rust client interaction policy.
extends GdUnitTestSuite


const __source: String = "res://scripts/world_interaction.gd"

var _interaction: WorldInteraction


func before_test() -> void:
	_interaction = load(__source).new()


func test_primary_click_selects_ship_over_other_candidates() -> void:
	var action := _interaction.interpret_primary_click(
		Vector2(100.0, 50.0), 1.0, false, 7, 42, 9, 3)

	assert_int(action.kind()).is_equal(WorldInteraction.ACTION_LOCAL)
	assert_int(action.local_kind()).is_equal(WorldInteraction.LOCAL_SELECTION_CHANGED)
	assert_int(_interaction.selected_target_id()).is_equal(42)
	assert_int(_interaction.selected_gate_id()).is_equal(-1)
	assert_int(_interaction.selected_body_id()).is_equal(-1)


func test_primary_click_selects_gate_when_no_ship_is_hit() -> void:
	var action := _interaction.interpret_primary_click(
		Vector2(100.0, 50.0), 1.0, false, 7, -1, 9, 3)

	assert_int(action.local_kind()).is_equal(WorldInteraction.LOCAL_SELECTION_CHANGED)
	assert_int(_interaction.selected_gate_id()).is_equal(9)
	assert_int(_interaction.selected_target_id()).is_equal(-1)


func test_primary_click_selects_body_when_it_is_the_only_hit() -> void:
	var action := _interaction.interpret_primary_click(
		Vector2(100.0, 50.0), 1.0, false, 7, -1, -1, 3)

	assert_int(action.local_kind()).is_equal(WorldInteraction.LOCAL_SELECTION_CHANGED)
	assert_int(_interaction.selected_body_id()).is_equal(3)
	assert_int(_interaction.selected_gate_id()).is_equal(-1)


func test_primary_click_selects_station_when_it_is_the_only_navigation_hit() -> void:
	var action := _interaction.interpret_primary_click(
		Vector2(100.0, 50.0), 1.0, false, 7, -1, -1, -1, 3)

	assert_int(action.local_kind()).is_equal(WorldInteraction.LOCAL_SELECTION_CHANGED)
	assert_int(_interaction.selected_station_id()).is_equal(3)
	assert_int(_interaction.selected_body_id()).is_equal(-1)


func test_double_click_returns_a_local_move_effect_without_changing_selection() -> void:
	_interaction.interpret_primary_click(Vector2(100.0, 50.0), 1.0, false, 7, 12, -1, -1)

	var action := _interaction.interpret_primary_click(
		Vector2(104.0, 53.0), 1.2, false, 7, 33, -1, -1)

	assert_int(action.local_kind()).is_equal(WorldInteraction.LOCAL_DOUBLE_CLICK_MOVE)
	assert_int(_interaction.selected_target_id()).is_equal(12)


func test_dragging_suppresses_double_click_move_effect() -> void:
	_interaction.interpret_primary_click(Vector2(100.0, 50.0), 1.0, false, 7, -1, -1, -1)

	var action := _interaction.interpret_primary_click(
		Vector2(101.0, 51.0), 1.2, true, 7, -1, -1, -1)

	assert_int(action.kind()).is_equal(WorldInteraction.ACTION_NONE)


func test_key_action_uses_the_core_owned_selection() -> void:
	_interaction.interpret_primary_click(Vector2(100.0, 50.0), 1.0, false, 7, -1, 5, -1)

	var action := _interaction.resolve_key_action(KEY_A, 7, -1, -1, -1, 10_000.0, 7)

	assert_int(action.kind()).is_equal(WorldInteraction.ACTION_REQUEST)
	assert_bool(action.request_result().ok).is_true()


func test_clear_target_if_matches_clears_only_a_selected_ship() -> void:
	_interaction.interpret_primary_click(Vector2(100.0, 50.0), 1.0, false, 7, 42, -1, -1)

	_interaction.clear_target_if_matches(42)

	assert_int(_interaction.selected_target_id()).is_equal(-1)


func test_lock_click_returns_a_network_action_with_a_typed_target() -> void:
	var action := _interaction.interpret_lock_click(7, 99)

	assert_int(action.kind()).is_equal(WorldInteraction.ACTION_REQUEST)
	assert_int(action.target_ship_id()).is_equal(99)


func test_invalid_ids_are_rejected_at_the_engine_boundary() -> void:
	var action := _interaction.resolve_key_action(KEY_A, 7, -1, -1, -1, 10_000.0, 7)
	assert_int(action.kind()).is_equal(WorldInteraction.ACTION_NONE)
