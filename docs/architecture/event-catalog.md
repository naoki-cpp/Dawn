---
scope    : Complete spec of every public DomainEvent and Command that exists. The single source of truth for public/domain facts and requests
audience : AI Agent / Human Developer
update   : Must be updated whenever an Event or Command is added or changed
related  : entity-model.md, tick-model.md, recovery-contract.md, event-schema-evolution.md
---

# Event Catalog

> **ADR-0049 recovery note (2026-08-07):** This catalog remains the detailed
> specification for public/business `DomainEvent`s and Commands. It is **not** the
> complete exact-state recovery schema. Exact operational Sector recovery is a
> compatible versioned checkpoint plus committed authoritative `RecoveryDelta`
> tail. Eventless Ticks and commands such as active-ship routing changes may have a
> durable recovery transition with no public event. `Replay` notes below describe
> the event's supported public/projection/legacy replay behavior; they do not imply
> that public-event history alone reconstructs every authoritative field. Transit
> EventStore-scan retry descriptions are historical behavior superseded by #276's
> durable Saga under the same ADR-0049 recovery contract.

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
| Persisted | no | yes, append-only public fact |
| File | `commands.rs` | `events.rs` |

Commands and Events must never share a type or enum (INV-006).

### Common field

Every event has `tick: Tick`. An event omitting `tick` is rejected as an INV-005 violation.

### Optional field policy

- Fields defined initially are always required (never `Option`)
- Fields added later are always `Option<T>`
- Never define a field as `Option` from the start (to avoid unintentional omission)

### Persistence model (ADR-0017 amended by ADR-0049)

Events in this catalog are persisted **append-only** public/business facts (INV-001 / FBD-001). Public history retains the ADR-0017 hot/cold archival model, but that archival tail is no longer the exact Sector recovery tail:

```
Public Event history:
  Hot log      : recent immutable segments for publication/projection/audit catch-up
  Cold archive : retained append-only history for audit / causal analysis

Exact operational recovery:
  newest complete compatible checkpoint
    + committed authoritative RecoveryDelta tail
```

- Committed public events are never updated/re-written in place. FBD-001 protects this public history.
- A public EventStore append **alone is not the complete authoritative commit boundary**. ADR-0049's logical durable transition includes the exact recovery outcome and its public facts/reliable obligations with one atomic visibility boundary; #271 owns physical framing.
- Position, capacitor, lock countdowns, module cycles, flight/routing state, authoritative queues, and other exact final values are checkpoint/RecoveryDelta authority even when no event is emitted.
- State-delta checkpoint compaction and public-event/outbox retention may use different watermarks. A state checkpoint does not prove a public output was delivered or archived.
- Each event's **Replay** note remains useful for its supported public projection/audit/legacy replay behavior; it is not a universal exact-recovery guarantee.

See [ADR-0017](../adr/ADR-0017-snapshot-compaction.md), [ADR-0049](../adr/ADR-0049-sector-recovery-state-delta-wal.md), and [recovery-contract.md](./recovery-contract.md).

---

## 3. Event List

### 3.1 Ship Lifecycle

| Event | Description | Emitter | Status |
|---|---|---|---|
| `ShipSpawned` | Ship appeared in the world | `SimulationNode::spawn_ship()` | ✅ implemented |
| `ClientAdmissionIdentityReserved` | Fresh admission durably consumed a `PlayerId`/`ShipId` pair without materializing a Ship; Replay advances allocation watermarks only | `SimulationNode::reserve_fresh_admission_identity()` | ✅ implemented |
| `ClientAdmissionCommitted` | Atomic fresh-admission starter public fact: Ship, fitting/cargo snapshot, ownership identity, and idempotent Station grant description | `SimulationNode::commit_reserved_fresh_admission()` | ✅ implemented |
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
| `SectorTransitRequested` | Durable public transfer-request fact carrying source-local request identity plus the resolved Gate/non-Gate route; current implementation also uses it while retaining a frozen source recovery copy until Ack | source `propose_transit()`; destination also records an incoming marker using its own local `tick` | ✅ implemented |
| `SectorTransitCompleted` | Public completion fact for destination import or source cleanup after Ack; carries the current source-local `request_tick` attempt identity | destination `handle_transit_commit()`; source `complete_outgoing_transit()` | ✅ implemented |
| `SectorTransitAborted` | Transit aborted (ownership stays with `from`) | destination failure path (not wired) | type + Replay |

