shader_type spatial;
render_mode diffuse_burley;

uniform vec4 base_color : source_color = vec4(0.30, 0.34, 0.40, 1.0);
uniform vec4 accent_color : source_color = vec4(0.12, 0.16, 0.20, 1.0);
uniform vec4 ice_color : source_color = vec4(0.80, 0.88, 0.96, 1.0);
uniform float seed = 1.0;
uniform float ocean_level = 0.35;
uniform float ice_amount = 0.15;

varying vec3 local_position;

float hash3(vec3 p) {
	p = fract(p * vec3(443.8975, 397.2973, 491.1871));
	p += dot(p.zxy, p.yxz + 19.19);
	return fract(p.x * p.y * p.z);
}

float noise3(vec3 p) {
	vec3 cell = floor(p);
	vec3 f = fract(p);
	vec3 u = f * f * (3.0 - 2.0 * f);
	float a = hash3(cell);
	float b = hash3(cell + vec3(1.0, 0.0, 0.0));
	float c = hash3(cell + vec3(0.0, 1.0, 0.0));
	float d = hash3(cell + vec3(1.0, 1.0, 0.0));
	float e = hash3(cell + vec3(0.0, 0.0, 1.0));
	float f2 = hash3(cell + vec3(1.0, 0.0, 1.0));
	float g = hash3(cell + vec3(0.0, 1.0, 1.0));
	float h = hash3(cell + vec3(1.0, 1.0, 1.0));
	return mix(mix(mix(a, b, u.x), mix(c, d, u.x), u.y),
		mix(mix(e, f2, u.x), mix(g, h, u.x), u.y), u.z);
}

float terrain(vec3 point) {
	float broad = noise3(point * 2.5 + vec3(seed, seed * 0.7, -seed));
	float detail = noise3(point * 7.0 - vec3(seed * 1.3, 2.0, seed * 0.5));
	return broad * 0.72 + detail * 0.28;
}

void vertex() {
	local_position = VERTEX;
}

void fragment() {
	vec3 point = normalize(local_position);
	float height = terrain(point);
	float shore = smoothstep(ocean_level - 0.08, ocean_level + 0.08, height);
	vec3 color = mix(accent_color.rgb, base_color.rgb, smoothstep(0.20, 0.78, height));
	color = mix(color, base_color.rgb * 0.55, 1.0 - shore);
	float polar = smoothstep(0.68, 0.94, abs(point.y));
	color = mix(color, ice_color.rgb, polar * ice_amount);
	ALBEDO = color;
	ROUGHNESS = mix(0.92, 0.58, shore);
	METALLIC = 0.02;
	// A restrained rim keeps the sphere readable without adding a second,
	// non-physical atmosphere shell whose radius would confuse navigation LOD.
	float rim = pow(1.0 - max(dot(normalize(NORMAL), normalize(VIEW)), 0.0), 3.0);
	EMISSION = ice_color.rgb * rim * 0.035;
}
