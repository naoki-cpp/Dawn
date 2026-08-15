// warp_tunnel.gdshader
//
// Full-screen overlay representing "inside the warp tunnel" for the locally
// piloted ship (ADR-0029 lore pass, 2026-06-23). The server's true warp speed
// (often ~10^8+ Godot units/tick at true AU) can't be rendered as literal
// motion without flinging the camera past the far plane and jittering in f32
// -- so instead of showing that, this overlay fades in to cover the cruise
// portion of the transit, with `intensity` driven by the client from the
// ship's speed (smoothed, so the fade itself reads as the accelerate-in /
// decelerate-out transition rather than a hard cut -- see
// WorldPresentation._update_warp_tunnel_effect()).
//
// Purely cosmetic: radial streaks moving away from a soft vanishing point,
// sparse light particles, and a dim converging core. A visible screen-space
// velocity biases the streaks toward the travel direction, while depth-only
// travel stays radially symmetric instead of choosing an arbitrary axis.

shader_type canvas_item;

uniform float intensity   : hint_range(0.0, 1.0) = 0.0;
uniform float opacity     : hint_range(0.0, 1.0) = 0.42;
uniform float scroll_speed: hint_range(0.0, 20.0) = 6.0;
uniform float pulse_speed : hint_range(0.0, 8.0) = 2.4;
uniform vec2  flow_direction = vec2(0.0, -1.0);
uniform float direction_confidence : hint_range(0.0, 1.0) = 0.0;
uniform vec3  tunnel_color: source_color = vec3(0.36, 0.62, 1.0);
uniform vec3  accent_color: source_color = vec3(1.0, 0.72, 0.38);
uniform vec3  core_color  : source_color = vec3(0.92, 0.98, 1.0);

float hash12(vec2 p) {
	return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

// A layer is made of broken radial lanes. The radial phase moves toward the
// screen edge, so the marks read as stars rushing away from the vanishing
// point rather than as a fixed set of horizontal or vertical bands.
float radial_layer(
	float dist,
	float angle,
	float time,
	float scale,
	float speed,
	float rays,
	float seed_offset
) {
	float ray_coord = (angle / TAU + 0.5) * rays + seed_offset * 0.37;
	float ray_id = floor(ray_coord);
	float ray_offset = abs(fract(ray_coord) - 0.5);
	float phase = fract(dist * scale - time * speed);
	float ray_width = mix(0.055, 0.014, phase);
	float ray_mask = 1.0 - smoothstep(ray_width, ray_width + 0.022, ray_offset);
	float density = smoothstep(
		0.42,
		0.92,
		hash12(vec2(ray_id + seed_offset * 19.0, floor(dist * scale) + seed_offset * 7.0))
	);
	float head = smoothstep(0.04, 0.20, phase);
	float tail = 1.0 - smoothstep(0.56, 0.92, phase);
	float depth_fade = smoothstep(0.04, 0.18, dist)
		* (1.0 - smoothstep(0.78, 1.02, dist));
	return ray_mask * density * head * tail * depth_fade;
}

float warp_particles(float dist, float angle, float time) {
	float depth_cell = floor(dist * 150.0 - time * 2.2);
	float ray_cell = floor((angle / TAU + 0.5) * 220.0);
	float seed = hash12(vec2(ray_cell, depth_cell));
	float particle = smoothstep(0.965, 0.995, seed);
	float depth_fade = smoothstep(0.08, 0.20, dist)
		* (1.0 - smoothstep(0.72, 1.0, dist));
	return particle * depth_fade;
}

void fragment() {
	vec2 c = UV - vec2(0.5);
	// Aspect-correct so radial streaks remain circular on wide screens.
	c.x *= SCREEN_PIXEL_SIZE.y / SCREEN_PIXEL_SIZE.x;

	float dist = length(c);
	float angle = atan(c.y, c.x);
	float time = TIME * scroll_speed;

	float aspect = SCREEN_PIXEL_SIZE.y / SCREEN_PIXEL_SIZE.x;
	vec2 flow = normalize(vec2(flow_direction.x * aspect, flow_direction.y));
	vec2 radial = c / max(dist, 0.0001);
	float direction_bias = 0.62 + 0.38 * max(dot(radial, flow), 0.0);
	float direction_mix = clamp(direction_confidence, 0.0, 1.0);
	direction_bias = mix(1.0, direction_bias, direction_mix);

	float near_layer = radial_layer(dist, angle, time, 22.0, 1.00, 34.0, 1.0);
	float mid_layer = radial_layer(dist, angle, time, 42.0, 1.18, 56.0, 2.0);
	float far_layer = radial_layer(dist, angle, time, 78.0, 1.36, 88.0, 3.0);
	float streaks = (near_layer * 0.62 + mid_layer * 0.82 + far_layer * 0.56) * direction_bias;
	float particles = warp_particles(dist, angle, time) * direction_bias;

	// A soft bloom and two faint axes sell the tunnel's vanishing point without
	// turning the whole overlay into a solid white flash.
	float halo = exp(-dist * dist * 5.0);
	float core = exp(-dist * dist * 28.0);
	float vanishing_flare = exp(-dist * 9.0) * direction_bias;
	float pulse = 0.92 + 0.08 * sin(TIME * pulse_speed);
	float edge_fade = 1.0 - smoothstep(0.58, 0.98, dist);
	float motion = clamp(intensity, 0.0, 1.0);

	float warm_mix = clamp(near_layer * 0.8 + particles * 0.35, 0.0, 1.0);
	vec3 streak_color = mix(tunnel_color, accent_color, warm_mix);
	vec3 color = streak_color + core_color * core * (0.25 + motion * 0.45);
	float glow = streaks * 0.54
		+ particles * 0.20
		+ halo * 0.08
		+ (vanishing_flare * 0.08 + core * 0.28) * pulse;
	float alpha = clamp(glow * edge_fade * motion * opacity, 0.0, 1.0);
	COLOR = vec4(color, alpha);
}
