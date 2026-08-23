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
//   Spiral arms     : Logarithmic spiral pitch angle ~12.5° (Reid et al. 2014)
//   Nebula colors   : Emission line wavelengths
//                       Hα  656.3 nm → (1.00, 0.11, 0.08)
//                       Hβ  486.1 nm → (0.22, 0.46, 1.00)
//                       OIII 500.7 nm → (0.14, 0.82, 0.70)
//                       SII  671.6 nm → (0.87, 0.06, 0.05)

shader_type sky;

// ── Uniforms ─────────────────────────────────────────────────────────────────

uniform float nebula_strength   : hint_range(0.0,  1.0)   = 0.50;
uniform float milkyway_strength : hint_range(0.0,  1.0)   = 0.20;
uniform vec3  ambient_color     : source_color             = vec3(0.002, 0.002, 0.006);
// Baked background (ADR-0055). The nebula and Milky Way are direction-only
// functions, so they are generated once into an equirectangular panorama and
// sampled from then on: one texture fetch replaces ~280 hash evaluations per
// pixel. The local star is NOT baked -- it moves with the ship.
uniform sampler2D nebula_panorama : filter_linear, repeat_enable;
uniform bool  use_baked_nebula = false;
// True only while generating that panorama: drops the presentation vignette
// so it is applied once at runtime rather than burned into the texture.
uniform bool  bake_pass = false;
uniform vec3  milkyway_core_color : source_color         = vec3(0.52, 0.62, 0.92);
uniform vec3  milkyway_outer_color : source_color        = vec3(0.72, 0.68, 0.58);
uniform vec3  nebula_primary_color : source_color       = vec3(1.00, 0.11, 0.08);
uniform vec3  nebula_secondary_color : source_color     = vec3(0.55, 0.18, 0.06);
uniform vec3  nebula_highlight_color : source_color     = vec3(1.00, 0.38, 0.12);

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
// Screen-space axes supplied by the camera for optical lens effects.
uniform vec3  sun_flare_right : source_color = vec3(1.0, 0.0, 0.0);
uniform vec3  sun_flare_up : source_color = vec3(0.0, 1.0, 0.0);

// ── Hash / Noise ──────────────────────────────────────────────────────────────
// Note: PI and TAU are Godot 4 built-in shader constants.

// Integer lattice hash. The previous construction -- multiply by a large
// irrational and take fract() -- loses its entropy as the input grows, because
// the spacing between representable 32-bit floats grows with magnitude. At a
// cell index near 800, index * 443.8975 lands around 3.5e5 where that spacing
// is 0.0625, so fract() kept only a handful of bits: measured over the finest
// star layer it produced ~5.6k distinct values for 353k cells, and the deep fbm
// octaves (whose coordinates pass 1600) collapsed to ~2.9k values across 2.9M
// lattice points. Mixing the integer coordinates has no magnitude dependence
// and measures flat (chi-square/dof ~1.0, neighbour correlation ~2e-4).
//
// One output per call, which is all a noise lattice consumes. A three-output
// mix such as PCG3D costs about twice as much, and fbm reaches this ~280 times
// per pixel, which measured as a 1.7x whole-shader regression.
float hash_lattice(ivec3 c) {
	uvec3 v = uvec3(c);
	uint h = v.x * 73856093u ^ v.y * 19349663u ^ v.z * 83492791u;
	h ^= h >> 13u;
	h *= 0x5bd1e995u;
	h ^= h >> 15u;
	return float(h) * (1.0 / 4294967296.0);
}

float snoise(vec3 p) {
	vec3 i = floor(p);
	vec3 f = p - i;
	vec3 u = f * f * (3.0 - 2.0 * f);
	ivec3 c = ivec3(i);
	float a = hash_lattice(c);
	float b = hash_lattice(c + ivec3(1, 0, 0));
	float d = hash_lattice(c + ivec3(0, 1, 0));
	float e = hash_lattice(c + ivec3(1, 1, 0));
	float g = hash_lattice(c + ivec3(0, 0, 1));
	float k = hash_lattice(c + ivec3(1, 0, 1));
	float m = hash_lattice(c + ivec3(0, 1, 1));
	float n = hash_lattice(c + ivec3(1, 1, 1));
	return mix(mix(mix(a, b, u.x), mix(d, e, u.x), u.y),
	           mix(mix(g, k, u.x), mix(m, n, u.x), u.y), u.z);
}

