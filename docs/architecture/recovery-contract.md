---
scope    : Normative Sector recovery sources, durability points, and crash outcomes
audience : AI Agent / Human Developer
update   : When authoritative state, transition ordering, snapshots, journal payloads, Station persistence, delivery cursors, or replication semantics change
related  : ../adr/ADR-0049-sector-recovery-state-delta-wal.md, tick-model.md, event-catalog.md, ../adr/ADR-0014-raft-consensus.md
---

# Sector Recovery Contract

This document is the normative recovery inventory and crash matrix for accepted
ADR-0049 and issue #284. It decides **what must survive and what a successful
transition means**. It deliberately does not absorb the implementation decisions
owned by the dependent refactor issues.

Work-package ownership is:

- **#284 / ADR-0049:** authoritative recovery content, exact recovery promise,
  eventless-Tick behavior, RPO/RTO contract, checkpoint coverage, and the
  authoritative/derived/lossy classification in this document;
- **#271:** fallible atomic journal framing, commit markers, durability evidence,
  fsync/quorum mechanics, corruption handling, indices/receipts, and physical
  retention implementation;
- **#272:** storage-independent engine API and prepare -> durable append -> live
  apply -> post-commit effect ordering;
- **#275:** in-memory state-owner decomposition. It consumes this table rather
  than redefining which fields are durable;
- **#276:** durable Transit Saga/attempt repository and retry lifecycle. It must
  satisfy this recovery contract but owns the concrete Saga representation;
- **#277:** admission/identity/Station repository schema and transaction APIs.
  Repository shape does not create a second Sector recovery authority;
- **#280:** peer transport, snapshot/catch-up transfer mechanics, and traffic
  isolation. It transports the representation selected here rather than defining
  a competing snapshot format or RPO.

Older documents are amended in PR #288 where they make a conflicting recovery
claim. Historical implementation descriptions may remain when explicitly labeled
as legacy/current behavior rather than normative recovery authority.

## 1. Recovery guarantees

- Exact operational recovery is: newest complete compatible checkpoint set plus
  every committed authoritative recovery batch after its covered position.
- Public `DomainEvent`s alone are not a complete recovery source.
- Public events and reliable outbox intents produced by a transition are durable
  and share the transition's atomic visibility boundary with its authoritative
  recovery delta.
- Every committed Tick has a recovery record, including an eventless Tick.
- Recovery, local live apply, and replica catch-up use the same recovery-delta
  semantics and invariant checks.
- A journal append failure cannot change committed live state or produce
  client-visible success.
- A post-append live-apply or required local-projection failure fences the Sector
  until journal recovery succeeds; processing from old/partial live state is
  forbidden.
- A completed auto-jump Warp has a durable retry obligation; that obligation
  cannot exist only in `pending_auto_jumps` memory. #276 may later absorb the
  concrete retry lifecycle into its Transit Saga.
- Public-event/outbox delivery is at-least-once unless a stronger downstream
  idempotent transaction protocol is explicitly provided.
- A replica is promotable only when authoritative recovery data and every
  promotion-critical retained output/projection are caught up to the required
  position.
- "Acknowledged RPO 0" is qualified by the selected durability profile and its
  explicit failure domain; it is never an unqualified machine-loss promise.
- No numeric production RTO is currently claimed. #284 must benchmark replay
  tail size/time and set the checkpoint budget before that acceptance item closes.

## 2. State and obligation classification

