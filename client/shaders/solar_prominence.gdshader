shader_type spatial;
#include "res://shaders/stellar_plasma.gdshaderinc"
render_mode unshaded, blend_mix, depth_draw_never, cull_disabled, fog_disabled;

uniform float detail_visibility = 1.0;
uniform vec3 sun_direction = vec3(0.0, 0.0, -1.0);
uniform float sun_angular_radius = 0.0;
varying vec3 world_position;

void vertex() {
	// Keep both feet anchored while shared plasma noise distorts the arch.
	float anchored = sin(PI * UV.x);
	float motion = fbm(vec3(UV.x * 8.0 + COLOR.g * 11.0, UV.y * 2.5,
		COLOR.g * 23.0 + TIME * 0.025)) - 0.5;
	VERTEX *= 1.0 + anchored * (0.008 * sin(UV.x * 18.0 + COLOR.g * 9.0 + TIME * 0.018)
		+ motion * 0.035);
	world_position = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
}

void fragment() {
	vec3 ray = normalize(world_position - CAMERA_POSITION_WORLD);
	// The photosphere lives in the sky, so it cannot write a scene depth mask.
	if (dot(ray, sun_direction) > cos(sun_angular_radius)) {
		discard;
	}
	float seed = COLOR.g;
	float layer = COLOR.b;
	vec3 drift = solar_surface_drift();
	float broad = fbm(vec3(UV.x * 4.0 + seed * 7.0, UV.y * 1.8, seed * 19.0 + layer * 11.0) + drift * 0.08);
	float turbulence = fbm(vec3(UV.x * 7.0 + seed * 13.0,
		UV.y * 5.0 + (broad - 0.5) * 1.2, seed * 23.0 + layer * 7.0) + drift * 2.0);
	float fibers = fbm(vec3(UV.x * 12.0 + seed * 31.0,
		UV.y * 8.0 + (turbulence - 0.5) * 1.4, seed * 37.0 + layer * 17.0) + drift * 4.0);
	float bend = 0.16 * sin(UV.x * 13.0 + seed * TAU) + (broad - 0.5) * 0.28
		+ (fibers - 0.5) * 0.05;
	float width = 0.43 + 0.30 * sin(PI * UV.x) + 0.11 * sin(UV.x * 17.0 + seed * TAU)
		+ (turbulence - 0.5) * 0.16;
	float envelope = 1.0 - smoothstep(width * 0.40, width, abs(UV.y - bend));
	float feet = smoothstep(0.0, 0.035, UV.x) * (1.0 - smoothstep(0.965, 1.0, UV.x));
	float knot_center = 0.66 + (seed - 0.5) * 0.16;
	float knot = 0.56 + 0.44 * exp(-pow((UV.x - knot_center) / 0.24, 2.0));
	float cores = smoothstep(0.26, 0.64, fibers);
	float spine = exp(-pow((UV.y - (broad - 0.5) * 0.18) * 3.4, 2.0));
	float strand_phase = UV.y * 8.5 + (broad - 0.5) * 2.4
		+ (turbulence - 0.5) * 1.8 + 0.14 * sin(UV.x * 11.0 + seed * TAU) + seed * TAU;
	float strand_wave = 0.5 + 0.5 * sin(strand_phase * TAU);
	float threads = smoothstep(0.20, 0.78, strand_wave);
	float plasma_mass = smoothstep(0.27, 0.70, mix(broad, turbulence, 0.45));
	float density = envelope * feet * knot * (0.36 + threads * 0.18 + cores * 0.28)
		* mix(0.58, 1.12, plasma_mass);
	float emission = 0.22 + cores * 0.50 + spine * 0.20;
	ALPHA = solar_optical_alpha(density, 0.62) * COLOR.r * detail_visibility;
	ALBEDO = solar_h_alpha_palette(emission) * (0.66 + cores * 0.92 + spine * 0.52);
}