float fbm(vec3 p) {
	float v = 0.0;
	float a = 0.5;
	vec3  s = vec3(100.0);
	for (int i = 0; i < 5; i++) {
		v += a * snoise(p);
		p  = p * 2.07 + s;
		a *= 0.5;
	}
	return v;
}

// ── Galactic coordinate transform ─────────────────────────────────────────────
//
// The Milky Way's galactic plane is tilted ~63 degrees from the celestial
// equator.  We apply a rotation so that the band appears at a realistic angle
// rather than lying flat along the equator.
//
// Galactic north pole: RA 12h 51m, Dec +27.1° (B1950)
// We approximate this with a simple rotation around the X axis.

vec3 to_galactic(vec3 d) {
	// ~60° tilt of galactic plane + ~90° longitude offset
	float tilt  = 1.05;   // radians (~60°)
	float lon   = 1.57;   // radians (~90°) — orients galactic center direction
	float ct = cos(tilt), st = sin(tilt);
	float cl = cos(lon),  sl = sin(lon);
	// First rotate around Y (longitude), then around X (tilt)
	vec3 r;
	r.x =  d.x * cl + d.z * sl;
	r.z = -d.x * sl + d.z * cl;
	r.y =  d.y;
	return vec3(r.x, r.y * ct - r.z * st, r.y * st + r.z * ct);
}

// ── Milky Way + galactic structure ───────────────────────────────────────────

float milkyway_density(vec3 gal) {
	// Galactic latitude (distance from the plane)
	float lat  = abs(gal.y);

	// Galactic longitude (angle around the plane)
	float lon  = atan(gal.z, gal.x);

	// --- Band (thin disk) ---
	// Real scale height: ~350 pc.  We widen it for visibility.
	float disk  = exp(-lat * 4.5);

	// --- Galactic bulge ---
	// The galactic center is ~8 kpc away, appearing as a bright bulge
	// in one direction.  We model it as a Gaussian in galactic coordinates.
	// Direction of galactic center in our rotated frame: positive X.
	float bulge_dist = length(gal - vec3(1.0, 0.0, 0.0));
	float bulge      = exp(-bulge_dist * bulge_dist * 5.0) * 1.8;

	// --- Spiral arm modulation ---
	// Milky Way has 4 main arms; pitch angle ~12.5° → b ≈ tan(12.5°) ≈ 0.222
	// Arm pattern repeats every π/2 radians in longitude.
	// We use a cosine to create 4-fold density variation.
	float arm_phase = lon * 2.0 - log(max(0.01, length(gal.xz))) / 0.222;
	float arms      = 0.5 + 0.5 * cos(arm_phase * 2.0);

	// --- Disk clumping (molecular cloud patchiness) ---
	float clump = fbm(gal * 2.5 + vec3(0.0, 0.0, 3.0)) * 0.5 + 0.5;

	return disk * (0.4 + 0.4 * arms + 0.2 * clump) + bulge * (1.0 - lat);
}

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

