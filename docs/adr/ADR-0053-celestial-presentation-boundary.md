---
id      : ADR-0053
title   : Physically grounded celestial presentation boundary
status  : accepted
date    : 2026-08-13
deciders: [human, ai-agent]
related : ADR-0025 (celestial bodies and sun direction), ADR-0029 (true-scale coordinates), ADR-0044 (absolute f64 authority), ADR-0054 (amends §3 — the star layer moves out of the sky shader), ADR-0055 (amends §3 — the nebula representation is under review)
---

# ADR-0053 - Physically grounded celestial presentation boundary

## Context

The galaxy data now uses physical metres for celestial radii and AU-authored
orbital positions. The client still had two compressed-scale assumptions:

- fresh players were admitted at a fixed `30_000 m` point, which placed them
  inside the physical radius of the local star;
- the background rendered a direction-based sun, while the scene's
  `DirectionalLight3D` kept an unrelated fixed transform and the planets all
  used one grey material.

The sky is also a presentation layer, not a complete astronomical catalogue.
It must communicate the scale and lighting of the authored system without
pretending that procedural nebula noise is a measured survey of the galaxy.

## Decision

### 1. Topology-derived fresh spawn

`SimulationNode::default_player_spawn_position()` is the single policy used by
production fresh admission adapters. It selects the local station first, then
falls back to a point two planet radii outside a local planet, then two star
radii outside the local star. A legacy constant remains only as a safe fixture
fallback and is not used by production admission.

This keeps a fresh player outside all authored bodies and makes station
operations available immediately. Resume and explicit duel/test positions keep
their existing caller-owned coordinates.

### 2. One sun direction, two presentation consumers

The star also has a selectable navigation bracket, aligned with the sky disc
from the camera position. It adds no stellar mesh. Existing body warp policy
places arrival at 1.5 stellar radii from the centre, outside the surface.

`WorldPresentation` continues to derive the observer-to-star direction from
absolute f64 positions. The sky uses that direction for the apparent disc;
`DirectionalLight3D` uses its inverse, the star-to-scene light ray, and the
spectral color. The light is disabled until valid star data is available.

