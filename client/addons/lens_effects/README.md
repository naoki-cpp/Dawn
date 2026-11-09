# Anamorphic Solar Camera Effect

The base RenderingDevice compositor is adapted from
https://github.com/ARez2/compositor-effect-lens-effects
at commit `0ce061271f672f05fbb9aed08a61e019a875c1ad`, under MIT (see LICENSE).
The base derives from Pink Arcana's
https://github.com/pink-arcana/godot-distance-field-outlines .

Dawn's lens script and compute shader now use an anisotropic point-spread
approximation: a narrow horizontal tail and a slightly wider, brighter response
near the source. A separate warm glare profile joins a steep inner falloff to a
much weaker extended veil; it is transparent optical scatter, not a solar disc.
Atlas textures, extra solar cores, reflection sprites, response
curves have been removed. This is an artistic
anamorphic response, not a calibrated optical simulation.

`WorldPresentation` supplies projected source position, colour, visibility,
physical occlusion radius and a bounded distance response. The camera preset
exposes streak intensity, length, width, squeeze, coating tint, glare strength
and glare radius. The stellar disc and
surrounding Environment glow remain separate.

Occlusion light shafts have independent strength, radius and a bounded 8-96
sample budget (48 by default). Following ARez's Godray approach, exponentially
decayed radial integration of depth occlusion creates a squared inverse mask
over local optical glow. Density controls integration reach and shadow weight
controls contrast; normalized weights keep unoccluded gain independent of sample
count. Jitter is pixel-stable rather than time-varying to avoid shimmer.
Radial integration of the resolved background-depth mask creates
shadows behind foreground silhouettes. This separate screen-space approximation
does not add light to foreground surfaces and is not volumetric dust scattering.
With no occluders it produces a smooth local veil, not procedural radial stripes.
Zero strength bypasses its depth march and excludes its radius from dispatch.

The compute pass reads resolved reverse-Z depth with nine source-visibility taps
per workgroup, adds linear HDR light before scene glow and tone mapping, and
preserves alpha. Its streak can overlay foreground geometry while the source
is visible: it is a lens artifact, not a world-space light shaft. Dispatch is
clipped to the finite streak support and actual viewport dimensions. No private
engine UBO or textures are used.

Forward+ or Mobile with RenderingDevice is required. The physical sun still
renders without the compositor in Compatibility/headless mode.

## Verification

Run `scripts/setup-godot.ps1 -RunTests` (or the shell equivalent).
`sun_flare_render_test.gd` reads GPU pixels to verify horizontal falloff, source
tracking, squeeze response, disabled effects, source occlusion with MSAA disabled
and 4x MSAA, foreground overlays, clipped bounds and viewport resize.
Removed-feature tests were deleted rather than retaining dormant implementations.
`world_presentation_test.gd` separately covers projection and distance response.