Validation-stage rejection is expressed via `CommandRejected`, not an event (INV-006); there is no `SectorTransitRejected` event. `propose_transit` returns `Err` without emitting an event if the Ship is absent or already in Transit.

The corresponding command is `TransitCommand { ship_id, to }`. Raft carries `TransitOp::Request`, `Commit`, and `Ack`: Request freezes the source, allocates a `TransitAttemptId`, and persists the canonical `OutgoingTransitAttempt` with `gate_id`, `entry_pos: AbsolutePosition`, and the complete handoff. Commit and Ack carry that same attempt ID. Commit materializes the destination by deriving its anchor and local offset from the absolute entry point through the same seam used by replay, then records an `IncomingTransitReceipt`; duplicate Commit is answered from that receipt without rematerializing. Ack removes the source recovery copy only when the keyed outgoing Saga attempt is still in transit. Retries read the checkpointed Saga directly with bounded exponential backoff, so public-event retention and checkpoint compaction are independent of Transit retry state.

> **ADR-0049 / #276:** the `TransitAttemptId` Saga is the exact recovery authority. It is checkpointed in `TransitSagaSnapshot` and carried by recovery deltas; public Transit events remain business facts and replay/projection inputs, but are not scanned to reconstruct outgoing retries or incoming receipts.
>
> Retry proposals use bounded exponential backoff and quarantine an attempt after
> the configured retry limit. `SimulationNode::transit_saga_diagnostics()` exposes
> structured counts for active, retrying, terminal, quarantined, and incoming
> receipt records without creating a second source of truth.

### 3.7 Jump Gate Navigation (ADR-0009, complete)

| Event | Description | Emitter | Status |
|---|---|---|---|
| `JumpGateUsed` | Ship moved to another Sector via a Jump Gate | `SimulationNode::append_jump_events` (Step 7.5, destination node in the current pipeline) | ✅ implemented (Raft pipeline) |
| `StarSystemChanged` | Ship moved to a different star system (concurrent with `JumpGateUsed`) | `SimulationNode::append_jump_events` (current destination path) | ✅ implemented (Raft pipeline) |

Corresponding Command: `JumpCommand { gate_id }`, committed over the same Raft Log path as `TransitCommand` (ADR-0014). The server resolves the command against the caller's active ship (ADR-0037). `TransitOp::Request`/`Commit` carries `gate_id: Option<JumpGateId>`; after destination Commit, the destination appends `JumpGateUsed` alongside `SectorTransitCompleted`, appends `StarSystemChanged` if needed, and then proposes Ack.

