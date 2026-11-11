// star_sprite.gdshader
//
// Billboard sprite for one star of the ADR-0054 starfield. Positions, colours
// and flux come from the MultiMesh, so this shader generates nothing: it only
// expands each instance into a screen-facing quad and shades the point spread.
//
// The quad is sized in PIXELS rather than in world or angular units. A real
// point source stays a point at any zoom, and a fixed pixel footprint is also
// what keeps the sprite above the sampling rate, so it cannot shimmer under
// camera rotation the way an analytically drawn sub-pixel Gaussian does.

shader_type spatial;
#include "res://shaders/stellar_plasma.gdshaderinc"
#include "res://shaders/galactic_dust.gdshaderinc"
// fog_disabled matters: the sprite shell sits 4,000,000 units out, far past
// the aerial-perspective fog's end distance, so without opting out every
// star would render as solid fog colour.
render_mode unshaded, blend_add, depth_draw_never, cull_disabled, fog_disabled;

// Lets the whole field be dimmed or lifted without rebuilding the MultiMesh.
uniform float flux_scale : hint_range(0.0, 4.0) = 1.0;
uniform float extinction_scale : hint_range(0.0, 1.0) = 1.0;
uniform vec3 sun_direction = vec3(0.0, 0.0, -1.0);
uniform float sun_angular_radius = 0.0;
// Half-width of every star's quad, in pixels. It contains both the point core
// and its faint halo; it does not represent the physical size of a star.
uniform float point_spread_pixels : hint_range(1.0, 12.0) = 10.0;
// Viewport size in pixels, pushed from WorldPresentation every frame.
// VIEWPORT_SIZE is a FRAGMENT-stage built-in in Godot's spatial shaders: it
// is silently unusable here in vertex(), which collapses the quad instead of
// raising a compile error, so the size has to arrive as a uniform.
uniform vec2 viewport_pixels = vec2(1280.0, 720.0);
uniform float core_sigma_pixels = 0.72;
uniform float halo_sigma_pixels = 3.0;
uniform float halo_peak_ratio = 0.02;

varying vec2 corner;
varying float flux;
varying vec3 dust_transmission;
varying float distance_kpc;

void vertex() {
	// INSTANCE_CUSTOM is packed by Starfield.build_multimesh().
	flux = INSTANCE_CUSTOM.r * flux_scale;
	distance_kpc = INSTANCE_CUSTOM.g * 22.0;
	vec3 star_direction = normalize(MODEL_MATRIX[3].xyz - CAMERA_POSITION_WORLD);
	vec3 galaxy = to_galactic(star_direction);
	dust_transmission = extinction_transmission(
		galactic_visual_extinction(galaxy, distance_kpc) * extinction_scale);
	if (sun_angular_radius > 0.0 && dot(star_direction, sun_direction) > cos(sun_angular_radius)) {
		flux = 0.0;
	}
	float pixels = max(1.0, point_spread_pixels);

	// QuadMesh spans -0.5 .. 0.5, so this is the unit corner offset.
	corner = VERTEX.xy * 2.0;

	// Project the instance centre, then widen in clip space. NDC spans two
	// units across the viewport, hence the factor of two.
	vec4 clip = PROJECTION_MATRIX * MODELVIEW_MATRIX * vec4(0.0, 0.0, 0.0, 1.0);
	clip.xy += corner * (pixels * 2.0 / max(viewport_pixels, vec2(1.0))) * clip.w;
	POSITION = clip;
}

void fragment() {
	float radius_pixels = length(corner * point_spread_pixels);
	float radius_squared = radius_pixels * radius_pixels;
	float core = exp(-0.5 * radius_squared / (core_sigma_pixels * core_sigma_pixels));
	float halo = halo_peak_ratio
		* exp(-0.5 * radius_squared / (halo_sigma_pixels * halo_sigma_pixels));
	ALBEDO = COLOR.rgb * dust_transmission * flux * (core + halo);
	ALPHA = 1.0;
}