The stellar disc uses an orange/red false-colour solar-observation treatment:
mottled fine-scale structure, sparse bright active regions and a subdued limb
with sparse radial plumes. `SolarProminences` owns six fixed-footpoint curtain
meshes with three shallow depth layers, a static torn-edge density mask and two
independently seeded/scrolled fibre textures. Shape is independent of flow;
the mask retains asymmetric knots and holes while the two detail layers move
at different rates. This follows the separation of image/mask and two UV
layers in [Trinity's EvePlaneSet](https://github.com/carbonengine/trinity/blob/main/trinity/Eve/SpaceObject/Attachments/Sets/EvePlaneSet.cpp),
not a copy of its shaders or game assets.
The surface uses one warped, stellar-space scalar field for long dark
filaments, neighbouring bright ribbons and compact activity knots. Shared
regional envelopes and fine surface detail connect these features instead of
adding independent white spots. Derivatives soften unresolved field contours.
The field is visual structure, not a recovered magnetic state or MHD model.
One is dominant; the others are faint. Alpha blending and an optical-depth
approximation create irregular, translucent red fibre bundles, not additive
wire outlines. The camera-relative proxy preserves the stellar angular radius
without narrowing absolute positions. Its material clips the photosphere's
projected disc, which is sky-rendered and cannot provide a depth buffer; scene
geometry still depth-tests normally. Curtains fade by projected pixel size,
including camera zoom, rather than angular radius alone. The sky corona has a
smooth inner falloff, asymmetric streamer lengths, curved radial fibres and
a faint diffuse outer component. Angular extent is capped independently of a
close stellar disc, and a smooth outer taper avoids a visible cutoff. Pixel
derivatives average unresolved fibres; the old blocky rim noise is removed.
It is still a false-colour density approximation, not volumetric scattering.
The former
per-pixel sky arch search is removed. This is not magnetic field/plasma
dynamics or volumetric radiative transfer. This is an artistic
reference-image match, not
natural visible-light appearance or a blackbody temperature map. The physical
angular radius and spectral scene illumination are unchanged. The external
glow stays narrow, rather than filling the screen. Point-source camera streaks
fade out as the stellar disc becomes resolved at close range; their kernel
does not scale with the stellar radius. Exposure and surface detail remain
artistic approximations, not calibrated radiometry.

At unresolved angular sizes the orange surface treatment blends to a bright,
nearly white HDR source. A pixel-filtered Gaussian conserves its apparent-area
scaling rather than enlarging the physical body or applying a fixed-size halo.
This is a display/exposure approximation, not an increase in scene light energy.
Angular separation uses cross/dot `atan` to avoid small-angle `acos` precision
loss. Resolved close views retain the detailed surface and its existing gain.

Camera optics additionally use Trinity's logarithmic distance response
`1.5 / log(distance_au + 2.71)`, bounded to `[0.1, 1.5]`. It affects only
optical extent and strength, never stellar radius, scene lighting or the
physical source's occlusion sampling footprint. Distance is derived from the
authoritative radius and apparent angle. The optical scale travels in the
same mutex-protected render snapshot as the source; CPU dispatch bounds and
GPU effect bounds use the same scaled horizontal length. Existing front/offscreen,
edge-fade and close-disc suppression remain. This borrows the response from
[EveLensflare](https://github.com/carbonengine/trinity/blob/main/trinity/Eve/EveLensflare.cpp),
not Trinity's complete foreground/background occlusion-buffer implementation.

The solar camera now uses a small anisotropic point-spread approximation:
a bright, slightly wider horizontal response near the source and a narrow,
smoothly decaying lateral tail. A warm radial glare joins a steep inner falloff
to a much weaker extended veil. It is added transparently with the same physical
source visibility, not drawn as another stellar disc. Squeeze changes its horizontal length, with
intensity compensation. Width is viewport-height-relative with a subpixel
sampling floor, not a stellar-radius-dependent halo. The shader adds linear HDR
light before Godot's glow and tone mapping; it does not compress or boost a
separate optical core.

`client/resources/solar_anamorphic_lens.tres` defines intensity, length, width,
squeeze, coating tint, glare strength and glare radius. Each presentation duplicates the preset. The main
thread publishes projected source state as one mutex-protected snapshot.
The physical solar-disc depth samples control the entire streak. When the
source is visible, the streak can overlay foreground objects: it is a camera
artifact, not a world-space shaft casting shadows.

The previous generated atlas, artificial core, reflection images, independent
response curves and the former combined god-ray implementation are removed rather than retained as
disabled alternatives. The stellar sky remains the source of the visible disc
and HDR core. Thresholded additive Environment glow supplies surrounding glare;
low-luminance background bloom is disabled. CPU dispatch covers only the finite
streak/glare support, with viewport clipping and source visibility checks.

Occlusion light shafts are controlled separately from lens streak/glare. A
bounded radial integration of resolved reverse-Z background visibility creates
light/shadow bands around foreground silhouettes, with a smooth local falloff.
They are not multiplied by point-source visibility, so light can remain beside
a partly obscured source. Foreground pixels receive no shaft light. Source
behind/offscreen/close suppression still applies to the entire effect. With no
occluders this is only a smooth veil, not generated stripes. This is a
screen-space presentation approximation, not volumetric dust transport or a
claim of visible beams in empty vacuum. Radius, strength and 8-96 samples are
camera settings; zero strength skips the march and its dispatch extent.
The Godray response follows ARez's decayed occlusion-mask approach rather than
the former cubed transmission fan: density sets radial reach, exponential decay
weights the samples and shadow weight controls a squared inverse mask over a
local inverse-square-like glow. Weights are normalized, samples are bounded to
8-96 (preset 48), and jitter is pixel-stable rather than time-varying. Glare
remains enabled independently. Source visibility guards, bounded dispatch and
foreground preservation remain Dawn-specific adaptations, not a wholesale
copy of the asset's flare or engine-private scene-buffer layout.

Glare and shaft support use a continuous compact falloff from the source, with
zero value and slope at the boundary. There is no late annular cutoff band.
The wide glare tail is kept weak to avoid a visibly painted circular halo;
occluder-driven structure remains deterministic, without added radial noise.

This is a deliberately restrained anamorphic camera look, not a ray-traced lens
prescription. Physical stellar size, hull illumination and close-disc suppression
remain separate from camera optics. References:
[ASC lens comparison](https://theasc.com/post/the-film-book/practical-optics-4-anamorphic-videos/)
and [ARRI Master Anamorphic](https://www.arri.com/en/cine-lenses/arri-zeiss-fujinon-lenses/master-anamorphics).

Granulation is a procedural cellular approximation on the stellar sphere:
bright interiors, dark intergranular lanes, approximately 1,000 km spacing
and a ten-minute evolution cycle for solar-like stars. Radius comes from the
body record rather than a disc-relative texture scale. Screen derivatives
fade unresolved cells to their mean instead of magnifying or aliasing them.
This is not a fluid or magnetohydrodynamic simulation; the solar scale is a
presentation assumption for other spectral types as well.
See [NASA solar surface features](https://solarscience.msfc.nasa.gov/feature1.shtml).

### 3. Physical body size with bounded visual approximation

Planet meshes retain `radius * render_scale` exactly. Their surface material is
procedural and deterministic, with body-specific palettes and noise seed, so
the renderer can distinguish rocky, oceanic, icy, and rust-toned bodies without
inventing a second gameplay radius or requiring an asset pipeline first.

The sky keeps a procedural Milky Way and nebula approximation, but adds a small
explicit bright-star catalogue layer. The catalogue is a visual landmark set,
not a replacement for authoritative sector topology or a claim of full-sky
astrometric accuracy.

The environment uses Filmic tonemapping and restrained HDR glow for the star
disc and catalogue landmarks. The camera still has a finite presentation far
plane (`5,000,000` render units, or `50,000,000 m` at the current `0.1`
render scale); distant bodies must therefore use the existing marker/label
fallback rather than pretending that the renderer can display the whole galaxy.

Material/lighting review uses a weak cool ambient fill independently of the
sky background and fixed exposure, preserving backlit material detail without
brightening every background pixel. The ship scene binds hull/engine materials
through Godot's `surface_material_override/0` property. The painted hull is lit,
not emissive; its engine alone has bounded emission and a compact glow mesh.
Godray contrast is restrained so it does not replace material shading.

`client/scenes/lighting_study.tscn` is an offline, fixed composition using the
production sky/optics, actual ship scene and seeded, lit procedural rock fixtures.
The rocks are art-review fixtures, not newly introduced gameplay asteroids.
It provides repeatable material/exposure comparisons without a live server.
The current ship geometry remains a placeholder, not a production-quality asset.

## Consequences

- A fresh player starts near the local station instead of at an arbitrary
  compressed-scale point.
- Moving toward or away from the star updates both the apparent disc and the
  3D illumination direction coherently.
- Celestial bodies remain physically sized at the WorldSpace boundary; labels,
  reticles, and distant markers remain presentation-only aids.
- The sky remains an observational subset over a stylized procedural backdrop;
  it is not a navigation or astrometry source of truth.
- A future texture/asset pipeline can replace the procedural planet material
  without changing the wire schema or server authority.

## Verification checklist

- [x] `default_player_spawn_position()` is used by single, cluster, and
      sector-node fresh admission.
- [x] Rust test proves the demo spawn equals the local station and is outside
      the local star radius.
- [x] GDScript tests cover the inverse star light ray and invalid inside-star
      sun state.
- [x] GDScript tests cover deterministic planet profiles and the fixed-size
      bright-star GPU arrays.
- [x] Full GdUnit4 suite passes with the physical planet radius assertions.
