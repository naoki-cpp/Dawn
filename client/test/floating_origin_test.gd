## floating_origin_test.gd
##
## Tests for floating_origin.gd (ADR-0029 step 6). Verifies the origin-relative
## transform keeps nearby objects precise at true-AU magnitudes and that a
## rebase preserves relative positions (no visible jump).
extends GdUnitTestSuite

const FloatingOrigin = preload("res://scripts/floating_origin.gd")

const AU_M: float = 1.495978707e11
const WORLD_SCALE: float = 0.1


## A +10 m object near a 5 AU planet keeps its offset under floating origin,
## where a fixed (zero) origin would lose it to f32 quantisation.
func test_floating_origin_preserves_nearby_offset_at_true_au() -> void:
	var planet := Vector3(5.0 * AU_M, 0.0, 0.0)
	var nearby := planet + Vector3(10.0, 0.0, 0.0)
	# Origin at the player (the planet): render coords are small and exact.
	var r := FloatingOrigin.to_godot(nearby, planet, WORLD_SCALE)
	# Decoded back: x = r.x / scale + origin.x.
	var decoded_x: float = r.x / WORLD_SCALE + planet.x
	assert_float(abs(decoded_x - nearby.x)).is_less(0.001)
	# Render coordinate itself stays tiny (10 m * 0.1 = 1 unit).
	assert_float(abs(r.x)).is_less(10.0)

	# Fixed (zero) origin: the +10 m vanishes into f32 ulp (~tens of km).
	var naive := FloatingOrigin.to_godot(nearby, Vector3.ZERO, WORLD_SCALE)
	var naive_decoded: float = naive.x / WORLD_SCALE
	assert_float(abs(naive_decoded - nearby.x)).is_greater(1.0)


## Rebasing the origin and shifting every node by rebase_shift leaves the
## relative render position of two objects unchanged (the spike C2-2 property).
func test_rebase_preserves_relative_render_position() -> void:
	var planet := Vector3(5.0 * AU_M, 0.0, 0.0)
	var player := planet
	var wingman := planet + Vector3(200.0, 0.0, 0.0)

	var player_r0 := FloatingOrigin.to_godot(player, player, WORLD_SCALE)
	var wingman_r0 := FloatingOrigin.to_godot(wingman, player, WORLD_SCALE)
	var rel0 := wingman_r0 - player_r0

	# Player drifts; origin rebases to the new player position.
	var new_origin := planet + Vector3(1.0e8, 0.0, 0.0)
	var shift := FloatingOrigin.rebase_shift(player, new_origin, WORLD_SCALE)
	# After rebase, re-render from the new origin; the world-node shift is `shift`.
	var player_r1 := FloatingOrigin.to_godot(player, new_origin, WORLD_SCALE)
	var wingman_r1 := FloatingOrigin.to_godot(wingman, new_origin, WORLD_SCALE)
	var rel1 := wingman_r1 - player_r1

	# Relative position is unchanged (no visible jump), and the shift equals the
	# render-position delta of a fixed point.
	assert_vector(rel1).is_equal_approx(rel0, Vector3(0.001, 0.001, 0.001))
	var moved := player_r1 - player_r0
	assert_vector(moved).is_equal_approx(shift, Vector3(0.5, 0.5, 0.5))


## should_rebase only triggers past the threshold (so it is a no-op at the
## compressed scale, where the world fits well inside the threshold).
func test_should_rebase_respects_threshold() -> void:
	var origin := Vector3.ZERO
	assert_bool(FloatingOrigin.should_rebase(Vector3(500_000.0, 0.0, 0.0), origin, 1.0e6)).is_false()
	assert_bool(FloatingOrigin.should_rebase(Vector3(2.0e6, 0.0, 0.0), origin, 1.0e6)).is_true()
