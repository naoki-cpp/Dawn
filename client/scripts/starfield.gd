## starfield.gd
##
## The star layer (ADR-0054). Star directions, colours and apparent flux are
## generated once on the CPU from a fixed seed and drawn as billboard sprites,
## instead of being evaluated per pixel inside the sky shader.
##
## Generating on the CPU removes four problems that belong to the fragment
## shader approach rather than to the art: a tight angular kernel built from
## `1.0 - dot()` loses its sign to float32 cancellation, a coordinate-scaled
## float hash loses its entropy as indices grow, the cost scales with pixels
## instead of with stars, and an analytically drawn point has no filter so its
## size has to be tuned against the output resolution. See ADR-0054 §背景.
class_name Starfield
extends RefCounted

const NavigationMarkerRendererScript = preload("res://scripts/navigation_marker_renderer.gd")
const SkyCatalogScript = preload("res://scripts/sky_catalog.gd")
const GalacticModelScript = preload("res://scripts/galactic_model.gd")

## Radius of the sprite shell. Inside the camera's 5,000,000 far plane but
## beyond anything the scene places, so ordinary depth testing keeps ships in
## front and the starfield needs no depth trickery of its own.
const SHELL_RADIUS: float = 4_000_000.0

## Sprites are expanded to a fixed number of PIXELS in the vertex shader, not
## to a fixed angular size. A real point source stays a point however far the
## camera zooms.
##
## Every star gets the SAME point spread. Stars are point sources; the only
## reason one looks bigger than another is that a brighter one keeps its
## Gaussian above the visible threshold further out. Varying the sprite size
## per star on top of that double-counts brightness and turns the bright end
## into balls rather than points.
## The shared core remains near one pixel while the quad carries a faint,
## photographic halo. These are screen-space optical parameters, not star size.
const POINT_SPREAD_PIXELS: float = 8.0
const POINT_CORE_SIGMA_PIXELS: float = 0.72
const POINT_HALO_SIGMA_PIXELS: float = 2.4
const POINT_HALO_PEAK_RATIO: float = 0.008
## Sigma must stay at or above roughly this, or the sprite falls below the
## pixel grid and flickers as the camera turns -- which would reintroduce the
## twinkle ADR-0054 §5 deliberately rejects, by accident.
const MIN_STABLE_SIGMA_PIXELS: float = 0.7

const DEFAULT_SEED: int = 0x5DA0
const DEFAULT_COUNT: int = 16000

## Share of the field drawn from the galactic disc rather than uniformly.
## A perfectly uniform field reads as flat wallpaper; the depth in a real sky
## comes from the Milky Way being a dense wall of distant stars behind a
## sparse scatter of near ones.
const DISC_FRACTION: float = 0.66
## The synthetic field combines a local foreground with distant Galactic
## populations; it is a rendering sample, not a volume-complete catalogue.
const GALACTIC_CENTER_DISTANCE_KPC: float = GalacticModelScript.GALACTIC_CENTER_RADIUS_KPC
const SUN_GALACTIC_POSITION_KPC := GalacticModelScript.SUN_POSITION_KPC
const MAX_STAR_DISTANCE_KPC: float = 22.0
const LOCAL_FIELD_MAX_DISTANCE_KPC: float = 2.5
const THIN_DISK_RADIAL_SCALE_KPC: float = GalacticModelScript.THIN_DISK_RADIAL_SCALE_KPC
const THICK_DISK_RADIAL_SCALE_KPC: float = GalacticModelScript.THICK_DISK_RADIAL_SCALE_KPC
const THIN_DISK_SCALE_HEIGHT_KPC: float = GalacticModelScript.THIN_DISK_SCALE_HEIGHT_KPC
const THICK_DISK_SCALE_HEIGHT_KPC: float = GalacticModelScript.THICK_DISK_SCALE_HEIGHT_KPC
const THICK_DISK_SHARE: float = GalacticModelScript.THICK_DISK_LOCAL_WEIGHT
const SPIRAL_ARM_SHARE: float = 0.28
const SPIRAL_ARM_COUNT: int = GalacticModelScript.SPIRAL_ARM_COUNT
const SPIRAL_ARM_PITCH_RADIANS: float = GalacticModelScript.SPIRAL_ARM_PITCH_RADIANS
const SPIRAL_ARM_WIDTH_KPC: float = GalacticModelScript.SPIRAL_ARM_WIDTH_KPC
const SPIRAL_ARM_MIN_RADIUS_KPC: float = GalacticModelScript.SPIRAL_ARM_MIN_RADIUS_KPC
const SPIRAL_ARM_MAX_RADIUS_KPC: float = GalacticModelScript.SPIRAL_ARM_MAX_RADIUS_KPC
## Disc stars stand in for a more distant population, so they run fainter.
## This is what makes the band read as haze instead of a bright stripe.
const DISC_FLUX_FACTOR: float = 0.42
## A visual sampling share, not the Milky Way's bulge-to-disk mass ratio.
const BULGE_SHARE: float = 0.24
const BULGE_FLUX_FACTOR: float = 0.30
const BAR_ANGLE_RADIANS: float = GalacticModelScript.BULGE_ANGLE_RADIANS
const BAR_SEMI_AXES_KPC := GalacticModelScript.BULGE_SIGMA_AXES_KPC

