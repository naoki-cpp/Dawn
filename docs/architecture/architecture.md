---
scope    : Map of the whole system. A bird's-eye view of what exists and how it connects
audience : AI Agent / Human Developer
update   : When crate composition changes / when a phase advances / when a cross-cutting architecture contract changes
related  : entity-model.md, event-catalog.md, ownership.md, tick-model.md, recovery-contract.md, durable-journal.md, database-strategy.md, ../process/roadmap.md, ../../CLAUDE.md, ../adr/ADR-0052-workspace-boundaries.md
---

# Dawn Architecture

## 1. How to read this document

This file is the project's **entry point**. For details, always follow the link to the dedicated document — do not append details here.

### Notes for AI agents

- Read **CLAUDE.md** before writing code (invariants, prohibitions)
- For the rationale behind design decisions, see **docs/adr/**
- For "what to implement," see **docs/process/roadmap.md**
- For exact Sector recovery/durability, follow **recovery-contract.md** / ADR-0049 rather than inferring recovery from public Event flow

### Document responsibility map

| File | Answers |
|---|---|
| `CLAUDE.md` | What must not be done / what to check before changing code |
| `docs/architecture/architecture.md` | How the whole system is structured (this file) |
| `docs/architecture/entity-model.md` | What exists (types, field definitions) |
| `docs/architecture/event-catalog.md` | What happens publicly (DomainEvent specs) |
| `docs/architecture/ownership.md` | Who manages what (ownership, state transitions) |
| `docs/architecture/tick-model.md` | When and in what order things are processed |
| `docs/architecture/recovery-contract.md` | Which state is authoritative/recoverable, what commit/ack means, and crash/failover guarantees |
| `docs/architecture/durable-journal.md` | #271's fallible journal framing, evidence, compaction, archive, and failure policy |
| `docs/architecture/database-strategy.md` | Which storage products/repositories serve each role and when to migrate |
| `docs/process/roadmap.md` | What to build, and in what order |
| `docs/design/game-design.md` | Why a feature exists / lessons from EVE, future candidates |
| `docs/adr/` | Why a decision was made (immutable decision record, amended forward by later ADRs) |

---

## 2. Project essence

### Purpose

The goal is to build a game that **surpasses EVE Online** (ADR-0016). The distributed simulation foundation is both the **means** to that end and a **competitive edge** in the area EVE gave up on with TiDi. The foundation embodies:

- Real-time sync of tens of thousands of entities in a Single Shard
- Durable public/business causal traceability through append-only `DomainEvent`s
- Exact operational Sector recovery through ADR-0049 versioned checkpoints + authoritative state-delta tail
- High throughput via separation of concerns between the Actor/runtime layer and ECS/domain engine

"World reproducibility" must be qualified: downstream audit/projection consumers
may replay public events for their own views, while exact acknowledged Sector
state is recovered through the ADR-0049 checkpoint and RecoveryDelta
representation. `SimulationNode` has no public-event reverse reducer, and genesis
public-event replay is not an operational recovery contract.

### Current scope (Phase 11 presentation / Phase 10 manual validation / Phase 9 economy validation)

```text
Runtime          : multi-process (`dawn-server --bin sector-node` or
                    `dawn-server --bin simulate`)
Inter-node comms : shared versioned TCP peer transport with control/bulk
                    channels (ADR-0050 / #280)
Client comms     : WebSocket (Godot <-> WsServer, ADR-0007), postcard binary
                    for every ServerMessage/ClientMessage envelope, including
                    InitialState/PlayerLoadout/AoI/PositionSnap (ADR-0042)
Node             : a physical process (`sector-node config/node-N.toml`)
Inter-node net   : TCP LAN plaintext (8D milestone; TLS is next phase)
Persistence      : DurableJournal is the sole persistent transition source.
                    Its PublicEvent stream is projected into a bounded,
                    rebuildable PublicEventTail for replication; StateSnapshot
                    remains the checkpoint path. The Sector engine owns neither.
```

See [ADR-0003](../adr/ADR-0003-local-first-development.md), [ADR-0027](../adr/ADR-0027-dawn-replication-crate.md), [ADR-0049](../adr/ADR-0049-sector-recovery-state-delta-wal.md), [ADR-0051](../adr/ADR-0051-server-composition-boundary.md), and [ADR-0052](../adr/ADR-0052-workspace-boundaries.md).

### Future scope (direction only, not implemented)

- TLS / QUIC (8E+)
- deployment-specific recovery benchmark and numeric RTO selection on top of
  #280's implemented peer transport

**Do not implement code ahead of the current phase or redefine a contract owned by another refactor issue.**

---

## 3. Cargo workspace layout

### Crate list

| Crate | Kind | Responsibility |
|---|---|---|
| `dawn-core` | library | Pure domain model and stateless simulation policies (including the shared one-tick movement policy). Zero network/I/O dependencies |
| `dawn-client-core` | library | Godot-independent client-side domain model (loadout, wire row types, pure WorldSession state, shared ship-motion prediction/dead-reckoning track, ClientInteraction input policy, and Station Inventory interaction policy). Depends only on `dawn-core` (ADR-0039, ADR-0041, ADR-0043, ADR-0045, ADR-0046) |
| `dawn-client-gdext` | library (cdylib) | GDExtension binding exposing `dawn-client-core` to the Godot client, including typed Station Inventory rows/actions. Thin type-conversion adapter only (ADR-0040, ADR-0041, ADR-0046) |
| `dawn-protocol` | library | Client<->server wire schema (`ClientRequest`/`ServerFact`, `ServerMessage`/`ClientMessage` binary envelope). `ServerFact` is an audience-scoped client projection distinct from durable `DomainEvent`; depends only on `dawn-core` + serde + postcard -- no transport/runtime dependency (ADR-0041, ADR-0042, #274) |
| `dawn-ecs` | library | ECS World wrapper. Component / System definitions |
| `dawn-storage` | library | Fallible atomic `DurableJournal` mechanics. One transition batch stores ADR-0049 recovery records, committed public facts, and reliable effects; there is no second public EventLog |
| `dawn-distributed` | library | Raft, replication, shared versioned peer transport, and the bounded rebuildable `PublicEventTail` read model. Raft and replication remain separate modules over one transport boundary; opaque domain payloads keep the transport independent of policy (ADR-0027, ADR-0050, #280, #336) |
| `dawn-market` | library | Player-to-player Market: pure bid/ask, Currency, escrow, and durable `SettlementIntent` outbox policy. `MarketDb` is the SQLite adapter that atomically persists orders, balances, stable settlement IDs, and delivery state. Depends only on `dawn-core` + thiserror + rusqlite -- no transport/runtime dependency, same DAG position as `dawn-protocol` (ADR-0034 §4/§5/§6, #279). It never imports Sector bridge commands; `dawn-server` translates intents and routes them to the owning Sector |
| `dawn-sector` | library | Per-Sector game logic plus the shared durable runtime frame. `SimulationNode` composes explicit Simulation/Player/Station/Transit/Topology/GameData/FrameOutput owners and a separate Persistence adapter; `run_durable_runtime_frame` owns the prepare -> durable append -> live-apply -> reconciliation -> output boundary with injected consensus, health, and durability-policy adapters. `aoi_frame::deliver_sector_sessions` owns the common rebuild -> session delivery -> stale-player cleanup loop; adapters inject only transport callbacks. AoI consumers read through the storage-free `SectorView` boundary while the owner split preserves ADR-0049 recovery semantics |
| `dawn-server` | library + package with binaries | Single server composition boundary. The library owns the shared `ClientConnection`/WebSocket framing, handshake, admission session, and in-process transport seams. `runtime_frame::RuntimeFrameHost` owns one Sector's authoritative node, journal, consensus adapter, health, and durability policy. `simulate` owns local benchmarks/demos/playtest modes; `sector-node` owns the production peer-connected process. Both select adapters around the same Host frame and neither defines a second Tick ordering (ADR-0051, #338) |

### Dependency DAG

~~~text
dawn-core
  +-- dawn-client-core
  |   \-- dawn-client-gdext
  +-- dawn-protocol
  +-- dawn-ecs
  +-- dawn-storage
  +-- dawn-distributed
  |   +-- Raft and replication policies
  |   \-- shared versioned peer transport
  +-- dawn-market
  +-- dawn-sector
  \-- dawn-server
      +-- simulate
      \-- sector-node
~~~

dawn-client-gdext also depends directly on dawn-protocol. The server-owned
client transport and the current dawn-sector runtime package consume the same
protocol authority.
dawn-distributed consumes dawn-storage for recovery-range adapters; the
physical peer transport itself remains policy-agnostic. dawn-server is the only
composition root and depends on the runtime-facing packages above.

Dependencies flow bottom-to-top only; any reverse or circular dependency is a
design failure. dawn-server is the only composition root. SimulationNode and
the transition engine do not own a journal or transport; the remaining
protocol, recovery, and transit adapter modules in dawn-sector are invoked by
server-owned orchestration and are not alternate persistence authorities.

### Rule for adding crate dependencies

`dawn-core` may only depend on:

```text
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
| **Sector** | A spatial partition and authoritative mutation/recovery scope | [ownership.md](./ownership.md) |
| **Node** | A runtime process/peer that may own or replicate Sectors | [ownership.md](./ownership.md) |
| **Ship** | The only Entity kind (MVP) | [entity-model.md](./entity-model.md) |
| **Tick** | Logical time unit, unrelated to wall-clock time | [tick-model.md](./tick-model.md) |
| **DomainEvent** | An immutable public/business fact about a committed transition | [event-catalog.md](./event-catalog.md) |
| **RecoveryDelta** | Versioned authoritative outcome record used for exact Sector recovery | [recovery-contract.md](./recovery-contract.md) |
| **Checkpoint** | Versioned complete recovery point at one authoritative journal position | [recovery-contract.md](./recovery-contract.md) |
| **Command** | A change request that may be rejected | [event-catalog.md](./event-catalog.md) |

---

## 5. Data flow overview

### Normative transition flow (ADR-0049 / #284)

```text
Receive input (Command / Tick / committed distributed input)
    |
    v
Validate + prepare bounded transition
    | reject -> no authoritative transition
    v
RecoveryDelta + public DomainEvents + reliable/runtime effects
    |
    v
Durably commit logical transition through #271 journal semantics
    | durable failure -> discard prepared mutation; no success
    v
Apply committed RecoveryDelta / prepared mutation to local live state
    |
    v
Apply required local projections/repositories idempotently
    |
    v
Publish public events / replication / reliable effects
    |
    v
Acknowledge after the selected durability + local-apply conditions
```

The concrete `SimulationNode` no longer owns a public-event store. The shared runtime
frame owns bounded prepare -> durable append -> live apply -> reconciliation ->
output ordering; local and production adapters select their consensus, journal,
durability policy, and repository ports. #275 now makes the remaining mutable
state owners explicit, while remote replicated durability and catch-up transport
belong to #280.

Command and `DomainEvent` remain separate types (INV-006), and neither is an accidental substitute for `RecoveryDelta`. See [recovery-contract.md](./recovery-contract.md), [tick-model.md](./tick-model.md), and [event-catalog.md](./event-catalog.md).

---

## 5-A. ClientConnection abstraction

The connection between client (Godot) and server (Rust) is abstracted behind a trait, so swapping the implementation for networking requires no Godot-side changes.

```text
Test:                             Production:
  InProcessConnection              WsClientConnection
  via in-memory channel            via WebSocket, postcard binary for
                                    fixed-type messages (ADR-0007, ADR-0042)

  Both implement the same ClientConnection trait

ADR-0007 ruled out a move to gRPC for the client-facing transport; that
holds (ADR-0042). The inter-node distributed-comms trigger it named
already fired, but was resolved by a separate TCP+postcard transport
(the peer transport control/bulk adapters) — it never applied to this
client<->server connection.
```

### Trait responsibility (these two directions only)

```text
Server -> Client : stream of committed public/projection messages
Client -> Server : Command submission
```

No other responsibility may be mixed into this trait. Connection-state management, auth, and reconnection belong to higher layers. Client transport messages do not define server recovery authority.

### Data flow

```text
Sector runtime/application layer
    | committed public events / projection messages
delivery / AoI / client connection
    |
ClientConnection (InProcess / WebSocket)
    |
Godot client
    ^ Command
```

The implementation lives in `dawn-server::client_connection` and
`dawn-server::ws_server`; see ADR-0005 (trait) / ADR-0007 (WebSocket session)
for the historical design decisions.

---

## 5-B. Design direction for productization

To grow the current technical foundation into an EVE-like 3D game, the following concepts are treated as design assumptions now; implementation follows the roadmap phases.

### Interest Management (AoI) — implemented in 8C (ADR-0019)

Critical: without this there is no real game.

```text
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

CQRS/public projections are distinct from exact recovery.

```text
Write side (current durable architecture):
  Input -> Prepare -> Durable Recovery Transition -> Live Apply -> DomainEvents

Read/projection side:
  committed DomainEvents / authoritative SectorView
      -> Projection -> Read Model
                        ├── SpatialIndex (proximity queries)
                        ├── ShipStateView (current Ship state)
                        └── SectorOccupancyView (Sector population)
```

A projection may be rebuildable from public Events when its contract says so. That statement does **not** imply that every exact ECS/Player/queue field is reconstructible from public Event history. Exact recovery follows ADR-0049.

### Client connection model

```text
Server (authoritative)         Client (presentation)
─────────────────────          ──────────────
Holds committed true state      Holds display/predicted state
     |                               |
     | Receive/validate Command      | Client-side prediction
     | Durable + live apply          | (look-ahead to hide latency)
     | Deliver committed output  ->  | Reconciliation
     |                               | (corrects prediction)
```

The server remains authoritative. The client can predict ahead of confirmation and reconciles against committed server state/messages. Client prediction is explicitly outside authoritative recovery (#284 non-goal).

### Bounded Context expansion order

```text
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

## 5-C. Persistence and recovery model (ADR-0017 amended by ADR-0049)

There are now two distinct persistence concerns that must not be conflated:

1. **exact operational Sector recovery** — ADR-0049 authoritative checkpoint + state-delta tail;
2. **public `DomainEvent` history** — append-only public facts with hot/cold archival semantics inherited from ADR-0017.

### Exact recovery

```text
Recovery = newest complete compatible versioned checkpoint
         + every contiguous committed authoritative RecoveryDelta after it
```

The checkpoint is an authoritative recovery point. Eventless Ticks and no-public-event authoritative commands still advance the recovery journal. Position, capacitor, lock/module counters, active-ship routing, pending bot commands, and other authoritative fields are represented by checkpoint/delta recovery even when public events are silent.

Recovery does not rerun historical Ticks to guess exact outcomes. Local live apply, restart recovery, and replica catch-up use the same RecoveryDelta semantics/invariants.

### Public Event history

Committed `DomainEvent`s remain append-only facts. DurableJournal retains their
`JournalStream::PublicEvent` entries, while `PublicEventTail` is a bounded,
rebuildable read model:

```text
DurableJournal PublicEvent entries -> PublicEventTail -> replication/catch-up
```

ADR-0049 does not authorize destructive in-place public-fact mutation. Recovery
compaction and public-event tail retention have distinct cursors; a cursor older
than the retained tail base must use snapshot catch-up.

### Checkpoint/compaction/catch-up

- checkpoints persist independent `covered_recovery_index` and
  `public_event_next_index` cursors; eventless RecoveryDelta transitions do not
  advance the public-event cursor;
- replica snapshot installation and public suffix catch-up use only the public
  event cursor, while recovery coverage remains checkpoint metadata;
- checkpoint formats become explicitly versioned/fingerprinted under #284/#271;
- replacement checkpoint publication remains write/validate/sync/atomic-select before old recovery material is retired;
- #271 owns physical journal/commit/compaction mechanics;
- #276 ensures Transit Saga/retry authority survives checkpoint/compaction;
- #280 transports the selected checkpoint/tail representation and cannot promote a staged-but-unapplied replica;
- numeric RTO remains deployment-specific until a reference recovery benchmark
  and deployment policy are selected.

See [ADR-0017](../adr/ADR-0017-snapshot-compaction.md), [ADR-0049](../adr/ADR-0049-sector-recovery-state-delta-wal.md), and [recovery-contract.md](./recovery-contract.md).

---

## 6. Current constraints (changing these requires an ADR)

| Constraint | Reason | Basis (ADR) |
|---|---|---|
| Inter-node network is TCP LAN plaintext only (no TLS/QUIC) | Current 8D milestone stage; encryption is next phase | [ADR-0003](../adr/ADR-0003-local-first-development.md) / #280 transport refactor |
| Sector count/assignment is fixed (no dynamic split/merge) | MVP scope limit | [entity-model.md §5](./entity-model.md) |
| Ship is the only entity (includes Fitting / Combat / Capacitor) | MVP scope limit | [AI_DEVELOPMENT_GUIDE.md "Project North Star"](../../AI_DEVELOPMENT_GUIDE.md) |
| Exact recovery model is state-delta + versioned checkpoint | #284 accepted architecture decision | [ADR-0049](../adr/ADR-0049-sector-recovery-state-delta-wal.md) |

> §3's crate table/DAG describes the current implementation topology after
> #272's prepared-transition boundary and #280's shared peer transport landed.

---

## 7. Update rules for this document

Update this file for:

- Crate addition/removal or dependency-direction change
- Addition/removal of a key cross-cutting concept
- Scope changes from phase progression
- Accepted architecture changes that would otherwise make this entry point direct readers to a superseded model

Detailed subsystem rules still belong in their dedicated documents.
