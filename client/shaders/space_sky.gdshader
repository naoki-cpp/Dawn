// space_sky.gdshader
//
// Procedural space background grounded in real astronomical data.
// No external textures required.
//
// Astronomical references used:
//   Star layer      : NOT here. ADR-0054 moved the stars to sprite geometry
//                      (client/scripts/starfield.gd + star_sprite.gdshader);
//                      this file draws the nebula, Milky Way and local star.
//   Star colors     : owned by NavigationMarkerRenderer.spectral_color()
//                      (blackbody RGB by spectral type, Ballesteros 2012)
//   Star distribution: Kroupa IMF — M:K:G:F:A:B:O ≈ 75:12:7:3:2:0.6:0.00003%
//                      A magnitude-limited (naked-eye) sample instead runs
//                      B:A:F:G:K:M ~= 9:22:14:14:31:9, with O effectively absent
//   Star rendering  : point sources only. Diffraction spikes come from the
//                      vanes holding a telescope's secondary mirror, so an
//                      eye or a ship camera never sees them; the wide halo
//                      around a bright star is left to Environment.glow
//   Milky Way tilt  : Galactic plane ~63° from celestial equator
//   Galactic center : Sagittarius direction, ~8 kpc from Sun, dense bulge
//   Background      : Axisymmetric stellar disk, triaxial bulge, turbulent dust
//   Galactic light  : unresolved stellar continuum attenuated by a turbulent
//                      3D dust disk; no unobserved emission sources are invented.

shader_type sky;
#include "res://shaders/stellar_plasma.gdshaderinc"
#include "res://shaders/galactic_dust.gdshaderinc"

// ── Uniforms ─────────────────────────────────────────────────────────────────

// Diagnostic only: production materials keep extinction_scale at one. The
// shared physical model owns emissivity and radiance display calibration.
uniform float extinction_scale : hint_range(0.0, 1.0) = 1.0;
uniform vec3  ambient_color     : source_color             = vec3(0.002, 0.002, 0.006);
// Baked background (ADR-0055). The nebula and Milky Way are direction-only
// functions, so they are generated once into an equirectangular panorama and
// sampled from then on: one texture fetch replaces the full line-of-sight
// integration and its correlated density fields. The local star is NOT baked.
uniform sampler2D nebula_panorama : filter_linear, repeat_enable;
uniform bool  use_baked_nebula = false;
// True only while generating that panorama: drops the presentation vignette
// so it is applied once at runtime rather than burned into the texture.
uniform bool  bake_pass = false;

// ── Local star (ADR-0025) ─────────────────────────────────────────────────────
// sun_direction: normalised world-space direction from ship toward the star.
// sun_active: 0=no star data yet, 1=render sun.
// sun_color: spectral blackbody colour of the local star.
uniform vec3  sun_direction : source_color = vec3(1.0, 0.0, 0.0);
uniform float sun_active               = 0.0;
uniform vec3  sun_color : source_color = vec3(1.00, 0.95, 0.80);
// Apparent angular radius in radians, derived from star radius / observer
// distance by WorldPresentation. The default preserves the original disc
// size until the first body record is available.
uniform float sun_angular_radius      = 0.0261799;
uniform float sun_radius_m = 696000000.0;
// Set from the shared StellarSource model; no second brightness authority.
uniform float sun_radiance = 0.0;
uniform float sun_surface_color_scale = 0.0;
uniform float sun_direct_fraction = 1.0;

// ── Hash / Noise ──────────────────────────────────────────────────────────────
// Note: PI and TAU are Godot 4 built-in shader constants.

// ── Main ──────────────────────────────────────────────────────────────────────

// Build a tangent frame around the local star. The sky shader has no
// finite-distance surface to shade, so this projects procedural granulation
// onto the apparent disc without introducing a second position authority.
void sun_frame(vec3 sun_dir, out vec3 sun_right, out vec3 sun_up) {
	vec3 reference_up = vec3(0.0, 1.0, 0.0);
	if (abs(sun_dir.y) > 0.95) {
		reference_up = vec3(1.0, 0.0, 0.0);
	}
	sun_right = normalize(cross(reference_up, sun_dir));
	sun_up = normalize(cross(sun_dir, sun_right));
}

