## Tests for the ADR-0054 star layer: deterministic CPU generation, an even
## spread over the sphere, and one shared magnitude scale with the catalogue.
extends GdUnitTestSuite

const StarfieldScript = preload("res://scripts/starfield.gd")
const SkyCatalogScript = preload("res://scripts/sky_catalog.gd")


func test_generation_is_deterministic_for_a_seed() -> void:
	var first: Array[Dictionary] = StarfieldScript.generate(1234, 32)
	var again: Array[Dictionary] = StarfieldScript.generate(1234, 32)
	var other: Array[Dictionary] = StarfieldScript.generate(1235, 32)

	assert_int(first.size()).is_equal(32)
	for i: int in range(first.size()):
		assert_vector(first[i]["direction"] as Vector3) \
			.is_equal(again[i]["direction"] as Vector3)
		assert_float(first[i]["flux"] as float) \
			.is_equal_approx(again[i]["flux"] as float, 0.0000001)
		assert_float(first[i]["distance_kpc"] as float) \
			.is_equal_approx(again[i]["distance_kpc"] as float, 0.0000001)
	assert_vector(other[0]["direction"] as Vector3) \
		.is_not_equal(first[0]["direction"] as Vector3)


func test_directions_are_unit_length() -> void:
	for star: Dictionary in StarfieldScript.generate(7, 64):
		assert_float((star["direction"] as Vector3).length()).is_equal_approx(1.0, 0.0001)


func test_star_distances_follow_local_and_galactic_sampling_ranges() -> void:
	for star: Dictionary in StarfieldScript.generate(41, 128, 0.0):
		var distance: float = star["distance_kpc"] as float
		assert_float(distance).is_greater(0.0)
		assert_float(distance).is_less_equal(StarfieldScript.LOCAL_FIELD_MAX_DISTANCE_KPC)

	var far_galactic_stars: int = 0
	for star: Dictionary in StarfieldScript.generate(41, 1024, 1.0):
		var distance: float = star["distance_kpc"] as float
		assert_float(distance).is_greater(0.0)
		assert_float(distance).is_less_equal(StarfieldScript.MAX_STAR_DISTANCE_KPC)
		if distance > StarfieldScript.LOCAL_FIELD_MAX_DISTANCE_KPC:
			far_galactic_stars += 1
	assert_int(far_galactic_stars).is_greater(0)


func test_galactic_center_projects_from_the_suns_position() -> void:
	var projection: Dictionary = StarfieldScript._project_position(Vector3.ZERO)
	var expected_distance: float = StarfieldScript.GALACTIC_CENTER_DISTANCE_KPC
	assert_float(projection["distance_kpc"] as float) \
		.is_equal_approx(expected_distance, 0.01)
	var expected_galactic_direction: Vector3 = \
		(-StarfieldScript.SUN_GALACTIC_POSITION_KPC).normalized()
	assert_vector(projection["direction"] as Vector3).is_equal(
		StarfieldScript.from_galactic(expected_galactic_direction))


func test_equatorial_and_galactic_frames_share_the_catalogue_orientation() -> void:
	var north_galactic_pole: Vector3 = SkyCatalogScript._direction(
		192.859481 / 15.0, 27.128251)
	var galactic_center: Vector3 = SkyCatalogScript._direction(
		266.4051 / 15.0, -28.936175)
	assert_vector(StarfieldScript.to_galactic(north_galactic_pole)) \
		.is_equal_approx(Vector3.UP, Vector3.ONE * 0.0001)
	assert_vector(StarfieldScript.to_galactic(galactic_center)) \
		.is_equal_approx(Vector3.RIGHT, Vector3.ONE * 0.001)
	for direction: Vector3 in [Vector3.RIGHT, Vector3.UP, Vector3.BACK,
		Vector3(0.32, -0.71, 0.62).normalized()]:
		assert_vector(StarfieldScript.from_galactic(
			StarfieldScript.to_galactic(direction))).is_equal_approx(
				direction, Vector3.ONE * 0.0001)


func test_disk_samples_have_the_expected_thin_and_thick_scale_height() -> void:
	var rng := RandomNumberGenerator.new()
	rng.seed = 813
	var height_sum: float = 0.0
	var radius_sum: float = 0.0
	var samples: int = 4000
	for _i: int in range(samples):
		var position: Vector3 = StarfieldScript._sample_disk_position(rng, false)
		height_sum += absf(position.y)
		radius_sum += Vector2(position.x, position.z).length()
	var mean_height: float = height_sum / float(samples)
	var mean_radius: float = radius_sum / float(samples)
	assert_float(mean_height).is_between(0.22, 0.30)
	assert_float(mean_radius).is_between(4.25, 4.95)


