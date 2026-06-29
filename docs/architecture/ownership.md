---
scope    : Complete rules for "who manages what" — ownership, state transitions, responsibilities.
audience : AI Agent / Human Developer
update   : When Actor composition changes / when Sector management rules change.
related  : entity-model.md, event-catalog.md
---

> **Implementation status**
>
> | Section | Content | Status |
> |---|---|---|
> | §1-2 | Ship ownership, basic state transitions | Implemented |
> | §2 Sector Transit / §3 Node failure | Cross-Sector move exclusion, Raft failover | Implemented (ADR-0014) |
> | §4 Actor ownership | Data isolation between Actors | Implemented |
> | §5 ID generation | NodeId + monotonic counter | Implemented |
>
> Sector Transit must always go through Raft (CLAUDE.md FBD-006).

# Ownership Rules

## 1. What ownership means

Ownership means **exactly one authority has the right to mutate a given entity's state.** Concurrent mutation by multiple authorities causes conflicts; ownership rules are encoded in code now so the same rules hold as the system distributes (ADR-0021: single ownership means CRDTs are unnecessary).

| | Single Process (current) | Distributed (future) |
|---|---|---|
| Conflicts | Impossible in-process | Occur from network latency/failures |
| Enforcement | Code convention | Physically enforced by Raft |
| Violation impact | Caught by tests | Production data inconsistency |

---

## 2. Ship ownership

### Basic rule

```
A Ship is owned by exactly one Sector at a time.
Multiple Sectors must never own the same Ship simultaneously.
```

### Ownership state transitions

```
[does not exist]
      │
      │ ShipSpawned { sector_id }
      ▼
[owned by Sector A]  ←─────────────────────────┐
      │                                        │
      │ SectorTransitRequested                  │
      ▼                                        │
[in Transit]                                   │
  ownership: remains with Sector A              │
      │                                        │
      │ SectorTransitCompleted                  │
      ▼                                        │
[owned by Sector B] ─────────────────────────────┘
      │
      │ ShipDespawned
      ▼
[does not exist]
```

While in Transit, ownership is logically still held by the origin Sector, which prevents another operation from interleaving before Transit completes.

### Operations forbidden during Transit

```
- Accepting a MoveCommand
- Starting another SectorTransit
- ShipDespawn
```

### Ownership-check responsibility

| Operation type | Responsible party | Current implementation |
|---|---|---|
| Sector-local mutation | The Sector Node itself | SimulationNode |
| Crossing Sector boundary | Consensus Layer | dawn-consensus (Raft, ADR-0014) |
| Read (reference only) | No check needed | — |

---

## 3. Sector management responsibility

### Basic rule

```
Each Sector is managed by exactly one Node.
Multiple Nodes must never manage the same Sector simultaneously.
```

### Sector → Node mapping

```
Current:  SimulationNode manages all Sectors (single process); production
          deploys (dawn-sector-node, 8D-4) split Sectors across processes
          per static TOML config.
Future:   Consensus Layer manages Sector → Node mapping dynamically.
```

`SectorId` semantics are stable across this transition regardless of which Node owns the mapping.

### Node failure handling

```
- Node crash → Raft leader re-election (ADR-0014)
- New Node takes over the failed Node's Sector
- State is rebuilt from the Event Log (snapshot + tail replay) to complete handover
```

---

## 4. Actor ownership

The Actor model lives in `dawn-actor` (ADR-0002).

### Actor-to-data mapping

| Actor | Owns | Accepts |
|---|---|---|
| `SectorSimulatorActor` | ECS World + Event Log (for its Sector) | `Tick`, `MoveCommand`, `SpawnShip`, `Transit` |

`SimulationNode` owns the Event Log directly; there is no dedicated EventStore actor.

### Inter-Actor data-sharing rules

```
Forbidden: sharing data via Arc<Mutex<T>>
Forbidden: an Actor reading another Actor's internal state directly
Allowed:   message copies via Mailbox (mpsc channel) only
```

This anticipates Actor-to-Actor communication moving over the network once physically distributed.

---

## 5. ID generation ownership

### Basic rule

```
EntityId generation is the exclusive right of the Node holding the relevant NodeId.
No coordination (locking, consensus) is required.
```

### Why no coordination is needed

```
EntityId = NodeId (8 bit) + Counter (56 bit)

As long as Counter increases monotonically within a given NodeId,
EntityId stays unique even if Counters collide across different Nodes.
```

Example:
```
Node(0), Counter(100) → EntityId: 0x00_00000000000064
Node(1), Counter(100) → EntityId: 0x01_00000000000064  ← different
```

This design requires no changes after distribution.

---

## 6. Invariants (ownership-related)

Where these duplicate CLAUDE.md INVs, only the link is listed.

| Invariant | Description | Ref |
|---|---|---|
| A Ship always belongs to exactly 1 Sector | No simultaneous ownership by multiple Sectors | §2 |
| EntityId is never reused | No same ID after Despawn | [CLAUDE.md INV-004](../../CLAUDE.md) |
| Operations on an in-Transit Ship are restricted | MoveCommand etc. rejected | §2 |
| A Sector is managed by exactly 1 Node | No simultaneous management by multiple Nodes | §3 |
| Actors never share data directly | Mailbox only | [CLAUDE.md INV-005](../../CLAUDE.md) |
