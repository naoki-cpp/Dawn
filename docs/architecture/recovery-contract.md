---
scope    : Normative Sector recovery sources, durability points, and crash outcomes
audience : AI Agent / Human Developer
update   : When authoritative state, transition ordering, snapshots, journal payloads, Station persistence, delivery cursors, or replication semantics change
related  : ../adr/ADR-0049-sector-recovery-state-delta-wal.md, tick-model.md, event-catalog.md
---

# Sector Recovery Contract

This document is the normative inventory and crash matrix for accepted ADR-0049
and issue #284. It distinguishes exact recovery state from public facts and
runtime-only effects. It is normative for #271 and #272: adding an authoritative
field or a reliable post-commit obligation requires assigning it a recovery source
before the change lands.

ADR-0001, ADR-0017, ADR-0038, `AI_DEVELOPMENT_GUIDE.md`, `tick-model.md`, and
`event-catalog.md` are amended in the same PR to match this contract. Public-event
append is not the complete state commit, SQLite is not an independent Station
authority, and an eventless Tick still has a durable recovery record.

## 1. Recovery guarantees

- Exact operational recovery is: newest complete compatible checkpoint set plus
  every committed authoritative recovery batch after its covered position.
- Public `DomainEvent`s alone are not a complete recovery source.
- Public events and reliable outbox intents produced by a transition are durable
  and committed atomically with its authoritative recovery delta.
- Every committed Tick has a recovery record, including an eventless Tick.
- Recovery, local commit, and replica application use the same delta reducer and
  invariant checks.
- A journal append failure cannot change committed live state or produce
  client-visible success.
- A post-append live-apply or SQLite-projection failure fences the Sector until
  journal recovery succeeds; processing from the old live state is forbidden.
- A completed auto-jump Warp has a durable Raft-proposal obligation; the obligation
  cannot exist only in `pending_auto_jumps` memory.
- Public-event/outbox delivery is at-least-once: a durable consumer cursor advances
  only after downstream acknowledgement or equivalent durable idempotency proof.
- A replica is promotable only when authoritative state, retained outputs/cursors,
  and the Station SQLite projection are all caught up to the promotion point.
- "Acknowledged RPO 0" is qualified by the selected durability profile and its
  explicit failure domain; it is never an unqualified machine-loss promise.
- No numeric production RTO is currently claimed. #284 must benchmark replay
  tail size/time and set the checkpoint budget before marking RTO complete.

## 2. State and obligation classification

