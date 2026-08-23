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
// fog_disabled matters: the sprite shell sits 4,000,000 units out, far past
// the aerial-perspective fog's end distance, so without opting out every
// star would render as solid fog colour.
render_mode unshaded, blend_add, depth_draw_never, cull_disabled, fog_disabled;

// Lets the whole field be dimmed or lifted without rebuilding the MultiMesh.
uniform float flux_scale : hint_range(0.0, 4.0) = 1.0;
// Half-width of every star's quad, in pixels. One value for the whole field:
// stars are point sources, so they share a point spread and only their flux
// differs. Pushed from Starfield.POINT_SPREAD_PIXELS.
uniform float point_spread_pixels : hint_range(1.0, 8.0) = 2.6;
// Viewport size in pixels, pushed from WorldPresentation every frame.
// VIEWPORT_SIZE is a FRAGMENT-stage built-in in Godot's spatial shaders: it
// is silently unusable here in vertex(), which collapses the quad instead of
// raising a compile error, so the size has to arrive as a uniform.
uniform vec2 viewport_pixels = vec2(1280.0, 720.0);
// How fast the point spread falls off inside the quad. Higher concentrates
// the star into a smaller visible dot for the same peak brightness, which is
// what keeps a bright star from reading as a ball rather than a point.
uniform float core_sharpness : hint_range(2.0, 24.0) = 6.0;
// Where the profile is truncated, in squared quad-relative radius. Pulling
// this in shortens the tail without shrinking the Gaussian itself, so the
// dot gets smaller without dropping below the sampling rate.
uniform vec2 core_cutoff = vec2(0.55, 1.0);

varying vec2 corner;
varying float flux;

void vertex() {
	// INSTANCE_CUSTOM is packed by Starfield.build_multimesh().
	flux = INSTANCE_CUSTOM.r * flux_scale;
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
	// Gaussian core, faded out before the quad edge so no square shows.
	float r2 = dot(corner, corner);
	float core = exp(-r2 * core_sharpness)
		* (1.0 - smoothstep(core_cutoff.x, core_cutoff.y, r2));
	ALBEDO = COLOR.rgb * flux * core;
	ALPHA = 1.0;
}
