// space_sky.gdshader
//
// Procedural space background grounded in real astronomical data.
// No external textures required.
//
// Astronomical references used:
//   Star colors     : Blackbody radiation RGB by spectral type (Ballesteros 2012)
//   Star distribution: Kroupa IMF — M:K:G:F:A:B:O ≈ 75:12:7:3:2:0.6:0.00003%
//                      Visible-sky weighting shifts toward A/F/G (dim M's invisible)
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

uniform float star_threshold    : hint_range(0.90, 0.999) = 0.955;
uniform float star_brightness   : hint_range(0.5,  8.0)   = 1.5;
uniform float nebula_strength   : hint_range(0.0,  1.0)   = 0.50;
uniform float milkyway_strength : hint_range(0.0,  1.0)   = 0.20;
uniform vec3  ambient_color     : source_color             = vec3(0.002, 0.002, 0.006);

// ── Hash / Noise ──────────────────────────────────────────────────────────────
// Note: PI and TAU are Godot 4 built-in shader constants.

float hash1(vec3 p) {
	p  = fract(p * vec3(443.8975, 397.2973, 491.1871));
	p += dot(p.zxy, p.yxz + 19.19);
	return fract(p.x * p.y * p.z);
}

float snoise(vec3 p) {
	vec3 i = floor(p);
	vec3 f = fract(p);
	vec3 u = f * f * (3.0 - 2.0 * f);
	float a = hash1(i);
	float b = hash1(i + vec3(1,0,0));
	float c = hash1(i + vec3(0,1,0));
	float d = hash1(i + vec3(1,1,0));
	float e = hash1(i + vec3(0,0,1));
	float g = hash1(i + vec3(1,0,1));
	float h = hash1(i + vec3(0,1,1));
	float k = hash1(i + vec3(1,1,1));
	return mix(mix(mix(a,b,u.x), mix(c,d,u.x), u.y),
	           mix(mix(e,g,u.x), mix(h,k,u.x), u.y), u.z);
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

// ── Spectral type color table ─────────────────────────────────────────────────
//
// Based on blackbody radiation approximations (Ballesteros 2012, EPJP).
// Colors are for linear light, pre-tonemapping.
//
//   O  > 30,000 K  : blue-violet
//   B  10-30,000 K : blue-white
//   A   7,500-10,000 K : white
//   F   6,000- 7,500 K : yellow-white
//   G   5,200- 6,000 K : yellow   (Sun)
//   K   3,700- 5,200 K : orange
//   M   2,400- 3,700 K : red-orange

vec3 spectral_color(float t) {
	// t in [0, 1]: 0 = hottest (O), 1 = coolest (M)
	if (t < 0.10) {
		// O type — blue-violet
		return mix(vec3(0.55, 0.65, 1.00), vec3(0.65, 0.76, 1.00), t / 0.10);
	} else if (t < 0.25) {
		// B type — blue-white
		return mix(vec3(0.65, 0.76, 1.00), vec3(0.88, 0.93, 1.00), (t - 0.10) / 0.15);
	} else if (t < 0.40) {
		// A type — white
		return mix(vec3(0.88, 0.93, 1.00), vec3(1.00, 0.99, 0.94), (t - 0.25) / 0.15);
	} else if (t < 0.55) {
		// F type — yellow-white
		return mix(vec3(1.00, 0.99, 0.94), vec3(1.00, 0.95, 0.82), (t - 0.40) / 0.15);
	} else if (t < 0.68) {
		// G type — yellow  (Sun ≈ t=0.60)
		return mix(vec3(1.00, 0.95, 0.82), vec3(1.00, 0.85, 0.58), (t - 0.55) / 0.13);
	} else if (t < 0.83) {
		// K type — orange
		return mix(vec3(1.00, 0.85, 0.58), vec3(1.00, 0.64, 0.32), (t - 0.68) / 0.15);
	} else {
		// M type — red-orange
		return mix(vec3(1.00, 0.64, 0.32), vec3(1.00, 0.40, 0.18), (t - 0.83) / 0.17);
	}
}

// ── Main ──────────────────────────────────────────────────────────────────────

void sky() {
	vec3 dir = normalize(EYEDIR);
	vec3 col = ambient_color;

	// Convert to galactic frame for Milky Way / nebula calculations.
	vec3 gal = to_galactic(dir);
	float gal_lat = abs(gal.y);   // 0 = in the plane, 1 = toward pole

	// ── Milky Way ────────────────────────────────────────────────────────
	float mw  = milkyway_density(gal);
	// Core color: warm blue-white (hot young O/B stars + scattered blue starlight)
	vec3 mw_core  = vec3(0.52, 0.62, 0.92);
	// Outer disk: slightly warmer (older K/G stars dominate by count)
	vec3 mw_outer = vec3(0.72, 0.68, 0.58);
	float mw_mix  = smoothstep(0.0, 1.5, mw);
	vec3 mw_col   = mix(mw_outer, mw_core, mw_mix);
	col += mw_col * mw * milkyway_strength;

	// ── Emission nebulae ──────────────────────────────────────────────────
	//
	// Real nebulae are concentrated near the galactic plane.
	// Plane concentration factor: drops off with galactic latitude.
	float plane_weight = exp(-gal_lat * 3.5);

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

	col += vec3(1.00, 0.11, 0.08) * neb1;   // Hα red
	col += vec3(0.14, 0.82, 0.70) * neb2;   // OIII teal
	col += vec3(0.87, 0.06, 0.05) * neb3;   // SII deep red

	// Dust absorption (dark lanes inside dense regions)
	float dust      = fbm(dir * 3.5 + vec3(0.0, 1.0, 0.3));
	float dust_mask = smoothstep(0.52, 0.72, dust) * max(neb1, neb2) * 0.50;
	col            *= 1.0 - dust_mask;

	// ── Stars ─────────────────────────────────────────────────────────────
	//
	// Equirectangular (spherical UV) grid — no column artifacts.
	// Galactic-plane density bonus: more stars near the Milky Way band.
	float eq_u = atan(dir.z, dir.x) / TAU;
	float eq_v = asin(clamp(dir.y, -1.0, 1.0)) / PI;
	vec2  eq   = vec2(eq_u, eq_v);

	// Milky Way density raises the local star count.
	float mw_density_bonus = mw * 0.018;

	float scales[6];
	scales[0] =  80.0;
	scales[1] = 150.0;
	scales[2] = 260.0;
	scales[3] = 400.0;
	scales[4] = 580.0;
	scales[5] = 800.0;

	float thresh_offset[6];
	thresh_offset[0] =  0.000;
	thresh_offset[1] = -0.012;
	thresh_offset[2] = -0.020;
	thresh_offset[3] = -0.026;
	thresh_offset[4] = -0.030;
	thresh_offset[5] = -0.034;

	for (int i = 0; i < 6; i++) {
		vec2  p       = eq * scales[i];
		vec2  pi      = floor(p);
		vec2  pf      = fract(p);
		float layer_z = float(i) * 17.3;
		float h       = hash1(vec3(pi, layer_z));

		// Milky Way plane gets more stars (lower effective threshold).
		float thr = star_threshold + thresh_offset[i] - mw_density_bonus;

		if (h > thr) {
			float t   = (h - thr) / (1.0 - thr);
			float bri = pow(t, 1.3) * star_brightness;
			bri      *= 1.0 - float(i) * 0.10;

			vec2  offset = pf - 0.5;
			float d      = length(offset);
			float glow   = exp(-d * 60.0) * bri;

			// Star temperature from a second independent hash.
			// Distribution weighted toward visible types (A/F/G dominate naked-eye view).
			// Real IMF: M=75%, K=12%, G=7%, F=3%, A=2%, B=0.6%, O=0.00003%
			// But bright stars (visible) skew toward hotter types.
			float raw_temp = hash1(vec3(pi + vec2(73.1, 21.9), layer_z + 0.5));
			// Remap: 0-0.40 → A/B/O (hot, 40%); 0.40-0.65 → F/G (25%); 0.65-1.0 → K/M (35%)
			float temp;
			if (raw_temp < 0.40) {
				temp = raw_temp * 0.55;           // maps to O-A range (0 - 0.40)
			} else if (raw_temp < 0.65) {
				temp = 0.40 + (raw_temp - 0.40) * 0.60;  // F-G range (0.40 - 0.55)
			} else {
				temp = 0.55 + (raw_temp - 0.65) * 1.29;  // K-M range (0.55 - 1.0)
			}

			vec3 star_col = spectral_color(temp);
			col += glow * star_col;

			// Only the very brightest stars (top 15%) get diffraction spikes.
			// Bloom is intentionally tight — Godot post-process glow handles
			// the wider soft halo so we avoid double-blooming.
			if (t > 0.85 && i < 3) {
				// Tight bloom: sharp halo, not a large disc.
				float bloom   = exp(-d * 22.0) * bri * 0.12;
				col          += star_col * bloom;
				// Short diffraction spikes (telescope cross pattern).
				float spike_x = exp(-abs(offset.x) * 130.0) * exp(-abs(offset.y) * 6.0);
				float spike_y = exp(-abs(offset.y) * 130.0) * exp(-abs(offset.x) * 6.0);
				col          += star_col * (spike_x + spike_y) * bri * 0.055;
			}
		}
	}

	col = max(col, vec3(0.0));
	COLOR = col;
}