float solar_granulation(vec3 surface_normal) {
	// Solar-like granules: ~1000 km cells, evolving on a ten-minute timescale.
	vec3 p = surface_normal * (sun_radius_m / 1000000.0);
	float footprint = max(length(dFdx(p)), length(dFdy(p)));
	float resolved = 1.0 - smoothstep(0.35, 1.25, footprint);
	if (resolved <= 0.0) {
		return 1.0;
	}
	// Bend cell boundaries so the surface does not read as polygonal tiles.
	vec3 warp = p * 1.7;
	p += 0.22 * (vec3(snoise(warp), snoise(warp + vec3(19.0, 7.0, 3.0)),
		snoise(warp + vec3(5.0, 23.0, 11.0))) - vec3(0.5));
	ivec3 cell = ivec3(floor(p));
	vec3 local = fract(p);
	float nearest = 100.0;
	float second = 100.0;
	float brightness = 1.0;
	for (int z = -1; z <= 1; z++) {
		for (int y = -1; y <= 1; y++) {
			for (int x = -1; x <= 1; x++) {
				ivec3 offset = ivec3(x, y, z);
				ivec3 id = cell + offset;
				vec3 seed = vec3(hash_lattice(id), hash_lattice(id + ivec3(37, 11, 53)),
					hash_lattice(id + ivec3(17, 71, 29)));
				vec3 centre = vec3(offset) + 0.5 + 0.4 * sin(seed * TAU + TIME * (TAU / 600.0) * (0.8 + seed.z * 0.4));
				float distance_squared = dot(centre - local, centre - local);
				if (distance_squared < nearest) {
					second = nearest;
					nearest = distance_squared;
					brightness = mix(0.96, 1.12, seed.x);
				} else {
					second = min(second, distance_squared);
				}
			}
		}
	}
	float boundary = sqrt(second) - sqrt(nearest);
	float lane_width = max(0.08, footprint * 0.35);
	float interior = smoothstep(0.0, lane_width, boundary);
	float hot_centre = mix(0.94, brightness, exp(-nearest * 2.0));
	return mix(1.0, mix(0.74, hot_centre, interior), resolved);
}

float filtered_solar_detail(vec3 p) {
	float footprint = max(length(dFdx(p)), length(dFdy(p)));
	float resolved = 1.0 - smoothstep(0.3, 1.2, footprint);
	if (resolved <= 0.0) return 0.48;
	return mix(0.48, fbm(p), resolved);
}

vec3 sun_surface_color(vec3 surface_normal, float mu) {
	// Limb darkening is the visible consequence of looking through less of the
	// photosphere near its edge.
	float limb_darkening = mix(0.68, 1.0, mu);
	vec3 drift = solar_surface_drift();
	float activity = fbm(surface_normal * 32.0 + drift);
	float filaments = filtered_solar_detail(surface_normal * 160.0 + drift);
	float heat = clamp(0.42 + (activity - 0.48) * 0.65
		+ (filaments - 0.48) * 1.1
		+ (solar_granulation(surface_normal) - 1.0) * 0.65, 0.0, 1.0);
	// False-colour solar-observation palette, not a blackbody temperature map.
	vec3 surface = solar_surface_palette(heat);
	surface *= mix(0.78, 1.0, smoothstep(0.30, 0.58, activity));
	// A shared, warped field relates dark channels, bright ribbons and knots.
	// This is a visual field, not a magnetic or plasma simulation.
	vec3 field_pos = surface_normal * 4.3 + drift;
	vec3 warp = vec3(snoise(field_pos + vec3(13.0)),
		snoise(field_pos + vec3(5.0, 19.0, 3.0)), snoise(field_pos + vec3(7.0, 2.0, 23.0)));
	field_pos += (warp - vec3(0.5)) * 0.8;
	float field = snoise(field_pos) * 2.0 - 1.0;
	float envelope = snoise(surface_normal * 2.8 + vec3(8.0, 17.0, 2.0));
	float field_width = max(fwidth(field), 0.008);
	float channel = 1.0 - smoothstep(0.004, 0.020 + field_width, abs(field));
	float filament = channel * smoothstep(0.30, 0.58, envelope);
	surface *= solar_filament_transmission(filament, 0.42);
	float region = smoothstep(0.56, 0.78, envelope);
	float ribbon_distance = abs(abs(field) - 0.23);
	float ribbon = 1.0 - smoothstep(0.008, 0.035 + field_width, ribbon_distance);
	float knots = pow(smoothstep(0.24, 0.62, abs(field)), 2.0);
	float structure = smoothstep(0.34, 0.64, snoise(field_pos * 5.0 + vec3(11.0)))
		* mix(0.25, 1.0, smoothstep(0.22, 0.78, filaments));
	float active = region * (ribbon * 0.25 + knots) * structure;
	surface += vec3(3.2, 1.05, 0.13) * active;
	return surface * limb_darkening * 2.0;
}

// Equirectangular layout of RenderingServer.sky_bake_panorama(), measured
// rather than assumed: u=0 is -Z, u=0.25 is -X, u=0.5 is +Z, u=0.75 is +X,
// and v runs from +Y at 0 to -Y at 1. Getting the u sign wrong samples a
// different part of the sky, which silently misaligns the painted band with
// the starfield's galactic clustering. Negative u relies on repeat_enable,
// which also keeps the seam filtering correctly.
vec2 panorama_uv(vec3 dir) {
	return vec2(atan(-dir.x, -dir.z) / TAU, acos(clamp(dir.y, -1.0, 1.0)) / PI);
}

