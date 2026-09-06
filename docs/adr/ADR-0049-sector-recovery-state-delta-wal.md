---
id      : ADR-0049
title   : Exact Sector recovery with a versioned state-delta journal
status  : accepted
date    : 2026-08-06
deciders: [human, ai-agent]
related : ADR-0001 (Event Sourcing), ADR-0014 (Raft / Transit), ADR-0017 (snapshot compaction), ADR-0021 (Sector-local replication), ADR-0027 (replication crate), ADR-0038 (Station inventory SQLite)
---

# ADR-0049 - Exact Sector recovery with a versioned state-delta journal

> **Implementation correction (2026-08-26, issue #344):** the #277 repository
> boundary is now physically split into private admission, identity, and
> Station projection modules. `SectorRepository` remains the sole SQLite
> connection owner and `SectorTransaction` remains the one explicit
> cross-view transaction boundary; the root only coordinates schema, shared
> codecs, views, and transaction creation. This changes no authority, schema,
> projection, or external repository behavior.

> **Implementation correction (2026-08-26, issue #343):** the production
> `RuntimeNodeMutation` closure bridge has been removed. Authenticated client
> requests are collected into `FrameInput` and their typed
> `RuntimeCommandDispatch` values are exposed only through the successful
> `RuntimeTickOutput`, after durable append, live apply, and required
> reconciliation. Admission and checkpoint access use narrow typed host
> methods; bootstrap/fixture mutation remains phase-gated. Regression coverage
> verifies that durable-append or reconciliation failure fences the host and
> invokes neither post-commit output nor command acknowledgement.

> **Implementation correction (2026-08-22):** `TickRecoveryDelta` now carries
> ordered Station mutations and `RECOVERY_DELTA_VERSION` is 4. The production
> runtime applies the SQLite Station projection only after durable append and
> live apply, advances its cursor across the complete journal batch, and fences
> before publication on projection failure. Fresh-admission prepared rows are
> finalized from the committed `ClientAdmissionCommitted` record only after the
> starter grant projects; recovery decodes that record and repeats the same
> idempotent reconciliation. The production repository is attached before tail
> replay so catch-up never writes into the temporary in-memory adapter.

> **Implementation correction (2026-08-20):** checkpoints now persist
> independent authoritative-recovery and public-event cursors. Adding
> `public_event_next_index` changes the postcard checkpoint payload, so
> `CHECKPOINT_FORMAT_VERSION` was bumped to 4. This pre-release repository
> does not retain an upcaster for the superseded layout.

> **Implementation correction (2026-08-15, issue #312):** this ADR's
> accepted decision already required flight-mode state, lock countdowns, and
> module-cycle counters to be exact recovery authority (see the Context
> below and `recovery-contract.md` rows for thrust/flight modes, lock
> entries/countdowns, and fitted-slot cycle counters -- all "Yes,
> checkpoint"). The implementation did not satisfy this for the checkpoint
> path: `StateSnapshot` built its own thin `ShipSnapshot` list independently
> of the tick-rollback/`TickRecoveryDelta` capture, so a checkpoint restore
> silently dropped warp/orbit/approach progress, active locks, and module
> cycle timers, even though a tick-rollback restore already carried them
> correctly. `TickRecoveryDelta` also omitted `applied_market_settlements`,
> present only in `StateSnapshot`.
>
> Issue #312 closes this gap by giving tick-rollback, `TickRecoveryDelta`,
> and `StateSnapshot` one shared capture/restore path (`ShipState` for
> per-ship optional-component state, `NodeState` for node-level
> scalars/maps), sourced from a single declarative list of a ship's optional
> ECS components in `dawn-ecs`. `CHECKPOINT_FORMAT_VERSION` and
> `RECOVERY_DELTA_VERSION` were both bumped (pre-release, no upcaster
> required). This is an implementation fix, not a new decision -- the
> recovery contract this ADR already committed to is unchanged.

> **Implementation correction (2026-08-17):** `TickRecoveryDelta` and
> `StateSnapshot` now carry one nested canonical `NodeState` directly. Tick
> transition metadata and checkpoint identity/context plus ship images remain
> outside that node-level payload. The postcard layouts are incompatible with
> the prior flattened fields, so both explicit format versions were bumped;
> no pre-release upcaster is required.

> **Implementation correction (2026-08-16, issue #315):** this ADR's
> accepted contract requires committed Sector state to enter through the
> prepare → durable-journal-append → apply ordering that `RuntimeFrameHost`
> (`dawn-server`) exists to enforce. Market settlement delivery
> (`serve/market_settlement.rs`) did not satisfy this: it mutated
> `SimulationNode` cargo state synchronously via a generic `with_node_mut`
> closure, entirely outside any tick boundary, and only became durable
> retroactively once the *next* tick happened to capture the already-mutated
> live state. A crash in that window (up to ~100ms at 10Hz) could silently
> lose a settlement the client had already been told succeeded.
>
> Issue #315 closes this gap by extending the Tick pipeline's input with
> `FrameInput::market_settlements`: `SimulationNode::prepare_tick_state_transition_with_result`
> now applies queued Market settlements before capturing the tick's write
> set, so a settlement's effect is captured in the same `TickRecoveryDelta`
> and durable journal append as the rest of that tick, and rolls back with
> everything else if preparation does not lead to a committed apply.
> `serve/market_settlement.rs` was rewritten from a synchronous
> apply-and-acknowledge loop into a two-phase outbox drain
> (delivery scans and routes a bounded page before the tick;
> `acknowledge_outcomes` follows its commit) built on this substrate. The
> delivery owner advances the scan cursor, and clustered delivery routes the
> same page across participating Sectors before reading another page.
> Fixture/NPC spawn remains bootstrap-only.
> Fresh admission still begins through a typed synchronous host surface, but
> its materialized Ship and starter Station grant are now finalized through the
> next durable frame as described in the 2026-08-22 correction above. This is
> an implementation fix, not a new decision -- the recovery contract this ADR
> already committed to is unchanged.

## Context

The current Sector runtime mutates authoritative ECS and aggregate state during a
command or Tick, then appends resulting public `DomainEvent`s. That ordering cannot
make a storage failure safe because live state has already changed.

The public event catalog is also not a complete recovery representation. A Tick can
advance position, capacitor charge, module-cycle counters, lock countdowns,
flight-mode state, bot queues, logical Tick, Player routing state, and other
authority while emitting no public event. Snapshot + public-event tail therefore
cannot reconstruct every acknowledged intermediate state.

Input-only deterministic rerun is not an adequate versioned recovery contract
either. Current behavior can depend on process randomness, floating point,
iteration order, catalog version, AI implementation, and implementation details
that are not stable replay inputs.

Issue #284 requires one explicit authoritative recovery model before #271 freezes
journal mechanics and #272 freezes the engine transition boundary.

## Scope inside the destructive-refactor work package

This ADR owns the **recovery semantics selected by #284**. It intentionally does
not decide implementation details already assigned to sibling issues:

- **#271** owns fallible atomic journal framing, indices, commit markers,
  corruption/torn-write policy, durable-write evidence, and physical retention;
- **#272** owns the storage-independent engine transition API and bounded
  prepare -> durable append -> live apply migration;
- **#275** owns decomposition of `SimulationNode` into explicit state owners;
- **#276** owns the first-class durable Transit Saga/attempt/receipt model;
- **#277** owns repository/schema/transaction APIs for admission, identity, and
  Station persistence. Admission/identity protocol state and pre-materialization
  identity consumption may be repository-owned authority; Station world state
  remains journal-owned;
- **#278** owns runtime/application orchestration, including configured durability
  profile, replica-set/quorum policy, durability receipt aggregation, ownership
  epoch/fencing integration, and final acknowledgement gating;
- **#280** owns peer transport and the physical snapshot/catch-up/durability-message
  transfer path, not the quorum or recovery policy itself.

Those issues must consume this contract rather than redefine which state is
recoverable or what acknowledgement means.

## Normative precedence and amendments

This ADR and `docs/architecture/recovery-contract.md` are normative for Sector
recovery, acknowledgement, checkpoint coverage, and the classification of
state/outputs as authoritative, derived, reliable, or intentionally lossy.

It amends older decisions as follows:

- **ADR-0001:** `DomainEvent`s remain durable public/business facts, audit history,
  and projection inputs, but are not the sole exact operational recovery reducer.
- **ADR-0014:** Transit remains consensus-controlled, but legacy public-event scans
  and public Transit event pairs are not the final exact recovery/outbox authority;
  #276 must use this recovery contract for its durable Saga.
- **ADR-0017:** operational recovery is not snapshot + public-event tail or
  historical Tick rerun. It is compatible checkpoint + committed authoritative
  recovery tail. Public-event hot/cold archival remains a separate concern.
- **ADR-0038:** SQLite is not an independent **Station world-state** durability
  authority. Station mutations participate in Sector recovery authority;
  SQLite/repository Station state is an idempotent projection/read model under
  #277. This does not force pre-materialization admission/identity protocol state
  to be a Station projection; §6A defines that separate boundary.
- **INV-001/INV-002/INV-005, tick-model, architecture, ownership, database
  strategy, and event-catalog:** any statement equating public-event append with
  the complete transition commit or claiming public-event-only exact recovery is
  superseded.

Committed public `DomainEvent`s remain append-only facts. Recovery-delta checkpoint
compaction is a distinct stream operation and does not authorize in-place mutation
of public history.

## Decision

Operational Sector world-state recovery uses a **versioned authoritative state-delta
journal plus periodic versioned checkpoints**.

A committed transition records authoritative outcomes needed for exact recovery,
not merely commands or RNG inputs. The authoritative delta is distinct from public
business events, although outputs produced by the same transition share one logical
atomic visibility boundary.

Durable protocol state that can exist before or outside materialized Sector world
state, such as a prepared admission reservation, may live in an explicit repository
owned by #277/#276 as long as its authority, reconciliation, promotion, and RPO
relationship to the Sector journal are documented. A durable fresh-admission
reservation also permanently **consumes** its reserved `PlayerId` / `ShipId`; a
later abort may create a gap, but crash recovery must never make that identity
reusable. "Single Sector recovery model" does not mean every bounded-context
protocol row must be serialized into one file.

### 1. Transition contract

Every accepted authoritative Sector-world operation produces one logical transition:

```text
current committed state
    -> prepare(input)
    -> RecoveryDelta + DomainEvents + runtime/reliable effects + PreparedMutation
    -> make the logical transition durable under the selected durability profile
    -> apply PreparedMutation / RecoveryDelta to local live state
    -> apply required local projections/reconciliations
    -> publish public outputs / execute reliable effects
    -> acknowledge after all acknowledgement conditions are satisfied
```

An authoritative operation includes:

- an admitted client request that mutates authoritative state;
- one simulation Tick, including an eventless Tick;
- an admitted Player/Station/Transit lifecycle mutation; or
- another explicitly durable Sector mutation.

A rejected request that never changes authority does not need a recovery transition
merely to record rejection telemetry.

A failed durable append cannot commit the prepared live mutation, advance an
externally visible authoritative Tick, publish success outputs, or acknowledge
success.

After durable commit succeeds, the transition cannot be converted into ordinary
rejection. If local application or a required local projection/reconciliation
errors, panics, or partially applies, the Sector must fence/fail-stop and recover
from durable data before serving later authority.

### 2. Authoritative recovery payload

The versioned `RecoveryDelta` contains enough outcome information to reconstruct
the exact committed Sector-world authority without rerunning historical
implementation code. At minimum the recovery representation can encode:

- format/schema version and Sector identity;
- catalog/schema fingerprint required to validate recovered values;
- stable transition identity and authoritative position;
- Tick before/after where relevant;
- stable create/delete operations;
- final-value component/aggregate patches for changed authoritative state;
- counter/map/set/queue changes outside ECS components;
- Player ownership and **active-ship routing** changes;
- Station aggregate changes;
- Transit/Saga-facing authoritative changes required by #276;
- enough metadata to detect duplicate, missing, out-of-order, corrupt,
  wrong-Sector, or incompatible recovery records.

Canonical encoding/order must be deterministic for durable representation. Exact
physical framing belongs to #271.

A whole-world clone per transition is not required. #272 may use a reversible write
set, copy-on-write overlay, prepared values, or another bounded mechanism as long as
pre-durable live authority is not exposed and the committed result is equivalent to
applying `RecoveryDelta`.

### 3. Public `DomainEvent`s and reliable effects

`DomainEvent`s remain durable public/business facts when produced, but they are not
the exact state reducer. A transition may contain zero public events and still be
fully authoritative/durable.

Public events produced by a committed transition cannot disappear in a crash gap
between state durability and publication. The logical transition therefore gives
authoritative recovery data and its public outputs one atomic visibility boundary.
#271 decides how that logical boundary is physically framed and retained.

Likewise, a post-commit action that must survive a crash needs durable retry state
or a stronger explicitly idempotent external protocol. The concrete representation
may be:

- an outbox record inside the #271 logical transition representation;
- a #276 Transit Saga state/attempt record participating in the same documented
  recovery boundary; or
- another mechanism proven to provide the same crash/retry guarantee.

The recovery invariant matters more than the type name `Outbox`.

### 4. Eventless Ticks and queued future intent

Every committed Tick has a recovery position even when no `DomainEvent` is emitted.
`events_emitted == 0` never means "nothing durable happened".

The pending bot lock-command queue is authoritative while it survives across Tick
boundaries because it changes the next Tick's outcome. Until redesigned for same-
Tick consumption, it belongs in checkpoint/delta recovery data.

`pending_auto_jumps` is not allowed to be the sole representation of an already-
committed Warp continuation. When an `auto_jump` Warp arrival commits, durable
retry/idempotency state for continuing Transit must exist in the same recovery
boundary. The current implementation keeps that queued continuation in the
checkpointed recovery delta; a future extraction may represent it as a
`TransitAttemptId` Saga rather than a generic outbox intent.

### 5. Player routing state

`PlayerState.active_ship` is authoritative Player routing state, not deliberately
lossy session decoration. It changes:

- which owned Ship receives helm/module commands;
- whether Undock is legal; and
- what authoritative routing state a reconnect resumes into.

Therefore `SelectActiveShip` and `Disembark` can require a recovery transition even
when they emit no public `DomainEvent`. The local checkpoint/RecoveryDelta path
persists the routing maps explicitly; #275 materializes them under the
`PlayerState` owner without weakening that recovery requirement.

Socket handles, transient connection queues, rendered selection, AoI caches, and
other presentation/session transport objects remain non-authoritative unless a
future ADR explicitly promotes them.

### 6. Station authority and projection

Station inventory changes participate in the Sector authoritative recovery
transition. SQLite/repository Station state does not create a second independent
world-state truth.

Normative ordering for a Station-changing transition is:

```text
prepare authoritative mutation
    -> durable Sector transition
    -> local live apply
    -> idempotent required Station projection
    -> publish / acknowledge
```

#277 owns the repository schema/API and may keep one SQLite connection/transaction
owner for repository concerns, but it must preserve this authority direction.

The Station projection carries:

- transition-id deduplication for Station-changing transitions; and
- a **global contiguous authoritative `applied_through` position**.

A non-Station transition advances the projection worker's global watermark as an
explicit no-op. A Station transition advances it only after its Station mutation is
successfully projected. Thus `applied_through >= promotion_position` has one clear
meaning and does not confuse "last Station mutation" with "all transitions through
this point have been considered".

A Station operation is acknowledged only after selected durability, successful
local live apply, and the required local projection application. Projection failure
after durable commit fences the owner until catch-up/rebuild succeeds.

### 6A. Admission and identity repository authority

Prepared fresh-admission and resume-ticket lifecycle state are not Station
inventory. A prepared reservation can exist before any Ship has been materialized
into Sector world state, so forcing it to be described as a Station projection
would create the wrong authority model.

#277 may therefore make admission/identity protocol state authoritative in a
dedicated durable repository, with these invariants:

- reserving a fresh `PlayerId`/`ShipId`/resume ticket must make **both the
  reservation and identity consumption durable** before `Welcome` exposes it;
- #277 may persist an explicit allocator watermark or make the set of durable
  reserved/materialized identities sufficient to derive it, but restart must
  choose the next values strictly above every consumed identity before accepting
  another allocation;
- aborting or expiring a prepared reservation may free protocol resources, but it
  must never make its reserved IDs reusable;
- a prepared reservation survives restart and is retried/aborted using the same
  stable reserved identity rather than allocating a conflicting identity;
- once admission materializes Sector world state, Ship existence, Player ownership,
  docking, Station starter-grant state, and active routing are
  RecoveryDelta/checkpoint authority; Station SQLite rows are projection data;
- current/pending resume-ticket bindings and admission protocol bookkeeping remain
  identity-repository authority;
- if the Sector transition commits while repository finalization is incomplete,
  restart reconciles repository state idempotently from stable admission identity
  before serving the affected admission/resume path; and
- replica promotion cannot serve admission/resume **or new allocations** with stale
  repository/allocator authority; #280/#278 must transfer or reconstruct enough
  #277 state to reconcile first.

This is an explicit multi-authority bounded-context boundary, not an accidental
SQLite-vs-journal race. #277 owns the concrete transaction/schema/reconciliation
mechanism and #278 owns orchestration/error policy.

### 7. Transit relationship

ADR-0014 remains the behavioral consensus baseline for ownership transfer, but its
legacy persistence details are amended:

- the public `SectorTransitRequested`/`Completed` events remain public facts where
  still useful;
- scanning those events from genesis/hot log is not the final recovery repository;
- pending retry/receipt/terminal state is represented by the durable handoff Saga
  in #276: `TransitSagaSnapshot` is part of the checkpoint and
  `TickRecoveryDelta`, `TransitAttemptId` keys the direct lookup, and
  `OutgoingTransitAttempt` / `IncomingTransitReceipt` carry the canonical
  handoff and destination idempotency state;
- that Saga must participate in this issue's checkpoint, RPO, compaction, crash,
  and replica-catch-up semantics;
- reliable Raft proposals cannot be crash-lossy memory-only work.

#276's current implementation stores Saga state directly in the recovery
`StateSnapshot`/`TickRecoveryDelta` boundary. The general #271 recovery journal
stores those authoritative records; public-event retention and event-log scans
are not part of the Saga recovery path. Any future extraction to a repository
must preserve the same checkpoint, compaction, crash, and promotion guarantees.

### 8. Checkpoints and compatibility

A checkpoint is a versioned authoritative Sector-world recovery point, not merely a
performance cache. Its complete manifest identifies one authoritative covered
position and all required members, for example:

- format magic/version;
- Sector identity and catalog/schema fingerprint;
- covered authoritative journal position;
- ECS/Simulation/Player snapshot members;
- Station aggregate checkpoint member when Station state is externalized;
- Transit/Saga member or references when required by #276's persistence choice;
- checksums/lengths; and
- metadata needed to locate retained authoritative/output tails.

Exact physical file/manifest layout belongs to #271. Snapshot transfer framing
belongs to #280. Old snapshot compatibility is not required; incompatible or
incomplete checkpoint data must fail clearly.

Repository-owned admission/identity protocol authority need not be serialized into
the world checkpoint if #277 chooses an independently durable repository. In that
case checkpoint/promotion metadata must make the repository version/epoch or
required reconciliation point explicit enough to prevent stale admission/resume or
allocator service after failover, and recovered allocator values must account for
every durably consumed reservation.

The existing crash-safe publication property is retained or strengthened:
replacement material is written, validated, synced, and atomically selected before
superseded authoritative recovery data is retired.

### 9. Independent retention and compaction

Atomic transition visibility does not require every logical substream to share one
retention lifetime.

After a complete checkpoint covers a position:

- covered state-delta material may become eligible for recovery compaction;
- public events remain according to delivery/audit/archive policy;
- reliable retry/Saga/output records remain until their durable terminal/delivery
  condition permits retirement;
- admission/identity repository records and consumed-ID evidence remain according
  to #277's terminal/reconciliation rules, but retiring a protocol row must never
  make a previously consumed identity reusable; and
- #271 must keep enough committed-index metadata to make covered ranges and retained
  obligations unambiguous.

State checkpoint coverage alone never proves that a public output was delivered,
that a reliable external obligation reached terminal state, that an externally
stored identity protocol record is safe to retire, or that a reserved ID can be
reused.

FBD-001 continues to prohibit destructive in-place public-event history mutation.

### 10. Durability profiles, RPO, acknowledgement, and client retry

"RPO 0" is always qualified by a failure domain. #284 fixes the semantic profiles;
#271 owns journal/durability evidence, #278 owns runtime quorum/fencing/ack policy,
and #280 owns transport mechanics.

#### LocalDurable

Before acknowledgement, the transition satisfies a documented local durable-write
condition (`fsync`/platform equivalent under #271). It gives **acknowledged RPO 0
for process crash, OS crash/reboot, and abrupt power loss while the authoritative
local storage medium remains readable and preserves completed durable writes**.
Permanent loss/corruption of that machine/storage is outside this profile.

#### ReplicatedDurable

Before acknowledgement, LocalDurable is satisfied and the same committed
transition is synchronously durable on a configured replica quorum. It may claim
**acknowledged RPO 0 for owner-node/storage loss only up to the explicitly
documented replica failure tolerance**.

A ReplicatedDurable implementation may stage committed bytes/evidence on remote
replicas before the owner's local live apply. This is **durability replication**,
not proof that the remote replica has applied or may publish/promote that position.
A staged replica must apply the shared recovery reducer and promotion-critical
projections/repositories before that range becomes applied authority.

If owner live apply fails after quorum durability, already-staged bytes remain valid
durable recovery material, but the owner fences and no application/publication
replication may advance beyond its last successfully applied position until
recovery succeeds.

#278 owns selection of the configured replica set and durability profile, quorum
threshold calculation, matching receipt aggregation, owner-epoch/fencing checks,
and acknowledgement gating. A valid remote durability receipt must be bound to
immutable transition context such as Sector identity, current ownership epoch/term
(or equivalent fencing token), authoritative position/transition identity, and
committed content hash/range. #271 owns evidence/framing and #280 transports it.

Production must not enable or advertise `ReplicatedDurable` until #271/#278/#280
implement and test one coherent quorum/failure/fencing model.

#### Ambiguous generic client retry

An unacknowledged request may be absent or already durably committed after a crash,
but the current generic `ClientRequest` wire protocol has no stable request ID.
Therefore an internal journal `transition_id` cannot by itself deduplicate a newly
submitted client payload.

The current contract is:

- do not transparently auto-retry arbitrary non-idempotent generic client commands
  after an ambiguous disconnect;
- reconnect/read authoritative state first, then treat a later client submission as
  a new request;
- protocols that require transparent retry/exactly-once domain effect carry their
  own stable operation identity and durable dedup state, e.g. #277 admission,
  #276 `TransitAttemptId`, and #279 `SettlementId`; and
- a future generic exactly-once client-command feature requires a stable `RequestId`
  (or equivalent) plus durable result/dedup retention at the #278/wire boundary.

RPO 0 protects committed authoritative state; it does not magically turn an
identity-less client request protocol into exactly-once RPC.

### 11. Delivery semantics

Durable public/reliable output delivery is **at-least-once** unless a stronger
protocol is explicitly provided.

This durable-consumer rule applies only to consumers/obligations explicitly marked
as durable. Ordinary WebSocket sessions, AoI membership, `PositionSnap`, and similar
presentation streams are ephemeral and do not acquire retention cursors merely by
observing committed state. Reconnect/current-state synchronization repairs them.

For each durable consumer:

1. choose next committed output after durable delivery state;
2. deliver using stable transition/output identity;
3. obtain downstream acknowledgement or equivalent durable idempotency proof;
4. only then durably advance consumer delivery state; and
5. only after all required durable-consumer/archive conditions may retained output
   become eligible for retirement.

A crash after downstream acknowledgement but before delivery-state durability can
cause duplicate delivery; durable consumers must tolerate that or provide stronger
transactional idempotency. A local cursor alone is not exactly-once.

#271/#276 own concrete cursor/attempt storage as appropriate. The #284 invariant is
that failover cannot skip a committed undelivered durable obligation. A disconnected
ephemeral client never holds public-event compaction open.

### 12. Replica catch-up and promotion

The recovery position carried by a checkpoint is not a public-event
replication position. Checkpoints record `covered_recovery_index` for the
authoritative `RecoveryDelta` stream and `public_event_next_index` for the
append-only public `DomainEvent` stream. Eventless Ticks may advance recovery
without advancing public events. #280 snapshot/catch-up transport must carry
both values and use only `public_event_next_index` when requesting or
installing a public suffix; it must never derive the public cursor from
recovery coverage.

#280's physical catch-up path must transport enough data to obtain:

- a complete compatible checkpoint and covered authoritative position;
- every contiguous authoritative recovery record after that point;
- retained public/reliable outputs still required after promotion;
- delivery/retry state sufficient to avoid skipping committed durable obligations;
- projection/checkpoint data required to make local promotion-critical projections
  current; and
- admission/identity repository data or deterministic reconciliation metadata,
  including consumed-ID/allocator evidence, sufficient for #277-backed
  admission/resume/new-allocation service.

A replica is promotable through position `P` only after:

1. compatible recovery data exists with no gap through `P`;
2. the shared recovery reducer has successfully applied through `P`;
3. invariants validate;
4. no promotion-critical retained public/reliable output is missing;
5. delivery/retry state cannot skip an undelivered committed obligation;
6. admission/identity repository authority and consumed-ID/allocator state are
   caught up or reconciled for every identity/allocation domain the promoted owner
   may serve; and
7. Station projection `applied_through >= P` before Station reads/writes are served.

Durably staged-but-unapplied quorum bytes do not satisfy promotion by themselves.

### 13. Fail-stop rule

Once a transition is durably committed it cannot be forgotten. A post-durable local
apply or required projection/reconciliation error/panic/partial apply requires
immediate fencing:

- admit/prepare/commit no later authoritative transition;
- do not publish the failed transition's public/reliable effects;
- do not acknowledge it;
- do not advance application/publication replication beyond last successfully
  applied contiguous position;
- do not serve affected Station/admission/resume/allocation paths from stale
  projections or repositories;
- do not serve the Sector as healthy; and
- terminate or reconstruct/reconcile local authority from durable recovery data
  before resuming.

Durability-quorum copies already staged before the failure may remain; they are not
healthy applied replicas until their own reducer/projection/repository checks
succeed.

### 14. Operational RTO

No portable numeric production RTO is selected by this architecture slice. After
#280 fixes the peer transport and reference hardware, deployment operations must
benchmark representative ship counts/eventless Ticks and define:

- maximum authoritative tail transitions/bytes;
- maximum replay duration on named reference hardware;
- checkpoint cadence/trigger thresholds; and
- the production recovery target derived from those measurements.

A recovery procedure is not itself an RTO, and #284 does not invent a number that
would only be valid for one machine and one replica topology.

## Alternatives considered

### Event-source every authoritative mutation as public events

Rejected because it would couple public/business event schemas to high-volume
internal per-Tick state such as position, capacitor, lock/module counters, queues,
and routing state.

### Journal deterministic inputs and rerun historical Ticks

Rejected because exact old-state reconstruction would remain dependent on RNG,
floating point, iteration order, catalog version, AI code, and implementation
version unless those were frozen as a much broader execution ABI.

### Bounded rollback to checkpoint

Rejected because a successful acknowledged operation could disappear after crash,
contradicting the selected authoritative simulation contract.

### Independent SQLite Station authority

Rejected as the Sector world-state recovery model because it creates a second
world-state authority and a cross-store atomicity/reconciliation problem. #277 may
still use SQLite as the Station projection implementation.

This rejection does **not** prohibit a distinct admission/identity repository from
being authoritative for protocol state that exists before Sector-world
materialization. That boundary is accepted only with the explicit reconciliation,
identity-consumption, and promotion rules in §6A.

### Public Transit events as the durable handoff repository

Retained only as historical public facts and replay/projection inputs. The #276
durable Saga now owns retry and receipt authority because log scanning and implicit
attempt identity are the exact problems it removes.

### Generic exactly-once client retry without a RequestId

Rejected. Payload equality is not a safe operation identity, and an internal
transition ID is unavailable to a client whose acknowledgement was lost. The
current protocol uses state refresh + new submission; workflows needing transparent
retry use explicit operation IDs.

## Consequences

- #271 must provide a generic/versioned atomic durable-journal substrate capable of
  storing this recovery representation without hardcoding public `DomainEvent` as
  the sole payload.
- #272 must expose a bounded pre-durable transition and commit live authority only
  after successful durable append.
- #275 must classify/split state owners according to `recovery-contract.md`,
  including Player active-ship routing as durable PlayerState.
- #276 replaces legacy Transit event scans with the durable
  `TransitAttemptId`/`TransitSagaSnapshot` attempt, receipt, retry, and terminal
  authority satisfying this contract.
- #277 must preserve Sector-journal authority for Station world state while making
  admission/identity repository authority, durable consumed-ID/allocator semantics,
  and its reconciliation boundary explicit.
- #278 must implement one runtime acknowledgement path, including durability-profile
  selection, replica-set/quorum policy, fencing/owner-epoch validation, repository
  reconciliation policy, and no-transparent-retry behavior for identity-less generic
  client commands.
- #280 must transport versioned checkpoint/tail/retry/repository catch-up data and
  durability receipts while distinguishing staged durability from applied/
  promotable state.
- `DomainEvent` remains valuable for public facts/audit/projections without silently
  defining exact ECS recovery.
- eventless Ticks and no-public-event authoritative commands become explicitly
  recoverable.
- numeric RTO remains a deployment benchmark deliverable after #280.

## Implementation sequence

1. Land this accepted architecture slice and align conflicting normative/entry-point
   documentation without rewriting unrelated historical specification detail.
2. #271 implements generic fallible atomic journal framing/durability semantics.
3. #272 migrates a simple command vertical slice to prepare -> durable -> live apply.
4. Add versioned checkpoint/recovery-tail support and eventless-Tick coverage.
5. Migrate Player routing/pending bot state required by the authority inventory.
6. #277 splits Station projection from authoritative admission/identity repository
   state, makes reservation/allocator consumption crash-safe, and defines
   reconciliation.
7. #276 replaces Transit scans/retry state with the durable Saga.
8. #278 unifies runtime orchestration and implements durability-profile/quorum/
   fencing/acknowledgement policy.
9. #280 consumes the final checkpoint/catch-up/durability representation in peer
   transport.
10. After #280 selects the peer transport and reference hardware, run recovery
    benchmarks and close the deployment RTO/checkpoint policy.

## Implementation checklist

- [x] Select state-delta + versioned-checkpoint recovery model.
- [x] Define exact acknowledged RPO semantics and failure-domain profiles.
- [x] Inventory/classify authoritative, derived, reliable, and lossy state/outputs.
- [x] Define eventless-Tick and active-ship-routing recovery behavior.
- [x] Define fail-stop ordering after post-durable apply failure.
- [x] Define Station authority direction and unambiguous global projection watermark.
- [x] Define admission/identity repository authority, durable reserved-ID consumption,
  allocator recovery, and reconciliation boundary.
- [x] Define reliable auto-jump/Transit continuation invariant without pre-empting #276.
- [x] Define staged durability replication versus applied/promotable replica state.
- [x] Assign replica-set/quorum/fencing/ack orchestration to #278.
- [x] Define generic ambiguous client retry as non-exactly-once without RequestId.
- [x] Define at-least-once durable output semantics and exclude ephemeral clients
  from durable retention cursors.
- [ ] Benchmark and define the deployment-specific numeric RTO/checkpoint budget
  after #280 selects the reference topology.
- [x] Implement generic fallible atomic journal in #271; #284 consumes its
  versioned recovery records, contiguous ranges, and corruption/failure fence.
- [x] Implement prepare -> durable -> live apply in #272 for Stop and the bounded full Tick write set; #278 now owns the shared runtime wiring and #275 now records the explicit state-owner decomposition.
- [x] Implement versioned checkpoint/tail and eventless-Tick persistence through the runtime-owned `FileJournal`.
- [x] Recover from a pre-checkpoint crash by reconstructing configured genesis
  state and replaying the RecoveryDelta journal from index 0; public-event
  genesis replay is not an authoritative requirement.
- [x] Add the `DAWNCKP1` checkpoint envelope, payload checksum, catalog fingerprint, explicit covered recovery position, and rejection of incompatible/corrupt checkpoints.
- [x] Persist Player routing, allocator state, docking context, and pending bot/auto-jump authoritative state.
- [x] Implement Station projection API plus admission/identity repository,
  allocator, and local identity-watermark reconciliation under #277; the
  shared runtime now supplies the production prepare -> durable -> live apply
  -> projection boundary. The projection advances by complete journal ranges,
  including no-op non-Station transitions, and #280 owns remote catch-up.
- [x] Establish one durable runtime Tick frame with injected consensus,
  durability-policy, and reconciliation ports; production, single-sector serve,
  clustered serve, and the in-memory `SectorRuntimeDriver` use the same
  prepare -> durable -> live-apply -> reconcile -> output ordering under #278.
- [x] Implement Transit durable `TransitAttemptId` Saga under #276, including
  checkpointed outgoing attempts, incoming receipts, bounded retry/backoff,
  terminal/quarantine state, and compaction-independent recovery.
- [x] Implement the unified runtime durability-profile/quorum/fencing and
  reconciliation policy boundary under #278. #280 still owns remote receipt and
  catch-up transport, so production remains explicitly `LocalDurable`.
- [x] Implement the shared versioned peer transport, isolated control/bulk
  channels, snapshot/catch-up adapters, repository byte channel, and validated
  durability envelopes under #280. Production `ReplicatedDurable` activation
  remains with the #278 runtime policy.
- [x] Add checkpoint-plus-tail equivalence and malformed/missing-boundary rejection tests for the local recovery path; replica/Transit/admission crash matrices remain with #276/#277/#280.
