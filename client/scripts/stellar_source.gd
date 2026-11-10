## Shared scene-linear source model for the sky and camera PSF, not SI radiometry.
extends RefCounted

const SURFACE_RADIANCE := 160.0
const LENS_RESPONSE_GAIN := 10.0
const LENS_RESPONSE_KNEE_RADIUS := 0.006


## Integrated image-plane energy in viewport-height coordinates.
static func flux(projected_radius: float) -> float:
	if not is_finite(projected_radius) or projected_radius <= 0.0:
		return 0.0
	return SURFACE_RADIANCE * PI * projected_radius * projected_radius


## Smooth camera response: unresolved stars retain lens visibility while large
## sources approach a bounded highlight level without a distance-based switch.
static func lens_flux(projected_radius: float) -> float:
	var incident := flux(projected_radius)
	if incident <= 0.0:
		return 0.0
	var knee := flux(LENS_RESPONSE_KNEE_RADIUS)
	return LENS_RESPONSE_GAIN * incident / (1.0 + incident / knee)


## Uniform disc's second moment along either image-plane axis.
static func variance(projected_radius: float) -> float:
	if not is_finite(projected_radius) or projected_radius <= 0.0:
		return 0.0
	return projected_radius * projected_radius * 0.25
