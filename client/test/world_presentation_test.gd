## world_presentation_test.gd
##
## Tests for the client WorldPresentation seam. These focus on the pure
## presentation policy now extracted from main.gd: marker clamping, warp-tunnel
## easing, and sun-direction derivation from star data.
extends GdUnitTestSuite

const WorldPresentation = preload("res://scripts/world_presentation.gd")


class FakeWorld:
	extends RefCounted

	var rebase_shift := Vector3.ZERO

	func rebase_to(_new_origin: Vector3) -> Vector3:
		return rebase_shift


class FakeShip:
	extends Node3D

	var motion_rebase_calls: Array[Vector3] = []

	func rebase_motion(shift: Vector3) -> void:
		motion_rebase_calls.append(shift)


func test_clamped_marker_position_leaves_nearby_marker_unchanged() -> void:
	var player := Vector3.ZERO
	var marker := Vector3(100.0, 0.0, 0.0)

	var result: Vector3 = WorldPresentation.clamped_marker_position(player, marker, 500.0)

	assert_vector(result).is_equal(marker)


func test_clamped_marker_position_pulls_far_marker_back_to_clamp_radius() -> void:
	var player := Vector3.ZERO
	var marker := Vector3(1000.0, 0.0, 0.0)

	var result: Vector3 = WorldPresentation.clamped_marker_position(player, marker, 250.0)

	assert_vector(result).is_equal(Vector3(250.0, 0.0, 0.0))


func test_next_warp_tunnel_amount_eases_toward_one_above_threshold() -> void:
	var amount: float = WorldPresentation.next_warp_tunnel_amount(0.0, 3000.0, 0.1, 2000.0, 3.0)

	assert_float(amount).is_equal_approx(0.3, 0.0001)


func test_next_warp_tunnel_amount_eases_back_toward_zero_below_threshold() -> void:
	var amount: float = WorldPresentation.next_warp_tunnel_amount(1.0, 0.0, 0.1, 2000.0, 3.0)

	assert_float(amount).is_equal_approx(0.7, 0.0001)


func test_sun_state_returns_inactive_when_no_star_exists() -> void:
	var state: Dictionary = WorldPresentation.sun_state([
		{"kind": "Planet", "position": Vector3(100.0, 0.0, 0.0), "spectral_type": 0.1},
	], Vector3.ZERO, func(diff: Vector3) -> Vector3:
		return diff
	)

	assert_bool(state.get("active", true) as bool).is_false()


func test_sun_state_returns_direction_and_color_from_star_data() -> void:
	var state: Dictionary = WorldPresentation.sun_state([
		{"kind": "Star", "position": Vector3.ZERO, "spectral_type": 0.0},
	], Vector3.ZERO, func(diff: Vector3) -> Vector3:
		return diff
	)

	assert_bool(state.get("active", false) as bool).is_true()
	assert_vector(state.get("direction", Vector3.ZERO) as Vector3) \
		.is_equal_approx(WorldPresentation.SUN_FAR_DIRECTION.normalized(), Vector3(0.0001, 0.0001, 0.0001))
	assert_vector(state.get("color", Vector3.ZERO) as Vector3) \
		.is_equal_approx(Vector3(0.55, 0.65, 1.00), Vector3(0.0001, 0.0001, 0.0001))


func test_origin_rebase_moves_ship_and_motion_track_together() -> void:
	var presentation := WorldPresentation.new()
	var world := FakeWorld.new()
	world.rebase_shift = Vector3(100.0, 2.0, -3.0)
	presentation._world = world

	var ship := FakeShip.new()
	ship.position = Vector3(10.0, 20.0, 30.0)
	presentation.apply_origin_rebase(Vector3.ZERO, false, -1, {7: ship})

	assert_vector(ship.position).is_equal(Vector3(110.0, 22.0, 27.0))
	assert_array(ship.motion_rebase_calls).contains_exactly([
		Vector3(100.0, 2.0, -3.0),
	])
	ship.free()