## Apparent flux follows a steep power law because a magnitude distribution is
## steep: one step of visual magnitude is a flux ratio of about 2.5, so a
## handful of bright stars sit over a dense field of faint ones and a long tail
## falls below visibility entirely.
const LUMINOSITY_PARETO_SHAPE: float = 1.8
const LUMINOSITY_PARETO_SCALE: float = 0.12
const LUMINOSITY_BASELINE: float = 0.035
const LUMINOSITY_RENDER_GAIN: float = 3.8
const MAX_GENERATED_FLUX: float = 8.0


## Area-preserving direction on the unit sphere. `u` is the SINE of the
## latitude sampled uniformly, not the latitude itself -- that is what keeps
## the density flat instead of piling stars up at the poles the way an
## equirectangular grid does.
static func direction_for(u: float, theta: float) -> Vector3:
	var clamped: float = clampf(u, -1.0, 1.0)
	var ring: float = sqrt(maxf(0.0, 1.0 - clamped * clamped))
	return Vector3(ring * cos(theta), clamped, ring * sin(theta))


## Transform between the catalogue's equatorial Cartesian axes and the IAU
## Galactic frame. The middle component is latitude/height above the plane.
static func to_galactic(equatorial: Vector3) -> Vector3:
	var standard_equatorial := Vector3(equatorial.x, equatorial.z, equatorial.y)
	var standard_galactic := Vector3(
		Vector3(-0.05487556, -0.87343709, -0.48383502).dot(standard_equatorial),
		Vector3(0.49410943, -0.44482963, 0.74698224).dot(standard_equatorial),
		Vector3(-0.86766615, -0.19807637, 0.45598378).dot(standard_equatorial))
	return Vector3(standard_galactic.x, standard_galactic.z, standard_galactic.y)


## Inverse of the IAU equatorial-to-Galactic rotation used by the sky shader.
static func from_galactic(galactic: Vector3) -> Vector3:
	var standard_galactic := Vector3(galactic.x, galactic.z, galactic.y)
	var standard_equatorial := Vector3(
		Vector3(-0.05487556, 0.49410943, -0.86766615).dot(standard_galactic),
		Vector3(-0.87343709, -0.44482963, -0.19807637).dot(standard_galactic),
		Vector3(-0.48383502, 0.74698224, 0.45598378).dot(standard_galactic))
	return Vector3(standard_equatorial.x, standard_equatorial.z, standard_equatorial.y)


## An area-weighted exponential disk has radial density proportional to
## R * exp(-R / scale_length), i.e. a gamma distribution with shape two.
static func _sample_disk_radius(rng: RandomNumberGenerator, scale_length: float) -> float:
	var first: float = maxf(rng.randf(), 1e-9)
	var second: float = maxf(rng.randf(), 1e-9)
	return -scale_length * (log(first) + log(second))


static func _sample_standard_normal(rng: RandomNumberGenerator) -> float:
	var radius: float = sqrt(-2.0 * log(maxf(rng.randf(), 1e-9)))
	return radius * cos(TAU * rng.randf())