func test_uniform_sampling_spreads_evenly_instead_of_bunching_at_the_poles() -> void:
	# Sampling the sine of the latitude uniformly is what makes the density flat.
	# An equirectangular grid samples the latitude itself and piles stars up at
	# the poles, which is the artefact this layer replaced.
	var bands: PackedInt32Array = PackedInt32Array()
	bands.resize(10)
	for star: Dictionary in StarfieldScript.generate(99, 20000, 0.0):
		var height: float = (star["direction"] as Vector3).y
		bands[clampi(int((height + 1.0) * 0.5 * 10.0), 0, 9)] += 1

	# Equal-height bands cover equal solid angle on a sphere, so each of the ten
	# should hold about a tenth of the stars.
	var expected: float = 20000.0 / 10.0
	for count: int in bands:
		assert_float(float(count)).is_between(expected * 0.9, expected * 1.1)


func test_disc_stars_concentrate_on_the_galactic_plane_and_run_fainter() -> void:
	# A uniform field reads as flat wallpaper. The depth cue is the Milky Way
	# being a dense wall of distant, fainter stars behind a sparse near scatter,
	# so the disc share has to actually pile up on the plane the sky shader
	# paints -- and be dimmer than the uniform share.
	var near_plane: int = 0
	var disc_flux: float = 0.0
	var samples: int = 4000
	for star: Dictionary in StarfieldScript.generate(5, samples, 1.0):
		var galactic_height: float = _galactic_height(star["direction"] as Vector3)
		if absf(galactic_height) < 0.2:
			near_plane += 1
		disc_flux += star["flux"] as float

	var uniform_near_plane: int = 0
	var uniform_flux: float = 0.0
	for star: Dictionary in StarfieldScript.generate(5, samples, 0.0):
		if absf(_galactic_height(star["direction"] as Vector3)) < 0.2:
			uniform_near_plane += 1
		uniform_flux += star["flux"] as float

	# |height| < 0.2 is 20% of a sphere's solid angle; the disc sample should be
	# far denser there than that.
	assert_float(float(near_plane) / float(samples)).is_greater(0.55)
	assert_float(float(uniform_near_plane) / float(samples)).is_between(0.15, 0.25)
	assert_float(disc_flux).is_less(uniform_flux)


func test_default_field_keeps_a_dense_galactic_band() -> void:
	var samples: int = 4000
	var near_plane: int = 0
	for star: Dictionary in StarfieldScript.generate(5, samples):
		if absf(_galactic_height(star["direction"] as Vector3)) < 0.2:
			near_plane += 1
	assert_float(float(near_plane) / float(samples)).is_greater(0.43)


## Galactic latitude in the same frame used by the sky shader.
func _galactic_height(direction: Vector3) -> float:
	return StarfieldScript.to_galactic(direction).y


func test_star_records_carry_no_twinkle_state() -> void:
	# Scintillation is atmospheric and does not happen in vacuum, so the
	# per-star flash phase/rate that Trinity's starfield carries is
	# deliberately absent here (ADR-0054 §4).
	var star: Dictionary = StarfieldScript.generate(3, 1)[0]
	var keys: Array = star.keys()
	keys.sort()
	assert_array(keys).contains_exactly(["color", "direction", "distance_kpc", "flux"])


func test_brightest_named_star_outshines_the_generated_field() -> void:
	var generated: Array[Dictionary] = StarfieldScript.generate(
		StarfieldScript.DEFAULT_SEED, 4000)
	var field_peak: float = 0.0
	for star: Dictionary in generated:
		field_peak = maxf(field_peak, star["flux"] as float)

	var catalog_peak: float = 0.0
	for star: Dictionary in StarfieldScript.catalog_stars():
		catalog_peak = maxf(catalog_peak, star["flux"] as float)

	# Sirius is the brightest star in the real sky, so the generated field must
	# not be able to out-shine the named landmarks.
	assert_float(catalog_peak).is_greater(field_peak)
	assert_float(field_peak).is_less_equal(StarfieldScript.MAX_GENERATED_FLUX)


func test_flux_follows_the_visual_magnitude_scale() -> void:
	# One step of visual magnitude is a flux ratio of 10 ^ 0.4.
	var sirius: float = StarfieldScript.flux_for_magnitude(-1.46)
	var vega: float = StarfieldScript.flux_for_magnitude(0.03)
	assert_float(sirius / vega).is_equal_approx(pow(10.0, -0.4 * (-1.46 - 0.03)), 0.001)


