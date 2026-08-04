## Regression coverage for typed InitialState -> main.tscn presentation.
##
## The renderer unit tests prove that a body node can be built, while this
## suite proves that a typed true-AU body remains inside the client presentation
## clamp after the real main scene has received its first ship.
extends GdUnitTestSuite

const MainScene := preload("res://scenes/main.tscn")
const AU_M: float = 1.495978707e11

var _main: Node


func before_test() -> void:
	_main = MainScene.instantiate()
	add_child(_main)
	await get_tree().process_frame
	_main.get_node("Connection").set_process(false)


func after_test() -> void:
	if _main != null and is_instance_valid(_main):
		_main.queue_free()
	await get_tree().process_frame


func test_initial_state_planet_is_rendered_near_the_player_at_true_au_scale() -> void:
	var connection: Node = _main.get_node("Connection")
	connection.ship_id = 11
	var outcome: ServerMessageOutcome = ServerMessageDecoder.new().test_outcome("InitialState")
	assert_object(outcome).is_not_null()
	assert_bool(outcome.dispatch(
		connection, _main, _main._session, _main._loadout, connection.ship_id
	)).is_true()
	await get_tree().process_frame

	var planet := CelestialBodyRecord.new()
	planet.body_id = 10
	planet.kind = "Planet"
	planet.name = "Forge"
	planet.position = PackedFloat64Array([0.8 * AU_M, 0.0, 0.5 * AU_M])
	planet.radius = 8_000.0
	planet.spectral_type = 0.0
	_main._presentation.respawn_navigation_markers(
		[],
		[planet],
		[],
		Callable(_main._interaction, "clear_navigation_selection")
	)
	await get_tree().process_frame

	var bodies_root: Node3D = _main.get_node("World/Bodies")
	assert_int(bodies_root.get_child_count()).is_equal(1)
	var rendered_planet: Node3D = bodies_root.get_child(0) as Node3D
	assert_bool(rendered_planet.has_meta("body_id")).is_true()
	assert_int(rendered_planet.get_meta("body_id") as int).is_equal(10)
	assert_bool(rendered_planet.global_position.is_finite()).is_true()
	assert_float(rendered_planet.global_position.distance_to(Vector3.ZERO)).is_less(30_000.1)
