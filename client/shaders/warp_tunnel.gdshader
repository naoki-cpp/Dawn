// warp_tunnel.gdshader
//
// Full-screen overlay representing "inside the warp tunnel" for the locally
// piloted ship (ADR-0029 lore pass, 2026-06-23). The server's true warp speed
// (often ~10^8+ Godot units/tick at true AU) can't be rendered as literal
// motion without flinging the camera past the far plane and jittering in f32
// -- so instead of showing that, this overlay fades in to cover the cruise
// portion of the transit, with `intensity` driven by the client from the
// ship's speed (smoothed, so the fade itself reads as the accelerate-in /
// decelerate-out transition rather than a hard cut -- see main.gd
// _update_warp_tunnel_effect()).
//
// Purely cosmetic: radial streaks scrolling outward + a bright converging
// core, intensity-modulated alpha so intensity=0 is fully transparent.

shader_type canvas_item;

uniform float intensity   : hint_range(0.0, 1.0) = 0.0;
uniform float scroll_speed: hint_range(0.0, 20.0) = 6.0;
uniform vec3  tunnel_color: source_color = vec3(0.55, 0.75, 1.0);

void fragment() {
	vec2 c = UV - vec2(0.5);
	// Aspect-correct so streaks read as radial, not stretched, on wide screens.
	c.x *= SCREEN_PIXEL_SIZE.y / SCREEN_PIXEL_SIZE.x;

	float dist  = length(c);
	float angle = atan(c.y, c.x);

	// Angular slots that scroll outward over time (radius term in the fract
	// makes the slot boundaries sweep toward the screen edge), reading as
	// light streaking past rather than a static starburst.
	float slots = 64.0;
	float slot  = fract(angle / TAU * slots + dist * 2.0 - TIME * scroll_speed);
	float streak = smoothstep(0.92, 1.0, slot) + smoothstep(0.08, 0.0, slot);
	streak *= smoothstep(0.05, 0.5, dist); // no streaks right at screen centre

	// Bright converging core + edge falloff so streaks read as rushing
	// toward (or away from) a vanishing point at screen centre.
	float core      = exp(-dist * 3.0);
	float edge_fade = 1.0 - smoothstep(0.55, 0.95, dist);

	float glow = clamp((streak * 0.8 + core * 0.6) * edge_fade, 0.0, 1.0);
	COLOR = vec4(tunnel_color, glow * intensity);
}
