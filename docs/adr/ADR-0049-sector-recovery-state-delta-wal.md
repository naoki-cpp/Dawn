---
id      : ADR-0049
title   : Exact Sector recovery with a versioned state-delta journal
status  : proposed
date    : 2026-08-06
deciders: [human, ai-agent]
related : ADR-0001 (Event Sourcing), ADR-0017 (snapshot compaction), ADR-0021 (Sector-local replication), ADR-0027 (replication crate)
---

# ADR-0049 - Exact Sector recovery with a versioned state-delta journal

## Context

The current Sector runtime mutates authoritative ECS and aggregate state during a
command or Tick, then appends the resulting public `DomainEvent`s. That ordering
cannot make an append failure safe because live state has already changed.

The public event catalog is also not a complete recovery representation. A Tick
may advance position, capacitor charge, module cycle counters, lock countdowns,
flight-mode state, bot state, and the logical Tick while emitting no public event.
The current snapshot stores some of that state, but a snapshot-plus-public-event
tail cannot reproduce an arbitrary acknowledged state between checkpoints.

Input-only deterministic replay is not currently sufficient either. Combat uses a
process RNG, and exact execution also depends on iteration/order details that are
not part of a versioned recovery contract. Event-sourcing every internal component
mutation would turn presentation/business facts into a high-volume implementation
log and couple the public event catalog to ECS layout.

Issue #284 requires one explicit model before #271 defines the durable journal
payload and #272 freezes the engine transition boundary.

## Decision

Operational Sector recovery uses a **versioned authoritative state-delta journal
plus periodic full snapshots**.

Public/business `DomainEvent`s remain a separate output. They are useful for
client facts, audit, analytics, and integrations, but they are not the authority
for exact state reconstruction.

### Transition contract

Every accepted authoritative operation produces one logical transition:

```text
current committed state
    -> prepare(input)
    -> RecoveryBatch + DomainEvents + RuntimeEffects + PreparedMutation
    -> atomically append RecoveryBatch
    -> commit PreparedMutation to live state
    -> publish DomainEvents / RuntimeEffects / acknowledgement
```

An operation includes:

- one admitted client request;
- one simulation Tick, including a Tick that emits no `DomainEvent`;
- one committed Transit/admission/station transaction; or
- another explicitly durable Sector mutation.

A failed durable append does not commit the prepared mutation, publish its
outputs, advance the externally visible Tick, or acknowledge success.

### Recovery batch

The journal payload is a versioned `RecoveryRecord` envelope whose transition
variant contains, at minimum:

- format/schema version;
- Sector identity and catalog/schema fingerprint;
- transition identity and covered journal range;
- Tick before and after the transition;
- stable ordered create/delete operations;
- final-value component/aggregate patches for every authoritative field changed;
- counter/map/set changes that are not ECS components;
- enough metadata to reject a duplicate, gap, incompatible version, or wrong
  Sector/catalog; and
- optional public `DomainEvent`s only when co-locating them is operationally
  useful. Their presence does not make them the recovery authority.

Deltas record authoritative **outcomes**, not merely commands or RNG inputs.
Therefore recovery does not depend on reproducing the current RNG algorithm,
hash iteration order, floating-point execution path, or AI implementation.

The canonical encoded order is deterministic. Entity/component patches are sorted
by stable domain identity and component tag; unordered maps/sets are serialized
through ordered views. A whole-world clone is not required: an implementation may
use a reversible mutation plan, component-level write set, copy-on-write pages, or
another bounded preparation mechanism.

### Eventless Ticks

Every committed Tick writes a recovery transition even when no public event is
emitted. Tick advancement and any transient-state changes are therefore covered
by a journal position. `events_emitted == 0` never means "nothing durable
happened".

### Randomness

The exact post-transition state is captured in the delta. Random draws may also be
recorded for debugging or causal audit, and a deterministic seeded RNG may be
introduced later, but neither is required to reconstruct committed state.

### Snapshots

A snapshot is a versioned full authoritative checkpoint and contains:

- explicit magic and format version;
- Sector identity and catalog/schema fingerprint;
- the exact committed journal position it covers;
- all authoritative ECS components and aggregate maps/counters needed to resume;
- no runtime handles, sockets, queues, caches, or pending external effects.