Static topology (3 star systems, 4 jump gates) is defined in `dawn-sector/src/galaxy.rs` (ADR-0026). The AoI delivery layer projects visible committed events through `dawn_wire::project_domain_event` into `ServerFact` before the postcard envelope (ADR-0042/#274), and the single `ClientRequest` admission seam handles `Jump`. The Godot client (`connection.gd`'s `send_jump_command`, `main.gd`'s `_handle_jump_gate_used` / `_handle_star_system_changed`) is also implemented (ADR-0009 checklist fully complete).

### 3.8 Tackle (ADR-0024)

| Event | Description | Emitter | Status |
|---|---|---|---|
| `TackleApplied` | Fold Disruptor activated on a target (in range + locked) | `SimulationNode::process_tackle()` (Step 4.5) | ✅ implemented |
| `TackleReleased` | Tackle effect ended (Module off / out of range / tackler destroyed) | `SimulationNode::process_tackle()` (Step 4.5) | ✅ implemented |

While tackled, `can_propose_warp()` / `can_propose_jump()` return false, blocking Warp/Jump. The current snapshot persists `TackledComp`; under ADR-0049 its exact value is also part of checkpoint/RecoveryDelta authority.

`TackleReleased` is never emitted without a matching prior `TackleApplied` (strict 1:1 pairing). With multiple simultaneous tacklers, each tackler gets its own pair.

### 3.9 Coordinate Anchoring (ADR-0029)

| Event | Description | Emitter | Status |
|---|---|---|---|
| `AnchorRebased` | Ship's coordinate anchor changed (absolute position unchanged; only the `(anchor, offset)` representation updates — e.g. star anchor → destination-body anchor on Warp arrival) | `SimulationNode` (Warp arrival, ADR-0029 step 4) | ✅ implemented (emitted from `warp_step`/`rebase_arrival_event`, appended via `tick.rs`'s `all_events`) |
| `ShipDocked` | Ship docked at an NPC station (ADR-0034 9B docking foundation) | `SimulationNode::dock_owned` | ✅ implemented |
| `ShipUndocked` | Ship undocked from an NPC station (ADR-0034 9B docking foundation) | `SimulationNode::undock_owned` | ✅ implemented |
| `PackagedShipBuilt` | Scrap Metal consumed in a docked station and converted into a packaged ship (ADR-0034 9B) | `SimulationNode::build_packaged_ship_owned` | ✅ implemented |
| `ShipDisassembled` | Docked undamaged unfitted ship converted into a packaged ship in station inventory (ADR-0034 9B) | `SimulationNode::disassemble_ship_owned` | ✅ implemented |
| `ShipAssembled` | A station-inventory `PackagedShip` item converted into a new live docked `Ship`, owned by the caller (ADR-0034 9B, ADR-0037). Fields: `ship_id` (freshly allocated), `player_id`, `station_id`, `ship_type_id`, `tick`. Public/legacy Replay allocates the ECS entity with `e.ship_id` directly, unfitted and docked, without selecting it active | `SimulationNode::assemble_ship_owned` | ✅ implemented |

`AnchorRebased` is a durable public representation-change fact. It stores anchor/post-rebase offset so public replay/projections can reproduce the representation. Exact authoritative position/anchor state is additionally covered by ADR-0049 RecoveryDelta/checkpoints. A rebase is a non-velocity-driven frame change, so INV-MOVE's public movement-event rule does not prohibit this fact.

Station inventory exact authority is likewise the ADR-0049 Station aggregate recovery delta, not public-event replay or SQLite alone (ADR-0038 as amended).

### 3.11 System (reserved for future use)

| Event | Description | Status |
|---|---|---|
| `TickStarted` | Tick started | not implemented |
| `TickCompleted` | Tick completed | not implemented |

Neither public event is required for exact recovery: ADR-0049 gives every committed Tick an authoritative recovery record even when no public Tick event exists.

### 3.12 AoI (Area of Interest) Delivery Filter (ADR-0019)

AoI introduces **no new domain events**. It's implemented by filtering `DomainEvent` delivery through each observer's 27-cell neighborhood.

| Message | Description | Status |
|---|---|---|
| `AoiEnter` | Ship entered an observer's AoI (WebSocket delivery message, not a domain event) | ✅ implemented (8C, ADR-0019) |
| `AoiLeave` | Ship left an observer's AoI (same) | ✅ implemented (8C, ADR-0019) |

`AoiEnter` / `AoiLeave` are not appended as `DomainEvent`s — they are delivery-control messages. They do not affect exact recovery; AoI consistency is rebuilt from current authoritative state plus client delivery/projection logic.

---

## 4. Command List

Commands are defined in `dawn-core/src/commands.rs`. Clients send them to the server as the single typed `ClientRequest` enum.

