---
scope    : Normative Sector recovery sources, durability points, and crash outcomes
audience : AI Agent / Human Developer
update   : When authoritative state, transition ordering, snapshots, journal payloads, or replication semantics change
related  : ../adr/ADR-0049-sector-recovery-state-delta-wal.md, tick-model.md, event-catalog.md
---

# Sector Recovery Contract

This document is the working inventory and crash matrix for ADR-0049 and issue
#284. It distinguishes exact recovery state from public facts and runtime-only
effects. The table is normative for #271 and #272: adding an authoritative field
requires assigning it a recovery source before the change lands.

## 1. Recovery guarantees

- A successfully acknowledged authoritative transition has RPO 0 under the
  production durability mode selected by #271.
- Exact operational recovery is: compatible snapshot + every committed recovery
  batch after the snapshot's covered journal position.
- Public `DomainEvent`s alone are not a complete recovery source.
- Public events produced by a transition are nevertheless durable and are committed
  atomically with its authoritative recovery delta.
- Every committed Tick has a recovery record, including an eventless Tick.
- Recovery, local commit, and replica application use the same delta reducer and
  invariant checks.
- A journal append failure cannot change committed live state or produce
  client-visible success.

## 2. State classification

| State / mutation | Authoritative? | Current source or mutation site | Required recovery source | Notes |
|---|---:|---|---|---|
| Logical `current_tick` | Yes | Incremented before Tick systems | Transition header/delta and snapshot | Must advance for eventless Ticks only after durable append. |
| Entity/Player ID counters | Yes | Admission/spawn helpers | Counter delta and snapshot | Reuse after crash violates identity invariants. |
| Ship existence and type | Yes | Spawn, destroy, assemble, disassemble, Transit | Create/delete delta and snapshot | Stable `ShipId` is the recovery key, not ECS entity handle. |
| Player ownership and active-ship maps | Yes | Admission, station operations, Transit | Ordered map delta and snapshot | Resume/admission authority depends on exact restoration. |
| Position, velocity, anchor | Yes | Movement, Warp, docking, Transit, commands | Component final-value delta and snapshot | `VelocityChanged` does not cover all position changes. |
| Thrust/braking and flight modes | Yes | Move/Stop/Approach/Orbit/KeepAtRange/Warp | Component add/remove/update delta and snapshot | Determines future motion even when no event is emitted. |
| Hull shield/armor/hull/destroyed state | Yes | Combat and repair | Component final-value delta and snapshot | Public damage/repair events are outputs, not the reducer authority. |
| Capacitor current | Yes | Capacitor system every Tick | Component final-value delta and snapshot | Recharge can occur without a public event. |
| Fitted slots, active flags, cycle counters, targets | Yes | Fit/Unfit/Activate/Deactivate/Cap/range gate | Component final-value delta and snapshot | Cycle countdown and forced deactivation affect later combat. |
| Derived ship stats from fitting/catalog | Reconstructible | `apply_fitting` | Recompute from recovered fitting + catalog fingerprint | Do not persist duplicate derived values unless needed for validation. |
| Lock entries, states, countdowns | Yes | Lock system and docking cleanup | Component final-value delta and snapshot | Current snapshot coverage must be expanded if incomplete. |
| Tackle membership | Yes | Tackle system | Component final-value delta and snapshot | Prevents Warp/Jump and must survive restart exactly. |
| Cargo inventory and destruction rewards | Yes | Station operations and combat reward | Ordered item-stack delta and snapshot | Reward mutation can accompany ship deletion. |
| Docked ship/player station context | Yes | Dock/Undock/Disembark/Select | Ordered map delta and snapshot | Required to authorize station operations. |
| Station inventory / packaged ships | Yes, separate aggregate | Station operation execution / SQL boundary | Transition-linked aggregate delta or atomic participant record | #272 must not acknowledge one side without the other. |
| Transit state, receipts, imported/exported handoff | Yes | Raft-committed Transit apply | Delta and snapshot; durable idempotency identity | External Redirect follows commit and is not recovery state. |
| Resume-ticket current/staged binding | Yes | Admission and Transit | Delta and snapshot | Ticket rotation must remain retry-safe across crash points. |
| Bot persistent behavior state | Yes when it affects future decisions | Bot components/state | Component delta and snapshot | Purely recomputable target selection may be derived only if specified. |
| Pending human command queue | No, until admitted into a transition | Runtime connection | Runtime input queue | Disconnect/crash may require client retry; not part of committed state. |
| Pending bot lock command queue | Transitional | Produced at end of one Tick, consumed by the next | Persist as authoritative queue state or eliminate by same-Tick deterministic plan | It affects future state and cannot remain an undocumented in-memory gap. |
| AoI index and client projections | Derived | Rebuilt/read from committed world | Recompute after recovery | Never journal presentation caches as authority. |
| Socket/session handles and channel queues | No | Runtime adapters | None | Re-established after recovery. |
| Replication send position / append receipt | Runtime durable metadata | Runtime/journal layer | Journal/checkpoint metadata | Must refer only to committed ranges. |
| `DomainEvent` list | Durable public fact | Command/Tick output | Stored atomically with the transition delta; not used as exact state reducer | Delivery resumes from a durable cursor after crash. |
| Runtime effects (Redirect, Raft proposal, loadout refresh) | No, but may need durable intent | Runtime/application layer | Outbox/idempotency record when retry is required | Execute only after state commit. |

