---
id      : ADR-0049
title   : Exact Sector recovery with a versioned state-delta journal
status  : accepted
date    : 2026-08-06
deciders: [human, ai-agent]
related : ADR-0001 (Event Sourcing), ADR-0017 (snapshot compaction), ADR-0021 (Sector-local replication), ADR-0027 (replication crate), ADR-0038 (Station inventory SQLite)
---

# ADR-0049 - Exact Sector recovery with a versioned state-delta journal

## Context

The current Sector runtime mutates authoritative ECS and aggregate state during a
command or Tick, then appends the resulting public `DomainEvent`s. That ordering
cannot make an append failure safe because live state has already changed.

The public event catalog is not a complete recovery representation. A Tick may
advance position, capacitor charge, module cycle counters, lock countdowns,
flight-mode state, bot state, pending bot commands, and the logical Tick while
emitting no public event. A snapshot plus a public-event tail therefore cannot
reproduce an arbitrary acknowledged state between checkpoints.

Input-only deterministic replay is not sufficient either. Combat uses process
randomness, and exact execution depends on iteration, floating-point, catalog, and
implementation details that are not a versioned recovery contract. Event-sourcing
every ECS implementation detail would also couple public business facts to a
high-volume internal mutation log.

Issue #284 requires one explicit model before #271 defines the durable journal
payload and #272 freezes the engine transition boundary.

## Normative precedence and amendments

This ADR is accepted and, together with
`docs/architecture/recovery-contract.md`, is normative for Sector recovery,
durability, acknowledgement, replication catch-up, and transition ordering.
Where an older document conflicts with this contract, this ADR takes precedence.
Specifically, this ADR:

- amends ADR-0001: `DomainEvent`s remain durable public/business facts and audit
  history, but they are not the sole authority for exact operational state;
- supersedes the ADR-0017 claim that operational recovery is a snapshot plus a
  public-event tail or Tick re-execution; operational recovery is a compatible
  authoritative snapshot plus a committed state-delta tail;
- supersedes the ADR-0038 claim that SQLite is the independent durable authority
  for Station inventory and that a journal/SQLite inconsistency window is
  acceptable; the Sector journal is the authority and SQLite is an idempotent
  projection;
- amends INV-001/INV-002/INV-005, `tick-model.md`, and `event-catalog.md` wherever
  they equate event append with the complete state commit, claim public-event-only
  recovery, or treat an eventless Tick as having no durable record; and
- leaves append-only public-event semantics intact. State-delta compaction is a
  separate recovery-stream operation and never rewrites a committed public event.

The older normative documents are amended in the same PR. A future change may not
reintroduce the superseded ordering or recovery claims without a new ADR.

## Decision

Operational Sector recovery uses a **versioned authoritative state-delta journal
plus periodic full checkpoints**.

Public/business `DomainEvent`s are a separate logical output and are not the
authority for exact state reconstruction. They are nevertheless durable facts:
the events produced by a transition are committed atomically with its recovery
delta and reliable effect intents. This prevents a crash after durable state
commit but before publication from deleting audit or delivery history.

### Transition contract

Every accepted authoritative operation produces one logical transition:

```text
current committed state
    -> prepare(input)
    -> RecoveryDelta + DomainEvents + RuntimeEffects + PreparedMutation
    -> atomically commit DurableTransitionBatch
         { RecoveryDelta, DomainEvents, durable effect intents }
    -> apply PreparedMutation / RecoveryDelta to live state and local projections
    -> publish DomainEvents / RuntimeEffects / acknowledgement
```

An operation includes:

- one admitted client request;
- one simulation Tick, including a Tick that emits no `DomainEvent`;
- one committed Transit, admission, or Station transaction; or
- another explicitly durable Sector mutation.

A failed durable append does not commit the prepared mutation, publish outputs,
advance the externally visible Tick, or acknowledge success.

