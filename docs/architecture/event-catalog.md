---
scope    : Complete spec of every Event and Command that exists. The single source of truth for "what can happen"
audience : AI Agent / Human Developer
update   : Must be updated whenever an Event or Command is added or changed
related  : entity-model.md, tick-model.md, event-schema-evolution.md
---

# Event Catalog

## 1. Using This Catalog

### Sync rule

This catalog must always match the definitions in `dawn-core/src/events.rs` / `dawn-core/src/commands.rs`. Update both code and catalog in the same PR.

### Procedure for adding an Event

```
1. Add the new event to this catalog
2. Add the type to dawn-core/src/events.rs
3. If a corresponding Command is needed, add it to dawn-core/src/commands.rs
4. Write a unit test in events.rs
5. List the changed events in the PR description
```

### Backward compatibility rules

**Pre-release (current):** No external user event logs exist yet, so breaking changes (removing fields, changing types, removing events) are allowed directly.

**Post-release:**
```
Allowed: add new fields as Option<T>
Forbidden: remove an existing field
Forbidden: change an existing field's type
Forbidden: rename an existing field
Forbidden: rename an event (introduce a V2 instead)
```

Post-release breaking changes must follow the [Upcaster procedure](#6-upcaster-catalog). See [event-schema-evolution.md](./event-schema-evolution.md) for details.

---

## 2. Event Design Principles

### Command vs Event

| | Command | Event |
|---|---|---|
| Meaning | a **request** for change | the **fact** that change occurred |
| Rejectable | yes | no (already happened) |
| Persisted | no | yes, append-only |
| File | `commands.rs` | `events.rs` |

Commands and Events must never share a type or enum (INV-006).

### Common field

Every event has `tick: Tick`. An event omitting `tick` is rejected as an INV-005 violation.

### Optional field policy

- Fields defined initially are always required (never `Option`)
- Fields added later are always `Option<T>`
- Never define a field as `Option` from the start (to avoid unintentional omission)

### Persistence model (two-tier log, ADR-0017)

Events in this catalog are persisted **append-only** (INV-001 / FBD-001). The physical log has two tiers, but this does not change event immutability or catalog semantics:

```
Hot log      : latest segment, kept bounded by compacting behind verified snapshots
Cold archive : segments moved out by compaction; retained forever, append-only (audit / DR)
```

- Compaction (`compact`) **relocates segments**; it never rewrites or deletes events. The EventStore trait stays append-only (no truncate / delete / rewrite).
- **The snapshot is the authoritative persistent checkpoint** (INV-002). Normal recovery and failover use "snapshot + hot-log tail catch-up"; full replay from genesis is off the critical path.
- Derived/transient state (position, capacitor, lock countdowns, thrust intent) is not recorded in events — it lives only in snapshots. Each event's **Replay** note describes reconstructing authoritative state from events; transient state is recomputed live each Tick.

See [ADR-0017](../adr/ADR-0017-snapshot-compaction.md) / [AI_DEVELOPMENT_GUIDE.md "Architecture Invariants"](../../AI_DEVELOPMENT_GUIDE.md) (INV-002).

---

## 3. Event List

### 3.1 Ship Lifecycle

| Event | Description | Emitter | Status |
|---|---|---|---|
| `ShipSpawned` | Ship appeared in the world | `SimulationNode::spawn_ship()` | ✅ implemented |
| `ShipDespawned` | Ship manually removed from the world | `SimulationNode` | type only (no emission site; Replay supported) |
| `ShipDestroyed` | Ship destroyed in combat | `CombatSystem` | ✅ implemented |

### 3.2 Movement

| Event | Description | Emitter | Status |
|---|---|---|---|
| `VelocityChanged` | Ship velocity changed | `MovementSystem::run()` | ✅ implemented (ADR-0008) |

### 3.3 Fitting

| Event | Description | Emitter | Status |
|---|---|---|---|
| `ShipFitted` | Ship's fitting slots and/or cargo projection changed | `SimulationNode::fit_module()` / Market item bridge | ✅ implemented |
| `ModuleActivated` | Active Module turned on | `SimulationNode::activate_module_owned()` | ✅ implemented |
| `ModuleDeactivated` | Active Module turned off (manual, or forced off by Capacitor exhaustion / target out of range) | `SimulationNode::deactivate_module_owned()` / `CapacitorSystem` / `SimulationNode::process_range_gate()` | ✅ implemented (cap exhaustion: ADR-0011, out-of-range: ADR-0035; `forced_reason` carries the cause) |

### 3.4 Lock-on

| Event | Description | Emitter | Status |
|---|---|---|---|
| `TargetLocked` | Lock-on completed | `LockSystem::run()` | ✅ implemented |
| `LockLost` | Lock lost | `LockSystem::run()` | ✅ implemented |

### 3.5 Combat

| Event | Description | Emitter | Status |
|---|---|---|---|
| `WeaponFired` | Weapon fired | `CombatSystem::run()` | ✅ implemented |
| `DamageTaken` | Ship took damage | `CombatSystem::run()` | ✅ implemented |
| `RepairApplied` | Ship's Shield/Armor restored by a local repair Module | `RepairSystem::run()` | ✅ implemented (ADR-0033) |

### 3.6 Sector Transit (ADR-0014)

| Event | Description | Emitter | Status |
|---|---|---|---|
| `SectorTransitRequested` | Durable transfer request carrying source-local request identity plus the resolved Gate/non-Gate route; source retains a frozen recovery copy until Ack | source `propose_transit()`; destination also records an incoming marker using its own local `tick` | ✅ implemented |
| `SectorTransitCompleted` | Destination completed the import, or source removed its recovery copy after Ack | destination `handle_transit_commit()`; source `complete_outgoing_transit()` | ✅ implemented |
| `SectorTransitAborted` | Transit aborted (ownership stays with `from`) | destination failure path (not wired) | type + Replay |

Validation-stage rejection is expressed via `CommandRejected`, not an event (INV-006); there is no `SectorTransitRejected` event. `propose_transit` returns `Err` without emitting an event if the Ship is absent or already in Transit.

The corresponding command is `TransitCommand { ship_id, to }`. Raft carries `TransitOp::Request`, `Commit`, and `Ack`: Request freezes the source and persists `request_tick`, `gate_id`, `entry_pos`, and `entry_pos_abs`; Commit materializes the destination and durably records completion using a destination-local event `tick`; Ack removes the source recovery copy. Unresolved source Requested events are retried from EventStore after restart with bounded exponential backoff, and checkpoint compaction is deferred while one is pending.

### 3.7 Jump Gate Navigation (ADR-0009, complete)

| Event | Description | Emitter | Status |
|---|---|---|---|
| `JumpGateUsed` | Ship moved to another Sector via a Jump Gate | `SimulationNode::append_jump_events` (Step 7.5, destination node) | ✅ implemented (Raft pipeline) |
| `StarSystemChanged` | Ship moved to a different star system (concurrent with `JumpGateUsed`) | `SimulationNode::append_jump_events` (Step 7.5, destination node) | ✅ implemented (Raft pipeline) |

Corresponding Command: `JumpCommand { gate_id }`, committed over the same Raft Log path as `TransitCommand` (ADR-0014). The server resolves the command against the caller's active ship (ADR-0037). `TransitOp::Request`/`Commit` carries `gate_id: Option<JumpGateId>`; after destination Commit, the destination appends `JumpGateUsed` alongside `SectorTransitCompleted`, appends `StarSystemChanged` if needed, and then proposes Ack.

Static topology (3 star systems, 4 jump gates) is defined in `dawn-sector/src/galaxy.rs` (ADR-0026). `dawn-wire`'s `domain_event_to_event_wire` serializes both events to clients over the postcard binary envelope (ADR-0042), and `client_command_from_wire` handles `JumpCommand`. The Godot client (`connection.gd`'s `send_jump_command`, `main.gd`'s `_handle_jump_gate_used` / `_handle_star_system_changed`) is also implemented (ADR-0009 checklist fully complete).

### 3.8 Tackle (ADR-0024)

| Event | Description | Emitter | Status |
|---|---|---|---|
| `TackleApplied` | Fold Disruptor activated on a target (in range + locked) | `SimulationNode::process_tackle()` (Step 4.5) | ✅ implemented |
| `TackleReleased` | Tackle effect ended (Module off / out of range / tackler destroyed) | `SimulationNode::process_tackle()` (Step 4.5) | ✅ implemented |

While tackled, `can_propose_warp()` / `can_propose_jump()` return false, blocking Warp/Jump. `TackledComp` is persisted in the snapshot (INV-002).

`TackleReleased` is never emitted without a matching prior `TackleApplied` (strict 1:1 pairing). With multiple simultaneous tacklers, each tackler gets its own pair.

### 3.9 Coordinate Anchoring (ADR-0029)

| Event | Description | Emitter | Status |
|---|---|---|---|
| `AnchorRebased` | Ship's coordinate anchor changed (absolute position unchanged; only the `(anchor, offset)` representation updates — e.g. star anchor → destination-body anchor on Warp arrival) | `SimulationNode` (Warp arrival, ADR-0029 step 4) | ✅ implemented (emitted from `warp_step`/`rebase_arrival_event`, appended via `tick.rs`'s `all_events`) |
| `ShipDocked` | Ship docked at an NPC station (ADR-0034 9B docking foundation) | `SimulationNode::dock_owned` | ✅ implemented |
| `ShipUndocked` | Ship undocked from an NPC station (ADR-0034 9B docking foundation) | `SimulationNode::undock_owned` | ✅ implemented |
| `PackagedShipBuilt` | Scrap Metal consumed in a docked station and converted into a packaged ship (ADR-0034 9B) | `SimulationNode::build_packaged_ship_owned` | ✅ implemented |
| `ShipDisassembled` | Docked undamaged unfitted ship converted into a packaged ship in station inventory (ADR-0034 9B) | `SimulationNode::disassemble_ship_owned` | ✅ implemented |
| `ShipAssembled` | A station-inventory `PackagedShip` item converted into a new live docked `Ship`, owned by the caller (ADR-0034 9B, ADR-0037). Fields: `ship_id` (freshly allocated), `player_id`, `station_id`, `ship_type_id`, `tick`. Replay: allocates the ECS entity with `e.ship_id` directly (no new ID counter increment beyond what's needed to keep it monotonic), unfitted, docked at `station_id`, owned by `player_id`; does **not** change `active_ship` (ADR-0037 -- a later `SelectActiveShipCommand` makes it active) | `SimulationNode::assemble_ship_owned` | ✅ implemented |

This is an authoritative event: it stores `anchor` and the post-rebase `offset` so Replay reproduces the representation exactly. A rebase is a non-velocity-driven frame change, so it's recorded as its own fact; INV-MOVE (the invariant for velocity-driven motion) doesn't apply since absolute position is preserved.

### 3.11 System (reserved for future use)

| Event | Description | Status |
|---|---|---|
| `TickStarted` | Tick started | not implemented |
| `TickCompleted` | Tick completed | not implemented |

### 3.12 AoI (Area of Interest) Delivery Filter (ADR-0019)

AoI introduces **no new domain events**. It's implemented by filtering `DomainEvent` delivery through each observer's 27-cell neighborhood.

| Message | Description | Status |
|---|---|---|
| `AoiEnter` | Ship entered an observer's AoI (WebSocket delivery message, not a domain event) | ✅ implemented (8C, ADR-0019) |
| `AoiLeave` | Ship left an observer's AoI (same) | ✅ implemented (8C, ADR-0019) |

`AoiEnter` / `AoiLeave` are not appended to the EventStore — they are delivery-control messages, not domain events. They don't affect Replay; AoI consistency comes from `InitialState` + `DomainEvent` filtering.

---

## 4. Command List

Commands are defined in `dawn-core/src/commands.rs`. Clients send them to the server wrapped in the `ClientCommand` enum (`dawn-actor`).

| Command | Description | Resulting Event(s) | Status |
|---|---|---|---|
| `MoveCommand` | Specify thrust direction | — | ✅ implemented |
| `LockOnCommand` | Request lock-on | `TargetLocked` | ✅ implemented |
| `FitModuleCommand` | Fit an inventory Module into a slot; requires the ship to be docked (ADR-0032, docked requirement added 2026-07-08) | `ShipFitted` | ✅ implemented |
| `UnfitModuleCommand` | Return a fitted Module to inventory; requires docked (ADR-0032) | `ShipFitted` | ✅ implemented |
| `ReorderFittedModuleCommand` | Reorder two fitted modules within the same slot kind (persisted -- iteration order assigns weapon hotkeys); requires docked (ADR-0032) | `ShipFitted` | ✅ implemented |
| `DockCommand` | Dock at an NPC station once within its docking radius (ADR-0034 9B) | `ShipDocked` | ✅ implemented |
| `UndockCommand` | Leave a previously-docked NPC station (ADR-0034 9B) | `ShipUndocked` | ✅ implemented |
| `BuildPackagedShipCommand` | Consume Scrap Metal in the current docked station and create a `PackagedShip` item there (ADR-0034 9B) | `PackagedShipBuilt` | ✅ implemented |
| `DisassembleShipCommand` | Convert the current docked ship into a station-side `PackagedShip` item after undamaged/unfitted validation (ADR-0034 9B) | `ShipDisassembled` | ✅ implemented |
| `AssembleCommand` | Convert a station-inventory `PackagedShip` item into a new live docked `Ship` owned by the caller; does not change `active_ship` (ADR-0034 9B, ADR-0037) | `ShipAssembled` | ✅ implemented |
| `DisembarkCommand` | Clear the caller's active ship while docked, without disassembling it or changing ownership; session-local, not event-sourced (ADR-0037) | — (no new event) | ✅ implemented |
| `TransferToStationCommand` | Move the entire stack of an item (`Module` or `ScrapMetal`) between a docked ship's own cargo and the caller's inventory at that station, in either direction (`direction: ToStation\|ToShip`); whole-stack only (ADR-0034 9B) | — (no new event; mirrors `BuildPackagedShipCommand`/`DisassembleShipCommand`'s silent station-inventory credit/debit) | ✅ implemented |
| `ActivateModuleCommand` | Turn on an Active Module | `ModuleActivated` | ✅ implemented |
| `DeactivateModuleCommand` | Turn off an Active Module | `ModuleDeactivated` | ✅ implemented |
| `AttackCommand` | Designate an attack target | `WeaponFired` | ⬜ type + WsServer JSON parser only; not wired into combat |
| `StopCommand` | Decelerate to zero velocity using acceleration | — | ✅ implemented |
| `ApproachCommand` | Semi-automatic approach to a target (Ship / Jump Gate); cancelled by Move/Stop (ADR-0015) | — (no new event) | ✅ implemented |
| `TransitCommand` | Request a Sector Transit (via Raft, ADR-0014) | `SectorTransitRequested` / `Completed` | ✅ implemented |
| `JumpCommand` | Move to another Sector via a Jump Gate (via Raft, ADR-0009). In range: proposed directly. Out of range: auto-warps toward the gate first (auto-warp-then-jump, ADR-0023). Too close to warp: auto-approaches instead. The 3-way decision is owned by `SimulationNode::apply_jump_with_fallback` (`dawn-sector::node::jump`), called identically from both `dawn-sector-node` and `dawn-simulation`'s cluster server | `JumpGateUsed` (+ `StarSystemChanged` if star system changes) | ✅ implemented |
| `WarpCommand` | Warp within the same Sector to a Jump Gate or celestial body (star/planet) (`WarpTarget::Gate` / `Body`; align → warping, two phases; ADR-0022 / ADR-0025) | — (no new event; movement recorded via `VelocityChanged`) | ✅ implemented |
| `OrbitCommand` | Orbit a target (Ship / Jump Gate) at a given radius (defaults to weapon range; cancelled by Move/Stop/other helm modes; ADR-0031) | — (no new event; movement via `VelocityChanged`) | ✅ implemented |
| `KeepAtRangeCommand` | Maintain a minimum distance from a target (Ship / Jump Gate) (defaults to weapon range; cancelled by Move/Stop/other helm modes; ADR-0031) | — (no new event; movement via `VelocityChanged`) | ✅ implemented |
| `SelectActiveShipCommand` | Switch the caller's active ship to another owned ship docked at the same station (station-local only; ADR-0037) | — (session-local routing state, not event-sourced; see ownership.md §7) | ✅ implemented |

> **ADR-0037 (2026-07-07):** `MoveCommand`/`StopCommand`/`ApproachCommand`/`WarpCommand`/`OrbitCommand`/`KeepAtRangeCommand`/`JumpCommand`/`LockOnCommand`/`ActivateModuleCommand`/`DeactivateModuleCommand`/`UndockCommand` no longer carry a `ship_id` field — the server resolves them against the caller's active ship (`ShipRegistry.active_ship`), so there is no wire-representable way to name a ship the player isn't flying. `FitModuleCommand`/`UnfitModuleCommand`/`DockCommand`/`BuildPackagedShipCommand`/`DisassembleShipCommand` are unaffected (station inventory-management, any owned ship). See `docs/architecture/ownership.md` §7.

---

### Internal Market bridge commands

The following `dawn-core` commands are generated by `dawn-market` and are not
variants of `ClientCommand`; they are routed by the caller to the Sector that
owns `ship_id`.

| Command | Description | Resulting Event(s) | Status |
|---|---|---|---|
| `RemoveItemCommand` | Remove a listed Ask quantity from the seller's ship cargo | `SimulationNode::remove_item_owned()` -> `ShipFitted` | ✅ implemented in `dawn-market` + `dawn-sector` 9D-4 |
| `ReturnItemCommand` | Return the remaining Ask quantity after cancellation | `SimulationNode::return_item_owned()` -> `ShipFitted` | ✅ implemented in `dawn-market` + `dawn-sector` 9D-4 |
| `CreditItemCommand` | Credit an item quantity to a buyer's ship cargo after settlement | `SimulationNode::credit_item_owned()` -> `ShipFitted` | ✅ implemented in `dawn-market` + `dawn-sector` 9D-4 |

These commands are deliberately one-sided. A trade between players in
different Sectors does not require an Item transfer command that spans both
owners, so it does not enter the Transit/Raft ownership path (ADR-0034 §4).

## 5. Event Field Specs

### `ShipSpawned`

Ship generated within a Sector.

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | unique identifier of the spawned Ship |
| `sector_id` | `SectorId` | ✓ | Sector it spawned into |
| `initial_position` | `AbsolutePosition` | ✓ | authoritative Sector-frame spawn coordinates |
| `ship_type_id` | `ShipTypeId` | ✓ | ship type ID (resolved via the `ShipTypeDefinition` registry) |
| `tick` | `Tick` | ✓ | spawn Tick |

**Invariant:** `ship_id` is globally unique and never reused (INV-004). Including `ship_type_id` lets Replay restore exact base_stats (INV-002).

---

### `VelocityChanged`

Ship velocity changed. `MovementSystem` runs the physics and emits this only when velocity differs from the previous Tick.

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id`  | `ShipId`  | ✓ | Ship whose velocity changed |
| `velocity` | `Velocity` | ✓ | new velocity vector (units/tick) |
| `tick`     | `Tick`    | ✓ | Tick the velocity was finalized |

**Invariant:** only emitted when `velocity` differs from the previous Tick (no-change is not emitted).

**Replay:** apply `VelocityChanged` in order, computing `position += velocity` each Tick. No physics simulation needed — `position += velocity` is pure arithmetic.

**Rationale:** position is derived state and excluded from authoritative events; thrust input is a Command and likewise excluded (ADR-0008).

---

### `ShipDespawned`

Ship permanently removed from the world (manual deletion).

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | removed Ship |
| `tick` | `Tick` | ✓ | removal Tick |

---

### `ShipFitted`

Ship's fitting slots and/or cargo projection changed.

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Ship whose fitting or cargo changed |
| `fitting` | `FittingSnapshot` | ✓ | snapshot of all slots after the change (list of Module IDs) |
| `inventory` | `Vec<ItemId>` | ✓ | snapshot of unfitted inventory after the change (ADR-0032, `#[serde(default)]`) |
| `tick` | `Tick` | ✓ | Tick the fitting or cargo change was finalized |

**Design note:** no `stats` field — Replay recomputes via `apply_fitting()` from `FittingSnapshot` (INV-002). Fit/Unfit always change both fitting and inventory together, so both are carried by this one event type rather than splitting into two (ADR-0032). Market item bridge commands reuse the same full snapshot event when only cargo changes, avoiding a second inventory source of truth and a new wire variant.

---

### `TargetLocked`

`LockSystem`'s countdown completed and lock-on was established.

| Field | Type | Required | Description |
|---|---|---|---|
| `locker_id` | `ShipId` | ✓ | Ship that locked |
| `target_id` | `ShipId` | ✓ | Ship that was locked |
| `tick` | `Tick` | ✓ | Tick lock completed |

**Replay:** update the matching `LockComp` entry to `Locked`.

---

### `LockLost`

Lock lost, e.g. because the target was destroyed or moved out of range.

| Field | Type | Required | Description |
|---|---|---|---|
| `locker_id` | `ShipId` | ✓ | Ship that lost the lock |
| `target_id` | `ShipId` | ✓ | Ship that was the lock target |
| `tick` | `Tick` | ✓ | Tick the lock was lost |

**Replay:** remove the matching entry from `LockComp`.

---

### `WeaponFired`

Weapon fired and hit. A miss (failed hit-chance check) does not emit this event. Damage appears in the same-Tick `DamageTaken`.

| Field | Type | Required | Description |
|---|---|---|---|
| `attacker_id` | `ShipId` | ✓ | firing Ship |
| `target_id` | `ShipId` | ✓ | targeted Ship |
| `damage` | `f32` | ✓ | actual damage dealt (base damage × random multiplier 0.49–1.49, 1% chance of 3.0) |
| `tick` | `Tick` | ✓ | Tick of firing |

**Emission conditions (ADR-0012):**
1. target is `Locked` in `LockComp`
2. the Capacitor cycle started this Tick (included in `fire_triggers`)
3. the hit-chance check passed (`rand() < hit_chance`)

Hit chance = `0.5 ^ ((angular / (tracking × sig))² + (max(0, dist − optimal) / falloff)²)`

**Replay:** does not mutate ECS state (fire log only). `damage` already holds the realized value, so Replay does not re-roll randomness.

---

### `DamageTaken`

Ship took damage and HP changed. HP is consumed Shield → Armor → Hull.

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Ship damaged |
| `damage` | `f32` | ✓ | damage received (pre-application) |
| `current_shield` | `f32` | ✓ | shield remaining after damage |
| `current_armor` | `f32` | ✓ | armor remaining after damage |
| `current_hull` | `f32` | ✓ | hull remaining after damage |
| `tick` | `Tick` | ✓ | Tick of damage |

**Design note:** carrying all three HP layers lets Replay reconstruct `HullComp` exactly (INV-002).

---

### `RepairApplied`

An active Shield Booster / Armor Repairer cycle restored a Ship's current HP. Only Shield or Armor is restored, never beyond max HP.

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Ship repaired |
| `amount` | `f32` | ✓ | actual amount restored (after max-HP clamp) |
| `layer` | `RepairLayer` | ✓ | layer restored (`Shield` / `Armor`) |
| `current_shield` | `f32` | ✓ | shield remaining after repair |
| `current_armor` | `f32` | ✓ | armor remaining after repair |
| `current_hull` | `f32` | ✓ | hull remaining after repair |
| `tick` | `Tick` | ✓ | Tick repair was applied |

**Design note:** modeled as its own event rather than a negative `DamageTaken`, since its client presentation and log meaning differ from damage (ADR-0033).

---

### `ModuleActivated`

Active Module turned on.

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id`   | `ShipId`  | ✓ | Ship performing the action |
| `module_id` | `ModuleId` | ✓ | target Module |
| `slot`      | `SlotKind` | ✓ | fitting slot type |
| `target_ship_id` | `Option<ShipId>` |  | target of a targeted module (Weapon/Tackle); `None` for self-only kinds (ADR-0035) |
| `tick`      | `Tick`    | ✓ | Tick of activation |

**Design note:** represents the fact "turned on", not the state `is_active: true`. Replay sets `FittedSlot.is_active = true` and re-runs `apply_fitting()`.

---

### `ModuleDeactivated`

Active Module turned off.

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id`   | `ShipId`  | ✓ | Ship performing the action |
| `module_id` | `ModuleId` | ✓ | target Module |
| `slot`      | `SlotKind` | ✓ | fitting slot type |
| `forced_reason` | `Option<ModuleDeactivationReason>` |  | `None` for a player-issued OFF; `CapacitorExhausted` (CapacitorSystem) or `OutOfRange` (Range Gate System) for a system-forced OFF (ADR-0035) |
| `tick`      | `Tick`    | ✓ | Tick of deactivation |

**Design note:** counterpart of `ModuleActivated`. Replay sets `FittedSlot.is_active = false` and clears `target_ship_id`. The wire protocol maps `forced_reason` to `reason: "cap" | "range"` (omitted for `None`) so the client labels CAP!/RANGE! from the authoritative reason.

---

### `ShipDestroyed`

Ship reached zero HP in combat and was destroyed.

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | destroyed Ship |
| `killer_id` | `ShipId` | ✓ | Ship that landed the final blow |
| `tick` | `Tick` | ✓ | Tick of destruction |

**Replay:** remove the matching Entity from ECS and from `ship_index`.

**Current downstream effect:** the `SimulationNode` tick pipeline immediately
credits the killer ship's inventory with `ItemId::ScrapMetal` (ADR-0034 MVP:
currently a fixed `1` per kill, no Wreck entity).

---

### `SectorTransitRequested`

A durable transfer request. On the source, ownership stays with `from`; the Ship remains as a frozen recovery copy until Ack. The freeze covers steering/warp/movement, capacitor and module cycles, lock admission, combat, and repair. On the destination, the same event shape is also used as an incoming transfer identity marker before materialization.

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Ship transiting |
| `from` | `SectorId` | ✓ | source Sector |
| `to` | `SectorId` | ✓ | destination Sector |
| `tick` | `Tick` | ✓ | source request Tick; part of transfer identity |

**Replay:** if the Ship exists on this Sector, set `TransitComp` to `InTransit { to }`. An incoming destination marker may replay before the Ship exists and is intentionally a state no-op; it remains useful for duplicate-Commit detection.

---

### `SectorTransitCompleted`

Self-contained completion event. `ship_state` carries the destination replay state without depending on the in-memory Raft actor surviving restart.

The destination appends this event when Commit materialization succeeds, then proposes Ack. The source appends it only after Ack, when it removes the frozen recovery copy. Thus a crash can temporarily retain two ECS copies, but never zero durable copies; only the destination copy is active after Commit.

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Ship that transited |
| `from` | `SectorId` | ✓ | previous active Sector |
| `to` | `SectorId` | ✓ | new active Sector |
| `entry_pos` | `AbsolutePosition` | ✓ | authoritative entry coordinates in the destination Sector frame |
| `velocity` | `Velocity` | ✓ | velocity on entry |
| `tick` | `Tick` | ✓ | local completion Tick |
| `ship_state` | `TransitShipState` | ✓ | type / HP / capacitor / fitting / inventory used by destination Replay |

**Replay:** on `from`, remove the Ship. On `to`, rebuild a `ShipSnapshot` from `ship_state` + `entry_pos`, materialize it, and redo anchor rebase directly. The live `AnchorRebased` event precedes Completed and may replay before the destination Ship exists.

---

### `SectorTransitAborted`

A committed Transit was aborted; ownership stays with `from`. Validation-stage rejection is expressed via `CommandRejected`, not this event (INV-006).

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Ship whose Transit was aborted |
| `from` | `SectorId` | ✓ | owning Sector (unchanged) |
| `to` | `SectorId` | ✓ | aborted destination Sector |
| `tick` | `Tick` | ✓ | Tick the abort was finalized |

**Replay:** clears the `InTransit` marker `SectorTransitRequested` Replay set (`SimulationNode::replay_sector_transit_aborted`).

**Status:** type + Replay implemented; nothing appends this event yet.

---

### `JumpGateUsed`

Ship passed through a Jump Gate to another Sector (ADR-0009). Does not replace `SectorTransitCompleted` — it's an additional record of *how* the move happened, appended in the same Tick.

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Ship that used the gate |
| `gate_id` | `JumpGateId` | ✓ | Jump Gate used |
| `from_sector` | `SectorId` | ✓ | originating Sector |
| `to_sector` | `SectorId` | ✓ | destination Sector |
| `entry_pos` | `AbsolutePosition` | ✓ | authoritative spawn coordinates in the destination Sector frame |
| `tick` | `Tick` | ✓ | Tick the gate transit was finalized |

**Status:** ✅ implemented (Step 7.5 `append_jump_events`).

---

### `StarSystemChanged`

Ship moved to a different star system (ADR-0009). Emitted in the same Tick as `JumpGateUsed`, only when the destination Sector belongs to a different `StarSystemId`.

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Ship that moved |
| `from_system` | `StarSystemId` | ✓ | originating star system |
| `to_system` | `StarSystemId` | ✓ | destination star system |
| `tick` | `Tick` | ✓ | Tick the move was finalized |

**Status:** ✅ implemented (Step 7.5 `append_jump_events`).

---

### `TackleApplied`

A Fold Disruptor Module was activated on a target Ship (in range + locked). The tackled Ship is barred from Warp/Jump (ADR-0024).

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Ship being tackled |
| `by` | `ShipId` | ✓ | tackling Ship (tackler) |
| `tick` | `Tick` | ✓ | Tick tackle was activated |

**Replay:** add `by` to `ship_id`'s `TackledComp.tacklers`.

---

### `TackleReleased`

Counterpart to `TackleApplied`. The tackle effect ended (Module off / out of range / tackler destroyed / lock lost). If other tacklers remain, the Ship stays tackled.

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Ship released from (or pending release from) tackle |
| `by` | `ShipId` | ✓ | Ship whose tackle ended |
| `tick` | `Tick` | ✓ | Tick of release |

**Replay:** remove `by` from `ship_id`'s `TackledComp.tacklers`; remove `TackledComp` entirely if it becomes empty.

---

## 6. Upcaster Catalog

Record breaking changes here only when they occur.

Breaking changes to date: **none**

### Upcaster procedure (for future reference)

```
1. Mark the old event as Deprecated (do not delete it)
2. Define the new event under a new name (V2)
3. Implement impl Upcaster for OldEvent { fn upcast(self) -> NewEvent }
4. Route Replay through the Upcaster
5. Record the change in this catalog
6. Create a new ADR
```