| State / mutation / obligation | Authoritative or reliable? | Current source or mutation site | Required recovery source | Notes |
|---|---:|---|---|---|
| Logical `current_tick` | Yes | Incremented before Tick systems | Transition header/delta and ECS snapshot | Must advance only after durable append, including eventless Ticks. |
| Entity/Player ID counters | Yes | Admission/spawn helpers | Counter delta and ECS snapshot | Reuse after crash violates identity invariants. |
| Ship existence and type | Yes | Spawn, destroy, assemble, disassemble, Transit | Create/delete delta and ECS snapshot | Stable `ShipId` is the recovery key, not ECS entity handle. |
| Player ownership and active-ship maps | Yes | Admission, Station operations, Transit | Ordered map delta and ECS snapshot | Resume/admission authority depends on exact restoration. |
| Position, velocity, anchor | Yes | Movement, Warp, docking, Transit, commands | Component final-value delta and ECS snapshot | `VelocityChanged` does not cover every position or representation change. |
| Thrust/braking and flight modes | Yes | Move/Stop/Approach/Orbit/KeepAtRange/Warp | Component add/remove/update delta and ECS snapshot | Determines future motion even when no event is emitted. |
| Hull shield/armor/hull/destroyed state | Yes | Combat and repair | Component final-value delta and ECS snapshot | Public damage/repair events are outputs, not reducer authority. |
| Capacitor current | Yes | Capacitor system every Tick | Component final-value delta and ECS snapshot | Recharge can occur without a public event. |
| Fitted slots, active flags, cycle counters, targets | Yes | Fit/Unfit/Activate/Deactivate/Cap/range gate | Component final-value delta and ECS snapshot | Countdown and forced deactivation affect later combat. |
| Derived ship stats from fitting/catalog | Reconstructible | `apply_fitting` | Recompute from recovered fitting plus catalog fingerprint | Persist only when needed for validation. |
| Lock entries, states, countdowns | Yes | Lock system and docking cleanup | Component final-value delta and ECS snapshot | Snapshot coverage must be complete. |
| Tackle membership | Yes | Tackle system | Component final-value delta and ECS snapshot | Prevents Warp/Jump and must survive restart exactly. |
| Ship cargo and destruction rewards | Yes | Station operations and combat reward | Ordered item-stack delta and ECS snapshot | Reward mutation can accompany ship deletion. |
| Docked ship/player Station context | Yes | Dock/Undock/Disembark/Select | Ordered map delta and ECS snapshot | Required to authorize Station operations. |
| Station inventory / packaged ships | Yes, journal-owned aggregate | Station command execution and SQLite projection | Station aggregate delta in the same transition envelope plus versioned Station checkpoint | SQLite is an idempotent projection keyed by transition identity, not a second authority. |
| Transit state, receipts, imported/exported handoff | Yes | Raft-committed Transit apply | Delta and ECS snapshot; durable idempotency identity | External Redirect follows commit and is not recovery state. |
| Resume-ticket current/staged binding | Yes | Admission and Transit | Delta and ECS snapshot | Ticket rotation must remain retry-safe. |
| Bot persistent behavior state | Yes when it affects future decisions | Bot components/state | Component delta and ECS snapshot | Purely recomputable selection may be derived only when specified. |
| Pending human command queue | No, until admitted into a transition | Runtime connection | Runtime input queue | Disconnect/crash may require client retry. |
| Pending bot lock-command queue | Yes until same-Tick consumption replaces it | Produced at end of one Tick, consumed by the next | Ordered queue delta and ECS snapshot | It changes the next Tick's outcome and cannot remain an in-memory gap. |
| Auto-jump Raft proposal after Warp arrival | Reliable post-commit obligation | `process_warp` -> `pending_auto_jumps` -> `run_runtime_tick` | Durable outbox intent committed with the Warp transition | The in-memory queue is only a convenience projection; crash before `raft.propose` must retry the intent. |
| Completed-warp client correction | Runtime presentation output | `completed_warps` | None unless delivery is promoted to a reliable protocol | It does not affect authoritative state; reconnect/state sync may repair presentation. |
| AoI index and client projections | Derived | Rebuilt/read from committed world | Recompute after recovery | Never journal presentation caches as authority. |
| Socket/session handles and channel queues | No | Runtime adapters | None | Re-established after recovery. |
| Replication send position / append receipt | Durable runtime metadata | Runtime/journal layer | Commit index/checkpoint metadata | Must refer only to committed ranges. |
| Station SQLite applied position | Durable projection metadata | Station projection layer | `(sector_id, transition_id)` dedup records plus projection watermark | Promotion/serving is forbidden while this watermark trails the required authoritative position. |
| `DomainEvent` list | Durable public fact | Command/Tick output | Public-event subrecord committed with the transition; not used as exact state reducer | Delivery resumes from a durable cursor. |
| Reliable runtime effects | Reliable post-commit obligation | Runtime/application layer | Outbox subrecord plus idempotency identity | Execute only after local state commit; cursor advances after downstream acknowledgement. |
| Deliberately lossy runtime effects | No | Runtime/application layer | None | Must be explicitly classified and cannot be required for authoritative continuity. |

## 3. Transition shape

A prepared transition separates exact recovery state, public facts, and runtime
effects:

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

The durable batch is one logical atomic commit envelope. Its state, event, and
outbox subrecords may live in separate immutable segment families, but no reader
may observe any subrecord as committed until all referenced bytes and the envelope
commit marker are durable under the selected durability profile.

`domain_events` may be empty but is never written through a second best-effort
persistence step. Effects that do not require reliable retry remain post-commit
runtime outputs; reliable effects are represented by outbox intents in the batch.

`PreparedMutation` must not mutate the committed live world before durable append.
It may own old/new values, a reversible write set, or a copy-on-write overlay.
Its canonical committed result must be equivalent to applying `RecoveryDelta`.

