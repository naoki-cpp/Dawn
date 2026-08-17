---
scope    : Normative Sector recovery sources, durability points, and crash outcomes
audience : AI Agent / Human Developer
update   : When authoritative state, transition ordering, snapshots, journal payloads, Station persistence, admission/identity persistence, delivery cursors, or replication semantics change
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
  fsync mechanics, corruption handling, indices/receipts, and physical retention
  implementation;
- **#272:** storage-independent engine API and prepare -> durable append -> live
  apply -> post-commit effect ordering;
- **#275:** in-memory state-owner decomposition. It consumes this table rather
  than redefining which fields are durable;
- **#276:** durable Transit Saga/attempt repository and retry lifecycle. The
  current implementation stores `TransitSagaSnapshot` inside the canonical
  `NodeState` carried by both `StateSnapshot` and `TickRecoveryDelta`; it owns
  the concrete attempt/receipt representation and must satisfy this recovery
  contract;
- **#277:** admission/identity/Station repository schema and transaction APIs.
  Admission/identity protocol state and pre-materialization identity consumption
  may be repository-owned authority as specified below; Station world-state
  authority remains the Sector recovery journal;
- **#278:** runtime/application orchestration. It owns configured durability-profile
  selection, replica-set/quorum policy, durability-ack aggregation, owner-epoch/
  fencing integration, and final acknowledgement gating;
- **#280:** peer transport, snapshot/catch-up transfer mechanics, and traffic
  isolation. It transports durability/catch-up messages and the representation
  selected here rather than defining a competing recovery or quorum policy.

Older documents are amended in PR #288 where they make a conflicting recovery
claim. Historical implementation descriptions may remain when explicitly labeled
as legacy/current behavior rather than normative recovery authority.

## 1. Recovery guarantees

- Exact operational Sector world-state recovery is: newest complete compatible
  checkpoint set plus every committed authoritative recovery batch after its
  covered position.
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
- Admission/identity protocol state that exists before or beside materialized
  Sector world state may be authoritative in #277's durable repository. Such
  state must have explicit reconciliation/catch-up semantics and cannot be treated
  as an ordinary Station read-model projection.
- Once a fresh `PlayerId`/`ShipId` has been durably reserved, allocator recovery
  must treat it as consumed even if no Ship was ever materialized. A crash may
  create an ID gap; it may never permit reuse of an exposed/reserved identity.
- Public-event/outbox delivery is at-least-once unless a stronger downstream
  idempotent transaction protocol is explicitly provided.
- Generic client commands do **not** currently have a protocol-level request ID.
  Therefore an ambiguous reconnect/retry is not promised exactly-once merely
  because the original transition has an internal journal identity.
- A replica is promotable only when authoritative recovery data and every
  promotion-critical retained output/projection/repository authority are caught
  up or deterministically reconciled to the required position.
- "Acknowledged RPO 0" is qualified by the selected durability profile and its
  explicit failure domain; it is never an unqualified machine-loss promise.
- No portable numeric production RTO is claimed by #284. Tail-size/time
  measurements and the checkpoint budget are operational follow-up work after
  the deployment hardware and peer transport in #280 are fixed.

### #284 implementation

The production checkpoint and recovery path has one explicit storage boundary:

- `StateSnapshot::covered_recovery_index` names the global position covered by
  the checkpoint; it is no longer exposed as the ambiguous public-event
  `log_index`.
- Durable snapshot files and replica snapshot bytes use the `DAWNCKP1` envelope
  with `CHECKPOINT_FORMAT_VERSION` and a payload checksum; unknown magic,
  versions, corrupt, or malformed payloads fail before state construction.
- `StateSnapshot` and full `TickRecoveryDelta` each carry one canonical nested
  `NodeState`. Checkpoint identity/context and ship images, and Tick transition
  metadata, remain outside that node-level payload.
- `FileJournal` is the authoritative runtime journal. Every production Tick is
  appended as one `JournalBatch` whose first record is a versioned
  `RecoveryDelta`, including eventless ticks. The full Tick image carries ship
  ECS state plus the canonical `NodeState` containing allocator,
  ownership/active-ship maps, docking context, and cross-tick bot/auto-jump
  queues.
- Startup restores the checkpoint with its catalog fingerprint, then applies a
  contiguous journal tail. If no checkpoint exists, the configured genesis
  state is constructed once and the same RecoveryDelta reader replays from
  index 0; public-event genesis replay is not required. A tail that starts in
  the middle of a batch, skips a record, repeats a delta, crosses Sectors, or
  has an incompatible payload is rejected before the node serves clients.
