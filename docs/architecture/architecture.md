---
scope    : Map of the whole system. A bird's-eye view of what exists and how it connects
audience : AI Agent / Human Developer
update   : When crate composition changes / when a phase advances
related  : entity-model.md, event-catalog.md, ownership.md, tick-model.md, ../process/roadmap.md, ../../CLAUDE.md
---

# Dawn Architecture

## 1. How to read this document

This file is the project's **entry point**. For details, always follow the link to the dedicated document — do not append details here.

### Notes for AI agents

- Read **CLAUDE.md** before writing code (invariants, prohibitions)
- For the rationale behind design decisions, see **docs/adr/**
- For "what to implement," see **docs/process/roadmap.md**

### Document responsibility map

| File | Answers |
|---|---|
| `CLAUDE.md` | What must not be done / what to check before changing code |
| `docs/architecture/architecture.md` | How the whole system is structured (this file) |
| `docs/architecture/entity-model.md` | What exists (types, field definitions) |
| `docs/architecture/event-catalog.md` | What happens (Event specs) |
| `docs/architecture/ownership.md` | Who manages what (ownership, state transitions) |
| `docs/architecture/tick-model.md` | When and in what order things are processed |
| `docs/process/roadmap.md` | What to build, and in what order |
| `docs/design/game-design.md` | Why a feature exists / lessons from EVE, future candidates |
| `docs/adr/` | Why a decision was made (immutable decision record) |

---

## 2. Project essence

### Purpose

The goal is to build a game that **surpasses EVE Online** (ADR-0016). The distributed simulation foundation is both the **means** to that end and a **competitive edge** in the area EVE gave up on with TiDi. The foundation embodies:

- Real-time sync of tens of thousands of entities in a Single Shard
- Full causal traceability and world reproducibility via Event Sourcing
- High throughput via separation of concerns between the Actor model and ECS

### Current scope (Phase 8D — TCP distributed wiring complete)

```
Runtime          : multi-process (`dawn-sector-node` or `dawn-simulation`)
Inter-node comms : TCP (TcpRaftTransport / TcpReplicationTransport, 8D-3/2c)
Client comms     : WebSocket + JSON (Godot <-> WsServer, ADR-0007)
Node             : a physical process (`sector-node config/node-N.toml`)
Inter-node net   : TCP LAN plaintext (8D milestone; TLS is next phase)
Persistence      : FileEventStore + checkpoint/restore wired into
                    `dawn-sector-node` (2026-07-01); each node's hot log,
                    snapshot, and cold archive paths are config fields
```

See [ADR-0003](../adr/ADR-0003-local-first-development.md) / [ADR-0027](../adr/ADR-0027-dawn-replication-crate.md) for details.

### Future scope (direction only, not implemented)

- TLS / QUIC (8E+)
- Raft log compaction + InstallSnapshot (snapshot + tail catch-up groundwork already exists via SnapshotTransfer)

**Do not implement code ahead of the current phase.**

---

## 3. Cargo workspace layout

### Crate list

| Crate | Kind | Responsibility |
|---|---|---|
| `dawn-core` | library | Pure domain model. Zero external dependencies |
| `dawn-ecs` | library | ECS World wrapper. Component / System definitions |
| `dawn-event-store` | library | Persistence/compaction of the two-tier Event Log (hot log + cold archive) (ADR-0017) |
| `dawn-consensus` | library | Raft implementation (leader election, log replication, RaftActor; ADR-0014) |
| `dawn-actor` | library | Client transport boundary (`ClientConnection` trait) |
| `dawn-replication` | library | Gossip distribution boundary for the append log (OutboundLogPublisher / InMemoryReplicationBus / ReplicationTransport / AntiEntropy / TcpReplicationTransport / SnapshotTransfer / ReplicaSet; ADR-0021/0027) |
| `dawn-sector` | library | Per-Sector game logic (SimulationNode, Tick, Transit, Warp, Bot AI, AoI, Snapshot; ADR-0026) |
| `dawn-simulation` | binary | Wiring/bootstrap only. WsServer (Godot), Raft cluster wiring, load generation, TOML loader |
| `dawn-sector-node` | binary | Production binary (8D-4). Wires TcpRaftTransport + TcpReplicationTransport from static TOML config. 3 processes = 3-Sector cluster |

### Dependency DAG

```
dawn-core
    ^
    ├── dawn-ecs
    ├── dawn-consensus
    └── dawn-event-store
            ^
            ├── dawn-actor
            ├── dawn-replication
            └── dawn-sector          <- game logic (also depends on dawn-ecs / dawn-consensus, ADR-0026)
                    ^
                    ├── dawn-simulation     (binary; also depends on dawn-actor / dawn-consensus)
                    └── dawn-sector-node    (production binary; also depends on dawn-actor / dawn-consensus / dawn-replication, 8D-4)
```

Dependencies flow **bottom-to-top only**; any reverse or circular dependency is a design failure. See [AI_DEVELOPMENT_GUIDE.md "Crate Boundaries"](../../AI_DEVELOPMENT_GUIDE.md) for the full rule.

### Rule for adding crate dependencies

`dawn-core` may only depend on:

```
serde / thiserror only
Network I/O, file I/O, and async runtimes are forbidden
```

Create an ADR before adding any dependency to `dawn-core`.

---

## 4. Key concepts

See the linked document for details; this table is a one-line definition only.

| Concept | Definition | Details |
|---|---|---|
| **World** | The entire simulated world; the set of all Sectors | — |
| **Sector** | A spatial partition; the management scope for Ship entities | [ownership.md](./ownership.md) |
| **Node** | A logical processing unit; currently in-process | [ownership.md](./ownership.md) |
| **Ship** | The only Entity kind (MVP) | [entity-model.md](./entity-model.md) |
| **Tick** | Logical time unit, unrelated to wall-clock time | [tick-model.md](./tick-model.md) |
| **Event** | An immutable fact about something that happened | [event-catalog.md](./event-catalog.md) |
| **Command** | A change request that may be rejected | [event-catalog.md](./event-catalog.md) |

---

## 5. Data flow overview

```
Receive Command
    |
    v
Validation (reject -> return CommandRejected)
    |
    v
Execute domain logic (update ECS World)
    |
    v
Generate Event -> Append to EventStore
    |
    v
(future) inter-Node replication
```

Command and Event are fully separate types. See [AI_DEVELOPMENT_GUIDE.md "Event Workflow"](../../AI_DEVELOPMENT_GUIDE.md) for flow details and [event-catalog.md](./event-catalog.md) for Event specs.

---

## 5-A. ClientConnection abstraction

The connection between client (Godot) and server (Rust) is abstracted behind a trait, so swapping the implementation for networking requires no Godot-side changes.

```
Test:                             Production:
  InProcessConnection              WsClientConnection
  via in-memory channel            via WebSocket + JSON (ADR-0007)

  Both implement the same ClientConnection trait

ADR-0007 ruled out a move to gRPC; revisit only once inter-node
distributed comms need it.
```

### Trait responsibility (these two directions only)

```
Server -> Client : stream of DomainEvents
Client -> Server : Command submission
```

No other responsibility may be mixed into this trait. Connection-state management, auth, and reconnection belong to higher layers.

### Data flow

```
SectorSimulatorActor
    | events
dawn-replication::InMemoryReplicationBus
    |
ClientConnection (InProcess / WebSocket)
    | DomainEvent stream
Godot client (GDScript)
    ^ Command
```

See ADR-0005 (trait) / ADR-0007 (WebSocket session) for detailed design.

---

## 5-B. Design direction for productization

To grow the current technical foundation into an EVE-like 3D game, the following concepts are treated as design assumptions now; implementation follows the roadmap phases.

### Interest Management (AoI) — implemented in 8C (ADR-0019)

Critical: without this there is no real game.

```
Problem: with 100k ships, broadcasting every Event to every client is infeasible
Solution: each client receives Events only for entities within its own
          bubble / Area of Interest (AoI)

              World
           +--------------+
           |  C           |
           |     +------+ |
           |  A  |[you] | |  <- only A, B in the bubble are received
           |     |  B   | |     C is not received
           |     +------+ |
           +--------------+
```

**Implementation (ADR-0019):**
- Static cell grid (3x3x3 = 27 cells) spatial index (`dawn-sector/src/aoi.rs`)
- Crossing a cell boundary sends `AoiEnter` / `AoiLeave` diff messages to the client
- `DomainEvent` delivery filter: deliver only when the involved Ship is within the observer's 27-cell neighborhood
- No new domain event types added — delivery filtering alone implements this
- `InitialState` delivers state scoped to the 3x3x3 neighborhood

### Projection / Read Model layer

The current CQRS design covers the write side only; the read side is specified here.

```
Write side (implemented):
  Command -> Validation -> Event -> EventStore

Read side (future):
  EventStore -> Projection -> Read Model
                                ├── SpatialIndex (proximity queries)
                                ├── ShipStateView (current Ship state)
                                └── SectorOccupancyView (Sector population)
```

Projections are **rebuildable by replaying Events** (extension of INV-002): if a Read Model is corrupted, it can be regenerated from the EventLog.

### Client connection model

```
Server (authoritative)         Client (presentation)
─────────────────────          ──────────────
Holds true state                Holds display state
     |                               |
     | 1. Receive Command            | Client-side prediction
     | 2. Validate, generate Event   | (look-ahead to hide latency)
     | 3. Deliver Event to client -> | Reconciliation
     |                               | (corrects prediction with Event)
```

The server remains authoritative (unchanged). The client shows a predicted state ahead of confirmation and reconciles it against server Events.

### Bounded Context expansion order

```
Implemented:
  Spatial + Movement + Combat (Fitting / Lock-on / Capacitor included)
  Navigation (Jump Gate / inter-system travel, ADR-0009)

Recommended next order (by dependency):
  Resource    <- idle/passive mining is banned by FBD-009; contested-only
      v
  Economy     <- Market / Trade / Manufacturing
      v
  Social      <- Corporation / Alliance / Chat

Principle: higher Contexts use Spatial, but Spatial never knows about
           higher Contexts (dependency always points downward)
```

---

## 5-C. Persistence and recovery model (two-tier log, ADR-0017)

The Event Log has two tiers, **hot log** and **cold archive**. The `EventStore` trait remains append-only (FBD-001: no truncate / delete / rewrite).

```
                       Append
                         |
                         v
   +---------------------- Hot Log ---------------------+
   |  [8B base_index header][len|payload]...             |  <- bounded; latest segment only
   +-------------------------------------------------------+
                         | compact(boundary)        <- migrates only the segments
                         |  (segment migration)        behind a verified snapshot
                         v
   +--------------- Cold Archive ------------------------+
   |  append-only forever (audit / disaster recovery; off the hot path) |
   +-------------------------------------------------------+
```

**The snapshot is the authoritative persistent checkpoint** (INV-002 revised). Crash recovery / failover (ADR-0014) follows:

```
Recovery = verified snapshot + catch-up from the hot log tail since that snapshot
```

- The only replay on the operational hot path is the tail catch-up. Full replay from genesis (log index 0) is **off-path** (audit / disaster recovery only).
- Derived/transient state (position, capacitor, lock countdowns, thrust intent, etc.) is **persisted in the snapshot**, not recorded as events; it is recomputed live on the next Tick after recovery.
- Compaction is segment migration, not event destruction. The hot log file holds a `base_index` header (the global log index of `records[0]`) and is swapped via atomic rename. Cold append happens before the hot swap, so an event is never lost at any point in time.
- Snapshots must be verifiable:
  1. snapshot -> restore -> snapshot round-trips to a byte-identical result
  2. snapshot + replay of the trailing Ticks == live state at that point

See [ADR-0017](../adr/ADR-0017-snapshot-compaction.md) / [AI_DEVELOPMENT_GUIDE.md "Architecture Invariants"](../../AI_DEVELOPMENT_GUIDE.md) (INV-002). This is distinct from the off-path Read Model rebuild in §5-B (that one is an optional audit-path replay).

---

## 6. Current constraints (changing these requires an ADR)

| Constraint | Reason | Basis (ADR) |
|---|---|---|
| Inter-node network is TCP LAN plaintext only (no TLS/QUIC) | Current 8D milestone stage; encryption is next phase | [ADR-0003](../adr/ADR-0003-local-first-development.md) (initial policy) / see §2 future scope |
| Sector count/assignment is fixed (no dynamic split/merge) | MVP scope limit | [entity-model.md §5](./entity-model.md) |
| Ship is the only entity (includes Fitting / Combat / Capacitor) | MVP scope limit | [AI_DEVELOPMENT_GUIDE.md "Project North Star"](../../AI_DEVELOPMENT_GUIDE.md) |

> §3's crate table and dependency DAG are the current source of truth for deployment topology.

---

## 7. Update rules for this document

Update this file only for:

- Crate addition/removal
- Addition of a key concept
- Scope changes from phase progression

Everything else belongs in the dedicated document.
