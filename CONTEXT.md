# Dawn Context

This file defines the shared domain language for Dawn. Read it before naming
issues, tests, refactors, or new concepts. Operational rules live in
`AI_DEVELOPMENT_GUIDE.md`; design decisions live in `docs/adr/`.

## Product Context

Dawn is an EVE-like single-shard space game. The distributed simulation is a
means to make large, risky, player-driven battles possible without EVE-style
global TiDi.

The design question is:

> Does this increase intentional player decisions?

Dawn should avoid systems where time, money, or unattended work directly makes a
player stronger.

## Core Terms

- World: the whole simulated universe.
- Sector: a spatial partition that owns ships and runs local simulation.
- Node: a process or logical runtime that hosts sectors and participates in
  replication or consensus.
- Ship: the current primary entity controlled by players, bots, or scripts.
- Command: a request to change the world. It may be rejected.
- Event: an immutable fact that already happened.
- Tick: deterministic logical time. It is not wall-clock time.
- Snapshot: an authoritative checkpoint used with tail replay for recovery.
- Transit: an ownership transfer between sectors.
- Warp: intra-sector fast movement represented through velocity changes and
  deterministic integration.
- Tackle: combat control that prevents escape actions such as warp or jump.
- Fitting: a ship's equipped modules and slot layout.
- Module: an item that changes capabilities or performs an active effect.
- Capacitor: the ship energy resource shared by active modules.
- TiDi: local real-time pacing degradation used only as a bounded last resort;
  logical ticks remain deterministic.

## Runtime Boundaries

- `dawn-core` defines pure domain types, commands, and events.
- `dawn-ecs` defines components and systems without owning persistence.
- `dawn-event-store` owns append-only persistence and snapshots.
- `dawn-consensus` owns Raft and sector transit consensus.
- `dawn-replication` owns sector-local append-log replication.
- `dawn-sector` owns sector game logic and writes committed facts.
- `dawn-actor` owns the client/server protocol boundary.
- `dawn-simulation` wires local simulations and demos.
- `dawn-sector-node` is the real hardware node binary.

## Vocabulary Preferences

Use these names consistently:

- Use "Sector" instead of "zone", "cell", or "shard".
- Use "Node" for runtime/process ownership, not for ships or graph vertices.
- Use "Command" for requests and "Event" for committed facts.
- Use "Transit" for cross-sector ownership transfer.
- Use "Warp" for intra-sector fast movement.
- Use "Tackle" for escape denial.
- Use "TiDi" only for real-time pacing, not for changing logical tick order.

## Non-Goals

- No skill-point growth, passive growth, pay-to-win, or AFK mining.
- No wall-clock time as causal order.
- No direct actor-to-actor state mutation.
- No sector ownership shortcut around consensus.
- No event-log rewrite, truncate, update, or delete path.

## Source Of Truth

- Game vision: `docs/adr/ADR-0016-game-vision.md`
- Architecture map: `docs/architecture/architecture.md`
- Event catalog: `docs/architecture/event-catalog.md`
- Ownership: `docs/architecture/ownership.md`
- Tick model: `docs/architecture/tick-model.md`
- Forbidden changes: `docs/architecture/forbidden-changes.md`