After a durable append succeeds, the transition cannot be rejected or forgotten.
If live application returns an error, panics, or can only partially apply, the
Sector immediately enters a **fail-stop fenced state**. While fenced it must not:

- accept or prepare another authoritative transition;
- replicate a later range;
- publish the transition's events or runtime effects;
- acknowledge success; or
- serve authoritative state as healthy.

The process must terminate or recover from the journal before resuming. Recovery
reapplies the committed delta through the same reducer. Continuing from the old
live state is forbidden.

### Durable transition batch and physical journal contract

`DurableTransitionBatch` is one **logical atomic commit envelope**, not one
indivisible retention blob. The storage format consists of immutable subrecords:

- an authoritative state-delta subrecord;
- a public-event subrecord, possibly empty;
- a durable outbox/effect-intent subrecord, possibly empty; and
- a commit envelope containing transition identity, positions, lengths, hashes,
  schema/catalog fingerprints, and the visibility commit marker.

The writer may place those subrecords in separate append-only segment families,
but no reader may observe any of them as committed until all referenced bytes and
the commit marker are durable. Recovery, publication, replication, and outbox
readers enumerate only committed envelopes.

The transition payload contains, at minimum:

- format/schema version;
- Sector identity and catalog/schema fingerprint;
- transition identity and covered journal range;
- Tick before and after the transition;
- stable ordered create/delete operations;
- final-value component/aggregate patches for every authoritative field changed;
- counter/map/set/queue changes that are not ECS components;
- the transition's public `DomainEvent`s in stable order, possibly empty;
- durable outbox/effect intents when an external action must survive a crash; and
- enough metadata to reject a duplicate, gap, incompatible version, wrong Sector,
  or wrong catalog.

Deltas record authoritative outcomes, not only commands or RNG inputs. Recovery
does not depend on reproducing the current RNG algorithm, hash iteration order,
floating-point execution path, AI implementation, or catalog version.

The canonical encoded order is deterministic. Entity/component patches are sorted
by stable domain identity and component tag; unordered maps, sets, and queues are
serialized through stable ordered views. A whole-world clone is not required: an
implementation may use a reversible mutation plan, write set, copy-on-write pages,
or another bounded preparation mechanism.

### Independent retention and crash-safe compaction

Atomic commit does not force all substreams to share one retention watermark.
After a validated checkpoint covers a committed position:

- state-delta subrecords at or before that position may be removed from the hot
  recovery stream;
- public-event subrecords remain until their audit/archive retention policy allows
  removal from the hot tier;
- outbox subrecords remain until every required delivery consumer has durably
  advanced beyond them; and
- the compact commit index retains enough metadata to prove which transition
  ranges are covered by the checkpoint and which output subrecords still exist.

Compaction is copy-and-publish, never in-place mutation:

1. write and validate the new checkpoint or compacted segment set;
2. fsync all new files and the new manifest;
3. atomically publish the manifest that selects them;
4. only then retire old state-delta segments; and
5. retain old files until recovery can no longer select the previous manifest.

A crash before manifest publication leaves the previous recovery path intact. A
crash after publication leaves the new checkpoint plus retained tails intact.
Public-event and outbox segments cannot be retired merely because state is covered
by a checkpoint.

### Eventless Ticks, pending bot commands, and auto-jump

Every committed Tick writes a recovery transition even when no public event is
emitted. Tick advancement and all authoritative changes are covered by a journal
position. `events_emitted == 0` never means that nothing durable happened.

The pending bot lock-command queue is authoritative because it changes the next
Tick's outcome. Until the Tick pipeline is redesigned to consume those commands in
the same transition, the queue is included in snapshots and in queue deltas.

`pending_auto_jumps` is **not** allowed to remain a crash-lossy in-memory handoff.
When a Tick completes an `auto_jump` Warp, that same durable transition records an
`AutoJumpProposalIntent` (or equivalent) in the outbox with the Ship, gate,
destination/routing data needed for retry, and the originating transition identity.
The post-commit runtime may drain an in-memory convenience queue, but that queue is
only a projection of the durable intent. A crash after Warp arrival and before the
Raft proposal must cause the intent to be retried after recovery, not silently lost.
The Raft proposal path must deduplicate by the durable transition/idempotency
identity so an ambiguous crash can produce a duplicate proposal attempt but not a
duplicate ownership transition.