The existing write/validate/sync/atomic-replace/rollback publication guarantees are
retained. Recovery loads the newest compatible snapshot and applies every complete
committed recovery batch after its covered position. A missing range, duplicate,
wrong Sector, wrong fingerprint, corrupt batch, or incompatible version fails
recovery explicitly.

### RPO, acknowledgement, and retries

- **Acknowledged RPO:** zero committed transitions. Once success is acknowledged,
  the corresponding recovery batch is durable according to the production journal
  mode selected by #271.
- **Unacknowledged operation:** may be absent or durably committed after a crash.
  Clients/runtime code must retry with an idempotency/transition identity where the
  operation requires exactly-once semantics.
- **RTO:** load the latest compatible snapshot, then apply the committed journal
  tail. Genesis replay is not an operational recovery promise.

### Replication and external effects

Replication publishes only committed recovery ranges/records. A replica applies
the same recovery reducer used by local recovery before exposing the new state.
Public events, redirects, client projections, loadout refreshes, Raft proposals,
and other runtime effects are emitted only after the local recovery batch commits.

Effects that require retry use durable intent/outbox state or an idempotent external
protocol; they are never inferred from an uncommitted live mutation.

### Terminology

- **Recovery record/delta:** authoritative persistence representation used to
  reconstruct exact committed Sector state.
- **Domain event:** public/business fact emitted by a committed transition.
- **Runtime effect:** non-state action performed after commit.
- **Snapshot:** full authoritative checkpoint at one committed journal position.
- **Replay:** applying recovery deltas to a compatible snapshot. "Genesis event
  replay" refers only to the explicitly supported audit subset.

## Alternatives considered

### Event-source every authoritative mutation

This can provide exact replay but would make the public event catalog mirror ECS
implementation details and generate high-volume events for position, capacitor,
cycle counters, lock countdowns, and other per-Tick state. A dedicated state delta
keeps recovery mechanics separate from business facts.

### Journal deterministic inputs and rerun the Tick

This is compact, but the current engine uses process randomness and does not expose
a versioned guarantee for floating-point behavior, iteration order, AI logic, or
catalog changes. It would require freezing more implementation details than the
state-delta model and makes old-tail replay sensitive to code changes.

### Bounded rollback to the last snapshot

This is operationally simpler but means an acknowledged command or Tick can vanish
after a crash. That conflicts with the intended authoritative simulation and
replication semantics.

### Full snapshot for every transition

This is straightforward but creates unacceptable write amplification. The selected
model retains periodic snapshots and journals bounded deltas between them.

## Consequences

- #271 must store generic/versioned recovery batches atomically rather than
  hard-coding `DomainEvent` as the complete payload.
- #272 must expose prepare -> persist -> commit and keep runtime effects outside
  the pure engine.
- Snapshot schemas become explicitly versioned and fingerprinted.
- Existing documentation that equates public-event append with complete Tick
  durability must be revised.
- Recovery and replica application share one reducer for authoritative deltas.
- Public event evolution no longer silently changes the recovery contract.

## Implementation sequence

1. Land this decision, the mutation/durability inventory, and crash matrix.
2. In #271, introduce the versioned batch envelope, atomic append receipt, and
   failure-injectable journal without assuming a concrete ECS delta layout.
3. Add an explicit versioned snapshot envelope and compatibility checks.
4. In #272, implement one command vertical slice with a bounded prepared write set,
   append-before-commit, and failure tests.
5. Extend the same transition boundary to eventless and event-producing Ticks.
6. Convert Transit, admission, station operations, replication, and checkpointing.
7. Remove the old `SimulationNode<S: EventStore>` ownership and public-event replay
   claims that exceed the selected contract.

## Implementation checklist

- [x] Select the recovery model and define RPO/RTO.
- [x] Classify current authoritative mutations and runtime-only state.
- [x] Define crash-point outcomes and acknowledgement ordering.
- [ ] Add generic atomic versioned recovery batches in #271.
- [ ] Add versioned/fingerprinted snapshots and incompatible-format tests.
- [ ] Introduce the prepare -> persist -> commit engine boundary in #272.
- [ ] Make eventless Ticks durable transitions.
- [ ] Apply committed deltas through one local/recovery/replica reducer.
- [ ] Add full crash-point and snapshot-plus-tail equivalence tests.
