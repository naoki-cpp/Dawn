---
scope    : Normative Sector recovery sources, durability points, and crash outcomes
audience : AI Agent / Human Developer
update   : When authoritative state, transition ordering, snapshots, journal payloads, Station persistence, or replication semantics change
related  : ../adr/ADR-0049-sector-recovery-state-delta-wal.md, tick-model.md, event-catalog.md
---

# Sector Recovery Contract

This document is the normative inventory and crash matrix for accepted ADR-0049
and issue #284. It distinguishes exact recovery state from public facts and
runtime-only effects. It is normative for #271 and #272: adding an authoritative
field requires assigning it a recovery source before the change lands.

If older text in ADR-0001, ADR-0017, ADR-0038, `AI_DEVELOPMENT_GUIDE.md`,
`tick-model.md`, or `event-catalog.md` conflicts with this document, ADR-0049 and
this contract take precedence. In particular, public-event append is not the
complete state commit, SQLite is not an independent Station authority, and an
eventless Tick still has a durable recovery record.

## 1. Recovery guarantees

- A successfully acknowledged authoritative transition has RPO 0 under the
  production durability mode selected by #271.
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
- A replica is promotable only when both authoritative state and retained
  public-output/outbox state are complete and contiguous.
- No numeric production RTO is currently claimed. #284 must benchmark replay
  tail size/time and set the checkpoint budget before marking RTO complete.

## 2. State classification

| State / mutation | Authoritative? | Current source or mutation site | Required recovery source | Notes |
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
| AoI index and client projections | Derived | Rebuilt/read from committed world | Recompute after recovery | Never journal presentation caches as authority. |
| Socket/session handles and channel queues | No | Runtime adapters | None | Re-established after recovery. |
| Replication send position / append receipt | Durable runtime metadata | Runtime/journal layer | Commit index/checkpoint metadata | Must refer only to committed ranges. |
| `DomainEvent` list | Durable public fact | Command/Tick output | Public-event subrecord committed with the transition; not used as exact state reducer | Delivery resumes from a durable cursor. |
| Runtime effects (Redirect, Raft proposal, loadout refresh) | No, but may need durable intent | Runtime/application layer | Outbox subrecord or explicit idempotent protocol | Execute only after local state commit. |

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
commit marker are durable.

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

Public-event and outbox delivery use their own committed cursors. They do not
rerun the state reducer and cannot expose data from an uncommitted envelope.

### 3.1 Station inventory projection rule

Station item changes are part of `RecoveryDelta`; they are not committed first to
SQLite. After the journal envelope is durable, the live reducer applies ECS and
aggregate state, then applies the same Station transition idempotently to SQLite.
The projection records `(sector_id, transition_id)` so replaying an already-applied
transition is a no-op.

A Station operation is acknowledged only after journal durability, successful
live application, and successful local SQLite projection. If projection application
fails after append, the Sector fences and recovers or catches up SQLite from the
Station checkpoint plus committed Station deltas before serving more authoritative
work.

## 4. Fail-stop rule

Once an envelope is durably committed, it cannot be converted into an ordinary
rejection. If live reduction or a required local projection errors, panics, or
partially applies, the Sector must immediately:

1. mark itself fenced/unhealthy;
2. stop admitting, preparing, or committing later transitions;
3. stop replication beyond the last successfully applied contiguous position;
4. suppress event delivery, outbox execution, and acknowledgement for the failed
   local apply; and
5. terminate or reconstruct state from the journal before resuming.

Continuing with the pre-transition live state after a successful append is a
contract violation.

## 5. Crash-point matrix

| Crash/failure point | Durable journal | Live state after restart | Client/external observation | Required behavior |
|---|---|---|---|---|
| Before prepare | No new envelope | Previous committed state | No success | Operation may be retried. |
| During prepare | No new envelope | Previous committed state | No success | Discard bounded prepared state. |
| Before/during failed atomic append | No valid committed envelope | Previous committed state | No success/effect | Reject, truncate, or quarantine partial bytes according to #271 framing. |
| After append commit, before live commit | Complete envelope exists | Recovery applies delta | Success may not have been observed | Retry detects committed transition identity. |
| During live commit | Complete envelope exists | Recovery reapplies complete delta | No success until local apply completes | Fence immediately; do not continue from old or partial state. |
| During SQLite projection apply | Complete envelope exists | Recovery reapplies Station delta idempotently | No acknowledgement | Fence; rebuild/catch up projection before resuming. |
| After live commit, before public-event delivery | Envelope and event subrecord exist | New committed state | Event may not yet be observed | Resume from durable event cursor; never regenerate by simulation. |
| After live commit, before runtime effect | Envelope and outbox intent exist when required | New committed state | Effect may be missing | Resume durable outbox or use explicit idempotent protocol. |
| After acknowledgement | Complete envelope exists and required local projection applied | New committed state | Success observed | RPO 0 for this transition. |
| During checkpoint member write/validation | Journal remains authority | Previous checkpoint set plus tail | No recovery gap | Keep previous complete checkpoint manifest. |
| After checkpoint manifest publication, before compaction | New checkpoint and old tails coexist | Either valid recovery path | None | Both cover the same committed position. |
| During/after state-delta compaction | Checkpoint plus remaining authoritative tail | Exact covered state plus tail | None | Never retire a delta before the complete checkpoint set and manifest are durable. |
| Before event/outbox retention cursor | Public facts/intents still retained | State checkpoint may already cover delta | Pending delivery remains possible | State compaction does not delete undelivered event/outbox subrecords. |
| Replica receives partial/out-of-order range | Local authority unchanged | Replica stops at last contiguous commit | No promoted state | Detect gap, duplicate, version, fingerprint, or hash mismatch. |
| Replica has state but lacks output segments/cursors | State may be current | Catch-up-only | Promotion forbidden | Synchronize retained outputs and cursors before promotion. |

