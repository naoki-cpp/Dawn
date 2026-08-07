---
scope    : Complete spec of every Event and Command that exists. The single source of truth for public/domain facts and requests
audience : AI Agent / Human Developer
update   : Must be updated whenever an Event or Command is added or changed
related  : entity-model.md, tick-model.md, recovery-contract.md, event-schema-evolution.md
---

# Event Catalog

> **ADR-0049 recovery note (2026-08-07):** This catalog specifies public/business
> `DomainEvent`s, not the complete exact-state recovery representation. Every
> `Replay` note below describes public-event replay/projection behavior for that
> event. Exact operational Sector recovery uses a compatible checkpoint plus the
> committed authoritative `RecoveryDelta` tail. A Tick may have no `DomainEvent`
> and still have a durable recovery transition. Reliable post-commit obligations
> live in the durable outbox, not necessarily as domain events.

## 1. Using This Catalog

### Sync rule

This catalog must always match the definitions in `dawn-core/src/events.rs` / `dawn-core/src/commands.rs`. Update both code and catalog in the same PR.

### Procedure for adding an Event

```text
1. Add the new event to this catalog
2. Add the type to dawn-core/src/events.rs
3. If a corresponding Command is needed, add it to dawn-core/src/commands.rs
4. Write a unit test in events.rs
5. List the changed events in the PR description
```

### Backward compatibility rules

**Pre-release (current):** No external user event logs exist yet, so breaking changes (removing fields, changing types, removing events) are allowed directly.

**Post-release:**
```text
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

### Persistence model (ADR-0017 + ADR-0049)

Events in this catalog are persisted **append-only** public/business facts (INV-001 / FBD-001). Their hot/cold retention does not change event immutability or catalog semantics:

```text
Hot public-event log : recent immutable segments, retained by consumer/archive policy
Cold archive         : long-term append-only audit / causal history
```

The exact state recovery stream is separate:

```text
Operational recovery = newest complete compatible checkpoint
                     + committed authoritative RecoveryDelta tail
