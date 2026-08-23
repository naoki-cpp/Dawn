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
	assert_vector(other[0]["direction"] as Vector3) \
		.is_not_equal(first[0]["direction"] as Vector3)


func test_directions_are_unit_length() -> void:
	for star: Dictionary in StarfieldScript.generate(7, 64):
		assert_float((star["direction"] as Vector3).length()).is_equal_approx(1.0, 0.0001)


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


## Height above the galactic plane, undoing Starfield.from_galactic().
func _galactic_height(direction: Vector3) -> float:
	var ct: float = cos(StarfieldScript.GALACTIC_TILT)
	var st: float = sin(StarfieldScript.GALACTIC_TILT)
	var cl: float = cos(StarfieldScript.GALACTIC_LON)
	var sl: float = sin(StarfieldScript.GALACTIC_LON)
	var rz: float = -direction.x * sl + direction.z * cl
	return direction.y * ct - rz * st


func test_star_records_carry_no_twinkle_state() -> void:
	# Scintillation is atmospheric and does not happen in vacuum, so the
	# per-star flash phase/rate that Trinity's starfield carries is
	# deliberately absent here (ADR-0054 §4).
	var star: Dictionary = StarfieldScript.generate(3, 1)[0]
	var keys: Array = star.keys()
	keys.sort()
	assert_array(keys).contains_exactly(["color", "direction", "flux"])


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
	assert_float(field_peak).is_less_equal(StarfieldScript.FLUX_SCALE)


func test_flux_follows_the_visual_magnitude_scale() -> void:
	# One step of visual magnitude is a flux ratio of 10 ^ 0.4.
	var sirius: float = StarfieldScript.flux_for_magnitude(-1.46)
	var vega: float = StarfieldScript.flux_for_magnitude(0.03)
	assert_float(sirius / vega).is_equal_approx(pow(10.0, -0.4 * (-1.46 - 0.03)), 0.001)


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