## 6. Checkpoint compatibility

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

Checkpoint coverage applies to the authoritative delta stream. Public-event and
outbox retention use independent consumer watermarks; state coverage does not
prove that outputs have been delivered or archived.

## 7. Independent retention and compaction

The commit envelope provides atomic visibility while substreams retain independently:

- authoritative state deltas may compact behind a complete validated checkpoint;
- public events retain according to hot-log/archive policy;
- outbox intents retain until required consumers durably advance; and
- compact commit-index metadata preserves range coverage and remaining subrecord
  references.

Compaction is copy-and-publish:

1. write and validate replacement checkpoint/segment files;
2. fsync files and replacement manifest;
3. atomically publish the manifest;
4. retire old state-delta segments only after publication; and
5. retain enough old material for rollback to the previous valid manifest.

No committed public event is rewritten in place.

## 8. Replica catch-up and promotion

A snapshot-transfer/catch-up bundle includes:

- the complete checkpoint set and its covered journal position;
- every contiguous committed authoritative tail record after that position;
- retained public-event and outbox segments that may still require delivery after
  promotion; and
- durable event/outbox consumer cursors or equivalent replicated cursor state.

Promotion eligibility requires all of the following:

1. compatible checkpoint members and fingerprints;
2. no authoritative range gaps through the promoted position;
3. all retained event/outbox bytes needed after promotion;
4. cursor state that cannot skip an undelivered committed output; and
5. successful invariant validation after applying the shared reducer.

A replica that satisfies state equivalence but lacks required output data remains
non-promotable.

## 9. Determinism boundary

The engine should remain deterministic where practical, but exact recovery does not
re-execute historical code. Process randomness, floating-point behavior, catalog
evolution, iteration ordering, AI implementation, and pending-queue contents can
change outcomes. The state-delta journal captures committed outcomes and keeps the
recovery contract stable across those implementation changes.

Diagnostic metadata such as accepted request identity, random seed/draws, or system
version may be included, but it is supplementary. `DomainEvent`s are mandatory
durable public outputs when produced, not inputs to the exact state reducer.

## 10. RPO and RTO status

Acknowledged RPO is exactly zero committed transitions. An unacknowledged request
may be absent or committed and therefore requires idempotent retry when duplicate
execution matters.

RTO is intentionally **TBD**, not implied by the recovery procedure. #284 must
benchmark representative ship counts and eventless Ticks, then define:

- maximum authoritative tail transitions and bytes;
- maximum replay time on named reference hardware;
- checkpoint cadence and trigger thresholds; and
- the production recovery target derived from those limits.

Until those measurements land, the checklist item for numeric RTO remains open.

## 11. Required tests

The implementation of #271/#272/#284 must eventually cover:

- append failure before commit for a simple command;
- append failure for an eventless Tick with movement/capacitor changes;
- crash after append and before live commit;
- live reducer error/panic/partial apply after append, proving fail-stop fencing;
- SQLite projection failure after append and idempotent catch-up by transition id;
- pending bot lock-command queue checkpoint-plus-tail equivalence;
- crash after live commit and before public-event publication, proving cursor-based
  delivery resumes from the committed event subrecord;
- crash before a reliable runtime effect, proving outbox/idempotent retry behavior;
- checkpoint plus tail equivalence for motion, capacitor, locks, module cycles,
  docking, Station inventory, admission, combat randomness, and Transit;
- duplicate, missing, out-of-order, corrupt, or incompatible envelopes;
- incomplete checkpoint set, corrupt member, and incompatible format/fingerprint;
- compaction crash before and after manifest publication;
- state-delta compaction cannot remove undelivered public events/outbox intents;
- snapshot-based replica catch-up cannot promote without retained outputs/cursors;
- no public event or external effect before durable commit; and
- idempotent recovery/retry of a transition already durably appended.