| State / mutation / obligation | Authoritative or reliable? | Current source or mutation site | Required recovery source | Notes |
|---|---:|---|---|---|
| Logical `current_tick` | Yes | Incremented before Tick systems | Transition header/delta and checkpoint | Must advance only as part of a durable transition, including eventless Ticks. |
| Entity/Player ID counters | Yes | Admission/spawn helpers | Counter delta and checkpoint | Reuse after crash violates identity invariants. |
| Ship existence and type | Yes | Spawn, destroy, assemble, disassemble, Transit | Create/delete delta and checkpoint | Stable `ShipId` is the recovery key, not ECS entity handle. |
| Player ownership maps | Yes | Admission, Station operations, Transit | Ordered map delta and checkpoint | Ownership is durable PlayerState under #275. |
| Active-ship routing map | Yes | Admission, `SelectActiveShip`, `Disembark`, removal/Transit | Ordered map delta and checkpoint | This is authoritative routing state because it changes which ship receives commands and whether Undock is legal. It may have no public `DomainEvent`. The current snapshot omission is implementation debt, not intended lossiness. |
| Position, velocity, anchor | Yes | Movement, Warp, docking, Transit, commands | Component final-value delta and checkpoint | `VelocityChanged` does not cover every exact position/representation change. |
| Thrust/braking and flight modes | Yes | Move/Stop/Approach/Orbit/KeepAtRange/Warp | Component add/remove/update delta and checkpoint | Determines future motion even when no event is emitted. |
| Hull shield/armor/hull/destroyed state | Yes | Combat and repair | Component final-value delta and checkpoint | Public damage/repair events are facts/outputs, not exact reducer authority. |
| Capacitor current | Yes | Capacitor system every Tick | Component final-value delta and checkpoint | Recharge can occur without a public event. |
| Fitted slots, active flags, cycle counters, targets | Yes | Fit/Unfit/Activate/Deactivate/Cap/range gate | Component final-value delta and checkpoint | Countdown and forced deactivation affect later combat. |
| Derived ship stats from fitting/catalog | Reconstructible | `apply_fitting` | Recompute from recovered fitting plus catalog fingerprint | Persist only if a future invariant requires it. |
| Lock entries, states, countdowns | Yes | Lock system and docking cleanup | Component final-value delta and checkpoint | Exact countdown/state must survive restart. |
| Tackle membership | Yes | Tackle system | Component final-value delta and checkpoint | Prevents Warp/Jump and must survive restart exactly. |
| Ship cargo and destruction rewards | Yes | Station operations and combat reward | Ordered item-stack delta and checkpoint | Reward mutation can accompany ship deletion. |
| Docked ship/player Station context | Yes | Dock/Undock/Disembark/Select | Ordered map delta and checkpoint | Required to authorize Station operations. |
| Station inventory / packaged ships | Yes, Sector-journal-owned aggregate | Station command execution | Station aggregate delta in the same logical transition plus versioned Station checkpoint | SQLite/repository storage is an idempotent projection/read model under this contract; #277 owns its final API/schema. |
| Transit ownership/freeze state and current handoff lifecycle state | Yes | Raft-committed Transit apply | Recovery delta/checkpoint plus the durable attempt/receipt authority selected by #276 | Legacy event scans are not the final recovery authority. #276 owns concrete Saga representation and reconciliation. |
| Resume-ticket current/staged binding | Yes | Admission and Transit | Delta/checkpoint or repository state participating in the same documented recovery transition | Ticket rotation must remain retry-safe; #277 owns repository shape. |
| Bot persistent behavior state | Yes when it affects future decisions | Bot components/state | Component delta and checkpoint | Purely recomputable selection may be derived only when specified. |
| Pending human command queue | No, until admitted into a transition | Runtime connection | Runtime input queue | Disconnect/crash may require client retry. |
| Pending bot lock-command queue | Yes until same-Tick consumption replaces it | Produced at end of one Tick, consumed by next | Ordered queue delta and checkpoint | It changes the next Tick's outcome and cannot remain an in-memory recovery gap. |
| Auto-jump Raft proposal after Warp arrival | Reliable post-commit obligation | `process_warp` -> `pending_auto_jumps` -> runtime | Durable outbox/idempotent retry intent committed with the Warp transition | Current in-memory queue is convenience only. #276 may represent this as Saga work, but crash after Warp commit must not lose the obligation. |
| Completed-warp client correction | Runtime presentation output | `completed_warps` | None unless promoted to a reliable protocol | Reconnect/current-state sync may repair presentation. |
| AoI index and client projections | Derived | Rebuilt/read from committed world | Recompute after recovery | Never journal presentation caches as authority. |
| Socket/session handles and channel queues | No | Runtime adapters | None | Re-established after recovery. |
| Journal append receipt / authoritative committed position | Durable runtime metadata | Journal/runtime layer | #271 commit index/checkpoint metadata | Must refer only to committed ranges. |
| Station projection applied-through position | Durable projection metadata | Runtime repository/projection layer | A contiguous **global authoritative journal position** plus transition-id dedup for Station-changing records | The projection worker advances through every contiguous transition: Station records apply changes; non-Station records are no-op progression. Thus `>= promotion_position` is unambiguous. |
| `DomainEvent` list | Durable public fact | Command/Tick output | Public-event subrecord committed with transition; not exact state reducer | Delivery resumes from durable consumer state. |
| Reliable runtime effects | Reliable post-commit obligation | Runtime/application layer | Outbox/idempotency representation compatible with the transition | Execute only after successful local live apply; delivery state advances after downstream acknowledgement. |
| Deliberately lossy runtime effects | No | Runtime/application layer | None | Must be explicitly classified and cannot be required for authoritative continuity. |