```

- The logical `DurableTransitionBatch` atomically commits its authoritative state delta, ordered public events (possibly empty), and reliable outbox intents.
- A public EventStore append **alone is not the complete commit boundary**. The ADR-0049 transition envelope is the durability boundary.
- Position, capacitor, lock countdowns, thrust/flight intent, module cycles, authoritative queues, and other exact final values are represented by RecoveryDelta/checkpoints even when no event is emitted.
- State-delta compaction behind a validated checkpoint is independent of public-event/outbox retention. An undelivered event or outbox intent cannot be deleted merely because state is checkpointed.
- Every `Replay` note in this catalog means how the public fact can rebuild/drive its supported projection or audit model. It does not promise exact arbitrary-Tick Sector reconstruction.

See [ADR-0049](../adr/ADR-0049-sector-recovery-state-delta-wal.md), [recovery-contract.md](./recovery-contract.md), and [ADR-0017](../adr/ADR-0017-snapshot-compaction.md).

---

## 3. Event List

### 3.1 Ship Lifecycle

| Event | Description | Emitter | Status |
|---|---|---|---|
| `ShipSpawned` | Ship appeared in the world | `SimulationNode::spawn_ship()` | ✅ implemented |
| `ClientAdmissionIdentityReserved` | Fresh admission durably consumed a `PlayerId`/`ShipId` pair without materializing a Ship; public Replay advances allocation projection watermarks | `SimulationNode::reserve_fresh_admission_identity()` | ✅ implemented |
| `ClientAdmissionCommitted` | Atomic fresh-admission public fact containing Ship, fitting/cargo snapshot, ownership identity, and idempotent Station grant description | `SimulationNode::commit_reserved_fresh_admission()` | ✅ implemented |
| `ShipDespawned` | Ship manually removed from the world | `SimulationNode` | type only (no emission site; public Replay supported) |
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
| `SectorTransitRequested` | Durable public transfer fact carrying source-local request identity plus the resolved Gate/non-Gate route | source `propose_transit()`; destination also records an incoming marker using its own local `tick` | ✅ implemented |
| `SectorTransitCompleted` | Destination completed the import, or source removed its recovery copy after Ack; carries the source-local `request_tick` for attempt-specific replay/dedup projection | destination `handle_transit_commit()`; source `complete_outgoing_transit()` | ✅ implemented |
| `SectorTransitAborted` | Transit aborted (ownership stays with `from`) | destination failure path (not wired) | type + public Replay |

Validation-stage rejection is expressed via `CommandRejected`, not an event (INV-006); there is no `SectorTransitRejected` event. `propose_transit` returns `Err` without emitting an event if the Ship is absent or already in Transit.

The corresponding command is `TransitCommand { ship_id, to }`. Raft carries `TransitOp::Request`, `Commit`, and `Ack`. Existing code reconstructs some retry information from EventStore; ADR-0049 migration must ensure reliable proposal/retry obligations have durable outbox/idempotency representation and that exact state recovery does not depend on public-event-only replay.

### 3.7 Jump Gate Navigation (ADR-0009, complete)

| Event | Description | Emitter | Status |
|---|---|---|---|
| `JumpGateUsed` | Ship moved to another Sector via a Jump Gate | `SimulationNode::append_jump_events` (destination) | ✅ implemented (Raft pipeline) |
| `StarSystemChanged` | Ship moved to a different star system (concurrent with `JumpGateUsed`) | `SimulationNode::append_jump_events` (destination) | ✅ implemented (Raft pipeline) |

Corresponding Command: `JumpCommand { gate_id }`, committed over the same Raft Log path as `TransitCommand` (ADR-0014). The server resolves the command against the caller's active ship (ADR-0037). `TransitOp::Request`/`Commit` carries `gate_id: Option<JumpGateId>`; after destination Commit, the destination appends `JumpGateUsed` alongside `SectorTransitCompleted`, appends `StarSystemChanged` if needed, and then proposes Ack.

Static topology (3 star systems, 4 jump gates) is defined in `dawn-sector/src/galaxy.rs` (ADR-0026). `dawn-wire`'s `domain_event_to_event_wire` serializes both events to clients over the postcard binary envelope (ADR-0042), and the single `ClientRequest` admission seam handles `Jump`.

### 3.8 Tackle (ADR-0024)

| Event | Description | Emitter | Status |
|---|---|---|---|
| `TackleApplied` | Fold Disruptor activated on a target (in range + locked) | `SimulationNode::process_tackle()` | ✅ implemented |
| `TackleReleased` | Tackle effect ended (Module off / out of range / tackler destroyed) | `SimulationNode::process_tackle()` | ✅ implemented |

While tackled, `can_propose_warp()` / `can_propose_jump()` return false, blocking Warp/Jump. Exact `TackledComp` state belongs in RecoveryDelta/checkpoints; these events remain public facts.

`TackleReleased` is never emitted without a matching prior `TackleApplied` (strict 1:1 pairing). With multiple simultaneous tacklers, each tackler gets its own pair.

### 3.9 Coordinate Anchoring / Station facts

| Event | Description | Emitter | Status |
|---|---|---|---|
| `AnchorRebased` | Ship's coordinate anchor changed while absolute position is preserved | `SimulationNode` (Warp arrival, ADR-0029) | ✅ implemented |
| `ShipDocked` | Ship docked at an NPC station | `SimulationNode::dock_owned` | ✅ implemented |
| `ShipUndocked` | Ship undocked from an NPC station | `SimulationNode::undock_owned` | ✅ implemented |
| `PackagedShipBuilt` | Scrap Metal consumed and converted into a packaged ship | `SimulationNode::build_packaged_ship_owned` | ✅ implemented |
| `ShipDisassembled` | Docked undamaged unfitted ship converted into a packaged ship | `SimulationNode::disassemble_ship_owned` | ✅ implemented |
| `ShipAssembled` | Station `PackagedShip` converted into a live docked Ship owned by the caller | `SimulationNode::assemble_ship_owned` | ✅ implemented |

`AnchorRebased` is a durable public representation-change fact. Exact position/anchor authority is also captured in RecoveryDelta. Station inventory authority is the Sector journal's Station aggregate delta; SQLite is an idempotent projection (ADR-0038/ADR-0049), not something reconstructed solely from these public events.

### 3.11 System (reserved for future use)

| Event | Description | Status |
|---|---|---|
| `TickStarted` | Tick started | not implemented |
| `TickCompleted` | Tick completed | not implemented |

Neither event is required for recovery. Eventless Ticks already have a durable ADR-0049 recovery transition.

### 3.12 AoI (Area of Interest) Delivery Filter (ADR-0019)

AoI introduces **no new domain events**. It's implemented by filtering `DomainEvent` delivery through each observer's 27-cell neighborhood.

| Message | Description | Status |
|---|---|---|
| `AoiEnter` | Ship entered an observer's AoI (delivery message, not a domain event) | ✅ implemented (8C, ADR-0019) |
| `AoiLeave` | Ship left an observer's AoI (same) | ✅ implemented (8C, ADR-0019) |

`AoiEnter` / `AoiLeave` are delivery-control messages, not domain events. They do not participate in exact recovery; AoI consistency comes from current authoritative state plus client delivery/projection logic.

---

## 4. Command List

Commands are defined in `dawn-core/src/commands.rs`. Clients send them to the server as the single typed `ClientRequest` enum.

| Command | Description | Resulting Event(s) | Status |
|---|---|---|---|
| `MoveCommand` | Specify thrust direction | — | ✅ implemented |
| `LockOnCommand` | Request lock-on | `TargetLocked` | ✅ implemented |
| `FitModuleCommand` | Fit an inventory Module into a slot; requires the ship to be docked | `ShipFitted` | ✅ implemented |
| `UnfitModuleCommand` | Return a fitted Module to inventory; requires docked | `ShipFitted` | ✅ implemented |
| `ReorderFittedModuleCommand` | Reorder two fitted modules within the same slot kind; requires docked | `ShipFitted` | ✅ implemented |
| `DockCommand` | Dock at an NPC station once within docking radius | `ShipDocked` | ✅ implemented |
| `UndockCommand` | Leave a previously-docked NPC station | `ShipUndocked` | ✅ implemented |
| `BuildPackagedShipCommand` | Consume Scrap Metal and create a `PackagedShip` at the current Station | `PackagedShipBuilt` | ✅ implemented |
| `DisassembleShipCommand` | Convert a docked undamaged unfitted ship into a packaged ship | `ShipDisassembled` | ✅ implemented |
| `AssembleCommand` | Convert a Station `PackagedShip` into a new live docked Ship | `ShipAssembled` | ✅ implemented |
| `DisembarkCommand` | Clear active ship while docked without changing ownership | — | ✅ implemented |
| `TransferCargo` | Move an item stack between docked ship cargo and Station inventory | — public event may be empty; RecoveryDelta is still mandatory | ✅ implemented |
| `ActivateModuleCommand` | Turn on an Active Module | `ModuleActivated` | ✅ implemented |
| `DeactivateModuleCommand` | Turn off an Active Module | `ModuleDeactivated` | ✅ implemented |
| `AttackCommand` | Designate an attack target | `WeaponFired` | ⬜ type + parser only; not wired into combat |
| `StopCommand` | Decelerate to zero velocity using acceleration | — | ✅ implemented |
| `ApproachCommand` | Semi-automatic approach to a target | — | ✅ implemented |
| `TransitCommand` | Request a Sector Transit via Raft | `SectorTransitRequested` / `Completed` | ✅ implemented |
| `JumpCommand` | Move to another Sector via Jump Gate; may auto-warp/approach first | `JumpGateUsed` (+ `StarSystemChanged`) | ✅ implemented |
| `WarpCommand` | Warp within the same Sector to a Gate/body | `VelocityChanged` as applicable; exact Warp state is RecoveryDelta | ✅ implemented |
| `OrbitCommand` | Orbit a target at a chosen/default radius | — | ✅ implemented |
| `KeepAtRangeCommand` | Maintain minimum range from a target | — | ✅ implemented |
| `SelectActiveShipCommand` | Switch active ship to another owned docked ship | — | ✅ implemented |

Commands with no resulting public event may still mutate authoritative state and therefore still require a durable RecoveryDelta transition under ADR-0049.

> **ADR-0037:** active-ship helm/module commands do not carry `ship_id`; the server resolves them against the admitted caller's active ship. Owned Station-management commands may carry explicit ship identity where defined. See `ownership.md` §7.

### Internal Market bridge commands

The following `dawn-core` commands are generated by `dawn-market` and are not variants of `ClientRequest`; callers route them to the Sector that owns `ship_id`.

| Command | Description | Resulting Event(s) | Status |
|---|---|---|---|
| `RemoveItemCommand` | Remove a listed Ask quantity from seller ship cargo | `ShipFitted` | ✅ implemented |
| `ReturnItemCommand` | Return remaining Ask quantity after cancellation | `ShipFitted` | ✅ implemented |
| `CreditItemCommand` | Credit an item quantity to buyer ship cargo after settlement | `ShipFitted` | ✅ implemented |

These commands are deliberately one-sided. A trade between players in different Sectors does not require an Item transfer command spanning both owners, so it does not enter the Transit/Raft ownership path (ADR-0034 §4).

## 5. Event Field Specs

### `ShipSpawned`

Ship generated within a Sector.

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | unique identifier of the spawned Ship |
| `sector_id` | `SectorId` | ✓ | Sector it spawned into |
| `initial_position` | `AbsolutePosition` | ✓ | authoritative Sector-frame spawn coordinates |
| `ship_type_id` | `ShipTypeId` | ✓ | ship type ID |
| `tick` | `Tick` | ✓ | spawn Tick |

**Invariant:** `ship_id` is globally unique and never reused (INV-004). Public Replay can reconstruct the spawn projection; exact committed state is RecoveryDelta authority.

---

### `VelocityChanged`

Ship velocity changed. `MovementSystem` emits this public fact only when velocity differs from the previous Tick.

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id`  | `ShipId` | ✓ | Ship whose velocity changed |
| `velocity` | `Velocity` | ✓ | new velocity vector |
| `tick`     | `Tick` | ✓ | Tick velocity was finalized |