### Station inventory authority

Station inventory participates in the same Sector transition and recovery journal.
The authoritative Station item delta is stored in the same committed envelope as
ECS, ownership, and public outputs. There is no SQLite/journal two-phase commit and
no accepted one-sided durability window.

SQLite remains the bounded, lazily loaded query projection used by the runtime. It
is updated idempotently by `(sector_id, transition_id)` after journal commit. A
Station transition is acknowledged only after:

1. the journal envelope is durable under the selected durability profile;
2. the live authoritative reducer succeeds; and
3. the local SQLite projection has applied that transition idempotently.

If SQLite application fails after durable append, the Sector follows the same
fail-stop rule and rebuilds or catches up the projection from a versioned Station
aggregate checkpoint plus committed Station deltas. Reapplying a transition must
be a no-op after the same `transition_id` is recorded.

To avoid loading all Station inventories into the ECS snapshot, a Sector checkpoint
manifest may reference a separate versioned Station aggregate checkpoint. The ECS
snapshot, Station checkpoint, and covered journal position form one validated
checkpoint set; compaction is forbidden until the complete set is durable and
mutually consistent.

### Snapshots and checkpoint manifests

A checkpoint is a versioned full authoritative recovery point. Its manifest
contains or references:

- explicit magic and format versions;
- Sector identity and catalog/schema fingerprint;
- the exact committed journal position it covers;
- the ECS/aggregate snapshot;
- the Station aggregate checkpoint when Station state exists;
- checksums and lengths for every member; and
- output-retention metadata needed to locate retained event/outbox segments.

Snapshots contain authoritative state, including the pending bot command queue,
but no runtime handles, sockets, transient channel buffers, or already-executed
external effects. Reliable post-commit actions such as auto-jump Raft proposals are
represented by retained outbox intents rather than by snapshotting runtime queues.

Recovery loads the newest complete compatible checkpoint set and applies every
complete committed recovery batch after its covered position. A missing range,
duplicate, wrong Sector, wrong fingerprint, corrupt member, or incompatible version
fails recovery explicitly.

### Durability profiles, RPO, acknowledgement, and retries

"RPO 0" is always stated together with a failure domain. #271 must expose the
configured durability profile and cannot acknowledge a transition before that
profile's durable-commit condition is satisfied.

#### LocalDurable

The commit envelope, all referenced subrecords, and required metadata are written,
flushed, and `fsync`/equivalent durable before acknowledgement. This profile gives
**acknowledged RPO 0 for process crash, OS crash/reboot, and abrupt power loss while
the authoritative storage medium remains readable and preserves completed durable
writes**. Catastrophic loss or corruption of that device/machine is outside this
profile's RPO promise.

#### ReplicatedDurable

In addition to `LocalDurable`, the committed envelope/range is synchronously made
durable on the configured replica quorum before acknowledgement. This profile may
claim **acknowledged RPO 0 for owner-node or owner-storage loss only up to the
explicitly configured replica-failure tolerance**. #271/#280 must define the quorum,
ack evidence, and tolerated failures before production documentation makes that
stronger claim.

A deployment must not use an ambiguous phrase such as "production journal mode"
in place of these failure-domain semantics.

- **Unacknowledged operation:** may be absent or durably committed after a crash.
  Retried operations that require exactly-once semantics use a stable idempotency
  or transition identity.
- **RTO:** not yet a numeric production guarantee. Recovery procedure is defined,
  but maximum tail bytes, transition count, replay duration, reference hardware,
  and checkpoint cadence remain to be benchmarked in #284. Until that benchmark
  lands, documents and PRs must not claim a measured RTO.

