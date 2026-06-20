## billboard_ring_test.gd
##
## Unit tests for billboard_ring.gd's shared selection-ring builder. Sprite3D
## properties (fixed_size, pixel_size, modulate, texture) can be read without
## scene-tree membership, so no add_child()/auto_free() is needed here.
extends GdUnitTestSuite

const __source: String = "res://scripts/billboard_ring.gd"


func test_build_returns_a_fixed_size_billboard_sprite() -> void:
	var sprite: Sprite3D = BillboardRing.build(Color.CYAN, 0.005)
	assert_bool(sprite.fixed_size).is_true()
	assert_int(sprite.billboard).is_equal(BaseMaterial3D.BILLBOARD_ENABLED)
	assert_object(sprite.texture).is_not_null()
	sprite.free()


func test_build_applies_the_requested_pixel_size_and_color() -> void:
	var sprite: Sprite3D = BillboardRing.build(Color.CYAN, 0.0123)
	assert_float(sprite.pixel_size).is_equal_approx(0.0123, 0.0001)
	assert_vector(Vector3(sprite.modulate.r, sprite.modulate.g, sprite.modulate.b)) \
		.is_equal_approx(Vector3(Color.CYAN.r, Color.CYAN.g, Color.CYAN.b), Vector3(0.0001, 0.0001, 0.0001))
	sprite.free()


func test_build_reuses_the_same_cached_texture_across_calls() -> void:
	var a: Sprite3D = BillboardRing.build(Color.WHITE, 0.003)
	var b: Sprite3D = BillboardRing.build(Color.RED, 0.005)
	assert_object(a.texture).is_same(b.texture)
	a.free()
	b.free()