func test_generated_brightness_has_a_sparse_pareto_tail() -> void:
	var median_rank: float = StarfieldScript.flux_from_rank(0.5)
	var bright_rank: float = StarfieldScript.flux_from_rank(0.99)
	var capped_rank: float = StarfieldScript.flux_from_rank(0.9999)
	var expected_median := (StarfieldScript.LUMINOSITY_BASELINE
		+ StarfieldScript.LUMINOSITY_PARETO_SCALE
		* (pow(2.0, 1.0 / StarfieldScript.LUMINOSITY_PARETO_SHAPE) - 1.0)) \
		* StarfieldScript.LUMINOSITY_RENDER_GAIN
	assert_float(median_rank).is_equal_approx(expected_median, 0.000001)
	assert_float(bright_rank).is_greater(median_rank)
	assert_float(capped_rank).is_equal(StarfieldScript.MAX_GENERATED_FLUX)

	var low_flux_count: int = 0
	var bright_flux_count: int = 0
	for star: Dictionary in StarfieldScript.generate(73, 4000):
		var flux: float = star["flux"] as float
		if flux < 1.0:
			low_flux_count += 1
		if flux > 2.0:
			bright_flux_count += 1
	assert_int(low_flux_count).is_greater(bright_flux_count)


func test_faint_stars_wash_toward_white_and_bright_stars_keep_their_tint() -> void:
	var faint: Color = StarfieldScript.apparent_color(0.95, 0.0)
	var bright: Color = StarfieldScript.apparent_color(0.95, 1.0)
	# A spectral type of 0.95 is an M star: strongly red once it is bright
	# enough to deliver the photons that carry colour.
	assert_float(bright.b).is_less(faint.b)
	assert_float(faint.b).is_greater(0.5)


func test_point_spread_stays_above_the_pixel_grid() -> void:
	# Sigma below about a pixel means the sprite falls between pixel centres as
	# the camera turns, so stars flicker. That is scintillation -- the very thing
	# ADR-0054 rejects as an atmospheric effect that cannot happen in vacuum --
	# arriving by accident through the sampling rate.
	assert_float(StarfieldScript.point_spread_sigma_pixels()) \
		.is_greater_equal(StarfieldScript.MIN_STABLE_SIGMA_PIXELS)
	assert_float(StarfieldScript.POINT_CORE_SIGMA_PIXELS).is_equal_approx(0.72, 0.0001)
	assert_float(StarfieldScript.POINT_SPREAD_PIXELS).is_greater(
		StarfieldScript.POINT_HALO_SIGMA_PIXELS)
	assert_float(StarfieldScript.POINT_HALO_PEAK_RATIO).is_between(0.0, 0.03)


func test_every_star_shares_one_point_spread() -> void:
	# Stars are point sources. One looks larger than another only because a
	# brighter Gaussian stays above the visible threshold further out, so the
	# sprite must not also vary in size -- that double-counts brightness and
	# turns the bright end into balls rather than points.
	for star: Dictionary in StarfieldScript.generate(11, 64):
		assert_bool(star.has("pixels")).is_false()


func test_multimesh_is_configured_to_carry_per_star_colour_and_custom_data() -> void:
	var stars: Array[Dictionary] = StarfieldScript.generate(11, 8)
	var multimesh: MultiMesh = StarfieldScript.build_multimesh(stars)

	assert_int(multimesh.instance_count).is_equal(8)
	assert_bool(multimesh.use_colors).is_true()
	assert_bool(multimesh.use_custom_data).is_true()
	assert_int(multimesh.transform_format).is_equal(MultiMesh.TRANSFORM_3D)
	assert_object(multimesh.mesh).is_instanceof(QuadMesh)
	assert_float(stars[0]["distance_kpc"] as float) \
		.is_between(0.0, StarfieldScript.MAX_STAR_DISTANCE_KPC)

	# The per-instance transforms/colours/custom data deliberately go unasserted.
	# MultiMesh stores them in the RenderingServer, and the headless dummy
	# renderer discards them -- get_instance_custom_data() returns zero there
	# even though it round-trips correctly on a real device. Asserting the
	# packing would mean asserting which renderer the test happens to run under.
	# The packing itself is verified by rendering the scene.


func test_catalog_stars_reuse_the_shared_magnitude_conversion() -> void:
	var entries: Array[Dictionary] = SkyCatalogScript.entries()
	var stars: Array[Dictionary] = StarfieldScript.catalog_stars()

	assert_int(stars.size()).is_equal(entries.size())
	for i: int in range(entries.size()):
		assert_float(stars[i]["flux"] as float).is_equal_approx(
			StarfieldScript.flux_for_magnitude(entries[i]["magnitude"] as float), 0.0001)
