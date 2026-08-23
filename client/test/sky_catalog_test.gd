## Tests for the explicit bright-star layer feeding the ADR-0054 starfield.
extends GdUnitTestSuite

const CatalogScript = preload("res://scripts/sky_catalog.gd")


func test_catalog_entries_carry_direction_colour_and_magnitude() -> void:
	var entries: Array[Dictionary] = CatalogScript.entries()

	assert_int(entries.size()).is_greater(0)
	for entry: Dictionary in entries:
		assert_float((entry["direction"] as Vector3).length()).is_equal_approx(1.0, 0.0001)
		assert_int(typeof(entry["color"])).is_equal(TYPE_COLOR)


func test_sirius_is_the_brightest_entry() -> void:
	# Visual magnitude runs backwards: the smaller the number, the brighter the
	# star. Sirius at -1.46 is the brightest star in the real sky, so it has to
	# stay the minimum here for the starfield to rank the landmarks correctly.
	var entries: Array[Dictionary] = CatalogScript.entries()
	var brightest: float = entries[0]["magnitude"] as float
	for entry: Dictionary in entries:
		brightest = minf(brightest, entry["magnitude"] as float)

	assert_float(brightest).is_equal_approx(-1.46, 0.0001)
	assert_float(entries[0]["magnitude"] as float).is_equal_approx(-1.46, 0.0001)