#271 must expose tail-size and replay-time measurements so checkpoint cadence can
later enforce the selected RTO budget rather than relying on genesis replay.

### Durable delivery cursors and external effects

Public-event and outbox delivery is **at-least-once** unless the downstream system
provides a stronger idempotent transaction protocol. For every durable consumer:

1. read the next committed output after the durable cursor;
2. attempt delivery with the transition/output idempotency identity;
3. receive a downstream acknowledgement or equivalent durable idempotency proof;
4. durably advance that consumer's cursor only after step 3; and
5. allow compaction past the output only after every required consumer/archive
   watermark has durably advanced.

Advancing a cursor before downstream acknowledgement is forbidden because a crash
could skip an undelivered committed output. Advancing after acknowledgement means a
crash can repeat delivery before the cursor update becomes durable; consumers must
therefore tolerate duplicates or use the provided idempotency identity. The Sector
does not claim exactly-once external side effects from a local cursor alone.

A reliable Raft proposal, including auto-jump, follows the same outbox rule. A
fire-and-forget runtime effect that is deliberately not recoverable must be
explicitly classified as such and cannot be required for authoritative continuity.

### Replication, catch-up, and promotion

Replication publishes only committed envelopes/ranges. A replica applies the same
recovery reducer used by local recovery before exposing new authoritative state.

A snapshot-based catch-up bundle must include:

- the complete checkpoint set and covered journal position;
- every committed authoritative tail record after that position;
- every retained public-event and outbox segment the replica may need after
  promotion;
- durable publication/outbox consumer cursors or an equivalent replicated cursor
  state; and
- enough Station checkpoint/delta information to bring the replica's local SQLite
  projection to a known transition watermark.

A replica is not promotable/servable as healthy until it can prove all of the
following at the promotion position:

1. the authoritative recovery range is contiguous and invariant-valid;
2. no committed public event or durable effect intent required after promotion is
   missing;
3. durable consumer cursors cannot skip an undelivered retained output; and
4. the local Station SQLite projection watermark is at least the promoted
   authoritative position, or the projection has been rebuilt to that position
   before any Station read/write is served.

A replica that has current ECS state but stale output segments, cursors, or SQLite
projection remains catch-up-only. Promotion must not expose stale Station inventory.

Public events are published only after local state commit. Redirects, client
projections, loadout refreshes, Raft proposals, and other runtime effects execute
only after local commit. Effects requiring reliable retry have a durable outbox
intent in the same envelope or use an explicitly idempotent external protocol.

### Terminology

- **Recovery record/delta:** authoritative persistence representation used to
  reconstruct exact committed Sector state.
- **Domain event:** durable public/business fact produced by a committed transition;
  stored atomically with it but not used as the exact state reducer.
- **Runtime effect:** post-commit non-state action; reliable effects have a durable
  outbox intent or an idempotent protocol.
- **Checkpoint:** validated manifest plus all authoritative snapshot members at one
  committed journal position.
- **Replay:** applying authoritative recovery deltas to a compatible checkpoint.
  Genesis public-event replay is only an explicitly supported audit/projection
  operation, not the operational recovery contract.

## Alternatives considered

### Event-source every authoritative mutation as public events

This can provide exact replay but would make the public event catalog mirror ECS
implementation details and generate high-volume events for position, capacitor,
cycle counters, lock countdowns, queues, and other per-Tick state. A dedicated
state delta keeps recovery mechanics separate from business facts.

### Journal deterministic inputs and rerun the Tick

This is compact, but the current engine does not expose a versioned guarantee for
randomness, floating-point behavior, iteration order, AI logic, or catalog changes.
Old-tail replay would remain code-version-sensitive.

### Bounded rollback to the last checkpoint

This is operationally simpler but allows an acknowledged command or Tick to vanish
after a crash, conflicting with authoritative simulation and replication semantics.

### Independent SQLite authority with cross-store reconciliation

