## Tests for the explicit bright-star layer used by space_sky.gdshader.
extends GdUnitTestSuite

const CatalogScript = preload("res://scripts/sky_catalog.gd")


func test_catalog_has_fixed_padded_gpu_arrays() -> void:
	assert_int(CatalogScript.directions().size()).is_equal(16)
	assert_int(CatalogScript.colors().size()).is_equal(16)
	assert_int(CatalogScript.brightness().size()).is_equal(16)


func test_catalog_directions_are_normalized_and_brightness_is_positive() -> void:
	var directions := CatalogScript.directions()
	var brightness := CatalogScript.brightness()

	assert_float(directions[0].length()).is_equal_approx(1.0, 0.0001)
	assert_float(brightness[0]).is_greater(0.0)
