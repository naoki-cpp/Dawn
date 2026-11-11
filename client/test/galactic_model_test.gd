extends GdUnitTestSuite

const GalacticModelScript = preload("res://scripts/galactic_model.gd")


func test_dust_and_stellar_profiles_are_normalized_at_the_solar_position() -> void:
	assert_float(GalacticModelScript.dust_density(
		GalacticModelScript.SUN_POSITION_KPC)).is_equal_approx(1.0, 0.000001)
	assert_float(GalacticModelScript.stellar_disk_density(
		GalacticModelScript.SUN_POSITION_KPC)).is_equal_approx(1.0, 0.000001)


func test_exponential_disk_obeys_its_radial_and_vertical_scale_lengths() -> void:
	var sun: Vector3 = GalacticModelScript.SUN_POSITION_KPC
	var radial_distance: float = GalacticModelScript.THIN_DISK_RADIAL_SCALE_KPC
	var radial_point := sun + Vector3.LEFT * radial_distance
	var vertical_point := sun + Vector3.UP * GalacticModelScript.THIN_DISK_SCALE_HEIGHT_KPC
	var expected_ratio := exp(-1.0)
	assert_float(GalacticModelScript.dust_density(
		sun + Vector3.LEFT * GalacticModelScript.DUST_RADIAL_SCALE_KPC)) \
		.is_equal_approx(expected_ratio, 0.00001)
	assert_float(GalacticModelScript.dust_density(
		sun + Vector3.UP * GalacticModelScript.DUST_SCALE_HEIGHT_KPC)) \
		.is_equal_approx(expected_ratio, 0.00001)
	var thin_local := (1.0 - GalacticModelScript.THICK_DISK_LOCAL_WEIGHT) \
		* exp(-absf(sun.y) / GalacticModelScript.THIN_DISK_SCALE_HEIGHT_KPC)
	var thick_local := GalacticModelScript.THICK_DISK_LOCAL_WEIGHT \
		* exp(-absf(sun.y) / GalacticModelScript.THICK_DISK_SCALE_HEIGHT_KPC)
	var radial_ratio := (
		thin_local * exp(-radial_distance / GalacticModelScript.THIN_DISK_RADIAL_SCALE_KPC)
		+ thick_local * exp(-radial_distance / GalacticModelScript.THICK_DISK_RADIAL_SCALE_KPC)
	) / (thin_local + thick_local)
	var vertical_ratio := (
		thin_local * exp(-GalacticModelScript.THIN_DISK_SCALE_HEIGHT_KPC
			/ GalacticModelScript.THIN_DISK_SCALE_HEIGHT_KPC)
		+ thick_local * exp(-GalacticModelScript.THIN_DISK_SCALE_HEIGHT_KPC
			/ GalacticModelScript.THICK_DISK_SCALE_HEIGHT_KPC)
	) / (thin_local + thick_local)
	assert_float(GalacticModelScript.stellar_disk_density(radial_point)) \
		.is_equal_approx(radial_ratio, 0.00001)
	assert_float(GalacticModelScript.stellar_disk_density(vertical_point)) \
		.is_equal_approx(vertical_ratio, 0.00001)


func test_one_magnitude_of_extinction_uses_the_logarithmic_magnitude_scale() -> void:
	var one_magnitude: Color = GalacticModelScript.transmission(1.0)
	assert_float(one_magnitude.r).is_equal_approx(pow(10.0, -0.4 * 0.84), 0.001)
	assert_float(one_magnitude.g).is_equal_approx(pow(10.0, -0.4), 0.001)
	assert_float(one_magnitude.b).is_equal_approx(pow(10.0, -0.4 * 1.32), 0.001)
	assert_float(one_magnitude.r).is_greater(one_magnitude.g)
	assert_float(one_magnitude.g).is_greater(one_magnitude.b)


