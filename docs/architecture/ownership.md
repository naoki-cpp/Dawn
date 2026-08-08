---
scope    : Complete rules for "who manages what" — ownership, state transitions, responsibilities.
audience : AI Agent / Human Developer
update   : When Actor composition changes / when Sector management or recovery ownership rules change.
related  : entity-model.md, event-catalog.md, recovery-contract.md, ../adr/ADR-0014-raft-consensus.md, ../adr/ADR-0049-sector-recovery-state-delta-wal.md
---

> **Implementation status**
>
> | Section | Content | Status |
> |---|---|---|
> | §1-2 | Ship ownership, basic state transitions | Implemented |
> | §2 Sector Transit / §3 Node failure | Cross-Sector move exclusion, Raft failover | Implemented baseline; recovery persistence migrating under ADR-0049/#276 |
> | §4 Actor ownership | Data isolation between Actors | Implemented baseline; storage ownership migrates under #272 |
> | §5 ID generation | NodeId + monotonic counter | Implemented |
> | §7-8 | Player ownership / active-ship routing | Implemented behavior; recovery persistence gap scheduled by #284/#275 |
>
> Sector Transit must always go through Raft (FBD-006).
>
> **ADR-0049 recovery note:** ownership/routing values that affect future
> authoritative command interpretation are recovery authority even when no public
> `DomainEvent` exists. In particular, `active_ship` is not intentionally lossy
> session presentation state.
>
> **Scope note:** this file documents *Sector*-level ownership (which Sector owns a
> Ship), Actor data isolation, and §7's *player*-level ownership/routing state.

# Ownership Rules

## 1. What ownership means

Ownership means **exactly one authority has the right to mutate a given entity's state.** Concurrent mutation by multiple authorities causes conflicts; ownership rules are encoded in code now so the same rules hold as the system distributes (ADR-0021: single ownership means CRDTs are unnecessary).

| | Single Process (current) | Distributed |
|---|---|---|
| Conflicts | Impossible in-process when APIs are obeyed | Can arise from network latency/failures |
| Enforcement | Code convention / engine boundaries | Consensus + recovery/promotion invariants |
| Violation impact | Caught by tests | Production data inconsistency |

Recovery ownership is separate from mutation ownership: ADR-0049 defines the durable representation that lets a new process/node reconstruct the same authority before serving it.

---

## 2. Ship ownership

### Basic rule

```text
A Ship is owned by exactly one Sector at a time.
Multiple Sectors must never actively own the same Ship simultaneously.
```

### Ownership state transitions

```text
[does not exist]
      │
      │ ShipSpawned { sector_id }
      ▼
[owned by Sector A]  ←─────────────────────────┐
      │                                        │
      │ SectorTransitRequested / Saga attempt   │
      ▼                                        │
[in Transit]                                   │
  ownership: remains with Sector A while frozen│
      │                                        │
      │ destination durable completion          │
      ▼                                        │
[owned by Sector B] ─────────────────────────────┘
      │
      │ ShipDespawned
      ▼
[does not exist]
```

While in Transit, the source may retain a frozen recovery copy until destination completion/Ack semantics allow cleanup. That copy is not an active second owner. ADR-0014 defines the behavioral consensus invariant; #276 will replace legacy EventStore-scan persistence with a durable handoff Saga under ADR-0049.

### Operations forbidden during Transit

```text
- Accepting a MoveCommand
- Starting another conflicting SectorTransit
- ShipDespawn outside the handoff lifecycle
```

The complete freeze set is specified by ADR-0014 and must remain consistent with the final #276 Saga.

### Ownership-check responsibility

| Operation type | Responsible party | Current / target implementation |
|---|---|---|
| Sector-local mutation | The Sector engine | `SimulationNode` today; storage-independent engine under #272 |
| Crossing Sector boundary | Consensus + Transit lifecycle | `dawn-consensus` Raft + current handoff seam; durable Saga under #276 |
| Exact restart/failover reconstruction | Recovery layer | ADR-0049 checkpoint + authoritative recovery tail |
| Read (reference only) | No mutation check | read-only Sector view after #272 |