| Command | Description | Resulting Event(s) | Status |
|---|---|---|---|
| `MoveCommand` | Specify thrust direction | — (no public event required; successful authoritative change is RecoveryDelta) | ✅ implemented |
| `LockOnCommand` | Request lock-on | `TargetLocked` | ✅ implemented |
| `FitModuleCommand` | Fit an inventory Module into a slot; requires the ship to be docked (ADR-0032, docked requirement added 2026-07-08) | `ShipFitted` | ✅ implemented |
| `UnfitModuleCommand` | Return a fitted Module to inventory; requires docked (ADR-0032) | `ShipFitted` | ✅ implemented |
| `ReorderFittedModuleCommand` | Reorder two fitted modules within the same slot kind (persisted -- iteration order assigns weapon hotkeys); requires docked (ADR-0032) | `ShipFitted` | ✅ implemented |
| `DockCommand` | Dock at an NPC station once within its docking radius (ADR-0034 9B) | `ShipDocked` | ✅ implemented |
| `UndockCommand` | Leave a previously-docked NPC station (ADR-0034 9B) | `ShipUndocked` | ✅ implemented |
| `BuildPackagedShipCommand` | Consume Scrap Metal in the current docked station and create a `PackagedShip` item there (ADR-0034 9B) | `PackagedShipBuilt` | ✅ implemented |
| `DisassembleShipCommand` | Convert the current docked ship into a station-side `PackagedShip` item after undamaged/unfitted validation (ADR-0034 9B) | `ShipDisassembled` | ✅ implemented |
| `AssembleCommand` | Convert a station-inventory `PackagedShip` item into a new live docked `Ship` owned by the caller; does not change `active_ship` (ADR-0034 9B, ADR-0037) | `ShipAssembled` | ✅ implemented |
| `DisembarkCommand` | Clear the caller's active ship while docked, without disassembling it or changing ownership; ADR-0049 classifies the routing change as authoritative even though it has no public event | — (RecoveryDelta routing transition, no `DomainEvent`) | ✅ implemented |
| `TransferCargo` | Move the entire stack of an item (`Module` or `ScrapMetal`) between a docked ship's own cargo and the caller's inventory at that station, in either direction (`direction: ToStation\|ToShip`); whole-stack only (ADR-0034 9B) | — public event may be empty; authoritative cargo/Station delta is still durable | ✅ implemented |
| `ActivateModuleCommand` | Turn on an Active Module | `ModuleActivated` | ✅ implemented |
| `DeactivateModuleCommand` | Turn off an Active Module | `ModuleDeactivated` | ✅ implemented |
| `AttackCommand` | Designate an attack target | `WeaponFired` | ⬜ type + WsServer JSON parser only; not wired into combat |
| `StopCommand` | Decelerate to zero velocity using acceleration | — (no public event required; RecoveryDelta records authority) | ✅ implemented |
| `ApproachCommand` | Semi-automatic approach to a target (Ship / Jump Gate); cancelled by Move/Stop (ADR-0015) | — (no public event required) | ✅ implemented |
| `TransitCommand` | Request a Sector Transit (via Raft, ADR-0014) | `SectorTransitRequested` / `Completed` | ✅ implemented; persistence model migrating under #276 |
| `JumpCommand` | Move to another Sector via a Jump Gate (via Raft, ADR-0009). In range: proposed directly. Out of range: auto-warps toward the gate first (auto-warp-then-jump, ADR-0023). Too close to warp: auto-approaches instead. The 3-way decision is owned by `SimulationNode::apply_jump_with_fallback` (`dawn-sector::node::jump`), called identically from both `dawn-sector-node` and `dawn-simulation`'s cluster server | `JumpGateUsed` (+ `StarSystemChanged` if star system changes) | ✅ implemented |
| `WarpCommand` | Warp within the same Sector to a Jump Gate or celestial body (star/planet) (`WarpTarget::Gate` / `Body`; align → warping, two phases; ADR-0022 / ADR-0025) | — / `VelocityChanged` as applicable; exact Warp state is RecoveryDelta | ✅ implemented |
| `OrbitCommand` | Orbit a target (Ship / Jump Gate) at a given radius (defaults to weapon range; cancelled by Move/Stop/other helm modes; ADR-0031) | — (no public event required; movement facts may appear separately) | ✅ implemented |
| `KeepAtRangeCommand` | Maintain a minimum distance from a target (Ship / Jump Gate) (defaults to weapon range; cancelled by Move/Stop/other helm modes; ADR-0031) | — (no public event required; movement facts may appear separately) | ✅ implemented |
| `SelectActiveShipCommand` | Switch the caller's active ship to another owned ship docked at the same station. ADR-0049 reclassifies this as authoritative Player routing state because it changes the target of later commands | — (RecoveryDelta routing transition, no `DomainEvent`) | ✅ implemented |