A recoverable participant protocol could preserve SQLite authority, but it would
introduce prepare/commit records, reconciliation, timeout, and acknowledgement
semantics across two stores. Using the Sector journal as the single authority and
SQLite as an idempotent projection is simpler and preserves one transition commit
boundary.

### One indivisible physical batch retained to the slowest consumer

This preserves atomicity but retains high-volume state deltas as long as the
slowest audit or effect consumer. Logical atomic envelopes over independently
retained immutable substreams preserve the commit boundary without coupling their
retention lifetimes.

## Consequences

- #271 must store generic/versioned logical transition envelopes atomically and
  support immutable state, event, and outbox substreams plus a committed index.
- #271 must expose `LocalDurable`/`ReplicatedDurable`-equivalent commit evidence so
  acknowledgement has an explicit failure domain.
- #272 must expose prepare -> persist -> commit, include Station aggregate changes,
  and implement fail-stop fencing after post-append application failure.
- ADR-0038's SQLite authority becomes a projection contract; Station writes are
  journal-authoritative and idempotently projected.
- Auto-jump Raft proposals become reliable outbox work tied to the Warp transition;
  an in-memory queue alone cannot represent the obligation.
- Snapshot schemas and checkpoint manifests become explicitly versioned and
  fingerprinted.
- Recovery and replica application share one reducer for authoritative deltas.
- Replica promotion requires both retained outputs/cursors and a caught-up Station
  projection, not only ECS state equivalence.
- Durable output delivery is at-least-once with cursor advance after downstream
  acknowledgement; exactly-once requires downstream idempotency/transactions.
- Public event evolution no longer silently changes the recovery contract, while
  committed facts cannot be lost between state commit and publication.
- Numeric RTO remains an open benchmark deliverable in #284.

## Implementation sequence

1. Land this accepted decision, the authoritative-state inventory, and crash matrix,
   while amending conflicting normative documents in the same PR.
2. In #271, introduce the versioned commit envelope, immutable substreams, atomic
   commit marker, append receipt, durability-profile evidence, failure injection,
   and independent retention manifests.
3. Add a versioned checkpoint manifest covering ECS and Station aggregate state.
4. In #272, implement one command vertical slice with a bounded prepared write set,
   append-before-commit, idempotent SQLite projection, and fail-stop tests.
5. Extend the transition boundary to eventless Ticks and persist the pending bot
   lock-command queue.
6. Convert auto-jump to a durable outbox intent and make Raft proposal retry
   idempotent by transition identity.
7. Convert Transit, admission, replication, publication cursors, outbox delivery,
   and checkpointing.
8. Add snapshot-transfer bundles, Station projection watermarks, and promotion
   eligibility checks.
9. Benchmark tail replay and set a numeric RTO/checkpoint policy in #284.

## Implementation checklist

- [x] Select the recovery model and exact acknowledged RPO failure domains.
- [x] Classify current authoritative mutations and runtime-only state.
- [x] Define crash-point outcomes, acknowledgement ordering, and fail-stop behavior.
- [x] Select Station journal authority with idempotent SQLite projection.
- [x] Define independently retained atomic substreams and crash-safe compaction.
- [x] Define replica catch-up/promotion requirements for outputs and Station projection.
- [x] Define reliable auto-jump as durable outbox work.
- [x] Define at-least-once cursor advancement semantics.
- [ ] Benchmark and define a numeric production RTO/checkpoint budget in #284.
- [ ] Add generic atomic versioned durable transition envelopes in #271.
- [ ] Persist public events/outbox intents atomically with each transition.
- [ ] Add versioned/fingerprinted checkpoint manifests and compatibility tests.
- [ ] Introduce the prepare -> persist -> commit engine boundary in #272.
- [ ] Make eventless Ticks and pending bot command queues durable.
- [ ] Persist/retry auto-jump outbox intents idempotently.
- [ ] Apply committed deltas through one local/recovery/replica reducer.
- [ ] Add full crash-point and checkpoint-plus-tail equivalence tests.
