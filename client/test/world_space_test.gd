## world_space_test.gd
##
## Tests for the Rust WorldSpace adapter (ADR-0029 #3). Verifies the origin-relative transform
## keeps nearby objects precise at true-AU magnitudes, that component transforms
## are mutual inverses, and that a rebase preserves relative positions.
extends GdUnitTestSuite

const AU_M: float = 1.495978707e11
const WORLD_SCALE: float = 0.1


## A +10 m object near a 5 AU planet keeps its offset under a floating origin,
## where a fixed (zero) origin would lose it to f32 quantisation.
func test_floating_origin_preserves_nearby_offset_at_true_au() -> void:
	var planet_x: float = 5.0 * AU_M
	var nearby_x: float = planet_x + 10.0

	# Origin at the player (the planet): render coords are small and exact.
	var w := WorldSpace.new()
	w.rebase_to_components(planet_x, 0.0, 0.0)
	var r := w.to_godot_components(nearby_x, 0.0, 0.0)
	var server := w.to_server_components(r)
	assert_float(abs(server[0] - nearby_x)).is_less(0.001)
	# Render coordinate itself stays tiny (10 m * 0.1 = 1 unit).
	assert_float(abs(r.x)).is_less(10.0)

	# Fixed (zero) origin: the +10 m vanishes into f32 ulp (~tens of km).
	var naive := Vector3(nearby_x * WORLD_SCALE, 0.0, 0.0)
	assert_float(abs(naive.x / WORLD_SCALE - nearby_x)).is_greater(1.0)


func test_f64_wire_position_is_subtracted_before_vector3_narrowing() -> void:
	var w := WorldSpace.new()
	w.rebase_to_components(5.0 * AU_M, 0.0, 0.0)

	var rendered := w.to_godot_components(5.0 * AU_M + 10.0, 0.0, 0.0)

	assert_float(rendered.x).is_equal_approx(1.0, 0.0001)


## Component transforms are exact inverses even when the origin has moved.
func test_component_transforms_are_mutual_inverses_with_a_moved_origin() -> void:
	var w := WorldSpace.new()
	w.rebase_to_components(3.0 * AU_M, -1.0e6, 2.0e6)
	var server := PackedFloat64Array([3.0 * AU_M + 123.0, -1.0e6 - 45.0, 2.0e6 + 678.0])
	var rendered := w.to_godot_components(server[0], server[1], server[2])
	var round_trip := w.to_server_components(rendered)
	assert_float(round_trip[0]).is_equal_approx(server[0], 0.01)
	assert_float(round_trip[1]).is_equal_approx(server[1], 0.01)
	assert_float(round_trip[2]).is_equal_approx(server[2], 0.01)


## Rebasing the origin and shifting every node by the returned delta leaves the
## relative render position of two objects unchanged (the spike C2-2 property).
func test_rebase_preserves_relative_render_position() -> void:
	var planet_x: float = 5.0 * AU_M
	var wingman_x: float = planet_x + 200.0

	var w := WorldSpace.new()
	w.rebase_to_components(planet_x, 0.0, 0.0)
	var rel0 := w.to_godot_components(wingman_x, 0.0, 0.0) \
		- w.to_godot_components(planet_x, 0.0, 0.0)

	# Player drifts; origin rebases to the new player position.
	w.rebase_to_components(planet_x + 1.0e8, 0.0, 0.0)
	var rel1 := w.to_godot_components(wingman_x, 0.0, 0.0) \
		- w.to_godot_components(planet_x, 0.0, 0.0)

	# Relative position is unchanged (no visible jump), and the shift equals the
	# render-position delta a fixed point sees.
	assert_vector(rel1).is_equal_approx(rel0, Vector3(0.001, 0.001, 0.001))


## should_rebase stays false within the threshold (so it is a no-op at the
## compressed scale, where the world fits well inside REBASE_THRESHOLD).
func test_should_rebase_respects_threshold() -> void:
	var w := WorldSpace.new()
	assert_bool(w.should_rebase_components(500_000.0, 0.0, 0.0)).is_false()
	assert_bool(w.should_rebase_components(2.0e6, 0.0, 0.0)).is_true()


func test_distance_components_preserves_au_scale_offsets() -> void:
	var w := WorldSpace.new()
	var first := PackedFloat64Array([5.0 * AU_M + 10.0, 0.0, 0.0])
	var second := PackedFloat64Array([5.0 * AU_M + 30.0, 0.0, 0.0])

	assert_float(w.distance_components(first, second)).is_equal_approx(20.0, 0.001)
