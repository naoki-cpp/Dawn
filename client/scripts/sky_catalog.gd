## A small, explicit bright-star layer for the procedural sky.
##
## The diffuse Milky Way remains a deliberately stylized approximation. These
## stars are the opposite: their directions, colors, and visual magnitudes are
## seeded from the brightest naked-eye stars so the sky has stable landmarks
## instead of only hash noise. Coordinates use the shader's equatorial frame.
class_name SkyCatalog
extends RefCounted

const _STARS: Array[Dictionary] = [
	{"ra": 6.7525, "dec": -16.7161, "color": Vector3(0.72, 0.84, 1.0), "magnitude": -1.46}, # Sirius
	{"ra": 6.3992, "dec": -52.6957, "color": Vector3(1.0, 0.94, 0.78), "magnitude": -0.74}, # Canopus
	{"ra": 14.2610, "dec": 19.1825, "color": Vector3(1.0, 0.68, 0.38), "magnitude": -0.05}, # Arcturus
	{"ra": 18.6156, "dec": 38.7837, "color": Vector3(0.72, 0.84, 1.0), "magnitude": 0.03}, # Vega
	{"ra": 5.2782, "dec": 45.9980, "color": Vector3(1.0, 0.91, 0.60), "magnitude": 0.08}, # Capella
	{"ra": 5.2423, "dec": -8.2016, "color": Vector3(0.60, 0.76, 1.0), "magnitude": 0.13}, # Rigel
	{"ra": 5.9195, "dec": 7.4071, "color": Vector3(1.0, 0.52, 0.28), "magnitude": 0.50}, # Betelgeuse
	{"ra": 7.6550, "dec": 5.2250, "color": Vector3(0.82, 0.90, 1.0), "magnitude": 0.38}, # Procyon
	{"ra": 1.6286, "dec": -57.2368, "color": Vector3(0.62, 0.78, 1.0), "magnitude": 0.45}, # Achernar
	{"ra": 19.8464, "dec": 8.8683, "color": Vector3(0.80, 0.88, 1.0), "magnitude": 0.77}, # Altair
	{"ra": 14.6608, "dec": -60.8350, "color": Vector3(0.70, 0.84, 1.0), "magnitude": 0.61}, # Alpha Centauri
	{"ra": 12.4433, "dec": -63.0991, "color": Vector3(0.62, 0.76, 1.0), "magnitude": 0.61}, # Acrux
]

static func _direction(ra_hours: float, dec_degrees: float) -> Vector3:
	var ra := deg_to_rad(ra_hours * 15.0)
	var dec := deg_to_rad(dec_degrees)
	var cos_dec := cos(dec)
	return Vector3(cos_dec * cos(ra), sin(dec), cos_dec * sin(ra)).normalized()


static func directions() -> PackedVector3Array:
	var result := PackedVector3Array()
	for star: Dictionary in _STARS:
		result.append(_direction(star.ra, star.dec))
	while result.size() < 16:
		result.append(Vector3.ZERO)
	return result


static func colors() -> PackedVector3Array:
	var result := PackedVector3Array()
	for star: Dictionary in _STARS:
		result.append(star.color)
	while result.size() < 16:
		result.append(Vector3.ZERO)
	return result


static func brightness() -> PackedFloat32Array:
	var result := PackedFloat32Array()
	for star: Dictionary in _STARS:
		# Relative visual flux, normalized around a zero-magnitude star.
		result.append(pow(10.0, -0.4 * float(star.magnitude)) * 0.12)
	while result.size() < 16:
		result.append(0.0)
	return result