A `RecoveryDelta` is applied through one reducer:

```text
apply_recovery_delta(state, delta)
```

The same function or shared primitives are used for:

- local commit after durable append;
- checkpoint-tail recovery;
- replica catch-up; and
- equivalence/failure-injection tests.

### 3.1 Station inventory projection rule

Station item changes are part of `RecoveryDelta`; they are not committed first to
SQLite. After the journal envelope is durable, the live reducer applies ECS and
aggregate state, then applies the same Station transition idempotently to SQLite.
The projection records `(sector_id, transition_id)` and a contiguous applied
watermark so replaying an already-applied transition is a no-op and promotion can
prove projection freshness.

A Station operation is acknowledged only after journal durability, successful
live application, and successful local SQLite projection. If projection application
fails after append, the Sector fences and recovers or catches up SQLite from the
Station checkpoint plus committed Station deltas before serving more authoritative
work.

### 3.2 Auto-jump outbox rule

When a committed Tick completes a Warp whose `WarpComp::auto_jump` is true, the
same transition includes a durable `AutoJumpProposalIntent` (name may vary) with:

- `ship_id` and `gate_id`;
- resolved route/destination information needed for retry;
- originating transition/idempotency identity; and
- any compatibility/version metadata required by the Raft adapter.

`pending_auto_jumps` may continue to exist as an in-memory post-commit work queue,
but it is derived from the retained outbox. Draining it is not the durable state
transition. A crash before, during, or after `raft.propose` may cause retry, so the
proposal/Transit path must deduplicate by the stable intent identity.

## 4. Fail-stop rule

Once an envelope is durably committed, it cannot be converted into an ordinary
rejection. If live reduction or a required local projection errors, panics, or
partially applies, the Sector must immediately:

1. mark itself fenced/unhealthy;
2. stop admitting, preparing, or committing later transitions;
3. stop replication beyond the last successfully applied contiguous position;
4. suppress event delivery, outbox execution, and acknowledgement for the failed
   local apply; and
5. terminate or reconstruct state/projections from the journal before resuming.

Continuing with the pre-transition live state after a successful append is a
contract violation.

## 5. Durability profiles and acknowledgement

The system has two semantic durability profiles. Implementations may use different
names, but must preserve these failure-domain distinctions.

### LocalDurable

Before acknowledgement, the envelope, every referenced subrecord, and required
commit metadata are written, flushed, and `fsync`/platform-equivalent durable.
Acknowledged RPO is **zero committed transitions for process crash, OS
crash/reboot, and abrupt power loss when the authoritative storage medium remains
readable and preserves completed durable writes**. Loss/corruption of that
machine/storage device is outside this profile.

### ReplicatedDurable

Before acknowledgement, `LocalDurable` is satisfied and the same committed range
is synchronously durable on the configured replica quorum. A deployment may claim
acknowledged RPO 0 for owner-node/storage loss only up to the explicitly documented
replica-failure tolerance. #271/#280 must define quorum size, commit evidence, and
failure tolerance before this profile is used as a production promise.

An unacknowledged request may be absent or already durably committed. Retried
operations that require exactly-once semantics use a stable idempotency/transition
identity.

## 6. Crash-point matrix