**Public Replay:** consumers may apply velocity changes/integration for supported projections. This is not the exact recovery contract: authoritative final position/velocity/anchor/flight state for every Tick is captured in RecoveryDelta/checkpoints, including Ticks with no `VelocityChanged`.

---

### `ShipDespawned`

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | removed Ship |
| `tick` | `Tick` | ✓ | removal Tick |

---

### `ShipFitted`

Ship's fitting slots and/or cargo public projection changed.

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Ship changed |
| `fitting` | `FittingSnapshot` | ✓ | fitting snapshot after change |
| `inventory` | `Vec<ItemId>` | ✓ | public cargo projection after change |
| `tick` | `Tick` | ✓ | Tick finalized |

**Public Replay:** recompute derived fitting stats as defined by the catalog. Exact fitting/cargo authority is also present in RecoveryDelta.

---

### `TargetLocked`

| Field | Type | Required | Description |
|---|---|---|---|
| `locker_id` | `ShipId` | ✓ | Ship that locked |
| `target_id` | `ShipId` | ✓ | Ship that was locked |
| `tick` | `Tick` | ✓ | Tick lock completed |

**Public Replay:** update the matching lock projection to `Locked`.

---

### `LockLost`

| Field | Type | Required | Description |
|---|---|---|---|
| `locker_id` | `ShipId` | ✓ | Ship that lost the lock |
| `target_id` | `ShipId` | ✓ | target |
| `tick` | `Tick` | ✓ | loss Tick |

