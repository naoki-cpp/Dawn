---
scope    : Current celestial presentation pipeline and approximation limits
audience : Human Developer / AI Agent
update   : When rendering ownership, processing order or approximations change
related  : ../adr/ADR-0053-celestial-presentation-boundary.md, ../adr/ADR-0054-starfield-sprite-geometry.md, ../adr/ADR-0055-nebula-background-representation.md, ../process/godot-client-testing.md
---

# Celestial Rendering

[ADR-0053](../adr/ADR-0053-celestial-presentation-boundary.md) records the decisions
and rationale. This document describes the current implementation. Numeric art
settings belong in their source/preset; test commands belong in
[Godot Client Testing](../process/godot-client-testing.md#fixed-lighting-review).

## Ownership

| Owner | Responsibility |
|---|---|
| Authoritative body records | Absolute positions, physical radii, body kind and spectral type |
| `CelestialProjection` / `WorldSpace` | f64 observer-relative projection before Godot vector conversion |
| [WorldPresentation](../../client/scripts/world_presentation.gd) | Connect projected source, navigation, sky, scene illumination and camera optics |
| [Space sky shader](../../client/shaders/space_sky.gdshader) | Local stellar disc, close surface and corona; direction-only background |
| [Starfield](../../client/scripts/starfield.gd) | CPU-generated background star sprites, independently of the local stellar disc |
| [SolarProminences](../../client/scripts/solar_prominences.gd) | Camera-relative prominence geometry and visibility |
| [StellarSource](../../client/scripts/stellar_source.gd) | Shared distant-source radiance, integrated image-plane flux and disc variance |
| [Lens compositor](../../client/addons/lens_effects/lens_flare_compositor_effect.gd) | Lens settings, coherent render-thread source snapshot and bounded dispatch |
| [Lens shader](../../client/addons/lens_effects/lens_flares.glsl) | Normalized glare/streak response and separate depth-occluded shafts |

The sky draws the star; its selectable bracket adds no duplicate stellar mesh.
Ordinary bodies retain their physical radius after render-scale conversion.
Navigation markers are not geometry authority.

## Processing Order

```text
body record + observer absolute f64 position
  -> observer-relative direction and angular radius
  -> projected source radius / viewport height
  -> common source radiance, flux and variance
      -> direct sky response
      -> lens glare and horizontal streak
  -> separate screen-space occlusion shafts
  -> scene glow and tone mapping
```

Scene illumination uses the same star direction, inverted to a star-to-scene
light ray. The photographic source model does not redefine hull-light energy
or authoritative stellar radius.

Source position, colour, visibility, projected radius and depth-sampling footprint
travel together in a mutex-protected snapshot. Shader uniforms do not sample
mutable main-thread curves or absolute world coordinates.

## Source And Lens Response

For an unclamped projected radius `r`, normalized to viewport height:

```text
image-plane flux = surface radiance * PI * r^2
disc axis variance = r^2 / 4
```

Surface radiance is a shared scene-unit art calibration, not SI radiometry.
The point-source path uses the calibrated value of 160 scene radiance units;
the resolved photosphere colour is normalized by its reciprocal so restoring
point-source visibility does not brighten the close-up surface a second time.
Camera optics apply a continuous highlight response to integrated source flux:
unresolved sources receive a 10x low-signal gain, while the response rolls off
smoothly toward a cap of 10 times the flux at projected radius 0.006. This keeps
distant stellar streaks visible without a distance mode and prevents a resolved
close-up star from flooding the lens kernels. The stellar disc radius and its
second-moment blur remain unmodified. Lens fractions partition this camera
response; they are not a claim of radiometric energy conservation.
The production lens has neutral RGB transmission: flare colour follows the
stellar spectral RGB supplied by the presentation state, not a fixed blue
coating. Optional `Lens_Tint` is a multiplicative transmission filter, not a
replacement light colour. This is RGB rendering, not wavelength-resolved optics;
the orange false-colour close surface is not the star's visible-light spectrum.
Unlike the old independent amplitude controls, source flux changes with
projected area; no logarithmic distance multiplier is applied to the lens.

Nonnegative camera fractions partition energy into direct light, glare, a
streak, ghosts and a halo. Separate shader functions evaluate each component
in one HDR pass, with no intermediate textures or component classes.
Glare combines compact neutral and broad coated Gaussian components;
the streak combines a long narrow Gaussian with a shorter, broader coating band.
Compact diagonal diffraction shoulders surround the neutral core. Ghosts are
three horizontally stretched defocused images on
the source/optical-centre axis; the halo is a smooth normalized annulus around
the inverted source image. These are analytic lens approximations, not optical
ray tracing. Off-axis ghost stretch/tilt and halo ellipticity vary smoothly with
source field position, using area-preserving transforms so deformation does not
create energy. Horizontal streaks remain sensor-aligned. This is an authored
camera response, not a calibrated model of a particular lens. The offline
lighting study accepts `--sweep` to capture central, intermediate and edge views.
Fractions are capped in glare/streak/ghost/halo order; direct light
receives the remainder. Kernels are normalized before coating
absorption and occlusion. Changing width or squeeze redistributes the allocated
light rather than multiplying its integrated energy.

Disc variance is added to Gaussian variances and broadens the halo width. This is a second-moment
approximation to finite-disc convolution, not exact integration of surface
detail. Finite kernel support and a subpixel sampling floor bound work and avoid
unstable narrow highlights. CPU dispatch includes that source-broadened support
and clips it to the viewport. Enabling internal reflections currently dispatches
the full viewport because their images can lie opposite the source; the shader
still skips each disabled reflection component. The production preset assigns
only a small fraction to these reflections.

White saturation emerges from source energy and the scene's exposure, glow and
tone mapping. There is no separate opaque optical disc, authored white shoulder,
root-specific gain or display-space flare brightness curve.

The bounded physical occlusion footprint is measured in pixels and is separate
from source flux/radius. All lens components share the source's depth visibility.
Lens light may overlay a foreground hull when the source remains visible, since
it is a camera artifact rather than a world-space shaft.

The same camera fractions apply to distant and resolved discs: there is no
near-distance lens-disable switch. Increasing disc variance broadens the lens
response. When source variance dominates, normalized Gaussian kernels blend
toward flat-topped fourth-order kernels with the same covariance and integral.
This reduces the artificial central peak of the moment approximation without
creating energy or switching the lens off. Glare also allocates more energy to
its broad wing instead of piling it into the compact core.
Resolved-source visibility uses a bounded equal-area disc sampling
pattern instead of treating a blocked centre as a fully hidden star. This is
an area-visibility and second-moment approximation, not spatial convolution
of the visible surface texture; a partially occulted source retains a symmetric
blur approximation. Without a RenderingDevice compositor the direct sky keeps
the full source. Close surface detail retains its separate false-colour art
calibration. Its intensity now scales with the shared source radiance, but
the procedural surface's mean and limb darkening are not an exact match to the
uniform-disc PSF approximation. Camera-response gain and its highlight knee
are authored in `StellarSource`; the same continuous response applies at every
distance.

## Surface, Corona And Prominences

Close stellar views use procedural orange/red false-colour detail, not calibrated
visible-light photographs or a blackbody temperature map. A shared warped field
relates subdued dark filaments, bright ribbons and active-region knots.
Derivative filtering softens unresolved features.
Fine surface noise fades to its mean when unresolved. A narrow continuous
chromospheric envelope spans the apparent limb, joining surface emission to
the outer corona rather than adding an independent opaque outline. Surface
brightness remains an art calibration, not exposure-correct solar radiometry.
Use the offline lighting study's `--close` capture for a resolved-disc review;
its placeholder rocks are hidden in that mode to leave the surface inspectable.

Granulation is a cellular approximation in stellar-radius units. Unresolved cells
fade toward their mean rather than expanding to a fixed fraction of the disc.
Its evolution and solar-like spatial scale are presentation assumptions, not a
fluid or magnetohydrodynamic simulation.

Corona rendering uses smooth falloff, asymmetric streamers and filtered fibres.
Its extent is bounded independently of a close, screen-filling disc. It does
not simulate volumetric radiative transfer.

Prominences use fixed-footpoint curtain meshes with shallow depth layers,
a static density envelope and independently moving detail textures. The
camera-relative proxy preserves apparent angular size without narrowing
absolute positions. It clips the sky-rendered photosphere, which cannot provide
scene depth; ordinary foreground geometry still depth-tests. Projected pixel
size, including camera zoom, controls detail visibility.

## Godray

Godray is a separate screen-space presentation effect, outside the normalized
lens PSF. Exponentially decayed radial integration of resolved reverse-Z depth
produces a squared inverse occlusion mask over local glow. Density controls
integration reach; shadow weight controls contrast. Normalized weights keep
unoccluded gain independent of sample quality, and pixel-stable jitter avoids
time-varying shimmer.

Shafts do not brighten foreground surfaces. They may remain beside a partially
obscured source rather than being gated by the central source-visibility sample.
With no occluders they produce a smooth veil, not procedural radial stripes.
Behind-camera and offscreen-centre suppression still applies. A disc whose
centre is outside the viewport is not yet handled as a partial offscreen source.

A continuous compact falloff has zero value and slope at its boundary, avoiding
a late circular cutoff band. Work and samples are bounded; zero shaft strength
skips both its march and its additional dispatch extent. Transparent surfaces
without depth writes do not participate in the depth mask.

This is not physical dust transport, volumetric scattering, or a claim that
light beams are visible in empty vacuum.

## Materials And Configuration

The production environment owns scene exposure, glow and a cool ambient fill.
Background brightness and fill lighting are independent; brightening the whole
image is not a substitute for readable backlit materials.

The ship scene binds materials through Godot's surface override property.
Its base hull material is lit/non-emissive and its engine has compact HDR
emission. This describes the scene asset, not every gameplay material override.

Settings have one owner:

| Settings | Source of truth |
|---|---|
| Source radiance and source-area conversion | `stellar_source.gd` |
| Lens fractions, widths, squeeze, coating and shaft settings | `client/resources/solar_anamorphic_lens.tres` |
| Background palette, fill, exposure and glow | `WorldPresentation._setup_space_environment()` |
| Close surface/corona detail | Sky shader and prominence implementation |
| Base hull and engine materials | `client/scenes/ship.tscn` |

Do not duplicate current numeric values here or in the ADR. Change the owner
and verify it with the fixed review scene and relevant regression tests.

## Review And Limits

The offline lighting study uses production sky/optics and the actual ship scene,
with deterministic lit rock fixtures. These are review fixtures, not newly
introduced gameplay asteroids. Current ship geometry remains a placeholder.

GPU regressions check absolute lens energy before clipping, source-area scaling,
squeeze/resolution invariance, FOV projection, occlusion, MSAA and resize.
The procedure and scene commands live in
[Godot Client Testing](../process/godot-client-testing.md#fixed-lighting-review);
player navigation checks live in the [Playtest Guide](../process/playtest-guide.md).

These checks do not establish exact optics across every angle, stellar size or
exposure. Resolved false-colour detail, approximate disc convolution and
screen-space shafts remain distinct limitations. Sustained GPU frame-time
budgeting requires separate measurement.

## References

- [Trinity EvePlaneSet](https://github.com/carbonengine/trinity/blob/main/trinity/Eve/SpaceObject/Attachments/Sets/EvePlaneSet.cpp): separation of mask/shape and moving detail, not copied game content.
- [ASC lens comparisons](https://theasc.com/post/the-film-book/practical-optics-4-anamorphic-videos/) and [ARRI Master Anamorphic](https://www.arri.com/en/cine-lenses/arri-zeiss-fujinon-lenses/master-anamorphics): visual lens characteristics.
- [NASA solar surface features](https://solarscience.msfc.nasa.gov/feature1.shtml): solar granulation reference.
- [ARez compositor](https://github.com/ARez2/compositor-effect-lens-effects): base compositor and decayed occlusion-mask reference; license/provenance live in the [addon README](../../client/addons/lens_effects/README.md).
