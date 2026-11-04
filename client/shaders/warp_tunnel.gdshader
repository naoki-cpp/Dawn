// A translucent cylinder aligned with travel, viewed through the actual camera.
// Background detail and the central ship remain visible; no shake or flash.
shader_type canvas_item;

uniform float intensity : hint_range(0.0, 1.0) = 0.0;
uniform float opacity : hint_range(0.0, 1.0) = 0.28;
uniform float scroll_speed : hint_range(0.0, 20.0) = 3.0;
uniform vec3 travel_direction = vec3(0.0, 0.0, -1.0);
uniform mat3 camera_basis = mat3(1.0);
uniform float camera_fov = 60.0;
uniform vec3 tunnel_color : source_color = vec3(0.38, 0.53, 0.65);
uniform vec3 streak_color : source_color = vec3(0.68, 0.80, 0.86);

float hash12(vec2 p) {
	return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

float noise(vec2 p) {
	vec2 cell = floor(p);
	vec2 f = fract(p);
	f = f * f * (3.0 - 2.0 * f);
	return mix(mix(hash12(cell), hash12(cell + vec2(1.0, 0.0)), f.x),
		mix(hash12(cell + vec2(0.0, 1.0)), hash12(cell + vec2(1.0)), f.x), f.y);
}

vec2 tunnel_layer(vec3 ray, vec3 axis, vec3 tangent, float radius, float time) {
	float along = dot(ray, axis);
	float across = length(ray - axis * along);
	vec3 hit = ray * (radius / max(across, 0.035));
	float depth = dot(hit, axis);
	vec3 around = (hit - axis * depth) / radius;
	float angle = atan(dot(around, cross(axis, tangent)), dot(around, tangent));
	float lane = (angle / TAU + 0.5) * 96.0;
	float lane_id = floor(lane);
	float seed = hash12(vec2(lane_id, radius));
	float phase = fract(depth * 0.24 + time + seed);
	float width = 1.0 - smoothstep(0.012, 0.055, abs(fract(lane) - 0.5));
	float segment = smoothstep(0.0, 0.12, phase) * (1.0 - smoothstep(0.22, 0.55, phase));
	float streak = width * segment * smoothstep(0.78, 0.98, seed);
	// Periodic angular coordinates avoid a seam around the cylinder.
	vec2 mist_uv = vec2(sin(angle) * 2.0 + cos(angle) * 3.0, depth * 0.10 + time * 0.30);
	float mist = smoothstep(0.30, 0.85, noise(mist_uv) * 0.7 + noise(mist_uv * 2.1) * 0.3);
	float horizon = smoothstep(0.035, 0.20, across);
	return vec2(mist, streak) * horizon;
}

void fragment() {
	float aspect = SCREEN_PIXEL_SIZE.y / SCREEN_PIXEL_SIZE.x;
	vec2 screen = (UV - vec2(0.5)) * vec2(aspect, 1.0);
	float focal = 0.5 / tan(radians(camera_fov) * 0.5);
	vec3 ray = normalize(camera_basis * vec3(screen.x, -screen.y, -focal));
	vec3 axis = normalize(travel_direction);
	vec3 reference = abs(axis.y) < 0.95 ? vec3(0.0, 1.0, 0.0) : vec3(1.0, 0.0, 0.0);
	vec3 tangent = normalize(cross(axis, reference));
	float time = TIME * scroll_speed;
	vec2 layers = tunnel_layer(ray, axis, tangent, 1.0, time)
		+ tunnel_layer(ray, axis, tangent, 1.65, time * 0.83) * 0.45;
	float ship_clearance = smoothstep(0.10, 0.36, length(screen));
	float coverage = layers.x * 0.22 + layers.y * 0.80;
	vec3 color = mix(tunnel_color, streak_color, clamp(layers.y, 0.0, 1.0));
	float alpha = clamp(coverage * ship_clearance * intensity * opacity, 0.0, 0.32);
	COLOR = vec4(color, alpha);
}