> **ADR-0037 (2026-07-07):** `MoveCommand`/`StopCommand`/`ApproachCommand`/`WarpCommand`/`OrbitCommand`/`KeepAtRangeCommand`/`JumpCommand`/`LockOnCommand`/`ActivateModuleCommand`/`DeactivateModuleCommand`/`UndockCommand` no longer carry a `ship_id` field — the server resolves them against the caller's active ship (`PlayerState.active_ship`), so there is no wire-representable way to name a ship the player isn't flying. `FitModuleCommand`/`UnfitModuleCommand`/`DockCommand`/`BuildPackagedShipCommand`/`DisassembleShipCommand` are unaffected (station inventory-management, any owned ship). See `docs/architecture/ownership.md` §7.
>
> **ADR-0049 amendment:** the absence of a public event does not make `active_ship` intentionally lossy. `SelectActiveShip`/`Disembark` successful routing changes belong to PlayerState checkpoint/RecoveryDelta recovery under #284/#275.

---

### Internal Market bridge commands

The following `dawn-core` commands are generated by `dawn-market` and are not
variants of `ClientRequest`; they are routed by the caller to the Sector that
owns `ship_id`.

| Command | Description | Resulting Event(s) | Status |
|---|---|---|---|
| `RemoveItemCommand` | Remove a listed Ask quantity from the seller's ship cargo | `SimulationNode::remove_item_owned()` -> `ShipFitted` | ✅ implemented in `dawn-market` + `dawn-sector` 9D-4 |
| `ReturnItemCommand` | Return the remaining Ask quantity after cancellation | `SimulationNode::return_item_owned()` -> `ShipFitted` | ✅ implemented in `dawn-market` + `dawn-sector` 9D-4 |
| `CreditItemCommand` | Credit an item quantity to a buyer's ship cargo after settlement | `SimulationNode::credit_item_owned()` -> `ShipFitted` | ✅ implemented in `dawn-market` + `dawn-sector` 9D-4 |

These commands are deliberately one-sided. A trade between players in
different Sectors does not require an Item transfer command that spans both
owners, so it does not enter the Transit/Raft ownership path (ADR-0034 §4).
The corresponding exact cargo mutation is still represented by the Sector recovery transition; the `ShipFitted` event remains its public fact/projection output.

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

**Invariant:** `ship_id` is globally unique and never reused (INV-004). Including `ship_type_id` lets public/legacy Replay rebuild the base-stats projection. Exact committed entity/type/state is RecoveryDelta authority.

---

### `VelocityChanged`

