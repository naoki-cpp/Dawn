## Regression coverage for the live InitialState -> main.tscn marker path.
##
## The renderer unit tests prove that a body node can be built, while this
## suite proves that a true-AU body remains inside the client presentation
## clamp after the real main scene has received its first ship.
extends GdUnitTestSuite

const MainScene := preload("res://scenes/main.tscn")

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
		connection, _main._session, _main._loadout, connection.ship_id
	)).is_true()

	await get_tree().process_frame

	var bodies_root: Node3D = _main.get_node("World/Bodies")
	var planet: Node3D = null
	for child: Node in bodies_root.get_children():
		if child.has_meta("body_id"):
			planet = child as Node3D
			break

	assert_object(planet).is_not_null()
	assert_int(planet.get_meta("body_id") as int).is_equal(10)
	assert_bool(planet.global_position.is_finite()).is_true()
	assert_float(planet.global_position.distance_to(Vector3.ZERO)).is_less(30_000.1)