Any newly added mutable field or post-commit obligation must be added to this table
before code lands. #275 may split these rows among sub-aggregates, but may not
silently downgrade an authoritative row to frame-local state.

## 3. Transition shape

A prepared transition separates exact recovery state, public facts, and runtime
effects. Names are illustrative; #272 owns the final engine API.

```rust
pub struct PreparedSectorTransition {
    pub recovery_delta: RecoveryDelta,
    pub domain_events: Vec<DomainEvent>,
    pub effects: Vec<SectorEffect>,
    mutation: PreparedMutation,
}

pub struct DurableTransitionBatch {
    pub recovery_delta: RecoveryDelta,
    pub domain_events: Vec<DomainEvent>,
    pub outbox: Vec<DurableEffectIntent>,
}
```

The durable batch is one **logical atomic visibility envelope**. #271 owns its
physical framing and may place state/event/outbox bytes in independently retained
immutable segment families. No recovery/publication reader may observe a subrecord
as committed unless the enclosing logical transition is committed according to
#271's documented framing/durability evidence.

`domain_events` may be empty. Reliable effects are represented by durable retry
state compatible with the logical transition; deliberately lossy effects remain
post-commit runtime outputs.

`PreparedMutation` must not mutate committed live authority before durable append.
It may own old/new values, a reversible write set, copy-on-write overlay, or another
bounded representation chosen by #272. Its committed result must be semantically
equivalent to applying `RecoveryDelta`.

A `RecoveryDelta` is applied through one reducer contract:

```text
apply_recovery_delta(state, delta)
```

The same semantics/shared primitives are used for:

- local live apply after durable append;
- checkpoint-tail recovery;
- replica catch-up; and
- equivalence/failure-injection tests.

### 3.1 Station projection rule

Station item changes are authoritative recovery data; they are not committed first
to SQLite. After journal commit and successful local recovery-delta application, a
required local Station projection applies the transition idempotently.

The projection maintains two distinct pieces of metadata:

1. transition-id deduplication for Station-changing records; and
2. `projection_applied_through`, a **global contiguous authoritative journal
   position**. The projection worker advances this watermark across non-Station
   transitions as explicit no-ops and may advance across a Station transition only
   after its Station mutation has applied successfully.

A Station operation is acknowledged only after the selected durability condition,
successful local live apply, and required local Station projection application. If
projection application fails after append, the Sector fences and catches up/rebuilds
the projection before serving more authoritative Station work.

#277 decides the concrete repository/table/transaction API. It must not turn the
SQLite/repository layer back into a competing independent Sector authority.

### 3.2 Auto-jump / Transit retry rule

When a Tick commits a Warp whose `auto_jump` completes, the same durable transition
must create enough durable retry identity/state to ensure the subsequent Transit
request cannot disappear in a crash.

Today that can be described as an `AutoJumpProposalIntent` outbox record with ship,
gate/route, and stable idempotency identity. #276 is free to absorb this obligation
into its first-class `TransitAttemptId`/Saga model. The invariant owned by #284 is
only:

```text
Warp arrival committed + auto_jump requested
    => a durable, replayable, idempotent obligation to continue the handoff exists
       before the transition can be acknowledged as complete.
```

`pending_auto_jumps` may remain temporarily as an in-memory work queue during
migration, but it is never the sole recovery source.

