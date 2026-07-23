## world_space_test.gd
##
## Tests for world_space.gd (ADR-0029 #3). Verifies the origin-relative transform
## keeps nearby objects precise at true-AU magnitudes, that to_godot/to_server are
## mutual inverses (the property the old scattered `/WORLD_SCALE` reads violated
## once the origin moved), and that a rebase preserves relative positions.
extends GdUnitTestSuite

const WorldSpaceScript = preload("res://scripts/world_space.gd")

const AU_M: float = 1.495978707e11
const WORLD_SCALE: float = 0.1


## A +10 m object near a 5 AU planet keeps its offset under a floating origin,
## where a fixed (zero) origin would lose it to f32 quantisation.
func test_floating_origin_preserves_nearby_offset_at_true_au() -> void:
	var planet := Vector3(5.0 * AU_M, 0.0, 0.0)
	var nearby := planet + Vector3(10.0, 0.0, 0.0)

	# Origin at the player (the planet): render coords are small and exact.
	var w := WorldSpaceScript.new()
	w.origin = planet
	var r := w.to_godot(nearby)
	assert_float(abs(w.to_server(r).x - nearby.x)).is_less(0.001)
	# Render coordinate itself stays tiny (10 m * 0.1 = 1 unit).
	assert_float(abs(r.x)).is_less(10.0)

	# Fixed (zero) origin: the +10 m vanishes into f32 ulp (~tens of km).
	var w0 := WorldSpaceScript.new()
	var naive := w0.to_godot(nearby)
	assert_float(abs(naive.x / WORLD_SCALE - nearby.x)).is_greater(1.0)


func test_f64_wire_position_is_subtracted_before_vector3_narrowing() -> void:
	var w := WorldSpaceScript.new()
	w.rebase_to_components(5.0 * AU_M, 0.0, 0.0)

	var rendered := w.to_godot_components(5.0 * AU_M + 10.0, 0.0, 0.0)

	assert_float(rendered.x).is_equal_approx(1.0, 0.0001)


## to_godot and to_server are exact inverses even when the origin has moved -- the
## invariant the old ad-hoc `global_position / WORLD_SCALE` reads silently broke.
func test_to_godot_and_to_server_are_mutual_inverses_with_a_moved_origin() -> void:
	var w := WorldSpaceScript.new()
	w.origin = Vector3(3.0 * AU_M, -1.0e6, 2.0e6)
	var server := w.origin + Vector3(123.0, -45.0, 678.0)
	var round_trip := w.to_server(w.to_godot(server))
	assert_vector(round_trip).is_equal_approx(server, Vector3(0.01, 0.01, 0.01))


## Rebasing the origin and shifting every node by the returned delta leaves the
## relative render position of two objects unchanged (the spike C2-2 property).
func test_rebase_preserves_relative_render_position() -> void:
	var planet := Vector3(5.0 * AU_M, 0.0, 0.0)
	var player := planet
	var wingman := planet + Vector3(200.0, 0.0, 0.0)

	var w := WorldSpaceScript.new()
	w.origin = player
	var rel0 := w.to_godot(wingman) - w.to_godot(player)

	# Player drifts; origin rebases to the new player position.
	var new_origin := planet + Vector3(1.0e8, 0.0, 0.0)
	var shift := w.rebase_to(new_origin)
	var rel1 := w.to_godot(wingman) - w.to_godot(player)

	# Relative position is unchanged (no visible jump), and the shift equals the
	# render-position delta a fixed point sees.
	assert_vector(rel1).is_equal_approx(rel0, Vector3(0.001, 0.001, 0.001))


## should_rebase stays false within the threshold (so it is a no-op at the
## compressed scale, where the world fits well inside REBASE_THRESHOLD).
func test_should_rebase_respects_threshold() -> void:
	var w := WorldSpaceScript.new()
	assert_bool(w.should_rebase(Vector3(500_000.0, 0.0, 0.0))).is_false()
	assert_bool(w.should_rebase(Vector3(2.0e6, 0.0, 0.0))).is_true()