vec3 sun_surface_color(vec2 disc_uv, vec3 base_sun_color, float disc_radius) {
	// Limb darkening is the visible consequence of looking through less of the
	// photosphere near its edge. Moving noise layers suggest convection cells.
	float mu = sqrt(max(0.0, 1.0 - disc_radius * disc_radius));
	float limb_darkening = mix(0.56, 1.0, mu);
	vec2 drift = vec2(TIME * 0.003, -TIME * 0.0017);
	float large_cells = fbm(vec3(disc_uv * 7.0 + drift, 2.0));
	float fine_cells = fbm(vec3(disc_uv * 19.0 - drift * 1.8, 7.0));
	float granulation = mix(large_cells, fine_cells, 0.35);
	float photosphere = 0.78 + smoothstep(0.18, 0.82, granulation) * 0.34;

	// A sparse, soft black-spot layer gives the surface a solar rather than
	// generic fireball character and fades at the foreshortened limb.
	float spot_field = fbm(vec3(disc_uv * 3.5 + vec2(4.0, -2.0), 13.0));
	float spot_detail = fbm(vec3(disc_uv * 17.0 + vec2(-1.0, 6.0), 19.0));
	float spots = smoothstep(0.68, 0.84, spot_field)
		* smoothstep(0.52, 0.76, spot_detail)
		* (1.0 - smoothstep(0.55, 0.96, disc_radius));
	photosphere = mix(photosphere, 0.43, spots);

	return base_sun_color * photosphere * limb_darkening;
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

void sky() {
	vec3 dir = normalize(EYEDIR);
	vec3 col;
	if (use_baked_nebula) {
		col = texture(nebula_panorama, panorama_uv(dir)).rgb;
	} else {
		col = ambient_color;

		// Convert to galactic frame for Milky Way / nebula calculations.
		vec3 gal = to_galactic(dir);
		float gal_lat = abs(gal.y);   // 0 = in the plane, 1 = toward pole

		// ── Milky Way ────────────────────────────────────────────────────────
		float mw  = milkyway_density(gal);
		// Core color: warm blue-white (hot young O/B stars + scattered blue starlight)
		vec3 mw_core  = milkyway_core_color;
		// Outer disk: slightly warmer (older K/G stars dominate by count)
		vec3 mw_outer = milkyway_outer_color;
		float mw_mix  = smoothstep(0.0, 1.5, mw);
		vec3 mw_col   = mix(mw_outer, mw_core, mw_mix);
		col += mw_col * mw * milkyway_strength;

		// ── Emission nebulae ──────────────────────────────────────────────────
		//
		// Real nebulae are concentrated near the galactic plane.
		// Plane concentration factor: drops off with galactic latitude.
		float plane_weight = exp(-gal_lat * 2.2);

		// A broad, low-frequency dust layer keeps the background visually rich
		// across the whole frame instead of confining all colour to a razor-thin
		// Milky Way stripe. It is presentation-only and has no gameplay meaning.
		float broad_cloud_noise = fbm(dir * 0.95 + vec3(-0.8, 1.4, 2.1));
		float broad_cloud = pow(max(0.0, broad_cloud_noise - 0.27), 1.45)
			* plane_weight;
		float cloud_highlight = pow(max(0.0, broad_cloud_noise - 0.48), 2.0)
			* plane_weight;
		col += nebula_secondary_color * broad_cloud * nebula_strength * 0.95;
		col += nebula_highlight_color * cloud_highlight * nebula_strength * 0.55;

		// Three FBM regions at different scales + offsets.
		// Spectral line colors (wavelength → linear RGB approximation):
		//   Hα  656.3 nm : (1.00, 0.11, 0.08)
		//   OIII 500.7 nm : (0.14, 0.82, 0.70)
		//   SII  671.6 nm : (0.87, 0.06, 0.05)
		float n1 = fbm(dir * 1.70 + vec3( 0.5,  0.3,  0.7));
		float n2 = fbm(dir * 2.10 + vec3( 2.1, -1.4,  0.8));
		float n3 = fbm(dir * 1.50 + vec3(-1.2,  0.6,  2.4));

		float neb1 = pow(max(0.0, n1 - 0.36), 2.2) * nebula_strength * plane_weight;
		float neb2 = pow(max(0.0, n2 - 0.40), 2.0) * nebula_strength * plane_weight * 0.75;
		float neb3 = pow(max(0.0, n3 - 0.43), 2.5) * nebula_strength * plane_weight * 0.55;

		col += nebula_primary_color * neb1;
		col += nebula_secondary_color * neb2;
		col += nebula_highlight_color * neb3;

		// Dust absorption (dark lanes inside dense regions)
		float dust      = fbm(dir * 3.5 + vec3(0.0, 1.0, 0.3));
		float dust_mask = smoothstep(0.52, 0.72, dust) * max(neb1, neb2) * 0.50;
		col            *= 1.0 - dust_mask;
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
		float disc_edge = cos(disc_radius_angle);
		if (cos_angle > disc_edge) {
			float disc_t = (cos_angle - disc_edge) / (1.0 - disc_edge);
			vec2 disc_uv = vec2(
				dot(dir, sun_right),
				dot(dir, sun_up)
			) / sin(disc_radius_angle);
			float disc_radius = length(disc_uv);
			float disc_mask = 1.0 - smoothstep(0.84, 1.0, disc_radius);
			vec3 surface = sun_surface_color(disc_uv, sun_color, disc_radius);
			col += surface * disc_mask * (1.8 + 5.5 * disc_t * disc_t);
		}

		// Layered corona: a tight inner halo plus animated outer streamers.
		float sun_angle = acos(clamp(cos_angle, -1.0, 1.0));
		vec2 corona_uv = vec2(dot(dir, sun_right), dot(dir, sun_up));
		float corona_noise = fbm(vec3(corona_uv * 22.0, TIME * 0.006));
		float sun_angle_ratio = sun_angle / disc_radius_angle;
		float inner_corona = exp(-sun_angle_ratio * 1.9) * 2.4;
		float outer_corona = exp(-sun_angle_ratio * 0.29)
			* mix(0.18, 0.70, smoothstep(0.26, 0.78, corona_noise));

		// The core is white-hot while the photosphere and corona retain the
		// star's spectral tint. This keeps the bloom from reading as a flat disk.
		float white_core = exp(-sun_angle_ratio * 6.8) * 8.0;
		vec3 core_color = mix(sun_color, vec3(1.0, 0.97, 0.88), 0.72);

		// Optical flare is screen-aligned and centered on the sun. The old
		// world-space tangent frame made the streak rotate as the ship moved.
		vec2 sun_screen = vec2(
			dot(sun_dir, sun_flare_right),
			dot(sun_dir, sun_flare_up)
		);
		vec2 flare_uv = vec2(
			dot(dir, sun_flare_right),
			dot(dir, sun_flare_up)
		) - sun_screen;
		// A projected sun direction can overlap the screen center even when the
		// star is behind the camera. Only emit lens artifacts on front-facing sky.
		float front_facing = smoothstep(-0.02, 0.08, cos_angle);
		float flare_distance = length(flare_uv);
		float flare_envelope = 1.0 - smoothstep(0.16, 0.34, flare_distance);
		float horizontal_flare = exp(-pow(flare_uv.y / 0.006, 2.0))
			* exp(-pow(flare_uv.x / 0.16, 2.0)) * 0.42 * flare_envelope * front_facing;
		float central_glint = exp(-pow(flare_uv.y / 0.0015, 2.0))
			* exp(-pow(flare_uv.x / 0.045, 2.0)) * 0.55 * front_facing;
		float scattered_glow = exp(-sun_angle_ratio * 0.12) * 0.28;
		vec3 flare_color = mix(core_color, sun_color,
			smoothstep(0.0, 0.12, flare_distance));
		col += core_color * white_core;
		col += sun_color * (inner_corona + outer_corona + scattered_glow);
		col += flare_color * (horizontal_flare + central_glint);

	}

	// The reference look falls into a darker brown-black floor while keeping
	// the starfield readable. This is a presentation gradient over world
	// direction, not a physical horizon or a change to world lighting.
	if (!bake_pass) {
		float lower_vignette = smoothstep(-0.86, 0.12, dir.y);
		col *= mix(0.62, 1.0, lower_vignette);
	}

	col = max(col, vec3(0.0));
	COLOR = col;
}