## 4. Commit, durability replication, and fail-stop

The normative local ordering consumed by #272 is:

```text
prepare
  -> make logical transition durable under selected profile
  -> apply RecoveryDelta / prepared mutation to local live state
  -> apply required local projections
  -> publish public outputs / execute reliable effects
  -> acknowledge when all acknowledgement conditions are satisfied
```

For `ReplicatedDurable`, "make durable" may require **durability replication** of
unapplied committed bytes/evidence to a quorum before the owner's live apply. This
is distinct from **state/application/publication replication**.

A remote node that has durably staged a transition for quorum purposes must not
pretend the transition is locally applied, publish its outputs, or become promotable
through that position until it has successfully applied the shared recovery reducer
and all promotion-critical projections.

Once an envelope is durably committed, it cannot be converted into an ordinary
rejection. If local live reduction or a required local projection errors, panics,
or partially applies, the owner must immediately:

1. mark itself fenced/unhealthy;
2. stop admitting/preparing/committing later transitions;
3. stop **application/publication replication** beyond its last successfully
   applied contiguous position (already-staged durability copies may remain);
4. suppress event delivery, outbox execution, and acknowledgement for the failed
   local apply; and
5. terminate or reconstruct local state/projections from the journal before
   resuming.

Continuing from pre-transition or partially applied live state after successful
durable commit is a contract violation.

#271 owns how local durability/quorum evidence is encoded and recovered. #280 owns
how those bytes are transported and isolated from Raft control/bulk traffic. Neither
issue may weaken the ordering or promotion rules above.

## 5. Durability profiles and acknowledgement

The system has two semantic profiles. The names may change in #271; the failure
domains may not be blurred.

### LocalDurable

Before acknowledgement, the transition is durable according to #271's documented
local `fsync`/platform-equivalent guarantee. It provides **acknowledged RPO 0 for
process crash, OS crash/reboot, and abrupt power loss while the authoritative local
storage medium remains readable and preserves completed durable writes**.
Permanent loss/corruption of that machine/storage is outside this profile.

### ReplicatedDurable

In addition to LocalDurable, the committed transition is synchronously durable on
a configured replica quorum before acknowledgement. It may claim **acknowledged
RPO 0 for owner-node/storage loss only up to the explicitly documented replica
failure tolerance**.

#284 fixes that semantic requirement. #271 chooses journal/quorum evidence and #280
chooses transport/channel mechanics. Production documentation must not claim the
stronger failure domain until those issues define and test the quorum/failure model.

An unacknowledged request may be absent or already durably committed. Retried
operations that require exactly-once domain semantics use a stable idempotency/
transition identity.

## 6. Crash-point matrix