func test_solar_vertical_sightline_integrates_the_exponential_dust_column() -> void:
	var distance_kpc := 0.1
	var expected_av := GalacticModelScript.LOCAL_AV_EXTINCTION_MAG_PER_KPC \
		* GalacticModelScript.DUST_SCALE_HEIGHT_KPC \
		* (1.0 - exp(-distance_kpc / GalacticModelScript.DUST_SCALE_HEIGHT_KPC))
	var actual_av: float = GalacticModelScript.visual_extinction(Vector3.UP, distance_kpc)
	assert_float(actual_av).is_equal_approx(expected_av, 0.0001)


func test_in_plane_path_accumulates_more_dust_than_the_vertical_path() -> void:
	var in_plane: float = GalacticModelScript.visual_extinction(Vector3.RIGHT, 1.0)
	var vertical: float = GalacticModelScript.visual_extinction(Vector3.UP, 1.0)
	assert_float(in_plane).is_greater(vertical)
	assert_float(vertical).is_greater(0.0)


func test_bulge_normalization_is_derived_from_the_global_mass_fraction() -> void:
	var gaussian_volume := pow(2.0 * PI, 1.5) \
		* GalacticModelScript.BULGE_SIGMA_AXES_KPC.x \
		* GalacticModelScript.BULGE_SIGMA_AXES_KPC.y \
		* GalacticModelScript.BULGE_SIGMA_AXES_KPC.z
	var bulge_mass := GalacticModelScript.bulge_density_normalization() * gaussian_volume
	var disk_mass: float = GalacticModelScript._disk_mass_integral_kpc3()
	var bulge_to_total := bulge_mass / (bulge_mass + disk_mass)
	assert_float(bulge_to_total).is_equal_approx(
		GalacticModelScript.BULGE_TO_TOTAL_STELLAR_MASS, 0.000001)


func test_dust_turbulence_weights_are_normalized_for_lognormal_density() -> void:
	var weights := GalacticModelScript.DUST_TURBULENCE_OCTAVE_WEIGHTS
	var squared_weight_sum := weights.length_squared()
	var interpolated_noise_variance := (1.0 / 12.0) * pow(26.0 / 35.0, 3.0)
	assert_float(weights.x + weights.y + weights.z).is_equal_approx(1.0, 0.000001)
	assert_float(GalacticModelScript.dust_turbulence_normalization()) \
		.is_equal_approx(1.0 / sqrt(interpolated_noise_variance * squared_weight_sum), 0.000001)
	assert_float(GalacticModelScript.stellar_mottle_noise_normalization()) \
		.is_equal_approx(1.0 / sqrt(interpolated_noise_variance), 0.000001)
	assert_float(GalacticModelScript.STELLAR_MOTTLE_WEIGHTS.x) \
		.is_greater(GalacticModelScript.STELLAR_MOTTLE_WEIGHTS.y)
	assert_float(GalacticModelScript.STELLAR_MOTTLE_WEIGHTS.y).is_greater(0.0)
	assert_float(GalacticModelScript.STELLAR_MOTTLE_ANGULAR_SCALES_RADIANS.x) \
		.is_greater(GalacticModelScript.STELLAR_MOTTLE_ANGULAR_SCALES_RADIANS.y)

	var dust_b_mach := GalacticModelScript.DUST_CLOUD_TURBULENCE_DRIVING_PARAMETER \
		* GalacticModelScript.DUST_CLOUD_SONIC_MACH_NUMBER
	assert_float(GalacticModelScript.dust_cloud_log_density_sigma()) \
		.is_equal_approx(sqrt(log(1.0 + dust_b_mach * dust_b_mach)), 0.000001)
	assert_float(pow(GalacticModelScript.dust_volume_log_density_sigma(), 2.0)
		+ pow(GalacticModelScript.dust_projected_log_density_sigma(), 2.0)) \
		.is_equal_approx(pow(GalacticModelScript.dust_cloud_log_density_sigma(), 2.0), 0.000001)