static func _sample_disk_position(
	rng: RandomNumberGenerator, in_spiral_arm: bool
) -> Vector3:
	var thick: bool = rng.randf() < THICK_DISK_SHARE
	var scale_length: float = THICK_DISK_RADIAL_SCALE_KPC if thick \
		else THIN_DISK_RADIAL_SCALE_KPC
	var scale_height: float = THICK_DISK_SCALE_HEIGHT_KPC if thick \
		else THIN_DISK_SCALE_HEIGHT_KPC
	var radius: float = _sample_disk_radius(rng, scale_length)
	if in_spiral_arm:
		for _attempt: int in range(16):
			radius = _sample_disk_radius(rng, scale_length)
			if radius >= SPIRAL_ARM_MIN_RADIUS_KPC \
				and radius <= SPIRAL_ARM_MAX_RADIUS_KPC:
				break
		radius = clampf(radius, SPIRAL_ARM_MIN_RADIUS_KPC, SPIRAL_ARM_MAX_RADIUS_KPC)
	var angle: float = rng.randf() * TAU
	if in_spiral_arm:
		var arm: int = rng.randi_range(0, SPIRAL_ARM_COUNT - 1)
		var arm_angle: float = GalacticModelScript.SPIRAL_ARM_BASE_ANGLE_RADIANS \
			+ float(arm) * TAU / float(SPIRAL_ARM_COUNT) \
			+ log(radius / GalacticModelScript.SPIRAL_ARM_REFERENCE_RADIUS_KPC) \
			/ tan(SPIRAL_ARM_PITCH_RADIANS)
		angle = arm_angle + _sample_standard_normal(rng) * SPIRAL_ARM_WIDTH_KPC / radius
	var height: float = -log(maxf(rng.randf(), 1e-9)) * scale_height
	if rng.randf() < 0.5:
		height = -height
	return Vector3(radius * cos(angle), height, radius * sin(angle))


static func _sample_bulge_position(rng: RandomNumberGenerator) -> Vector3:
	var local := Vector3(
		_sample_standard_normal(rng) * BAR_SEMI_AXES_KPC.x,
		_sample_standard_normal(rng) * BAR_SEMI_AXES_KPC.y,
		_sample_standard_normal(rng) * BAR_SEMI_AXES_KPC.z)
	var c: float = cos(BAR_ANGLE_RADIANS)
	var s: float = sin(BAR_ANGLE_RADIANS)
	return Vector3(local.x * c + local.z * s, local.y, -local.x * s + local.z * c)


## Convert a physical Galactic position into an observer-relative direction
## and path length so foreground dust is integrated only to that source.
static func _project_position(position_kpc: Vector3) -> Dictionary:
	var offset: Vector3 = position_kpc - SUN_GALACTIC_POSITION_KPC
	var distance: float = maxf(offset.length(), 0.01)
	return {
		"direction": from_galactic(offset / distance),
		"distance_kpc": minf(distance, MAX_STAR_DISTANCE_KPC),
	}


## Spectral mix for the stars an observer actually sees. The underlying initial
## mass function is dominated by M dwarfs, but they are far too faint to
## appear; a magnitude-limited sample runs roughly B 9% / A 22% / F 14% /
## G 14% / K 31% / M 9%, with O effectively absent. Returns the `t` that
## NavigationMarkerRenderer.spectral_color() expects, 0 = hottest.
static func spectral_t(u: float) -> float:
	var bands: Array = [
		[0.010, 0.00, 0.10],  # O
		[0.100, 0.10, 0.25],  # B
		[0.320, 0.25, 0.40],  # A
		[0.460, 0.40, 0.55],  # F
		[0.600, 0.55, 0.68],  # G
		[0.910, 0.68, 0.83],  # K
		[1.001, 0.83, 1.00],  # M
	]
	var low_u: float = 0.0
	for band: Array in bands:
		var high_u: float = band[0] as float
		if u < high_u:
			var span: float = high_u - low_u
			var lerp_t: float = 0.0 if span <= 0.0 else (u - low_u) / span
			return lerpf(band[1] as float, band[2] as float, lerp_t)
		low_u = high_u
	return 1.0


## Faint stars deliver too few photons to read as strongly coloured, so their
## tint washes toward neutral white while the bright end keeps its spectral
## colour.
static func apparent_color(spectral: float, rarity: float) -> Color:
	var pure: Color = NavigationMarkerRendererScript.spectral_color(spectral)
	var saturation: float = clampf(0.45 + rarity * 0.90, 0.0, 1.0)
	return Color(1.0, 1.0, 1.0).lerp(pure, saturation)


## Visual magnitude to relative flux, normalised so a zero-magnitude star sits
## just above the procedural field. Shared by the named catalogue so landmarks
## and the statistical field live on one brightness scale.
static func flux_for_magnitude(magnitude: float) -> float:
	return pow(10.0, -0.4 * magnitude) * 2.2


## Capped Pareto-like brightness sample: many dim points, sparse bright stars.
## The distribution is a visual population prior, not a luminosity function.
static func flux_from_rank(rank: float) -> float:
	var tail_probability := maxf(1.0 - clampf(rank, 0.0, 1.0), 1e-9)
	var pareto := pow(tail_probability, -1.0 / LUMINOSITY_PARETO_SHAPE) - 1.0
	var relative_luminosity := LUMINOSITY_BASELINE + LUMINOSITY_PARETO_SCALE * pareto
	return minf(relative_luminosity * LUMINOSITY_RENDER_GAIN, MAX_GENERATED_FLUX)


