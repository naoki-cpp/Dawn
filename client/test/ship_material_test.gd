extends GdUnitTestSuite


func test_ship_scene_applies_lit_hull_and_emissive_engine_materials() -> void:
	var ship: Node3D = auto_free(preload("res://scenes/ship.tscn").instantiate())
	var hull := (ship.get_node("Hull") as MeshInstance3D).get_active_material(0) as StandardMaterial3D
	var engine := (ship.get_node("EngineGlow") as MeshInstance3D).get_active_material(0) as StandardMaterial3D
	assert_object(hull).is_not_null()
	assert_object(engine).is_not_null()
	if hull == null or engine == null:
		return
	assert_bool(hull.emission_enabled).is_false()
	assert_int(hull.shading_mode).is_equal(BaseMaterial3D.SHADING_MODE_PER_PIXEL)
	assert_bool(engine.emission_enabled).is_true()
	assert_float(engine.emission_energy_multiplier).is_greater(1.0)