---

## 3. Sector management responsibility

### Basic rule

```text
Each Sector is served as authoritative by exactly one healthy owner at a time.
Multiple Nodes must never simultaneously serve the same Sector as active authority.
```

Durability replicas may retain committed recovery bytes without being active owners. Under `ReplicatedDurable`, a replica can even have bytes staged beyond its locally applied position; this does **not** make it promotable until ADR-0049 reducer/projection promotion conditions are satisfied.

### Sector → Node mapping

```text
Current:  production dawn-sector-node processes own configured Sectors through
          static TOML composition.
Future:   consensus/runtime control may manage Sector -> Node mapping dynamically.
```

`SectorId` semantics remain stable regardless of which process currently serves the Sector.

### Node failure handling

```text
- Node/process failure is detected by the runtime/consensus control path
- Replacement owner is selected according to the distributed runtime policy
- Replacement loads a compatible ADR-0049 checkpoint
- It applies every contiguous committed authoritative recovery transition after it
- It catches up promotion-critical retry/output/projection state
- Only after promotion invariants pass may it serve the Sector as authoritative
```

Public `DomainEvent` replay by itself is not the failover reconstruction authority. #280 owns the transport that moves the selected checkpoint/tail representation; it does not redefine the recovery model.

---

## 4. Actor / runtime ownership

The Actor model lives in `dawn-actor` (ADR-0002).

### Current baseline

Today `SimulationNode<S>` still combines ECS/domain authority with EventStore/repository access in places. `SectorSimulatorActor` and production adapters call into that broad object.

### Target boundary (#272 / #275)

The authoritative pure Sector engine owns domain state, while the runtime/application layer owns persistence and external effects:

```text
runtime receives input
    -> pure/bounded engine prepare
    -> runtime persists ADR-0049 recovery transition via #271 journal
    -> engine applies committed transition
    -> runtime applies required repositories/projections
    -> runtime publishes replication/client/effect outputs
```

Repositories, SQL connections, journal handles, sockets, and replication transports must not become hidden fields of the final pure engine merely to preserve the old `SimulationNode<S>` shape.

### Inter-Actor data-sharing rules

```text
Forbidden: sharing mutable actor internals through Arc<Mutex<T>> as a shortcut
Forbidden: an Actor reading another Actor's private mutable state directly
Allowed:   message/value transfer through the defined boundary
```

This preserves actor isolation and makes a later network boundary explicit.

---

## 5. ID generation ownership

### Basic rule

```text
EntityId generation is the exclusive right of the authority holding the relevant NodeId/allocator domain.
No ID may be reused after it has been durably consumed.
```

### Why no coordination was originally needed

```text
EntityId = NodeId (8 bit) + Counter (56 bit)

As long as Counter increases monotonically within a given NodeId,
EntityId stays unique even if Counters collide across different Nodes.
```

Example:

```text
Node(0), Counter(100) -> EntityId: 0x00_00000000000064
Node(1), Counter(100) -> EntityId: 0x01_00000000000064
```

ADR-0049 adds a recovery requirement: allocation counters/consumed identity state are authoritative and must be restored exactly enough that a crash cannot reissue an already-consumed ID. #275 may move counters into a Player/Simulation owner, but cannot downgrade them to a derived cache.

---

## 6. Invariants (ownership-related)

| Invariant | Description | Ref |
|---|---|---|
| A Ship has one active Sector owner | No simultaneous active mutation by multiple Sectors | §2 / ADR-0014 |
| EntityId is never reused | No same ID after durable consumption | AI_DEVELOPMENT_GUIDE INV-004 / ADR-0049 |
| Operations on an in-Transit Ship are restricted | Handoff source remains frozen | §2 / ADR-0014 |
| A Sector has one healthy serving owner | Replica staging is not active ownership | §3 / ADR-0049 |
| Actors/runtime components do not bypass ownership boundaries | Communication uses explicit message/value seams | §4 |
| Recovery cannot silently change command routing | Ownership and active-ship routing restore before serving | §7 / ADR-0049 |

