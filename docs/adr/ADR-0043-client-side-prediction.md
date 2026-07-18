---
id      : ADR-0043
title   : Client-Side Prediction and Motion Reconciliation
status  : proposed
date    : 2026-07-18
deciders: [human, ai-agent]
related : ADR-0008 (VelocityChanged authority), ADR-0023 (movement physics),
          ADR-0029 (true-scale coordinates), ADR-0040 (GDExtension binding),
          ADR-0042 (postcard wire envelope), docs/process/roadmap.md section 13
---

# ADR-0043 - Client-Side Prediction and Motion Reconciliation

## Context

The client currently dead-reckons every ship from the last
`VelocityChanged` event. A local `MoveCommand` only changes the thrust arrow,
so the player's ship does not react until the server's next velocity event
arrives. The result is visible input latency and a presentation path that is
different from the server's movement rule.

The server must remain authoritative. Prediction is only a local presentation
optimization and must be corrected by a server-owned position. Warp, docking,
undocking, and jump arrival are discontinuities and already have explicit
authoritative messages or domain events.

## Decision

Add a `MotionPredictor` deep module to `dawn-client-core`. The shared
one-tick movement policy lives in `dawn-core::movement::MovementProfile` and
is used by both the server and client adapters. `MotionPredictor` owns:

- the effective movement profile (`max_speed`, `mass`, `inertia_modifier`),
- the motion mode (local prediction or remote dead-reckoning),
- local thrust/brake intent,
- predicted position and velocity, and
- tick-aware reconciliation against server state.

`MovementProfile::step` is pure and stateless: it receives the current
velocity and one `MovementInput`, then returns the next velocity and braking
completion state. `dawn-ecs::MovementSystem` remains responsible for ECS
updates, position integration, warp exclusion, and `VelocityChanged` emission;
`MotionPredictor` remains responsible for client prediction and
reconciliation. The same track also owns constant-velocity dead-reckoning for
remote ships, so local and remote ships share reset, fractional-tick, and
authoritative-state behavior. Neither adapter owns a second copy of the
movement formula.

The GDExtension exposes this module to `ship_controller.gd`; GDScript remains
responsible only for coordinate conversion, rendering, visual speed capping,
rotation, and input routing. A floating-origin rebase shifts the track's
position together with the rendered node; velocity and protocol tick are
unchanged by a coordinate-frame shift.

The `ShipStateWire` spawn payload includes the movement profile and initial
velocity. During normal flight, `AoiDelivery` sends an owner-only
`ServerMessage::MotionCorrection` containing:

```text
ship_id, absolute position, velocity, authoritative tick
```

The correction is sent when the owner's `VelocityChanged` is delivered. The
client accepts only non-stale ticks, resets its predictor position and
velocity, and preserves the current local input so prediction resumes without
waiting for another click.

`VelocityChanged` remains unchanged. It is still the domain event containing
velocity only, as required by ADR-0008. `MotionCorrection` is a transport
message, not a new domain event or client command.

Committed warp is excluded from normal-flight corrections. The existing
`PositionSnap` remains the authority for warp arrival, and docking/jump
handlers reset the shared motion track at their authoritative position. Remote
ships use that same track in dead-reckoning mode between authoritative
velocity updates.

## Invariants

- The server remains the sole authority for simulation position and velocity.
- A client cannot use prediction to bypass command validation or movement rules.
- The predictor and `dawn-ecs::MovementSystem` both delegate one-tick movement
  to `dawn-core::movement::MovementProfile::step`.
- A stale correction cannot move the local ship backwards in protocol time.
- Warp, dock, and jump discontinuities clear local input and reset the shared
  motion track at the authoritative position.
- Local prediction and remote dead-reckoning use one client motion track; the
  GDScript ship controller is only its visual adapter.
- Floating-origin rebases shift every affected ship's motion track and node
  together, while preserving velocity and protocol tick.
- No per-frame position stream or new DomainEvent is introduced.

## Rejected alternatives

- **Client-only acceleration with no correction:** leaves drift and makes the
  presentation diverge after packet delay or a server-side state change.
- **Adding position to `VelocityChanged`:** couples a domain event to a
  presentation transport concern and breaks ADR-0008's event contract.
- **Sending a full position snapshot every frame:** consumes bandwidth and
  duplicates the correction already needed only when authoritative velocity
  changes.
- **Keeping the predictor in GDScript:** duplicates server physics outside the
  Rust test boundary and makes the fitted movement profile harder to share.
- **Keeping separate Rust formulas in the server and client:** allows small
  numeric or edge-case differences to drift silently, so the pure one-tick
  policy is shared from `dawn-core` instead.
- **Keeping separate local and remote client motion paths:** duplicates
  discontinuity and coordinate-frame handling, making remote interpolation and
  local prediction disagree after resets or floating-origin rebases.

## Implementation checklist

- [x] Add the pure Rust `MotionPredictor` and unit tests.
- [x] Add movement profile/initial velocity to `ShipStateWire`.
- [x] Add owner-only `MotionCorrection` delivery and wire decoding.
- [x] Bind prediction/reconciliation through `dawn-client-gdext`.
- [x] Route local Move/Stop input into the predictor.
- [x] Reset prediction for warp, docking, and jump discontinuities.
- [x] Share the one-tick movement policy from `dawn-core` between server and
  client adapters.
- [x] Use one client motion track for local prediction and remote
  dead-reckoning.
- [x] Shift the motion track during floating-origin rebases.
- [ ] Verify the Godot playtest with the built GDExtension DLL.
- [ ] Obtain human approval and change status from `proposed` to `accepted`.
