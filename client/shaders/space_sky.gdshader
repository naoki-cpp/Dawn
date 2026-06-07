// space_sky.gdshader
//
// Procedural space background — no external textures required.
// Targets an EVE Online / EVE Frontier aesthetic:
//   realistic deep black, colorful nebulae, bright star glow.
//
// Generated elements:
//   Stars     : 6 layers (near to far), color-temperature variation,
//               diffraction spikes + bloom on the brightest stars
//   Nebulae   : 3 FBM-noise regions (hydrogen red, ionized blue, OIII green)
//   Dust lanes: dark absorption within dense nebula regions
//   Milky Way : denser core band with internal brightness variation
//   Ambient   : near-black deep-space haze

shader_type sky;
render_mode use_half_res_pass;

// ── Uniforms ─────────────────────────────────────────────────────────────────

/// Minimum hash value for a cell to contain a star (higher = fewer stars).
uniform float star_threshold    : hint_range(0.90, 0.999) = 0.960;
/// Peak brightness of the brightest stars.
uniform float star_brightness   : hint_range(0.5,  8.0)   = 3.5;
/// Overall intensity of nebula regions.
uniform float nebula_strength   : hint_range(0.0,  1.0)   = 0.40;
/// Milky Way band brightness.
uniform float milkyway_strength : hint_range(0.0,  0.5)   = 0.12;
/// Milky Way core color (blue-white starfield glow).
uniform vec3  milkyway_color    : source_color             = vec3(0.48, 0.58, 0.90);
/// Deep-space ambient (keep very dark).
uniform vec3  ambient_color     : source_color             = vec3(0.004, 0.003, 0.010);

// ── Constants ─────────────────────────────────────────────────────────────────

const float PI  = 3.14159265358979;
const float TAU = 6.28318530717959;

// ── Hash / Noise ──────────────────────────────────────────────────────────────

float hash1(vec3 p) {
	p  = fract(p * vec3(443.8975, 397.2973, 491.1871));
	p += dot(p.zxy, p.yxz + 19.19);
	return fract(p.x * p.y * p.z);
}

float hash1_2d(vec2 p) {
	p  = fract(p * vec2(443.8975, 397.2973));
	p += dot(p.yx, p.xy + 19.19);
	return fract(p.x * p.y);
}

/// Smooth trilinear-interpolated 3-D noise in [0, 1].
float snoise(vec3 p) {
	vec3 i = floor(p);
	vec3 f = fract(p);
	vec3 u = f * f * (3.0 - 2.0 * f);   // smoothstep

	float a = hash1(i);
	float b = hash1(i + vec3(1.0, 0.0, 0.0));
	float c = hash1(i + vec3(0.0, 1.0, 0.0));
	float d = hash1(i + vec3(1.0, 1.0, 0.0));
	float e = hash1(i + vec3(0.0, 0.0, 1.0));
	float g = hash1(i + vec3(1.0, 0.0, 1.0));
	float h = hash1(i + vec3(0.0, 1.0, 1.0));
	float k = hash1(i + vec3(1.0, 1.0, 1.0));

	return mix(mix(mix(a, b, u.x), mix(c, d, u.x), u.y),
	           mix(mix(e, g, u.x), mix(h, k, u.x), u.y), u.z);
}

/// Fractal Brownian Motion — 5 octaves.
float fbm(vec3 p) {
	float v = 0.0;
	float a = 0.50;
	vec3  s = vec3(100.0);
	for (int i = 0; i < 5; i++) {
		v += a * snoise(p);
		p  = p * 2.07 + s;
		a *= 0.50;
	}
	return v;
}

// ── Main ──────────────────────────────────────────────────────────────────────

