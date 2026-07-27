## Regression coverage for the live InitialState -> main.tscn marker path.
##
## The renderer unit tests prove that a body node can be built, while this
## suite proves that a true-AU body remains inside the client presentation
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
	_main.get_node("Connection").ship_id = 11
	_main._on_initial_state({
		"system_name": "Alpha",
		"systems": [{"id": 0, "name": "Alpha"}],
		"jump_gates": [],
		"stations": [],
		"celestial_bodies": [
			{
				"id": 0,
				"kind": "Star",
				"name": "Helios",
				"position": {"x": 0.0, "y": 0.0, "z": 0.0},
				"radius": 15_000.0,
				"spectral_type": 0.6,
			},
			{
				"id": 1,
				"kind": "Planet",
				"name": "Forge",
				"position": {"x": 0.8 * AU_M, "y": 0.0, "z": 0.5 * AU_M},
				"radius": 8_000.0,
				"spectral_type": 0.0,
			},
		],
		"buildable_ship_types": [],
		"ships": [{
			"ship_id": 11,
			"ship_type_name": "Magpie",
			"position": {"x": 0.0, "y": 0.0, "z": 0.0},
			"velocity": {"dx": 0.0, "dy": 0.0, "dz": 0.0},
			"max_speed": 500.0,
			"mass": 10_000_000.0,
			"inertia_modifier": 0.3,
			"max_shield": 100.0,
			"max_armor": 100.0,
			"max_hull": 100.0,
			"current_shield": 100.0,
			"current_armor": 100.0,
			"current_hull": 100.0,
			"cap_max": 100.0,
			"cap_recharge_per_tick": 1.0,
			"is_player": true,
		}],
	})

	await get_tree().process_frame

	var bodies_root: Node3D = _main.get_node("World/Bodies")
	assert_int(bodies_root.get_child_count()).is_equal(1)
	var planet: Node3D = bodies_root.get_child(0) as Node3D
	assert_bool(planet.has_meta("body_id")).is_true()
	assert_int(planet.get_meta("body_id") as int).is_equal(1)
	assert_bool(planet.global_position.is_finite()).is_true()
	assert_float(planet.global_position.distance_to(Vector3.ZERO)).is_less(30_000.1)