| Crash/failure point | Durable recovery data | Live/recovered state | Client/external observation | Required behavior |
|---|---|---|---|---|
| Before prepare | No new transition | Previous committed state | No success | Operation may be retried. |
| During prepare | No new transition | Previous committed state | No success | Discard bounded prepared state. |
| Before/during failed atomic append | No valid committed transition | Previous committed state | No success/effect | #271 detects/rejects/truncates/quarantines partial framing according to its documented policy. |
| After durable commit, before local live apply | Complete transition exists; ReplicatedDurable may already have staged quorum copies | Recovery applies delta | Success may not have been observed | Retry detects committed identity; staged replicas do not publish/apply implicitly. |
| During local live apply | Complete transition exists | Recovery reapplies complete delta | No success until local apply completes | Fence immediately; do not continue from old/partial state. |
| During required Station projection apply | Complete transition exists | Recovery reapplies Station delta idempotently | No acknowledgement | Fence; rebuild/catch up projection before serving authoritative Station work. |
| After Warp commit, before handoff proposal | Durable auto-jump/Transit continuation obligation exists | Warp arrival remains committed | Proposal may be absent | Resume durable retry/Saga work using stable identity. |
| After handoff proposal attempt, before durable delivery/retry progress | Durable obligation exists | Warp arrival remains committed | Proposal may have been accepted | Duplicate attempt is allowed only through idempotent #276 semantics. |
| After local apply, before public-event delivery | Event subrecord exists | New committed state | Event may not yet be observed | Resume from durable delivery state; never regenerate by rerunning simulation. |
| After downstream delivery ack, before cursor durability | Output remains retained | New committed state | Consumer may already have output | Retry may duplicate; idempotency prevents duplicate domain effect. |
| After cursor durability | Output may become retention-eligible | New committed state | Delivery acknowledged | Compaction still obeys every required consumer/archive watermark. |
| After acknowledgement | Transition exists under selected profile and required local apply/projections succeeded | New committed state | Success observed | RPO 0 only inside selected profile's documented failure domain. |
| During checkpoint member write/validation | Journal remains authority | Previous checkpoint set plus tail | No recovery gap | Keep previous complete checkpoint manifest. |
| After checkpoint manifest publication, before state compaction | New checkpoint and old tails coexist | Either valid recovery path | None | Both cover same committed position. |
| During/after state-delta compaction | Checkpoint plus remaining authoritative tail | Exact covered state plus tail | None | Never retire required delta before complete checkpoint publication. |
| Before event/outbox retention watermark | Public facts/intents retained | State may already be checkpointed | Pending delivery remains possible | State coverage does not delete undelivered output. |
| Replica receives partial/out-of-order range | Local applied authority unchanged | Stops at last contiguous applied position | No promoted state | Detect gap/duplicate/version/fingerprint/hash mismatch. |
| Replica has staged durable bytes but has not applied them | Quorum durability may be satisfied | Applied state lags staged position | Promotion/publication forbidden through staged-only range | Apply shared reducer/projections first. |
| Replica has state but lacks promotion-critical retained outputs | State may be current | Catch-up-only | Promotion forbidden | Synchronize required outputs/delivery state. |
| Replica has applied state/outputs but Station projection watermark trails | State may be current | Catch-up-only for Station-authoritative service | Promotion/Station serving forbidden | Advance global projection watermark through promotion point first. |

## 7. Checkpoint compatibility

A complete checkpoint manifest must reject incompatible or incomplete data before
constructing live state. At minimum it carries or references:

```text
magic
checkpoint_manifest_version
sector_id / identity metadata required by deployment
catalog_fingerprint
covered_authoritative_journal_position
ECS / Player / other authoritative snapshot members + versions/checksums
Station aggregate checkpoint member when Station state is externalized
recovery/output-retention metadata required to locate retained tails
```

The exact physical manifest/file framing belongs to #271. The exact transport
framing belongs to #280. #284 requires only that the checkpoint is explicit,
versioned, mutually consistent at one authoritative position, and that incompatible
or incomplete data fail clearly. Old snapshot compatibility is not required.

Reliable post-commit obligations are retained as durable retry state; snapshotting
a transient runtime queue is not a substitute.

Checkpoint coverage applies to authoritative recovery data. Public-event/reliable
output retention uses independent delivery/archive conditions.

## 8. Independent retention and compaction

The logical transition has atomic visibility while physical retention may be
independent:

- authoritative state deltas may compact behind a complete validated checkpoint;
- public events retain according to audit/archive/delivery policy;
- reliable retry/outbox records retain until their required delivery/Saga terminal
  condition is durable; and
- #271 must preserve enough committed-index metadata to make recovery ranges and
  remaining output references unambiguous.

Compaction must preserve the parent's existing crash-safe publication property:
write/validate/sync replacement material -> atomically publish the selecting
manifest -> only then retire superseded recovery material. No state checkpoint may
silently discard a still-required public fact or reliable obligation.

FBD-001 continues to protect committed public `DomainEvent` history from in-place
destructive mutation; recovery-delta checkpoint compaction is a distinct stream.

## 9. Durable delivery state

For each durable public-event/outbox consumer, delivery is at-least-once unless a
stronger protocol is explicitly selected:

1. select the next committed output after durable delivery state;
2. attempt delivery with stable transition/output identity;
3. obtain downstream acknowledgement or equivalent durable idempotency proof;
4. durably advance consumer delivery state only after step 3; and
5. allow output retirement only after every required consumer/archive condition
   is satisfied.