## Deterministic star records for a seed. Pure: no scene tree, no rendering.
## `disc_fraction` of the field is concentrated on the galactic plane and runs
## fainter; pass 0.0 for a plain uniform sphere.
static func generate(
	star_seed: int, count: int, disc_fraction: float = DISC_FRACTION
) -> Array[Dictionary]:
	var rng := RandomNumberGenerator.new()
	rng.seed = star_seed
	var stars: Array[Dictionary] = []
	for _i: int in range(maxi(0, count)):
		var structured: bool = rng.randf() < disc_fraction
		var in_bulge: bool = structured and rng.randf() < BULGE_SHARE
		var in_arm: bool = structured and not in_bulge \
			and rng.randf() < SPIRAL_ARM_SHARE
		var position: Vector3
		var direction: Vector3
		var distance_kpc: float
		var projection: Dictionary
		if in_bulge:
			position = _sample_bulge_position(rng)
			projection = _project_position(position)
			direction = projection["direction"] as Vector3
			distance_kpc = projection["distance_kpc"] as float
		elif structured:
			position = _sample_disk_position(rng, in_arm)
			projection = _project_position(position)
			direction = projection["direction"] as Vector3
			distance_kpc = projection["distance_kpc"] as float
		else:
			direction = direction_for(rng.randf() * 2.0 - 1.0, rng.randf() * TAU)
			distance_kpc = LOCAL_FIELD_MAX_DISTANCE_KPC * pow(rng.randf(), 1.0 / 3.0)

		var rarity: float = rng.randf()
		var spectral: float = spectral_t(rng.randf())
		# Spiral arms host young, luminous stars; keep them a minority of the
		# sampled arm population rather than tinting every arm star blue.
		if in_arm and rng.randf() < 0.08:
			spectral = rng.randf() * 0.40
		var flux: float = flux_from_rank(rarity)
		if in_bulge:
			flux *= BULGE_FLUX_FACTOR
		elif structured:
			flux *= DISC_FLUX_FACTOR
		stars.append({
			"direction": direction,
			"color": apparent_color(spectral, rarity),
			"flux": flux,
			"distance_kpc": distance_kpc,
		})
	return stars


## The named bright stars, on the same record shape and the same flux scale as
## the statistical field, so Sirius outshines it instead of hiding under it.
static func catalog_stars() -> Array[Dictionary]:
	var stars: Array[Dictionary] = []
	for entry: Dictionary in SkyCatalogScript.entries():
		var flux: float = flux_for_magnitude(entry["magnitude"] as float)
		stars.append({
			"direction": entry["direction"] as Vector3,
			"color": entry["color"] as Color,
			"flux": flux,
			# Named bright stars are explicit sky landmarks; a short local
			# foreground path avoids pretending their catalogue distances exist.
			"distance_kpc": 0.25,
		})
	return stars


## Packs star records into a MultiMesh. Instance colour carries the spectral
## tint and custom data carries flux and pixel footprint, so the sprite shader
## needs no per-star texture or uniform array.
static func build_multimesh(stars: Array[Dictionary]) -> MultiMesh:
	var quad := QuadMesh.new()
	quad.size = Vector2.ONE

	var multimesh := MultiMesh.new()
	multimesh.transform_format = MultiMesh.TRANSFORM_3D
	multimesh.use_colors = true
	multimesh.use_custom_data = true
	multimesh.mesh = quad
	multimesh.instance_count = stars.size()

	for i: int in range(stars.size()):
		var star: Dictionary = stars[i]
		var origin: Vector3 = (star["direction"] as Vector3) * SHELL_RADIUS
		multimesh.set_instance_transform(i, Transform3D(Basis.IDENTITY, origin))
		multimesh.set_instance_color(i, star["color"] as Color)
		multimesh.set_instance_custom_data(i, Color(
			star["flux"] as float,
			clampf((star["distance_kpc"] as float) / MAX_STAR_DISTANCE_KPC, 0.0, 1.0),
			0.0,
			0.0))
	return multimesh


## Standard deviation of the Gaussian core, in pixels. Exposed so its sampling
## stability guarantee can be asserted rather than just asserted about.
static func point_spread_sigma_pixels() -> float:
	return POINT_CORE_SIGMA_PIXELS