void sky() {
	vec3 dir = normalize(EYEDIR);
	vec3 col = ambient_color;

	// ── Nebulae — 3 independent FBM regions ──────────────────────────────
	//
	// Each region is sampled at a different scale + offset so they appear
	// in different parts of the sky and have different density profiles.

	float n1 = fbm(dir * 1.80 + vec3( 0.50,  0.30,  0.70));
	float n2 = fbm(dir * 2.20 + vec3( 2.10, -1.40,  0.80));
	float n3 = fbm(dir * 1.55 + vec3(-1.20,  0.60,  2.40));

	// Raise to a power to concentrate into cloud-like clumps.
	float neb1 = pow(max(0.0, n1 - 0.38), 2.3) * nebula_strength;
	float neb2 = pow(max(0.0, n2 - 0.42), 2.0) * nebula_strength * 0.80;
	float neb3 = pow(max(0.0, n3 - 0.44), 2.6) * nebula_strength * 0.55;

	// EVE-style palette:
	//   neb1 — hydrogen-alpha emission (deep red/crimson)
	//   neb2 — ionized oxygen (electric blue)
	//   neb3 — OIII / reflection (teal-green)
	col += vec3(0.80, 0.10, 0.06) * neb1;
	col += vec3(0.06, 0.16, 0.90) * neb2;
	col += vec3(0.10, 0.65, 0.38) * neb3;

	// Dust absorption lanes inside dense nebula regions.
	float dust      = fbm(dir * 3.60 + vec3(0.0, 1.0, 0.3));
	float dust_mask = smoothstep(0.52, 0.72, dust) * max(neb1, neb2) * 0.55;
	col            *= 1.0 - dust_mask;

	// ── Milky Way band ───────────────────────────────────────────────────
	float band_y = abs(dir.y);
	float angle  = atan(dir.z, dir.x);
	// Several frequency terms for organic waviness.
	float wiggle = sin(angle *  3.0         ) * 0.18
	             + sin(angle *  7.3 +  1.20 ) * 0.07
	             + sin(angle * 13.1 -  0.80 ) * 0.04;
	float band   = exp(-(band_y + wiggle) * 5.0) * milkyway_strength;
	// Internal clumping via noise.
	float mw_var = snoise(dir * 5.0 + vec3(5.0, 0.0, 0.0)) * 0.5 + 0.5;
	col += milkyway_color * band * (0.6 + 0.8 * mw_var);

	// ── Stars — 6 layers in equirectangular (spherical UV) space ───────────
	//
	// Stars are placed on a 2D grid in (longitude, latitude) space so
	// cells are uniform in angular size — no vertical column artifacts.
	//
	// eq.x = longitude  atan(z,x) / TAU     ∈ [-0.5, 0.5]
	// eq.y = latitude   asin(y)   / PI      ∈ [-0.5, 0.5]

	float eq_u = atan(dir.z, dir.x) / TAU;
	float eq_v = asin(clamp(dir.y, -1.0, 1.0)) / PI;
	vec2  eq   = vec2(eq_u, eq_v);

	float scales[6];
	scales[0] =  80.0;
	scales[1] = 150.0;
	scales[2] = 260.0;
	scales[3] = 400.0;
	scales[4] = 580.0;
	scales[5] = 800.0;

	float thresh_offset[6];
	thresh_offset[0] =  0.000;
	thresh_offset[1] = -0.010;
	thresh_offset[2] = -0.018;
	thresh_offset[3] = -0.024;
	thresh_offset[4] = -0.028;
	thresh_offset[5] = -0.032;

	for (int i = 0; i < 6; i++) {
		vec2  p   = eq * scales[i];
		vec2  pi  = floor(p);
		vec2  pf  = fract(p);
		// Use layer index as z so different layers never share hash.
		float h   = hash1_2d(pi + vec2(float(i) * 137.1, float(i) * 251.3));
		float thr = star_threshold + thresh_offset[i];

		if (h > thr) {
			float t   = (h - thr) / (1.0 - thr);
			float bri = pow(t, 1.4) * star_brightness;
			bri      *= 1.0 - float(i) * 0.11;   // farther = dimmer

			vec2  offset = pf - 0.5;
			float d      = length(offset);
			float glow   = exp(-d * 34.0) * bri;

			// Color temperature from a second independent hash.
			float temp = hash1_2d(pi + vec2(7.3 + float(i), 2.1));
			vec3  star_col;
			if (temp < 0.50) {
				star_col = mix(vec3(0.60, 0.75, 1.00), vec3(0.95, 0.97, 1.00), temp * 2.0);
			} else if (temp < 0.80) {
				star_col = mix(vec3(0.95, 0.97, 1.00), vec3(1.00, 0.92, 0.72), (temp - 0.50) / 0.30);
			} else {
				star_col = mix(vec3(1.00, 0.85, 0.55), vec3(1.00, 0.48, 0.22), (temp - 0.80) / 0.20);
			}

			col += glow * star_col;

			// Brightest stars: bloom halo + 4-point diffraction spikes.
			if (t > 0.72 && i < 4) {
				float bloom   = exp(-d * 9.0) * bri * 0.22;
				col          += star_col * bloom;

				float spike_x = exp(-abs(offset.x) * 90.0) * exp(-abs(offset.y) *  5.0);
				float spike_y = exp(-abs(offset.y) * 90.0) * exp(-abs(offset.x) *  5.0);
				col          += star_col * (spike_x + spike_y) * bri * 0.09;
			}
		}
	}

	// Ensure no negative values (floating-point noise can go slightly negative).
	col = max(col, vec3(0.0));

	COLOR = col;
}
