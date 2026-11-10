---
id      : ADR-0053
title   : Physically grounded celestial presentation boundary
status  : accepted
date    : 2026-08-13
deciders: [human, ai-agent]
related : ADR-0025 (celestial bodies and sun direction), ADR-0029 (true-scale coordinates), ADR-0044 (absolute f64 authority), ADR-0054 (amends the star layer in section 3), ADR-0055 (amends the nebula representation in section 3)
---

# ADR-0053 - Physically grounded celestial presentation boundary

## Context

The galaxy uses physical metres for celestial radii and AU-authored orbital
positions. Compressed-scale assumptions placed fresh players inside the local
star and disconnected the sky's sun from scene illumination.

Presentation must communicate system scale without creating a second gameplay
radius, narrowing absolute coordinates, or claiming that procedural skies are
measured astronomical data. Frequently adjusted art settings should not become
architectural contracts.

## Decision

### 1. Topology-derived fresh spawn

Use `SimulationNode::default_player_spawn_position()` as the single
topology-derived fresh-spawn policy across production admission:
prefer a local station, then a safe position outside an authored body.
This avoids invalid inside-star spawns and makes station operations available
without tying admission to a compressed-scale constant. Resume and explicit
duel/test positions remain caller-owned.

### 2. One sun direction, two presentation consumers

Derive observer-to-star direction and apparent angular radius from absolute
f64 positions. The sky consumes that direction for the local stellar disc;
scene illumination uses the inverse light ray and spectral colour.
Disable stellar effects when valid source data is unavailable.

Navigation brackets and camera-relative proxies may represent distant bodies,
but must not introduce a second stellar mesh, gameplay radius or position
authority. This keeps navigation, illumination and the visible star aligned
through travel and floating-origin rebases.

### 3. Physical body size with bounded visual approximation

Keep physical radii and render-scale conversion at the existing projection
boundary. Materials, corona, prominences, glare and navigation aids are
presentation-only approximations. They must not change simulation geometry.

Use one scene-linear source model for distant stellar direct light and its
lens response. Partition source energy through a normalized point-spread
function (PSF); let exposure, glow and tone mapping determine visible saturation.
This replaces independently amplified white cores/glare/streaks whose apparent
relationship changes with source size or resolution. Lens characteristics and
exposure still require art calibration; this is not a calibrated optical model.

Keep screen-space occlusion shafts (Godray) separate from the normalized lens
response. They approximate visibility around occluders, not physical scattering
in empty vacuum.

The background is a stylized observational subset, not navigation authority or
a full astronomical catalogue. ADR-0054 owns the starfield representation;
ADR-0055 owns the nebula/background representation.

## Consequences

- Fresh admission respects authored celestial scale.
- Navigation, stellar direction and scene illumination share authoritative inputs.
- Source size and camera response can change without redefining gameplay radii.
- Approximate optics and false-colour surface rendering remain explicit limits.
- Art tuning does not require an ADR amendment unless it changes these boundaries
  or the source/energy ownership decision.
- A future texture/asset pipeline can replace procedural materials without
  changing wire schemas or simulation authority.

## Implementation And Verification

Current rendering ownership, processing order and approximation limits:
[Celestial Rendering](../architecture/celestial-rendering.md).

Review scene, commands and regression-test procedures:
[Godot Client Testing](../process/godot-client-testing.md#fixed-lighting-review).

Spawn/arrival behaviour and player controls:
[Playtest Guide](../process/playtest-guide.md).

Tuning values belong in source code and the camera preset, not this ADR.