| Crash/failure point | Durable journal | Live/recovered state | Client/external observation | Required behavior |
|---|---|---|---|---|
| Before prepare | No new envelope | Previous committed state | No success | Operation may be retried. |
| During prepare | No new envelope | Previous committed state | No success | Discard bounded prepared state. |
| Before/during failed atomic append | No valid committed envelope | Previous committed state | No success/effect | Reject, truncate, or quarantine partial bytes according to #271 framing. |
| After append commit, before live commit | Complete envelope exists | Recovery applies delta | Success may not have been observed | Retry detects committed transition identity. |
| During live commit | Complete envelope exists | Recovery reapplies complete delta | No success until local apply completes | Fence immediately; do not continue from old or partial state. |
| During SQLite projection apply | Complete envelope exists | Recovery reapplies Station delta idempotently | No acknowledgement | Fence; rebuild/catch up projection before resuming. |
| After Warp commit, before auto-jump Raft proposal | Envelope plus auto-jump intent exists | Warp arrival remains committed | Proposal may be absent | Resume outbox and retry proposal by intent identity. |
| After Raft proposal attempt, before outbox cursor advance | Envelope plus intent exists | Warp arrival remains committed | Proposal may have been accepted | Retry is allowed; Raft/Transit path deduplicates stable identity. |
| After live commit, before public-event delivery | Envelope and event subrecord exist | New committed state | Event may not yet be observed | Resume from durable event cursor; never regenerate by simulation. |
| After downstream delivery ack, before cursor fsync | Output remains retained | New committed state | Consumer may already have output | Retry may duplicate; idempotency prevents duplicate effect. |
| After cursor fsync | Output may become retention-eligible | New committed state | Delivery acknowledged | Never redeliver before cursor; compaction still obeys all consumer/archive watermarks. |
| After acknowledgement | Envelope exists under selected durability profile and required local projections applied | New committed state | Success observed | RPO 0 only within the selected profile's documented failure domain. |
| During checkpoint member write/validation | Journal remains authority | Previous checkpoint set plus tail | No recovery gap | Keep previous complete checkpoint manifest. |
| After checkpoint manifest publication, before compaction | New checkpoint and old tails coexist | Either valid recovery path | None | Both cover the same committed position. |
| During/after state-delta compaction | Checkpoint plus remaining authoritative tail | Exact covered state plus tail | None | Never retire a delta before the complete checkpoint set and manifest are durable. |
| Before event/outbox retention watermark | Public facts/intents still retained | State checkpoint may already cover delta | Pending delivery remains possible | State compaction does not delete undelivered event/outbox subrecords. |
| Replica receives partial/out-of-order range | Local authority unchanged | Replica stops at last contiguous commit | No promoted state | Detect gap, duplicate, version, fingerprint, or hash mismatch. |
| Replica has state but lacks output segments/cursors | State may be current | Catch-up-only | Promotion forbidden | Synchronize retained outputs and cursors before promotion. |
| Replica has state/outputs but stale SQLite watermark | State may be current | Catch-up-only | Station serving/promotion forbidden | Rebuild/apply Station projection through promotion position first. |

## 7. Checkpoint compatibility

A complete checkpoint manifest must reject incompatible or incomplete data before
constructing live state. At minimum it carries:

```text
magic
checkpoint_manifest_version
sector_id / node identity as required
catalog_fingerprint
covered_journal_position
ECS snapshot member identity, length, checksum, format version
Station aggregate checkpoint identity, length, checksum, format version
retained event/outbox segment locations or retention metadata
```

A field-list change is an explicit format-version change. Backward migration may
be added deliberately, but postcard decode failure is not a versioning strategy.

The ECS snapshot and Station aggregate checkpoint form one recovery point. The
manifest is publishable only after every required member is durable, validated,
and mutually consistent at the same covered journal position.

Reliable post-commit obligations such as auto-jump proposals live in retained
outbox records. Snapshotting a transient runtime queue is not a substitute for
retaining those intents.

Checkpoint coverage applies to the authoritative delta stream. Public-event and
outbox retention use independent consumer watermarks; state coverage does not
prove that outputs have been delivered or archived.

## 8. Independent retention and compaction

The commit envelope provides atomic visibility while substreams retain independently:

- authoritative state deltas may compact behind a complete validated checkpoint;
- public events retain according to hot-log/archive policy;
- outbox intents retain until every required consumer durably advances; and
- compact commit-index metadata preserves range coverage and remaining subrecord
  references.

Compaction is copy-and-publish:

1. write and validate replacement checkpoint/segment files;
2. fsync files and replacement manifest;
3. atomically publish the manifest;
4. retire old state-delta segments only after publication; and
5. retain enough old material for rollback to the previous valid manifest.

No committed public event is rewritten in place. No outbox intent may be removed
while a required consumer cursor still points at or before it.

## 9. Durable delivery cursor contract

Each durable public-event/outbox consumer has a cursor representing the last
output durably acknowledged by that consumer (or equivalently the next output to
attempt). Delivery is at-least-once:

1. select the next committed output after the durable cursor;
2. send it with its transition/output idempotency identity;
3. wait for downstream acknowledgement or equivalent durable idempotency proof;
4. durably advance the consumer cursor; and
5. only then permit retention/compaction policies to consider that output covered.

The cursor must never advance before step 3. A crash between steps 3 and 4 can
redeliver an already accepted output; downstream consumers/protocols must therefore
be idempotent. A local cursor alone does not provide exactly-once external effects.

Cursor state required after failover is replicated/transferred with retained output
segments. A replica with a cursor ahead of available output bytes or output bytes
behind the cursor proof is invalid and non-promotable.

## 10. Replica catch-up and promotion

A snapshot-transfer/catch-up bundle includes:

- the complete checkpoint set and its covered journal position;
- every contiguous committed authoritative tail record after that position;
- retained public-event and outbox segments that may still require delivery after
  promotion;
- durable event/outbox consumer cursors or equivalent replicated cursor state; and
- Station checkpoint/delta data sufficient to advance the local SQLite projection
  to the promotion position.

Promotion eligibility requires all of the following:

1. compatible checkpoint members and fingerprints;
2. no authoritative range gaps through the promoted position;
3. all retained event/outbox bytes needed after promotion;
4. cursor state that cannot skip an undelivered committed output;
5. successful invariant validation after applying the shared reducer; and
6. a Station SQLite applied watermark at or beyond the promoted authoritative
   position, established before any Station read/write is served.

A replica that satisfies ECS state equivalence but lacks required output data,
cursor proof, or Station projection freshness remains non-promotable.

## 11. Determinism boundary

The engine should remain deterministic where practical, but exact recovery does not
re-execute historical code. Process randomness, floating-point behavior, catalog
evolution, iteration ordering, AI implementation, and pending-queue contents can
change outcomes. The state-delta journal captures committed outcomes and keeps the
recovery contract stable across those implementation changes.

Diagnostic metadata such as accepted request identity, random seed/draws, or system
version may be included, but it is supplementary. `DomainEvent`s are mandatory
durable public outputs when produced, not inputs to the exact state reducer.

## 12. RTO status

RTO is intentionally **TBD**, not implied by the recovery procedure. #284 must
benchmark representative ship counts and eventless Ticks, then define:

- maximum authoritative tail transitions and bytes;
- maximum replay time on named reference hardware;
- checkpoint cadence and trigger thresholds; and
- the production recovery target derived from those limits.

Until those measurements land, the checklist item for numeric RTO remains open.

## 13. Required tests

The implementation of #271/#272/#284 must eventually cover:

- append failure before commit for a simple command;
- append failure for an eventless Tick with movement/capacitor changes;
- crash after append and before live commit;
- live reducer error/panic/partial apply after append, proving fail-stop fencing;
- SQLite projection failure after append and idempotent catch-up by transition id;
- replica promotion blocked by a stale SQLite projection watermark;
- pending bot lock-command queue checkpoint-plus-tail equivalence;
- crash after Warp commit and before auto-jump Raft proposal, proving outbox retry;
- ambiguous crash after auto-jump Raft proposal, proving idempotent retry;
- crash after live commit and before public-event publication, proving cursor-based
  delivery resumes from the committed event subrecord;
- crash after downstream delivery acknowledgement but before cursor durability,
  proving duplicate delivery is safe and no output is skipped;
- checkpoint plus tail equivalence for motion, capacitor, locks, module cycles,
  docking, Station inventory, admission, combat randomness, and Transit;
- duplicate, missing, out-of-order, corrupt, or incompatible envelopes;
- incomplete checkpoint set, corrupt member, and incompatible format/fingerprint;
- compaction crash before and after manifest publication;
- state-delta compaction cannot remove undelivered public events/outbox intents;
- snapshot-based replica catch-up cannot promote without retained outputs/cursors;
- LocalDurable acknowledgement survives process/OS/power-loss injection with the
  durable medium intact;
- ReplicatedDurable acknowledgement is not emitted before configured quorum commit;
- no public event or external effect before durable commit; and
- idempotent recovery/retry of a transition already durably appended.
