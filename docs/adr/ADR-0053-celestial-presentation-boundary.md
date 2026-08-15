---
id      : ADR-0053
title   : Physically grounded celestial presentation boundary
status  : accepted
date    : 2026-08-13
deciders: [human, ai-agent]
related : ADR-0025 (celestial bodies and sun direction), ADR-0029 (true-scale coordinates), ADR-0044 (absolute f64 authority)
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

`WorldPresentation` continues to derive the observer-to-star direction from
absolute f64 positions. The sky uses that direction for the apparent disc;
`DirectionalLight3D` uses its inverse, the star-to-scene light ray, and the
same spectral color. The light is disabled until valid star data is available.

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
