# Compositor lens effects (Dawn adaptation)

Source: https://github.com/ARez2/compositor-effect-lens-effects
Pinned commit: `0ce061271f672f05fbb9aed08a61e019a875c1ad`.
License: MIT, see LICENSE. The base compositor derives from Pink Arcana's
https://github.com/pink-arcana/godot-distance-field-outlines .
Shader source attributions to Shadertoy are retained in `lens_flares.glsl`.

Only the three runtime files are vendored; no editor plugin or demo is needed.
`WorldPresentation` supplies the local star's projected direction, spectral
color and apparent radius from the same f64-derived state as the physical sun.

Dawn changes:

- Remove the fake fixed-size solar disc and always-on rear-facing flare.
- Keep photosphere and corona in the sky; remove its old optical streak.
- Fade at screen edges, disable behind the camera, and bound all depth reads
  and compute dispatches. Occlusion uses nine taps over the apparent solar disc.
- Share these taps per workgroup and skip optical calculations when occluded.
- Add a separate faint optical glow with depth-marched shadows, following the
  God rays section (pages 40-42) of Páll Ragnar Pálsson's
  [Shaders in Eve Online](https://cadia.ru.is/wiki/_media/public:t-gede-12-1:gede-guest-pall.pdf).
  This is camera optics, not a simulation of atmospheric scattering in space.
- Dispatch only a clipped quad around the sun, with a soft circular edge.
  Default radius is 0.32 viewport heights, with 24 stable-jitter depth samples.
  Rays do not brighten opaque foreground surfaces. Lens and ray strengths are
  independent; disabling rays skips their sampling loop without losing flares.
- Publish changing sun parameters through a mutex-protected render snapshot.
- Reuse the compute pipeline instead of recreating it on every sun update.
- Use the actual color-image size instead of Godot's private SceneData UBO
  include (not available in the standard 4.7.2 distribution).
- Request resolved color/depth after transparency, preserving opaque occlusion
  with MSAA. Transparent surfaces without depth writes do not occlude the sun.

Forward+ or Mobile is required. Compatibility/headless leaves the physical sun
intact but does not attach this effect. Only camera-relative rendering data
crosses this GPU boundary; there is no dependency on the engine's precision mode.

## Verification

Run `scripts/setup-godot.ps1 -RunTests` (or the shell equivalent).
`sun_flare_render_test.gd` reads actual GPU pixels at 321x241, checking visible,
occluded, rear-facing and offscreen sources with MSAA disabled and 2x MSAA.
It also checks radial shadows from an off-center occluder, unchanged opaque
surfaces/screen corners, and clipped dispatch bounds.
`world_presentation_test.gd` checks camera projection and directional tracking.
These rendering tests require the normal Forward+ test environment, not the
headless/Compatibility renderer.

Local verification on 2026-11-02: Godot 4.7.2 / Intel UHD / Forward+,
231 GdUnit4 cases passed. Main-scene screenshots checked at 1280x720 and
481x641 with a ship near the solar disc, flares only, rays, and rear view.
A 40-frame 1280x720 GPU sample measured 34.7 ms off, 35.1 ms flares only,
and 35.9 ms with rays. These short samples share the GPU with other processes
and are indicative, not a performance guarantee.

Interactive warp gameplay and a sustained frame-time budget still need playtest.