- Recovery applies into a newly owned node value and returns that node only
  after the complete tail succeeds. A semantic apply failure consumes/fences
  the candidate rather than leaving a partially recovered node available for
  reuse. When starting from journal index 0, the configured NPC genesis is a
  deterministic deployment invariant and must remain unchanged for that
  journal's lifetime.
- Checkpoint scheduling and recovery-journal compaction use separate recovery
  paths from the legacy public-event log. Public events remain an independent
  projection/audit stream.
- Snapshot publication remains crash-safe and uses the same encode/decode path
  for disk and replica transfer.

The remaining work is outside this issue's local world recovery boundary:
replica transport/promotion (#280), durable Transit continuation (#276), and
admission/Station repository reconciliation (#277). Those consumers must use
this checkpoint-plus-tail contract rather than reintroducing public-event-only
recovery.

### #278 shared runtime frame (current implementation)

The runtime adapters now use one durable frame implementation:

```text
drain committed consensus input through RuntimeConsensus
  -> collect/admit Sector commands through one command pass
  -> prepare the complete Tick RecoveryDelta/public output
  -> append the transition to the selected DurableJournal
  -> apply the same RecoveryDelta to live state
  -> reconcile required repositories/projections or fence the runtime
  -> invoke the output hook and advance consensus time
  -> propose validated auto-jump work and drain presentation transients
```

`dawn-server::runtime_frame::RuntimeFrameHost` is the shared one-Sector owner
boundary. It owns the `SimulationNode`, journal, consensus adapter, runtime
health, and selected durability policy, and delegates the actual transition to
`dawn-sector::transit::run_durable_runtime_frame`.
`dawn-server --bin sector-node` supplies the Raft/FileJournal production adapters;
single-sector serve supplies `LocalRuntimeConsensus`/InMemoryJournal; clustered
serve and `SectorRuntimeDriver` supply Raft/InMemoryJournal adapters. The latter
are test/local durability adapters, not a claim that their in-memory journal
survives process loss.

Server entry points collect normalized commands before invoking the Host and
consume its typed `RuntimeTickOutput` afterward. The Host's output hook is the
production publication point: replication publication is allowed only after
live apply and required reconciliation. AoI delivery, network sends, and
cross-Sector handoff remain outside the Host because they are adapter/coordinator
concerns rather than part of one Sector's durable state transition.

The frame now exposes a `RuntimeDurabilityPolicy` port. Its replicated policy
validates distinct replica membership, current owner epoch, transition/range/
content equality, remote evidence source, and the configured quorum before
live apply. #280 now provides the shared delivery adapter and framing for those
remote receipts and catch-up messages; until #278 wires that adapter into the
runtime quorum policy, production continues to use the local `Synced` profile
and must not advertise remote-loss RPO 0.

The same frame also exposes a required reconciliation hook and an adapter-owned
`RuntimeHealth` gate. It runs after live apply but before presentation or
consensus acknowledgement. A projection or repository error fences the runtime,
returns a fail-stop error, and suppresses publication; the committed recovery
bytes remain available for restart/catch-up. A later Tick is rejected until the
adapter has replayed/reconciled the committed transition and explicitly marks
the runtime recovered. A successful frame return is the final acknowledgement
point for the local profile.

Fencing applies to the Host's entire live-mutation boundary, not only to its Tick
entry point. While fenced, typed admission, ownership adoption, command
collection, Transit/Jump proposal, pending-output drain, checkpoint access, and
the generic mutable-node/state bridges all return `RuntimeFrameHostError::Fenced`
without invoking their mutation closure. Read-only inspection remains available
for diagnosis. Fixture population is separately limited to `Bootstrapping` and
returns `BootstrapClosed` after the first frame. An adapter may return the Host
to `Running` only through the explicit recovery operation after the committed
transition and required repositories have been reconciled; ordinary mutators
cannot serve as an implicit recovery path.

The common presentation boundary is now `dawn-sector::aoi_frame::deliver_sector_sessions`.
It owns the rebuild, committed-event/warp delivery, jump-session removal, and
stale-player cleanup sequence for both the production node and single-sector
serve. Transport adapters provide only session identity, message delivery, and
redirect callbacks. Cluster serve retains a thin multi-Sector routing wrapper
because it must select the destination frame and reseed a handoff observer
before invoking the same per-session `AoiFrame` delivery operation.

## 2. State and obligation classification

| State / mutation / obligation | Authoritative or reliable? | Current source or mutation site | Required recovery source | Notes |
|---|---:|---|---|---|
| Logical `current_tick` | Yes | Incremented before Tick systems | Transition header/delta and checkpoint | Must advance only as part of a durable transition, including eventless Ticks. |
| Entity/Player allocator watermarks | Yes | Admission/spawn helpers | Checkpoint/RecoveryDelta for materialized allocation **plus every durable #277 reservation/allocator record** | Recovery must choose a next value above every materialized or durably reserved ID. Gaps are allowed; reuse is not. #277 may store an explicit allocator watermark or make reserved rows sufficient to derive it. |
| Ship existence and type | Yes | Spawn, destroy, assemble, disassemble, Transit | Create/delete delta and checkpoint | Stable `ShipId` is the recovery key, not ECS entity handle. |
| Player ownership maps | Yes | Admission, Station operations, Transit | Ordered map delta and checkpoint | Materialized world ownership is durable PlayerState under #275. |
| Active-ship routing map | Yes | Admission, `SelectActiveShip`, `Disembark`, removal/Transit | Ordered map delta and checkpoint | This is authoritative routing state because it changes which ship receives commands and whether Undock is legal. It may have no public `DomainEvent`; the local checkpoint/RecoveryDelta path persists it explicitly. |
| Position, velocity, anchor | Yes | Movement, Warp, docking, Transit, commands | Component final-value delta and checkpoint | `VelocityChanged` does not cover every exact position/representation change. |
| Thrust/braking and flight modes | Yes | Move/Stop/Approach/Orbit/KeepAtRange/Warp | Component add/remove/update delta and checkpoint | Determines future motion even when no event is emitted. Checkpoint did not carry this until issue #312 (2026-08-15) unified `StateSnapshot`'s per-ship capture with the tick-rollback/`TickRecoveryDelta` path; see ADR-0049's implementation-correction note. |
| Hull shield/armor/hull/destroyed state | Yes | Combat and repair | Component final-value delta and checkpoint | Public damage/repair events are facts/outputs, not exact reducer authority. |
| Capacitor current | Yes | Capacitor system every Tick | Component final-value delta and checkpoint | Recharge can occur without a public event. |
| Fitted slots, active flags, cycle counters, targets | Yes | Fit/Unfit/Activate/Deactivate/Cap/range gate | Component final-value delta and checkpoint | Countdown and forced deactivation affect later combat. Cycle counters/targets did not survive a checkpoint restore until issue #312 (2026-08-15); see ADR-0049's implementation-correction note. |
| Derived ship stats from fitting/catalog | Reconstructible | `apply_fitting` / `base_stats` | Recompute from recovered ship type/fitting plus catalog fingerprint | Persist only if a future invariant requires it. |
| Lock entries, states, countdowns | Yes | Lock system and docking cleanup | Component final-value delta and checkpoint | Exact countdown/state must survive restart. Was absent from `StateSnapshot` entirely until issue #312 (2026-08-15); see ADR-0049's implementation-correction note. |
| Tackle membership | Yes | Tackle system | Component final-value delta and checkpoint | Prevents Warp/Jump and must survive restart exactly. |
| Ship cargo and destruction rewards | Yes | Station operations and combat reward | Ordered item-stack delta and checkpoint | Reward mutation can accompany ship deletion. Market settlement delivery (`serve/market_settlement.rs`) mutated this synchronously outside any tick boundary, durable only retroactively via the next tick's capture, until issue #315 (2026-08-16) routed it through `FrameInput::market_settlements` into the same prepare/durable/apply pipeline; see ADR-0049's implementation-correction note. |
| Docked ship/player Station context | Yes | Dock/Undock/Disembark/Select | Ordered map delta and checkpoint | Required to authorize Station operations. |
| Station inventory / packaged ships | Yes, Sector-journal-owned aggregate | Station command execution | Station aggregate delta in the same logical transition plus versioned Station checkpoint | SQLite/repository storage is an idempotent projection/read model under this contract; #277 owns its final API/schema. |
| Prepared fresh-admission reservation | Yes, repository-owned protocol state | `client_admission_prepared` / admission lifecycle | #277 durable AdmissionRepository keyed by stable reserved identities/ticket, including durable identity consumption | Exists before a Ship is materialized. The reservation must make the IDs non-reusable before `Welcome`. |
| Admission grant / resume-ticket current and staged binding | Yes, repository-owned identity state | admission commit/resume/rotation | #277 durable IdentityRepository transaction + explicit reconciliation with committed Sector transitions | Ticket rotation and ownership lookup must be crash-safe. Once a Ship is materialized, its world ownership/active routing are also RecoveryDelta authority. |
| `pending_fresh_admissions` in-memory claim set | Derived concurrency guard | Current admission runtime | Rebuild/reacquire from #277 prepared reservation + live handshake ownership | It must not be the authority for whether an ID is consumed. Crash may release the lock, but not the durable reservation. |
| `pending_resume_admissions` in-memory claim map | Runtime concurrency guard | Current resume handshake | None; reacquire from durable identity/world state on retry | A crash may release the in-flight lock. It cannot change durable ownership/ticket authority. |
| Transit ownership/freeze state and current handoff lifecycle state | Yes | Raft-committed Transit apply | `NodeState.transit_saga` in the checkpoint and `TickRecoveryDelta`, keyed by `TransitAttemptId` | `OutgoingTransitAttempt` owns the canonical handoff, retry deadline/count, and terminal state; `IncomingTransitReceipt` makes destination Commit idempotent. Public event scans replay/project facts only and never rebuild Saga state. |
| Bot persistent behavior state | Yes when it affects future decisions | Bot components/state | Component delta and checkpoint | Purely recomputable selection may be derived only when specified. |
| Pending human command queue | No, until admitted into a transition | Runtime connection | Runtime input queue | Disconnect/crash may require client resubmission. The current generic `ClientRequest` protocol has no request ID, so resubmission is a new request unless an operation-specific idempotency identity exists. |
| Generic client-command dedup state | Not provided today | `ClientRequest` envelope | None | Transparent exactly-once retry is not promised. A future generic retry feature must add a stable `RequestId`/equivalent at the #278/wire admission boundary and durable dedup state. |
| Pending bot lock-command queue | Yes until same-Tick consumption replaces it | Produced at end of one Tick, consumed by next | Ordered queue delta and checkpoint | It changes the next Tick's outcome and cannot remain an in-memory recovery gap. |
| Auto-jump Raft proposal after Warp arrival | Reliable post-commit obligation | `process_warp` -> `pending_auto_jumps` -> runtime | Durable outbox/idempotent retry intent committed with the Warp transition | Current in-memory queue is convenience only. #276 may represent this as Saga work, but crash after Warp commit must not lose the obligation. |
| Completed-warp client correction | Runtime presentation output | `completed_warps` | None unless promoted to a reliable protocol | Reconnect/current-state sync may repair presentation. |
| AoI index and client projections | Derived | Rebuilt/read from committed world | Recompute after recovery | Never journal presentation caches as authority. |
| Station inventory cache | None in `SimulationNode`; optional runtime optimization only | `StationInventoryRepository` read model | Refill only from a caught-up Station projection | Never checkpoint/cache-authorize it independently. The current node reads SQLite directly and owns no interior-mutable cache. |
| Socket/session handles and channel queues | No | Runtime adapters | None | Re-established after recovery. |
| Journal append receipt / authoritative committed position | Durable runtime metadata | Journal/runtime layer | #271 commit index/checkpoint metadata | Must refer only to committed ranges. |
| Station projection applied-through position | Durable projection metadata | Runtime repository/projection layer | A contiguous **global authoritative journal position** plus transition-id dedup for Station-changing records | The projection worker advances through every contiguous transition: Station records apply changes; non-Station records are no-op progression. Thus `>= promotion_position` is unambiguous. |
| Admission/identity repository reconciliation position/state | Durable protocol metadata | Runtime + #277 repositories | Repository transaction metadata, allocator watermark/reservations, and/or stable operation identities defined by #277 | Promotion must not expose stale tickets/prepared admissions or reuse an already reserved identity. The exact representation need not be the Station projection watermark. |
| `DomainEvent` list | Durable public fact | Command/Tick output | Public-event subrecord committed with transition; not exact state reducer | Delivery resumes from durable consumer state. |
| Reliable runtime effects | Reliable post-commit obligation | Runtime/application layer | Outbox/idempotency representation compatible with the transition | Execute only after successful local live apply; delivery state advances after downstream acknowledgement. |
| Deliberately lossy runtime effects | No | Runtime/application layer | None | Must be explicitly classified and cannot be required for authoritative continuity. |

Any newly added mutable field or post-commit obligation must be added to this table
before code lands. #275 splits Sector-world rows among explicit state owners, while #277
may split repository-owned admission/identity protocol state, but neither may
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

The durable batch is one **logical atomic visibility envelope** for Sector-world
state and outputs produced by that transition. #271 owns its physical framing and
may place state/event/outbox bytes in independently retained immutable segment
families. No recovery/publication reader may observe a subrecord as committed unless
the enclosing logical transition is committed according to #271's documented
framing/durability evidence.

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
Station SQLite/repository layer back into a competing independent Sector-world
authority.

### 3.2 Admission and identity repository rule

Prepared admission and resume-ticket lifecycle state are different from Station
inventory. They can exist before a Ship has a materialized Sector-world transition,
so this contract deliberately permits #277 to make them **repository-owned durable
protocol authority** instead of pretending they are merely a projection.

The boundary is:

- reserving a fresh `PlayerId`/`ShipId`/resume ticket must atomically or
  equivalently make **both the reservation and identity consumption** durable
  before a `Welcome` exposes it;
- #277 may persist an explicit next-ID allocator watermark or make the set of
  durable reserved/materialized identities sufficient to derive the next value,
  but restart must choose a value strictly above every consumed identity before
  accepting another allocation;
- aborting/expiring a prepared reservation may free protocol resources, but it
  does **not** make the previously reserved ID reusable;
- a prepared reservation that has no committed materialization transition remains
  directly retryable/abortable from the #277 repository after restart;
- once admission materializes a Ship, Sector-world existence, ownership, docking,
  Station starter-grant state, and active routing are authoritative
  RecoveryDelta/checkpoint state; Station SQLite rows are then projection data;
- the identity repository remains authoritative for current/pending resume-ticket
  bindings and admission protocol bookkeeping;
- if a world transition commits but a repository finalization step is incomplete,
  restart must reconcile the repository idempotently from a stable admission
  identity before the Sector serves the affected resume/admission path; and
- promotion requires the admission/identity repository and its consumed-ID/
  allocator state to be caught up or deterministically reconciled for all
  identities that can be served on the new owner.

#277 owns the exact transaction/schema/reconciliation representation. #278 owns the
runtime ordering that drives it. Neither may silently treat a stale repository as
healthy merely because ECS/checkpoint recovery reached the promotion position.

### 3.3 Auto-jump / Transit retry rule

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

The normative local ordering consumed by #272/#278 is:

```text
prepare
  -> make logical transition durable under selected profile
  -> apply RecoveryDelta / prepared mutation to local live state
  -> apply required local projections/repository reconciliations
  -> publish public outputs / execute reliable effects
  -> acknowledge when all acknowledgement conditions are satisfied
```

For `ReplicatedDurable`, "make durable" may require **durability replication** of
unapplied committed bytes/evidence to a quorum before the owner's live apply. This
is distinct from **state/application/publication replication**.

A remote node that has durably staged a transition for quorum purposes must not
pretend the transition is locally applied, publish its outputs, or become promotable
through that position until it has successfully applied the shared recovery reducer
and all promotion-critical projections/repositories.

Once an envelope is durably committed, it cannot be converted into an ordinary
rejection. If local live reduction or a required local projection/reconciliation
errors, panics, or partially applies, the owner must immediately:

1. mark itself fenced/unhealthy;
2. stop admitting/preparing/committing later transitions;
3. stop **application/publication replication** beyond its last successfully
   applied contiguous position (already-staged durability copies may remain);
4. suppress event delivery, outbox execution, and acknowledgement for the failed
   local apply; and
5. terminate or reconstruct local state/projections/repositories from durable data
   before resuming.

Continuing from pre-transition or partially applied live state after successful
durable commit is a contract violation.

#271 owns local journal durability evidence and the encoded evidence required for a
durable replica receipt. #280 owns transport/channel mechanics. #278 owns the
runtime policy that selects the configured replica set/profile, calculates the
required quorum, aggregates matching durability receipts, binds them to the current
Sector owner epoch/fencing state, and gates acknowledgement. Neither issue may
weaken the ordering or promotion rules above.

A quorum durability receipt must identify enough immutable context to reject stale
or cross-owner evidence, at least the Sector identity, ownership epoch/term (or an
equivalent fencing token), authoritative transition position/identity, and committed
content hash/range. Exact encoding belongs to #271/#280; ownership-epoch source and
runtime validation belong to the consensus/runtime boundary consumed by #278.

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

#284 fixes that semantic requirement. #271 provides journal/durability evidence,
#280 carries durability messages, and #278 defines/implements the configured
replica-set, quorum, owner-epoch validation, and acknowledgement policy. Production
must not enable or advertise `ReplicatedDurable` until those three layers define and
test one coherent quorum/fencing model.

### Ambiguous client retry

An unacknowledged request may be absent or already durably committed after a crash.
That durability ambiguity does **not** itself imply generic exactly-once command
retry.

The current `ClientRequest` protocol has no stable generic request ID, so a
reconnected client cannot prove that a newly submitted payload is the same logical
operation as an earlier ambiguous request. Therefore:

- the runtime must not automatically replay/re-submit an arbitrary non-idempotent
  client command after an ambiguous disconnect;
- the client first refreshes authoritative state and may then issue a new command;
- operation-specific protocols that require transparent retry must carry their own
  stable identity and durable dedup state (for example #277 admission identity,
  #276 `TransitAttemptId`, and #279 `SettlementId`); and
- if generic exactly-once client-command retry is later required, #278 together with
  the wire/API layer must introduce a stable `RequestId` (or equivalent) plus a
  documented durable dedup/result-retention policy.

An internal journal `transition_id` identifies a committed transition for recovery;
it is not automatically a client-visible idempotency key.

## 6. Crash-point matrix

| Crash/failure point | Durable recovery data | Live/recovered state | Client/external observation | Required behavior |
|---|---|---|---|---|
| Before prepare | No new transition | Previous committed state | No success | Operation may be retried according to its protocol. |
| During prepare | No new transition | Previous committed state | No success | Discard bounded prepared state. |
| Before/during failed atomic append | No valid committed transition | Previous committed state | No success/effect | #271 detects/rejects/truncates/quarantines partial framing according to its documented policy. |
| After durable commit, before local live apply | Complete transition exists; ReplicatedDurable may already have staged quorum copies | Recovery applies delta | Success may not have been observed | Recovery preserves the committed transition. A protocol retry deduplicates only when it carries a stable operation identity; generic client resubmission is otherwise a new request after state refresh. Staged replicas do not publish/apply implicitly. |
| During local live apply | Complete transition exists | Recovery reapplies complete delta | No success until local apply completes | Fence immediately; do not continue from old/partial state. |
| During required Station projection apply | Complete transition exists | Recovery reapplies Station delta idempotently | No acknowledgement | Fence; rebuild/catch up projection before serving authoritative Station work. |
| After admission reservation durability, before `Welcome` | #277 prepared admission + consumed-ID evidence exist | No materialized Ship required yet; recovered allocator is above the reserved IDs | Client may have seen nothing | Reuse/abort the same prepared reservation; never allocate a conflicting identity and never reuse the consumed IDs. |
| After `Welcome`, before admission materialization | #277 prepared admission + consumed-ID evidence exist | No materialized Ship yet; allocator remains advanced | Client knows reserved identity/ticket | Resume/retry resolves the same reservation from repository authority. Expiry may abandon it but not recycle IDs. |
| After admission world transition commit, before identity-repository finalization | Sector transition plus stable admission identity exist | Ship/ownership/Station grant recover from RecoveryDelta; allocator remains above consumed IDs | Client acknowledgement may be absent | Fence the affected admission/resume path until #277 reconciliation completes idempotently; do not create a second Ship/identity or duplicate starter grant. |
| After Warp commit, before handoff proposal | Durable auto-jump/Transit continuation obligation exists | Warp arrival remains committed | Proposal may be absent | Resume durable retry/Saga work using stable identity. |
| After handoff proposal attempt, before durable delivery/retry progress | Durable obligation exists | Warp arrival remains committed | Proposal may have been accepted | Duplicate attempt is allowed only through idempotent #276 semantics. |
| After local apply, before public-event delivery | Event subrecord exists | New committed state | Event may not yet be observed | Resume from durable delivery state; never regenerate by rerunning simulation. |
| After downstream delivery ack, before cursor durability | Output remains retained | New committed state | Consumer may already have output | Retry may duplicate; idempotency prevents duplicate domain effect. |
| After cursor durability | Output may become retention-eligible | New committed state | Delivery acknowledged | Compaction still obeys every required durable-consumer/archive watermark. |
| After acknowledgement | Transition exists under selected profile and required local apply/projections/reconciliations succeeded | New committed state | Success observed | RPO 0 only inside selected profile's documented failure domain. |
| During checkpoint member write/validation | Journal remains authority | Previous checkpoint set plus tail | No recovery gap | Keep previous complete checkpoint manifest. |
| After checkpoint manifest publication, before state compaction | New checkpoint and old tails coexist | Either valid recovery path | None | Both cover same committed position. |
| During/after state-delta compaction | Checkpoint plus remaining authoritative tail | Exact covered state plus tail | None | Never retire required delta before complete checkpoint publication. |
| Before event/outbox retention watermark | Public facts/intents retained | State may already be checkpointed | Pending durable delivery remains possible | State coverage does not delete undelivered durable output. Ephemeral client sessions do not hold this watermark. |
| Replica receives partial/out-of-order range | Local applied authority unchanged | Stops at last contiguous applied position | No promoted state | Detect gap/duplicate/version/fingerprint/hash mismatch. |
| Replica has staged durable bytes but has not applied them | Quorum durability may be satisfied | Applied state lags staged position | Promotion/publication forbidden through staged-only range | Apply shared reducer/projections first. |
| Replica has state but lacks promotion-critical retained outputs | State may be current | Catch-up-only | Promotion forbidden | Synchronize required outputs/delivery state. |
| Replica has world state but stale admission/identity repository authority | ECS may be current | Admission/resume/allocator service is not safe | Promotion for affected service forbidden | Catch up or deterministically reconcile #277 repository state, including consumed-ID evidence, before serving admission/resume/new allocations. |
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

Repository-owned admission/identity protocol authority need not be serialized into
the Sector world checkpoint if #277 chooses an independently durable repository.
If it is externalized, the manifest/promotion procedure must carry enough repository
version/epoch/reconciliation metadata to prove that the repository can be made
consistent with the recovered Sector authority **and that its consumed-ID state is
included when computing the next allocator values** before its service is enabled.

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
  condition is durable;
- repository-owned admission/identity protocol records and consumed-ID evidence
  retain according to #277's terminal/reconciliation rules, but ID retirement may
  never make a previously consumed ID reusable; and
- #271 must preserve enough committed-index metadata to make recovery ranges and
  remaining output references unambiguous.

Compaction must preserve the parent's existing crash-safe publication property:
write/validate/sync replacement material -> atomically publish the selecting
manifest -> only then retire superseded recovery material. No state checkpoint may
silently discard a still-required public fact, reliable obligation, repository
record needed to reconcile an exposed identity, or the allocator information needed
to prevent ID reuse.

FBD-001 continues to protect committed public `DomainEvent` history from in-place
destructive mutation; recovery-delta checkpoint compaction is a distinct stream.

## 9. Durable delivery state

This section applies only to consumers/obligations explicitly classified as
**durable**. Ordinary WebSocket sessions, AoI membership, position snapshots, and
other ephemeral presentation streams do **not** acquire durable retention cursors
merely because they consume committed state. Reconnect/current-state synchronization
repairs those streams unless a separate protocol explicitly promotes them to a
durable consumer.

For each durable public-event/outbox consumer, delivery is at-least-once unless a
stronger protocol is explicitly selected:

1. select the next committed output after durable delivery state;
2. attempt delivery with stable transition/output identity;
3. obtain downstream acknowledgement or equivalent durable idempotency proof;
4. durably advance consumer delivery state only after step 3; and
5. allow output retirement only after every required durable-consumer/archive
   condition is satisfied.

A crash between steps 3 and 4 may redeliver; durable consumers must tolerate
duplicates or provide stronger transactional idempotency. A local cursor alone is
not exactly-once.

The concrete cursor/index representation belongs to #271/#276 as appropriate.
#284 requires that failover cannot advance past an undelivered committed durable
obligation. Disconnected ephemeral clients never hold public-event compaction open.

## 10. Replica catch-up and promotion

A snapshot/catch-up representation consumed by #280 must be sufficient to obtain:

- a complete compatible checkpoint and its authoritative covered position;
- every contiguous authoritative recovery transition after that position;
- every retained public/reliable output still required after promotion;
- delivery/retry state sufficient to prevent skipping committed durable
  obligations;
- Station checkpoint/delta information sufficient to advance the local projection's
  **global contiguous applied-through position**; and
- admission/identity repository data or deterministic reconciliation metadata,
  including consumed-ID/allocator evidence, sufficient for #277-backed
  admission/resume/new-allocation service on the promoted owner.

Promotion eligibility requires:

1. compatible checkpoint members/fingerprints;
2. no authoritative recovery gaps through the promotion position;
3. successful application of the shared recovery reducer through that position;
4. no missing promotion-critical retained public/reliable output;
5. delivery/retry state that cannot skip an undelivered committed obligation;
6. admission/identity repository authority and consumed-ID/allocator state are
   caught up or reconciled for every identity/allocation domain the promoted owner
   may serve;
7. successful invariant validation; and
8. `StationProjection.applied_through >= promotion_position` before Station
   reads/writes are served. Non-Station transitions count as explicit no-op
   progression for this watermark.

A node with bytes durably staged for quorum but not yet applied is not promotable
through those bytes. A node with ECS equivalence but stale retry/output/projection/
identity-repository state is likewise not healthy for the affected service.

## 11. Determinism boundary

The engine should remain deterministic where practical, but exact recovery does not
re-execute historical implementation code. Process randomness, floating-point
behavior, catalog evolution, iteration ordering, AI implementation, and queue
contents can alter rerun outcomes. The state-delta journal captures committed
outcomes and keeps recovery independent of those implementation details.

Diagnostic accepted-input/RNG metadata may be included, but it is supplementary.
`DomainEvent`s remain mandatory durable public outputs when produced, not the sole
exact-state reducer.

## 12. Operational RTO status

RTO is intentionally **deployment-specific**, not implied by having a recovery
procedure. After the peer transport and reference hardware are fixed, the
operator must benchmark representative ship counts and eventless Ticks, then
define:

- maximum authoritative tail transitions and bytes;
- maximum replay time on named reference hardware;
- checkpoint cadence/trigger thresholds; and
- the production recovery target derived from those measurements.

This does not weaken the #284 correctness contract: a checkpoint plus its
contiguous RecoveryDelta tail must reproduce the exact authoritative state. It
only avoids pretending that a machine-independent RTO number exists before the
deployment topology is selected.

## 13. Required tests

Implementation across #271/#272/#276/#277/#278/#280/#284 must eventually cover the
relevant layer of each guarantee:

- append failure before commit for a simple command;
- append failure for an eventless Tick with movement/capacitor changes;
- crash after durable append and before local live apply;
- live reducer error/panic/partial apply after append, proving fail-stop fencing;
- ReplicatedDurable receipts are bound to the configured replica set, current owner
  epoch/fencing token, transition position/identity, and committed content;
- staged-quorum bytes cannot be published/promoted before reducer application;
- Station projection failure after append and idempotent catch-up;
- Station global applied-through watermark advances across non-Station no-op
  transitions and blocks promotion when stale;
- fresh admission crash before/after `Welcome`, proving the same prepared identity
  is recovered from #277 authority and the allocator advances past it;
- expiration/abort of a prepared admission never permits reserved ID reuse;
- admission world commit followed by repository-finalization failure reconciles
  idempotently without a second Ship/identity/starter grant;
- replica promotion cannot serve stale admission/resume/allocator repository state;
- `SelectActiveShip`/`Disembark` recovery equivalence even when no public event is
  emitted;
- pending bot lock-command queue checkpoint-plus-tail equivalence;
- `pending_fresh_admissions`/`pending_resume_admissions` may be reconstructed or
  reacquired without becoming durability authority;
- generic non-idempotent `ClientRequest` is not transparently retried after an
  ambiguous disconnect without a stable request identity;
- protocols with stable operation IDs deduplicate ambiguous retry as specified;
- crash after Warp commit and before Transit continuation proposal, proving durable
  retry survives;
- ambiguous crash after proposal, proving #276 idempotency;
- crash after local apply and before public-event publication;
- crash after downstream durable-consumer acknowledgement but before delivery-state
  durability, proving duplicate-safe retry and no skip;
- disconnected ephemeral WebSocket/AoI clients do not hold durable retention
  watermarks;
- checkpoint-plus-tail equivalence for motion, capacitor, locks, module cycles,
  docking, Player/active-ship state, Station inventory, admission, combat randomness,
  and Transit;
- duplicate, missing, out-of-order, corrupt, or incompatible recovery records;
- incomplete checkpoint, corrupt member, and incompatible format/fingerprint;
- interrupted checkpoint publication and compaction boundaries;
- state compaction cannot remove still-required public/reliable outputs;
- snapshot-based catch-up cannot promote with staged-but-unapplied ranges or stale
  promotion-critical projections/repositories;
- LocalDurable acknowledgement survives the documented local failure injections;
- ReplicatedDurable acknowledgement is not emitted before configured quorum
  durability and required owner-side apply/projection/reconciliation success;
- no public event or reliable external effect before local live apply; and
- idempotent recovery of an already durably committed transition.