**Public Replay:** remove the matching lock projection entry.

---

### `WeaponFired`

Weapon fired and hit. A miss does not emit this event. Damage appears in same-Tick `DamageTaken`.

| Field | Type | Required | Description |
|---|---|---|---|
| `attacker_id` | `ShipId` | ✓ | firing Ship |
| `target_id` | `ShipId` | ✓ | target |
| `damage` | `f32` | ✓ | realized damage value |
| `tick` | `Tick` | ✓ | firing Tick |

**Public Replay:** fire log only; does not reroll randomness. Exact combat outcome is captured by RecoveryDelta.

---

### `DamageTaken`

Ship took damage and HP changed.

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Ship damaged |
| `damage` | `f32` | ✓ | damage received |
| `current_shield` | `f32` | ✓ | shield remaining |
| `current_armor` | `f32` | ✓ | armor remaining |
| `current_hull` | `f32` | ✓ | hull remaining |
| `tick` | `Tick` | ✓ | damage Tick |

The full HP values make the public projection self-contained; RecoveryDelta remains exact-state authority.

---

### `RepairApplied`

An active repair cycle restored Shield or Armor.

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Ship repaired |
| `amount` | `f32` | ✓ | actual amount restored |
| `layer` | `RepairLayer` | ✓ | `Shield` / `Armor` |
| `current_shield` | `f32` | ✓ | shield remaining |
| `current_armor` | `f32` | ✓ | armor remaining |
| `current_hull` | `f32` | ✓ | hull remaining |
| `tick` | `Tick` | ✓ | repair Tick |

---