Ship velocity changed. `MovementSystem` runs the physics and emits this only when velocity differs from the previous Tick.

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id`  | `ShipId`  | ✓ | Ship whose velocity changed |
| `velocity` | `Velocity` | ✓ | new velocity vector (units/tick) |
| `tick`     | `Tick`    | ✓ | Tick the velocity was finalized |

**Invariant:** only emitted when `velocity` differs from the previous Tick (no-change is not emitted).

**Public/legacy Replay:** supported motion projections may apply `VelocityChanged` in order and integrate `position += velocity` according to their contract. This remains useful for public motion history/client projection.

**ADR-0049 exact recovery note:** position is not merely a throwaway derived value. Exact final position/velocity/anchor/flight state at each committed recovery position is covered by RecoveryDelta/checkpoint, including Ticks with no `VelocityChanged`. Historical Tick rerun or velocity-event integration is not the exact operational recovery authority.

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

**Design note:** no `stats` field — public Replay can recompute fitting-derived stats from `FittingSnapshot`. Fit/Unfit always change both fitting and inventory together, so both are carried by this one event type rather than splitting into two (ADR-0032). Market item bridge commands reuse the same full snapshot event when only cargo changes, avoiding another public cargo event shape. Exact fitting/cargo authority is also represented by RecoveryDelta.

---

### `TargetLocked`

`LockSystem`'s countdown completed and lock-on was established.

| Field | Type | Required | Description |
|---|---|---|---|
| `locker_id` | `ShipId` | ✓ | Ship that locked |
| `target_id` | `ShipId` | ✓ | Ship that was locked |
| `tick` | `Tick` | ✓ | Tick lock completed |

**Public Replay:** update the matching lock projection to `Locked`. Exact lock entries/countdowns are checkpoint/RecoveryDelta authority.

---

### `LockLost`

Lock lost, e.g. because the target was destroyed or moved out of range.

| Field | Type | Required | Description |
|---|---|---|---|
| `locker_id` | `ShipId` | ✓ | Ship that lost the lock |
| `target_id` | `ShipId` | ✓ | Ship that was the lock target |
| `tick` | `Tick` | ✓ | Tick the lock was lost |

**Public Replay:** remove the matching lock projection entry. Exact lock state remains RecoveryDelta authority.

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

**Replay:** does not mutate the fire-result state beyond its supported log/projection semantics. `damage` already holds the realized value, so public Replay does not re-roll randomness. Exact combat outcome/Hull state is RecoveryDelta authority.

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

**Design note:** carrying all three HP layers makes the public event/projection self-contained. Exact `HullComp` recovery is also recorded by RecoveryDelta, so event completeness is not the recovery invariant.

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

**Design note:** represents the fact "turned on", not merely the implementation field `is_active: true`. Public Replay can set the fitting projection active and re-run fitting-derived calculations. Exact active/cycle/target state is RecoveryDelta authority.

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

**Design note:** counterpart of `ModuleActivated`. Public Replay can set the projection inactive and clear `target_ship_id`. The wire protocol maps `forced_reason` to `reason: "cap" | "range"` (omitted for `None`) so the client labels CAP!/RANGE! from the public authoritative reason. Exact fitting/cycle state is RecoveryDelta authority.

---

### `ShipDestroyed`

Ship reached zero HP in combat and was destroyed.

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | destroyed Ship |
| `killer_id` | `ShipId` | ✓ | Ship that landed the final blow |
| `tick` | `Tick` | ✓ | Tick of destruction |

**Public Replay:** remove the matching Ship projection/entity where that projection supports it. Exact existence/index/ownership deletion is RecoveryDelta authority.

**Current downstream effect:** the `SimulationNode` tick pipeline immediately
credits the killer ship's inventory with `ItemId::ScrapMetal` (ADR-0034 MVP:
currently a fixed `1` per kill, no Wreck entity). That reward mutation must be present in the same authoritative RecoveryDelta even though it is not fully represented by `ShipDestroyed` alone.

---

### `SectorTransitRequested`

A durable public transfer request. On the source, current behavior keeps ownership with `from` and retains a frozen recovery copy until Ack. The freeze covers steering/warp/movement, capacitor and module cycles, lock admission, combat, and repair. On the destination, the same event shape is also used by current code as an incoming transfer identity marker before materialization.

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Ship transiting |
| `from` | `SectorId` | ✓ | source Sector |
| `to` | `SectorId` | ✓ | destination Sector |
| `tick` | `Tick` | ✓ | source request Tick; part of the current attempt identity |

**Public/legacy Replay:** if the Ship exists on this Sector, set the transit projection to `InTransit { to }`. An incoming destination marker may replay before the Ship exists and intentionally be a state no-op useful for current duplicate-Commit handling.

**Migration:** exact Transit recovery/retry authority is moving to ADR-0049 RecoveryDelta plus #276's durable Saga; this public event does not have to remain the durable attempt repository.

---

### `SectorTransitCompleted`

Self-contained public completion event. `handoff` is the same canonical
`TransitHandoffState` carried by the current Raft Commit, so legacy/public replay
does not depend on an in-memory Raft actor or persistence `ShipSnapshot` crossing
the protocol boundary.

The current destination appends this event when Commit materialization succeeds, then proposes a minimal identity-only Ack. The source appends it only after Ack, when it removes the frozen recovery copy. Thus current behavior can temporarily retain two ECS copies while only the destination copy is active after Commit.

| Field | Type | Required | Description |
|---|---|---|---|
| `handoff` | `TransitHandoffState` | ✓ | Ship identity, durable owner identity when player-owned, type, velocity, HP, capacitor, fitting, and inventory |
| `from` | `SectorId` | ✓ | previous active Sector |
| `to` | `SectorId` | ✓ | new active Sector |
| `request_tick` | `Tick` | ✓ | current source-local attempt identity |
| `entry_pos` | `AbsolutePosition` | ✓ | authoritative entry coordinates in the destination Sector frame |
| `tick` | `Tick` | ✓ | local completion Tick |

**Public/legacy Replay:** on `from`, remove `handoff.ship_id`; on `to`, feed `handoff` through the current destination materialization seam and derive anchor/offset from `entry_pos`. The current live `AnchorRebased` fact precedes Completed where emitted. For player-owned Ships, current replay can also restore the Ship-to-Player public/domain projection.

**ADR-0049/#276:** exact owner/routing/state recovery and attempt/receipt retry semantics are RecoveryDelta/Saga authority. Transit operations use the opaque `TransitAttemptId`; the pre-release event `request_tick` shape is not a persistence compatibility contract.

---

### `SectorTransitAborted`

A committed Transit was aborted; ownership stays with `from`. Validation-stage rejection is expressed via `CommandRejected`, not this event (INV-006).

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Ship whose Transit was aborted |
| `from` | `SectorId` | ✓ | owning Sector (unchanged) |
| `to` | `SectorId` | ✓ | aborted destination Sector |
| `tick` | `Tick` | ✓ | Tick the abort was finalized |

**Public/legacy Replay:** clears the matching `InTransit` projection marker where supported (`SimulationNode::replay_sector_transit_aborted` in current code).

**Status:** type + Replay implemented; nothing appends this event yet.

---

### `JumpGateUsed`

Ship passed through a Jump Gate to another Sector (ADR-0009). Does not replace `SectorTransitCompleted` — it's an additional public record of *how* the move happened, appended in the same Tick by current code.

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Ship that used the gate |
| `gate_id` | `JumpGateId` | ✓ | Jump Gate used |
| `from_sector` | `SectorId` | ✓ | originating Sector |
| `to_sector` | `SectorId` | ✓ | destination Sector |
| `entry_pos` | `AbsolutePosition` | ✓ | authoritative spawn coordinates in the destination Sector frame |
| `tick` | `Tick` | ✓ | Tick the gate transit was finalized |

**Status:** ✅ implemented (current `append_jump_events` path).

---

### `StarSystemChanged`

Ship moved to a different star system (ADR-0009). Emitted in the same Tick as `JumpGateUsed`, only when the destination Sector belongs to a different `StarSystemId`.

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Ship that moved |
| `from_system` | `StarSystemId` | ✓ | originating star system |
| `to_system` | `StarSystemId` | ✓ | destination star system |
| `tick` | `Tick` | ✓ | Tick the move was finalized |

**Status:** ✅ implemented (current `append_jump_events` path).

---

### `TackleApplied`

A Fold Disruptor Module was activated on a target Ship (in range + locked). The tackled Ship is barred from Warp/Jump (ADR-0024).

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Ship being tackled |
| `by` | `ShipId` | ✓ | tackling Ship (tackler) |
| `tick` | `Tick` | ✓ | Tick tackle was activated |

**Public Replay:** add `by` to the tackle projection. Exact `TackledComp` membership is RecoveryDelta authority.

---

### `TackleReleased`

Counterpart to `TackleApplied`. The tackle effect ended (Module off / out of range / tackler destroyed / lock lost). If other tacklers remain, the Ship stays tackled.

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Ship released from (or pending release from) tackle |
| `by` | `ShipId` | ✓ | Ship whose tackle ended |
| `tick` | `Tick` | ✓ | Tick of release |

**Public Replay:** remove `by` from the tackle projection; remove the projection entry entirely if it becomes empty. Exact tackle state is RecoveryDelta authority.

---

## 6. Upcaster Catalog

Record breaking changes here only when they occur.

Breaking changes to date: **none**

### Upcaster procedure (for future reference)

```
1. Mark the old event as Deprecated (do not delete it)
2. Define the new event under a new name (V2)
3. Implement impl Upcaster for OldEvent { fn upcast(self) -> NewEvent }
4. Route public-event Replay through the Upcaster
5. Record the change in this catalog
6. Create a new ADR
```