---

## 7. Player-level ship ownership and routing (ADR-0037, amended by ADR-0049)

`ShipRegistry` splits the "which player controls which ship" question into three independently tracked concerns:

- **Owned ship** (`owners: ShipId -> PlayerId`): every Ship a Player owns. A Player may own more than one.
- **Active ship** (`active_ship: PlayerId -> ShipId`): the one owned Ship currently routable for helm/module commands and Undock.
- **Docked station context** (`docked_players: PlayerId -> StationId`): which Station context the Player currently occupies.

```text
Player owns {Ship A, Ship B}, both docked at Station S, active = A

  SelectActiveShipCommand{ship_id: B}
        |
        v  (must own B, B != A, B docked at S)
  active = B
```

Flight/steering commands and Undock resolve through `active_ship`. These wire requests intentionally do not carry arbitrary `ship_id` values for the active helm surface, so `active_ship` directly changes **which authoritative Ship a command mutates**.

Station inventory-management commands may name an owned Ship explicitly because they operate on owned docked Ships rather than only the active helm target.

### Recovery classification of `active_ship`

The earlier ADR-0037-era implementation treated switching/disembarking as "session-local, not event-sourced" because it changed no Ship HP/fitting/position and had no public `DomainEvent`. ADR-0049 clarifies that **public-event sourcing and authoritative recovery are different questions**.

`active_ship` is authoritative Player routing state because losing it can change:

- the target selected for the next helm/module command;
- whether Undock is accepted;
- the authoritative loadout/ship context resumed after reconnect; and
- cross-domain Station/Transit routing decisions that depend on the active Ship.

Therefore:

- `SelectActiveShipCommand` and `DisembarkCommand` may emit no public `DomainEvent`, but a successful authoritative routing change still gets an ADR-0049 recovery transition;
- `active_ship` belongs in PlayerState checkpoint/delta recovery under #284/#275;
- removal/Transit/admission operations that clear or set active routing update the same authority; and
- socket identity, open UI panel, selected inventory row, and other presentation/session transport state remain non-authoritative unless separately promoted.

`ShipRegistry::remove()` only clears a Player's `active_ship` entry if the removed Ship was active. Removing another owned Ship must not silently clear the pointer to the Ship the Player is still flying.

---

## 8. A docked player with no active ship cannot Undock

Flight/steering commands and Undock require `is_active_ship`. A Player can be docked with no active Ship via `DisassembleShipCommand` or `DisembarkCommand` and has nothing routable for helm commands until:

- `AssembleCommand` creates a new owned docked Ship (without necessarily selecting it), or
- `SelectActiveShipCommand` selects an owned docked Ship.

`DisembarkCommand` has no public `DomainEvent`, but under ADR-0049 its successful clearing of `active_ship` is still authoritative recoverable state. `AssembleCommand` remains authoritative and its public `ShipAssembled` fact does not by itself replace the RecoveryDelta authority.

`PlayerLoadout` carries `active_ship_id: Option<u64>` and owned-ship rows so the client can render a shipless docked Player and full Ship roster. Presentation shape does not define persistence authority; the recovered Player routing state does.

`TransferToStationCommand { ship_id, station_id, item_id, direction }` moves an item stack between a docked Ship's cargo and the caller's Station inventory according to the Station rules. Exact Station durability follows ADR-0049/ADR-0038 as amended; #277 owns repository APIs.

**Current implementation debt:** the current `StateSnapshot` does not persist all `ShipRegistry.owners` / `active_ship` state. #284's recovery implementation and #275's PlayerState extraction must remove this gap before the exact-recovery acceptance criterion can close. It is not an accepted lossy-state exception.
