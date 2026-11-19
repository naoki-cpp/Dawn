---
id      : ADR-0059
title   : Rust-generated angular Galactic backgrounds
status  : accepted
date    : 2026-11-20
deciders: [human, ai-agent]
related : ADR-0053 (local stellar authority retained), ADR-0054 (replaces synthetic parallax and Godot population generation), ADR-0055 (replaces renderer-dependent background baking)
---

# ADR-0059 - Rust-generated angular Galactic backgrounds

## Context

The distant sky combines individual stellar sprites with unresolved Galactic
light and foreground dust. Its physical definitions previously spanned
GDScript generation and per-frame shader integration. WorldPresentation also
owned the renderer-dependent panorama bake and its timing. Finite star shells
following the Ship produced artificial parallax during orbit-camera movement.

The approved direction prioritizes a reusable scientific structure, rather than
independent art tuning of stellar populations and the unresolved background.

## Decision

`dawn-client-core` owns a deterministic, Godot-independent Galactic model and
generates a scene-linear RGB panorama and angular stellar records. Physical
model calculations use f64; final rendering buffers may use f32. A shared
stellar-emissivity and dust model supplies both representations. Their light
allocation is explicit; separately rendered stars must not simply be added to
an unchanged full-population unresolved emissivity.

The GDExtension adapter owns one cancellable CPU worker and a bounded desired
request/result cache. It polls without blocking the main thread. No worker
accesses Godot objects. Replaced requests cannot publish stale results, and
leaving the scene requests cancellation without blocking on a worker join.

`SkyBackground` owns the Godot resources and installs the panorama and stellar
geometry together after completion. Until then it retains the previous
background, or a minimal initial background. Generation failure leaves that
background usable and reports the failure once. The same model settings reuse
the last completed CPU result. This is a bounded in-memory cache, not a persistent
asset or an unbounded cache of visited systems.

System-map arrival requests generation. All systems currently select the same
configuration. Different system configurations can be selected in the future;
this ADR does not introduce a wire field, infer a galaxy position from a Sector
ID, or claim system-specific authored backgrounds already exist.

Distant stars and continuum are angular backgrounds: camera rotation changes
the viewed direction; translation, orbit distance and floating-origin rebases
must not create artificial stellar parallax. Local stellar discs, illumination,
lens response, planets and navigation retain ADR-0053 projection semantics.
The camera scheduling refactor is separate.

## Approximation Limits

This is a synthetic Milky-Way-like observational model, not Gaia data or a
calibrated population synthesis. Smooth stellar/dust scale profiles and seeded
correlated structure describe statistical structure, not measured cloud
locations. RGB spectra, finite integration, finite star sampling, light
allocation and display scaling remain approximations. No observed H-alpha
emission or gas scattering is invented from a dust-noise field.

The existing physical-scale rationale is retained from ADR-0055. Stellar
population and extinction must be coupled: see
[Just et al., consistent Milky Way disk model](https://doi.org/10.1051/0004-6361/202038840)
for how dust choices change stellar samples, and
[Barros et al. 2016](https://www.aanda.org/articles/aa/pdf/2016/09/aa27535-15.pdf)
for the adopted disk-scale context. These references do not validate the
renderer as a measured or radiometrically calibrated model.

## Alternatives

Keeping galaxy integration in the sky shader preserves duplicated model
ownership and per-frame work. Renderer baking adds timing and local-star
contamination hazards. Pre-generating one shipped panorama avoids runtime work
but was rejected in favor of future per-system generation. Generating on the
main thread would stall camera interaction. Moving local celestial authority
into the distant background would mix gameplay coordinates with a synthetic
galaxy and is not permitted.

## Verification

- Rust tests cover deterministic generation, finite outputs, shared extinction,
  light allocation, invalid budgets and cancellation.
- Adapter tests cover bounded replacement, stale-result rejection and cache reuse.
- Godot tests cover background installation, translation-invariant directions,
  local-star independence and failure/teardown behavior.
- Measure generation time and output memory; inspect rendered screenshots.
  Performance and visual equivalence are not implied by unit tests.