vec3 solar_corona(vec3 ray, vec3 sun_dir, float angle, float radius) {
	// Angular extent is bounded independently of a close, screen-filling disc.
	float height = (angle - radius) / min(radius, 0.22);
	float outer_height = max(height, 0.0);
	vec3 radial = normalize(ray - sun_dir * dot(ray, sun_dir));
	// Slowly bend streamers; preserve long radial structure instead of noisy teeth.
	float broad = snoise(radial * 5.3 + vec3(4.0, 1.0, 9.0));
	float bend = snoise(radial * 8.0 + vec3(17.0)) - 0.5;
	vec3 flow = normalize(radial + cross(sun_dir, radial) * (outer_height * bend * 0.6));
	float reach = mix(0.10, 0.45, pow(smoothstep(0.2, 0.8, broad), 2.0));
	vec3 fibre_pos = flow * 52.0 + vec3(height * 1.8 + TIME * 0.0007);
	float footprint = max(length(dFdx(fibre_pos)), length(dFdy(fibre_pos)));
	float fibres = mix(snoise(fibre_pos), 0.5, smoothstep(0.4, 1.2, footprint));
	float strands = mix(0.65, 1.0, smoothstep(0.28, 0.76, fibres));
	float streamers = exp(-outer_height / reach * 3.5) * strands * (0.25 + broad);
	// A continuous narrow chromospheric envelope straddles the silhouette.
	float rim_width = max(fwidth(height) * 1.5, 0.008);
	float rim = exp(-abs(height) / rim_width) * (0.8 + broad * 0.4);
	float inner = exp(-outer_height * 35.0) * 0.08;
	float diffuse = exp(-outer_height * 9.0) * 0.025;
	float fade = smoothstep(-0.03, 0.0, height) * (1.0 - smoothstep(0.60, 0.85, height));
	return vec3(1.6, 0.38, 0.025) * rim
		+ (vec3(0.9, 0.055, 0.003) * (streamers * 0.08 + inner)
		+ vec3(0.65, 0.013, 0.001) * diffuse) * fade;
}

void sky() {
	vec3 dir = normalize(EYEDIR);
	vec3 col;
	if (use_baked_nebula) {
		col = texture(nebula_panorama, panorama_uv(dir)).rgb;
	} else {
		col = ambient_color;

		vec3 galactic_direction = to_galactic(dir);
		col += galactic_integrated_radiance(galactic_direction, extinction_scale);
	}

	// ── Local star disc + corona (ADR-0025) ──────────────────────────────
	// Always live: the star's direction changes as the ship moves, so it can
	// never be part of a baked, direction-only background.
	if (sun_active > 0.5) {
		vec3 sun_dir = normalize(sun_direction);
		float cos_angle = dot(dir, sun_dir);
		vec3 sun_right;
		vec3 sun_up;
		sun_frame(sun_dir, sun_right, sun_up);

		// Photosphere disc uses the star's actual apparent angular radius.
		float disc_radius_angle = max(sun_angular_radius, 0.0001);
		// atan2 remains stable where dot/acos rounds a subpixel separation to zero.
		float sun_angle = atan(length(cross(dir, sun_dir)), cos_angle);
		float pixel_angle = max(max(length(dFdx(dir)), length(dFdy(dir))), 0.000001);
		float resolved = smoothstep(4.0, 12.0, disc_radius_angle / pixel_angle);
		if (resolved > 0.0 && sun_angle < disc_radius_angle) {
			vec2 disc_uv = vec2(
				dot(dir, sun_right),
				dot(dir, sun_up)
			) / sin(disc_radius_angle);
			float disc_radius = length(disc_uv);
			// Ray/sphere intersection in stellar-radius units, including near views.
			float distance_in_radii = 1.0 / sin(disc_radius_angle);
			float discriminant = max(0.0, 1.0 - dot(disc_uv, disc_uv));
			float ray_distance = distance_in_radii * cos_angle - sqrt(discriminant);
			vec3 surface_normal = normalize(dir * ray_distance - sun_dir * distance_in_radii);
			float edge_width = max(fwidth(disc_radius), 0.0001);
			float disc_mask = 1.0 - smoothstep(1.0 - edge_width, 1.0, disc_radius);
			vec3 surface = sun_surface_color(surface_normal, sqrt(discriminant));
			col = mix(col, surface * sun_radiance * sun_surface_color_scale
				* sun_direct_fraction, disc_mask * resolved);
		}
		// Exposure/display approximation for an unresolved bright source. The
		// integrated kernel scales with apparent disc area, not a fixed halo size.
		float psf_variance = pixel_angle * pixel_angle * 0.64;
		float variance = disc_radius_angle * disc_radius_angle * 0.25 + psf_variance;
		if (resolved < 1.0 && sun_angle * sun_angle < variance * 100.0) {
			float flux = disc_radius_angle * disc_radius_angle / (2.0 * variance);
			float separation = sun_angle * sun_angle / (2.0 * variance);
			float source = flux * exp(-separation);
			vec3 point_tint = sun_color;
			col += point_tint * source * sun_radiance * sun_direct_fraction * (1.0 - resolved);
		}

		// A structured corona, separate from camera optics and prominence meshes.
		if (sun_angle > disc_radius_angle - min(disc_radius_angle, 0.22) * 0.03 && sun_angle < disc_radius_angle + min(disc_radius_angle, 0.22) * 0.85) {
			col += solar_corona(dir, sun_dir, sun_angle, disc_radius_angle);
		}

	}

	// No world-up vignette: empty space has no horizon or atmospheric gradient.

	col = max(col, vec3(0.0));
	COLOR = col;
}
