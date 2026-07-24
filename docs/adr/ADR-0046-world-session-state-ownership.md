---
id      : ADR-0046
title   : WorldSession pure state ownership in dawn-client-core
status  : accepted
date    : 2026-07-24
deciders: [human, ai-agent]
related : ADR-0039 (dawn-client-core client domain model), ADR-0040 (Godot adapter),
          ADR-0045 (single-owner client motion state), docs/architecture/architecture.md
---

# ADR-0046 - WorldSession pure state ownership in dawn-client-core

## Context

The former `client/scripts/world_session.gd` mixed live-world state with the
Godot scene-tree representation of ships. That made the state model depend on
`Node3D`, made state transitions difficult to exercise without a scene, and
allowed `main.gd` to retain mutable aliases to session state. The client-core
crate already owns the pure Loadout and motion models, so WorldSession state
belongs at the same boundary.

## Decision

`dawn-client-core::WorldSessionState` owns pure client state:

- ship metadata, health, player/opponent membership, and lock state;
- navigation records and current system state;
- tick, capacitor, and dock transition state;
- typed transition outcomes for registration, removal, destruction, health, and
  docking operations.

`dawn-client-gdext::WorldSession` is a thin adapter. It parses JSON only at the
Godot boundary, delegates state transitions to `WorldSessionState`, and returns
Godot `Dictionary` snapshots/outcomes. It must not store or accept `Node3D`
references.

`main.gd` remains responsible for Godot presentation and lifecycle: its
`_ships` dictionary maps ship IDs to scene nodes, creates/frees those nodes,
and applies returned state to visual components. It synchronizes scalar and
collection state through `WorldSession.snapshot()` instead of retaining aliases
to Rust-owned collections.

The alternative of keeping the GDScript session and adding Rust helpers was
rejected because it would leave two state owners and preserve scene-tree
coupling. Moving the entire scene registry to Rust was rejected because Rust
must not own Godot scene nodes or presentation lifecycle.

## Implementation checklist

- [x] Add `WorldSessionState` and typed input/record/outcome types to
      `crates/dawn-client-core`.
- [x] Add the `WorldSession` GDExtension adapter in
      `crates/dawn-client-gdext`.
- [x] Keep the Godot scene-node registry in `client/scripts/main.gd` and remove
      `client/scripts/world_session.gd`.
- [x] Add pure Rust and GdUnit4 coverage for navigation, ship lifecycle, HP,
      locks, ticks/capacitor, and docking state.
- [x] Update crate-boundary and architecture documentation.