A crash between steps 3 and 4 may redeliver; consumers must tolerate duplicates or
provide stronger transactional idempotency. A local cursor alone is not exactly-once.

The concrete cursor/index representation belongs to #271/#276 as appropriate.
#284 requires that failover cannot advance past an undelivered committed output.

## 10. Replica catch-up and promotion

A snapshot/catch-up representation consumed by #280 must be sufficient to obtain:

- a complete compatible checkpoint and its authoritative covered position;
- every contiguous authoritative recovery transition after that position;
- every retained public/reliable output still required after promotion;
- delivery/retry state sufficient to prevent skipping committed obligations; and
- Station checkpoint/delta information sufficient to advance the local projection's
  **global contiguous applied-through position**.

Promotion eligibility requires:

1. compatible checkpoint members/fingerprints;
2. no authoritative recovery gaps through the promotion position;
3. successful application of the shared recovery reducer through that position;
4. no missing promotion-critical retained public/reliable output;
5. delivery/retry state that cannot skip an undelivered committed obligation;
6. successful invariant validation; and
7. `StationProjection.applied_through >= promotion_position` before Station
   reads/writes are served. Non-Station transitions count as explicit no-op
   progression for this watermark.

A node with bytes durably staged for quorum but not yet applied is not promotable
through those bytes. A node with ECS equivalence but stale retry/output/projection
state is likewise not healthy for promotion.

## 11. Determinism boundary

The engine should remain deterministic where practical, but exact recovery does not
re-execute historical implementation code. Process randomness, floating-point
behavior, catalog evolution, iteration ordering, AI implementation, and queue
contents can alter rerun outcomes. The state-delta journal captures committed
outcomes and keeps recovery independent of those implementation details.

Diagnostic accepted-input/RNG metadata may be included, but it is supplementary.
`DomainEvent`s remain mandatory durable public outputs when produced, not the sole
exact-state reducer.

## 12. RTO status

RTO is intentionally **TBD**, not implied by having a recovery procedure. #284 must
benchmark representative ship counts and eventless Ticks, then define:

- maximum authoritative tail transitions and bytes;
- maximum replay time on named reference hardware;
- checkpoint cadence/trigger thresholds; and
- the production recovery target derived from those measurements.

Until those measurements land, the numeric-RTO acceptance criterion remains open.

## 13. Required tests

Implementation across #271/#272/#276/#277/#280/#284 must eventually cover the
relevant layer of each guarantee:

- append failure before commit for a simple command;
- append failure for an eventless Tick with movement/capacitor changes;
- crash after durable append and before local live apply;
- live reducer error/panic/partial apply after append, proving fail-stop fencing;
- ReplicatedDurable staged-quorum bytes cannot be published/promoted before local
  reducer application;
- Station projection failure after append and idempotent catch-up;
- Station global applied-through watermark advances across non-Station no-op
  transitions and blocks promotion when stale;
- `SelectActiveShip`/`Disembark` recovery equivalence even when no public event is
  emitted;
- pending bot lock-command queue checkpoint-plus-tail equivalence;
- crash after Warp commit and before Transit continuation proposal, proving durable
  retry survives;
- ambiguous crash after proposal, proving #276 idempotency;
- crash after local apply and before public-event publication;
- crash after downstream delivery acknowledgement but before delivery-state
  durability, proving duplicate-safe retry and no skip;
- checkpoint-plus-tail equivalence for motion, capacitor, locks, module cycles,
  docking, Player/active-ship state, Station inventory, admission, combat randomness,
  and Transit;
- duplicate, missing, out-of-order, corrupt, or incompatible recovery records;
- incomplete checkpoint, corrupt member, and incompatible format/fingerprint;
- interrupted checkpoint publication and compaction boundaries;
- state compaction cannot remove still-required public/reliable outputs;
- snapshot-based catch-up cannot promote with staged-but-unapplied ranges or stale
  promotion-critical projections;
- LocalDurable acknowledgement survives the documented local failure injections;
- ReplicatedDurable acknowledgement is not emitted before configured quorum
  durability and required owner-side apply/projection success;
- no public event or reliable external effect before local live apply; and
- idempotent recovery/retry of a transition already durably committed.
