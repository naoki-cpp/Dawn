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
const POINT_SPREAD_PIXELS: float = 2.6
## Falloff inside the quad. Together with the quad size this fixes the
## Gaussian's sigma at POINT_SPREAD_PIXELS / sqrt(2 * POINT_SHARPNESS).
const POINT_SHARPNESS: float = 6.0
## Sigma must stay at or above roughly this, or the sprite falls below the
## pixel grid and flickers as the camera turns -- which would reintroduce the
## twinkle ADR-0054 §5 deliberately rejects, by accident.
const MIN_STABLE_SIGMA_PIXELS: float = 0.7

const DEFAULT_SEED: int = 0x5DA0
const DEFAULT_COUNT: int = 8000

## Share of the field drawn from the galactic disc rather than uniformly.
## A perfectly uniform field reads as flat wallpaper; the depth in a real sky
## comes from the Milky Way being a dense wall of distant stars behind a
## sparse scatter of near ones.
const DISC_FRACTION: float = 0.55
## Matches the sky shader's `disk = exp(-lat * 4.5)`, so the star density and
## the painted band agree on where the plane is.
const DISC_CONCENTRATION: float = 4.5
## Disc stars stand in for a more distant population, so they run fainter.
## This is what makes the band read as haze instead of a bright stripe.
const DISC_FLUX_FACTOR: float = 0.42
## Share of the disc population piled onto the galactic bulge. The sky shader
## already paints a bulge toward galactic +X; concentrating stars there too
## gives the band a centre to fall away from instead of an even stripe.
const BULGE_SHARE: float = 0.24
## Matches the shader's `exp(-bulge_dist * bulge_dist * 5.0)`.
const BULGE_CONCENTRATION: float = 5.0
## The bulge is the most distant population in view.
const BULGE_FLUX_FACTOR: float = 0.30

## Galactic frame, same rotation the sky shader's to_galactic() applies:
## a longitude spin then the ~60 degree tilt of the plane.
const GALACTIC_TILT: float = 1.05
const GALACTIC_LON: float = 1.57

## Apparent flux follows a steep power law because a magnitude distribution is
## steep: one step of visual magnitude is a flux ratio of about 2.5, so a
## handful of bright stars sit over a dense field of faint ones and a long tail
## falls below visibility entirely.
const FLUX_EXPONENT: float = 4.5
const FLUX_SCALE: float = 3.8


## Area-preserving direction on the unit sphere. `u` is the SINE of the
## latitude sampled uniformly, not the latitude itself -- that is what keeps
## the density flat instead of piling stars up at the poles the way an
## equirectangular grid does.
static func direction_for(u: float, theta: float) -> Vector3:
	var clamped: float = clampf(u, -1.0, 1.0)
	var ring: float = sqrt(maxf(0.0, 1.0 - clamped * clamped))
	return Vector3(ring * cos(theta), clamped, ring * sin(theta))


## Galactic direction back to world space -- the inverse of the sky shader's
## to_galactic(), so a latitude sampled in the galactic frame lands on the
## band that space_sky.gdshader paints.
static func from_galactic(galactic: Vector3) -> Vector3:
	var ct: float = cos(GALACTIC_TILT)
	var st: float = sin(GALACTIC_TILT)
	var cl: float = cos(GALACTIC_LON)
	var sl: float = sin(GALACTIC_LON)
	var ry: float = galactic.y * ct + galactic.z * st
	var rz: float = -galactic.y * st + galactic.z * ct
	return Vector3(galactic.x * cl - rz * sl, ry, galactic.x * sl + rz * cl)


## A direction concentrated toward the galactic plane. The height above the
## plane is drawn from an exponential so the density in solid angle follows
## exp(-|height| * DISC_CONCENTRATION), matching the painted band.
static func disc_direction(rng: RandomNumberGenerator) -> Vector3:
	var height: float = 1.0
	# Rejection: an exponential is unbounded but a direction's height is not.
	# Acceptance is ~99% at this concentration, and the bound keeps it finite.
	for _attempt: int in range(8):
		height = -log(maxf(1.0 - rng.randf(), 1e-9)) / DISC_CONCENTRATION
		if height <= 1.0:
			break
		height = rng.randf()
	if rng.randf() < 0.5:
		height = -height
	return from_galactic(direction_for(height, rng.randf() * TAU))


## A direction concentrated on the galactic bulge, which sits toward +X in the
## galactic frame. The angle off centre is Rayleigh-distributed, which is what
## a Gaussian in solid angle looks like once the sin(theta) area factor is
## folded in.
static func bulge_direction(rng: RandomNumberGenerator) -> Vector3:
	var angle: float = sqrt(
		-log(maxf(1.0 - rng.randf(), 1e-9)) / BULGE_CONCENTRATION)
	angle = minf(angle, PI)
	var phi: float = rng.randf() * TAU
	var galactic := Vector3(
		cos(angle), sin(angle) * cos(phi), sin(angle) * sin(phi))
	return from_galactic(galactic)


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
		var direction: Vector3
		if in_bulge:
			direction = bulge_direction(rng)
		elif structured:
			direction = disc_direction(rng)
		else:
			direction = direction_for(rng.randf() * 2.0 - 1.0, rng.randf() * TAU)

		var rarity: float = rng.randf()
		var spectral: float = spectral_t(rng.randf())
		var flux: float = pow(rarity, FLUX_EXPONENT) * FLUX_SCALE
		if in_bulge:
			flux *= BULGE_FLUX_FACTOR
		elif structured:
			flux *= DISC_FLUX_FACTOR
		stars.append({
			"direction": direction,
			"color": apparent_color(spectral, rarity),
			"flux": flux,
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
		multimesh.set_instance_custom_data(
			i, Color(star["flux"] as float, 0.0, 0.0, 0.0))
	return multimesh


## Standard deviation of the rendered point spread, in pixels. Exposed so the
## sampling guarantee above can be asserted rather than just asserted about.
static func point_spread_sigma_pixels() -> float:
	return POINT_SPREAD_PIXELS / sqrt(2.0 * POINT_SHARPNESS)