## 3. Transition shape

A prepared transition separates exact state recovery, public facts, and runtime
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

The durable batch is appended atomically. `domain_events` may be empty but is never
written through a second best-effort persistence step. Effects that do not require
reliable retry remain post-commit runtime outputs; reliable effects are represented
by an outbox intent in the batch.

`PreparedMutation` is not serialized directly unless its canonical representation
is the recovery delta. It may own old/new values, a reversible write set, or a
copy-on-write overlay. It must not mutate the committed live world before the
journal append succeeds.

A `RecoveryDelta` is applied through one reducer:

```text
apply_recovery_delta(state, delta)
```

The same function or shared primitives are used for:

- local commit after durable append;
- snapshot-tail recovery;
- replica catch-up; and
- equivalence/failure-injection tests.

Public-event and outbox delivery use their own committed journal cursors. They do
not re-run the state reducer and cannot expose data from an uncommitted batch.

## 4. Crash-point matrix

| Crash/failure point | Durable journal | Live state after restart | Client/external observation | Required behavior |
|---|---|---|---|---|
| Before prepare | No new batch | Previous committed state | No success | Operation may be retried. |
| During prepare | No new batch | Previous committed state | No success | Discard bounded prepared state. |
| Before/during failed atomic append | No valid committed batch | Previous committed state | No success/effect | Partial bytes are rejected/truncated/quarantined by #271 policy. |
| After append commit, before live commit | Complete batch exists | Recovery applies delta | Success may not have been observed | Retry must be idempotent or detect committed transition identity. |
| During live commit | Complete batch exists | Recovery re-applies complete delta | No success until commit completes | Commit reducer must be atomic at the engine visibility boundary. |
| After live commit, before public-event delivery | Complete batch and events exist | New committed state | Event may not yet be observed | Resume delivery from the durable event cursor; do not regenerate by simulation. |
| After live commit, before runtime effect | Complete batch exists | New committed state | Effect may be missing | Resume durable outbox intent or rely on an explicit idempotent protocol. |
| After acknowledgement | Complete batch exists | New committed state | Success observed | RPO 0 for this transition. |
| During snapshot temp write/validation | Journal remains authority | Previous snapshot + tail | No recovery gap | Keep previous authoritative snapshot. |
| After snapshot replace, before compaction | New snapshot and old tail coexist | Either valid recovery path | None | Both cover the same committed position. |
| During/after state compaction | Snapshot + remaining delta tail | Exact state at covered position + tail | None | Never delete a delta not covered by a durable validated snapshot. |
| Before event/outbox retention cursor | Public facts/intents still retained | State snapshot may already cover delta | Pending delivery remains possible | State compaction must not delete undelivered facts/intents. |
| Replica receives partial/out-of-order range | Local authority unchanged | Replica stops at last contiguous commit | No promoted state | Detect gap/duplicate/version/fingerprint mismatch. |

## 5. Snapshot compatibility

The snapshot envelope must reject incompatible data before constructing live state.
At minimum it carries:

```text
magic
snapshot_format_version
sector_id / node identity as required
catalog_fingerprint
covered_journal_position
payload_length/checksum (at storage framing layer or snapshot envelope)
authoritative state payload
```

A field-list change is an explicit format version change. Backward migration may be
added deliberately, but postcard decode failure is not treated as a versioning
strategy.

Snapshot coverage applies to the authoritative delta stream. Public-event and
outbox retention use independent consumer/retention cursors; a state snapshot does
not prove that those durable outputs have been delivered or archived.

## 6. Determinism boundary

The engine should remain deterministic where practical, but exact recovery does not
re-execute historical code. Current obstacles to input-only replay include process
randomness in combat, floating-point implementation details, catalog evolution, and
iteration ordering. The state-delta journal captures committed outcomes and therefore
keeps the recovery contract stable across those implementation changes.

A durable transition may also include diagnostic causal metadata such as accepted
request identity, random seed/draws, or system version. Such metadata is
supplementary. `DomainEvent`s are mandatory durable public outputs of the transition,
but they are not inputs to the exact state reducer.

## 7. Required tests

The implementation of #271/#272/#284 must eventually cover:

- append failure before commit for a simple command;
- append failure for an eventless Tick with movement/capacitor changes;
- crash after append and before live commit;
- crash after live commit and before public-event publication, proving delivery
  resumes from the durable batch;
- crash before a reliable runtime effect, proving outbox/idempotent retry behavior;
- snapshot plus tail equivalence for motion, capacitor, locks, module cycles,
  docking, admission, combat randomness, and Transit;
- duplicate/missing/out-of-order/incompatible recovery batches;
- snapshot corruption and incompatible version/catalog fingerprint;
- compaction boundary and replica catch-up from a snapshot;
- state compaction cannot remove undelivered public events/outbox intents;
- no public event or external effect before durable commit; and
- idempotent recovery/retry of a transition already durably appended.
