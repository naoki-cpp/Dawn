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

- Anchor: a per-body local coordinate origin a Ship's position is stored
  relative to (`AnchorId` + f64 offset), keeping ship-scale precision at
  true-AU distances from the Sector origin. Composed to/from Sector-frame
  absolute coordinates by `AnchorTable` (`dawn_sector::anchor`).
- World: the whole simulated universe.
- Sector: a spatial partition that owns ships and runs local simulation.
- Node: a process or logical runtime that hosts sectors and participates in
  replication or consensus.
- Ship: the current primary entity controlled by players, bots, or scripts.
- Owned ship: a Ship a `PlayerId` holds ownership of. A player may own more
  than one (ADR-0037).
- Active ship: the one owned ship currently routable for flight/steering/
  module commands and Undock. Singular per player; switched explicitly via
  `SelectActiveShipCommand`, never automatically (ADR-0037).
- Disembark: voluntarily clearing a player's active ship while docked, via
  `DisembarkCommand`. The ship stays owned and docked; only which ship the
  player's commands route to changes (ADR-0037). Re-entered via
  `SelectActiveShipCommand`.
- Command: a request to change the world. It may be rejected.
- Event: an immutable fact that already happened.
- Tick: deterministic logical time. It is not wall-clock time.
- Snapshot: an authoritative checkpoint used with tail replay for recovery.
- Transit: an ownership transfer between sectors.
- Warp: intra-sector fast movement represented through velocity changes and
  deterministic integration.
- Tackle: combat control that prevents escape actions such as warp or jump.
- Area-of-Interest (AoI): the policy deciding which ships and events a given
  player's session is owed each frame, based on spatial proximity
  (`dawn_sector::aoi`).
- Fitting: a ship's equipped modules and slot layout.
- Item: something a player can own, store in inventory, and consume/produce
  via Station operations (build cost, Assemble/Disassemble). Includes Module,
  Packaged Ship, and Scrap Metal.
- Scrap Metal: the raw resource consumed to build a new Packaged Ship (and,
  later, other Items). Dropped by ShipDestroyed — acquired only through
  active combat, never passive gathering (Non-Goals: no AFK mining).
- Module: an Item that changes capabilities or performs an active effect.
- Packaged Ship: the Item form of a Ship — storable, not pilotable. Converts
  to a Ship via Assemble at a Station, and back via Disassemble. Assemble
  requires the Packaged Ship to be fully unfitted (no Modules attached).
  Disassemble requires the Ship to be undamaged and fully unfitted (prevents
  free repair by round-tripping through Item form, and reuses the existing
  Module fit/unfit path instead of a new fitted-Item data shape).
- Station: a location where Assemble/Disassemble (and future building)
  happens. Initially NPC-provided only; player-built Stations are a later
  extension (see Source Of Truth: roadmap.md Phase 9).
- Currency: a per-Player balance held in the Market's own ledger, not an Item.
  Unlike Item (which a Ship physically carries and can lose on ShipDestroyed),
  Currency survives ship loss. Priced trades settle via the Market's own
  bid/ask order book, not a fixed or formula-driven price.
- Capacitor: the ship energy resource shared by active modules.
- TiDi: local real-time pacing degradation used only as a bounded last resort;
  logical ticks remain deterministic.

## Runtime Boundaries

- `dawn-core` defines pure domain types, commands, and events.
- `dawn-client-core` defines the Godot-independent client-side domain model
  (loadout, wire row types), depending only on `dawn-core` (ADR-0039).
- `dawn-client-gdext` is the GDExtension binding exposing `dawn-client-core`
  to the Godot client (ADR-0040).
- `dawn-ecs` defines components and systems without owning persistence.
- `dawn-event-store` owns append-only persistence and snapshots.
- `dawn-consensus` owns Raft and sector transit consensus.
- `dawn-replication` owns sector-local append-log replication.
- `dawn-sector` owns sector game logic and writes committed facts.
- `dawn-actor` owns the client/server protocol boundary.
- `dawn-server` owns the server composition boundary: `simulate` wires local
  simulations/demos, and `sector-node` is the real hardware node binary.

## Vocabulary Preferences

Use these names consistently:

- Use "Sector" instead of "zone", "cell", or "shard".
- Use "Node" for runtime/process ownership, not for ships or graph vertices.
- Use "Command" for requests and "Event" for committed facts.
- Use "Transit" for cross-sector ownership transfer.
- Use "Warp" for intra-sector fast movement.
- Use "Tackle" for escape denial.
- Use "TiDi" only for real-time pacing, not for changing logical tick order.
- Use a `*Wire` suffix for adapter-only `dawn-wire` schema types
  (`AbsPosWire`, `VelWire`, ...). `ServerFact` is the typed client projection,
  not a durable-event mirror -- ADR-0042/#274 moved every client<->server
  message onto the postcard binary envelope. The Sector
  request exception is the shared typed `ClientRequest` authority, re-exported
  by `dawn-wire` rather than duplicated as a `*Wire` mirror. Functions that
  genuinely produce a JSON Schema document (`server_fact_json_schema()`,
  `client_request_json_schema()`) keep `json` in their names.

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
