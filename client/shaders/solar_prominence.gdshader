shader_type spatial;
render_mode unshaded, blend_mix, depth_draw_never, cull_disabled, fog_disabled;

uniform sampler2D shape_mask : filter_linear_mipmap, repeat_disable;
uniform sampler2D layer_one : filter_linear_mipmap, repeat_enable;
uniform sampler2D layer_two : filter_linear_mipmap, repeat_enable;
uniform float detail_visibility = 1.0;
uniform vec3 sun_direction = vec3(0.0, 0.0, -1.0);
uniform float sun_angular_radius = 0.0;
varying vec3 world_position;

void vertex() {
	// Keep both feet attached while neighbouring sheets move independently.
	float anchored = sin(PI * UV.x);
	VERTEX *= 1.0 + anchored * 0.003 * sin(UV.x * 18.0 + COLOR.g * 9.0 + TIME * 0.018);
	world_position = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
}

void fragment() {
	vec3 ray = normalize(world_position - CAMERA_POSITION_WORLD);
	// The photosphere lives in the sky, so it cannot write a scene depth mask.
	if (dot(ray, sun_direction) > cos(sun_angular_radius)) {
		discard;
	}
	vec2 mask_uv = vec2(UV.x, UV.y * 0.5 + 0.5);
	float envelope = texture(shape_mask, mask_uv).r;
	vec2 flow_one = vec2(UV.x * 0.08 - TIME * 0.0012, UV.y * 0.13 + COLOR.b * 0.73);
	vec2 flow_two = vec2(UV.x * 0.19 + TIME * 0.0005, UV.y * 0.24 - COLOR.g);
	float filaments = texture(layer_one, flow_one).r;
	float fine = texture(layer_two, flow_two + vec2(0.0, (filaments - 0.5) * 0.14)).r;
	float density = envelope * mix(0.2, 1.0, smoothstep(0.25, 0.75, filaments))
		* mix(0.35, 1.0, smoothstep(0.25, 0.75, fine));
	// Layered optical-depth approximation, not an additive glowing outline.
	ALPHA = (1.0 - exp(-density * 3.0)) * COLOR.r * detail_visibility;
	ALBEDO = vec3(0.12, 0.001, 0.0002);
	EMISSION = mix(vec3(0.9, 0.02, 0.002), vec3(2.8, 0.12, 0.008), fine) * (0.4 + filaments);
}