### `ModuleActivated`

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Ship performing action |
| `module_id` | `ModuleId` | ✓ | Module |
| `slot` | `SlotKind` | ✓ | fitting slot |
| `target_ship_id` | `Option<ShipId>` |  | targeted module target |
| `tick` | `Tick` | ✓ | activation Tick |

**Public Replay:** set projection `is_active = true` and recompute derived fitting state where supported.

---

### `ModuleDeactivated`

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Ship performing action |
| `module_id` | `ModuleId` | ✓ | Module |
| `slot` | `SlotKind` | ✓ | fitting slot |
| `forced_reason` | `Option<ModuleDeactivationReason>` |  | player OFF / capacitor / range cause |
| `tick` | `Tick` | ✓ | deactivation Tick |

**Public Replay:** set projection inactive and clear target where applicable.

---

### `ShipDestroyed`

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | destroyed Ship |
| `killer_id` | `ShipId` | ✓ | final-blow Ship |
| `tick` | `Tick` | ✓ | destruction Tick |

**Public Replay:** remove matching Ship projection.

Current downstream reward mutation is authoritative state and must appear in the same RecoveryDelta transition; it cannot rely only on this public event.

---

### `SectorTransitRequested`

A durable public transfer-request fact. The source may retain a frozen recovery copy until Ack; destination may also record an incoming marker.

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Ship transiting |
| `from` | `SectorId` | ✓ | source Sector |
| `to` | `SectorId` | ✓ | destination Sector |
| `tick` | `Tick` | ✓ | source request Tick |

**Public Replay:** can update the supported transit projection. Exact handoff/recovery state and reliable retries follow ADR-0049/ADR-0014.

---

### `SectorTransitCompleted`

Self-contained public completion event; `handoff` mirrors the canonical Transit handoff payload.

| Field | Type | Required | Description |
|---|---|---|---|
| `handoff` | `TransitHandoffState` | ✓ | Ship/owner/type/motion/HP/cap/fitting/inventory handoff fact |
| `from` | `SectorId` | ✓ | previous Sector |
| `to` | `SectorId` | ✓ | new Sector |
| `request_tick` | `Tick` | ✓ | source-local attempt identity |
| `entry_pos` | `AbsolutePosition` | ✓ | destination entry coordinates |
| `tick` | `Tick` | ✓ | local completion Tick |

**Public Replay:** supports reconstruction of the Transit projection and audit trail. Exact live/recovery reducer authority is the committed RecoveryDelta.

---

### `SectorTransitAborted`

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Ship whose Transit aborted |
| `from` | `SectorId` | ✓ | owning Sector |
| `to` | `SectorId` | ✓ | aborted destination |
| `tick` | `Tick` | ✓ | abort Tick |

**Status:** type + public Replay implemented; nothing appends this event yet.

---

### `JumpGateUsed`

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Ship |
| `gate_id` | `JumpGateId` | ✓ | Gate |
| `from_sector` | `SectorId` | ✓ | source |
| `to_sector` | `SectorId` | ✓ | destination |
| `entry_pos` | `AbsolutePosition` | ✓ | destination coordinates |
| `tick` | `Tick` | ✓ | finalized Tick |

**Status:** ✅ implemented.

---

### `StarSystemChanged`

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Ship |
| `from_system` | `StarSystemId` | ✓ | source system |
| `to_system` | `StarSystemId` | ✓ | destination system |
| `tick` | `Tick` | ✓ | finalized Tick |

**Status:** ✅ implemented.

---

### `TackleApplied`

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Ship being tackled |
| `by` | `ShipId` | ✓ | tackler |
| `tick` | `Tick` | ✓ | applied Tick |

**Public Replay:** add `by` to tackle projection.

---

### `TackleReleased`

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Ship released |
| `by` | `ShipId` | ✓ | tackler whose effect ended |
| `tick` | `Tick` | ✓ | release Tick |

**Public Replay:** remove `by` from tackle projection.

---

## 6. Upcaster Catalog

Record breaking changes here only when they occur.

Breaking changes to date: **none**

### Upcaster procedure (for future reference)

```text
1. Mark the old event as Deprecated (do not delete it)
2. Define the new event under a new name (V2)
3. Implement impl Upcaster for OldEvent { fn upcast(self) -> NewEvent }
4. Route public-event Replay through the Upcaster
5. Record the change in this catalog
6. Create a new ADR
```
