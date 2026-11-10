extends GdUnitTestSuite

const Source = preload("res://scripts/stellar_source.gd")


func test_projected_disc_energy_and_variance_scale_with_area() -> void:
	assert_float(Source.flux(0.008) / Source.flux(0.004)).is_equal_approx(4.0, 0.00001)
	assert_float(Source.variance(0.008) / Source.variance(0.004)).is_equal_approx(4.0, 0.00001)
	for invalid: float in [0.0, -1.0, INF, NAN]:
		assert_float(Source.flux(invalid)).is_equal(0.0)
		assert_float(Source.variance(invalid)).is_equal(0.0)
		assert_float(Source.lens_flux(invalid)).is_equal(0.0)


func test_lens_response_preserves_unresolved_visibility_and_soft_limits_large_sources() -> void:
	var distant_radius := 0.0033
	var distant_flux: float = Source.lens_flux(distant_radius)
	assert_float(distant_flux / Source.flux(distant_radius)).is_greater(7.0)

	var near_flux: float = Source.lens_flux(0.32)
	var larger_near_flux: float = Source.lens_flux(0.64)
	assert_float(larger_near_flux).is_greater(near_flux)
	assert_float(larger_near_flux / near_flux).is_less(1.01)
