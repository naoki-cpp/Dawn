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
	float value = 0.0;
	float amplitude = 0.5;
	vec3 offset = vec3(100.0);
	for (int octave = 0; octave < 5; octave++) {
		value += amplitude * snoise(p);
		p = p * 2.07 + offset;
		amplitude *= 0.5;
	}
	return value;
}

vec3 solar_surface_palette(float heat) {
	return mix(vec3(0.35, 0.012, 0.0005), vec3(2.4, 0.48, 0.018), clamp(heat, 0.0, 1.0));
}

vec3 solar_h_alpha_palette(float intensity) {
	return mix(vec3(1.15, 0.018, 0.003), vec3(3.2, 0.19, 0.012), clamp(intensity, 0.0, 1.0));
}

float solar_optical_alpha(float density, float depth) {
	return 1.0 - exp(-max(density, 0.0) * max(depth, 0.0));
}

float solar_filament_transmission(float density, float contrast) {
	return 1.0 - clamp(density, 0.0, 1.0) * clamp(contrast, 0.0, 1.0);
}

vec3 solar_surface_drift() {
	return vec3(TIME * 0.00015, 0.0, -TIME * 0.0001);
}
