---
scope    : Complete specification of the simulation time model and the per-Tick processing order
audience : AI Agent / Human Developer
update   : When the Tick processing order, recovery commit boundary, or performance targets change
related  : event-catalog.md, recovery-contract.md, ownership.md, ../adr/ADR-0049-sector-recovery-state-delta-wal.md
---

# Tick Model

## 1. Tick Definition

```text
Tick is a logical time unit, unrelated to wall-clock time.
It is a monotonically increasing u64 newtype.
```

Tick counts simulation steps, not elapsed real time.

### Why wall-clock time is forbidden

```text
Problem 1: clocks across Nodes always drift (NTP precision is tens of ms)
Problem 2: NTP step correction can move time backward
Problem 3: test and production environments can't reproduce identical results
```

Using wall-clock time for causal ordering produces non-deterministic results.
**Using `std::time::SystemTime` instead of Tick is forbidden (INV-005).**

### Comparability range

```text
Now:    comparable only within a single Node (all processing is single-process)
Future: causal order across Sectors will use a VectorClock (not yet implemented)
```

---

## 2. Tick vs. Wall-Clock Time

### Benchmark run (`simulate` binary): unbounded

The Tick loop runs as fast as possible; per-Tick time depends on hardware and entity count.

### Server run (`--serve`): fixed interval

```text
Current target  : 100 ms / Tick (10 Tick/s)
Implementation   : async timer via tokio::time::interval (active in WsServer mode)
Future target    : 16 ms / Tick (62.5 Tick/s) — revisit at Phase 8+
```

Exceeding 100 ms is logged as a system anomaly. Logical Tick monotonicity and determinism
are always preserved. Overload is handled in this order: split → LoD → local TiDi →
admission control (ADR-0018). Local TiDi only slows real-time pacing; it never changes the
logical Tick's durable ordering or recovery semantics.

---

## 3. Per-Tick Processing Order (Normative)

**This order and the ADR-0049 durability boundary must not change without an ADR.**

The simulation systems below keep their relative order, but ADR-0049 changes the mutation
model: Steps 1–7 execute against a bounded prepared mutation/write set (overlay, reversible
plan, copy-on-write state, or equivalent). They must not make the committed live world
visible before the durable transition envelope commits. #272 implements this boundary.

```text
Pre-step: Apply already-committed Raft inputs to the prepared transition input set
          transit::apply_committed_raft_entries() is authoritative input, but any
          resulting Sector mutation/public output must join the same recovery contract.

Step 1: Prepare the next logical Tick
         prepared_tick = current_tick + 1
         The committed current_tick remains unchanged until Step 9 live apply.

Step 2: Process the command queue in prepared state
         MoveCommand              -> updates prepared ThrustComp.direction
         StopCommand              -> prepared ThrustComp.is_braking = true
         LockOnCommand            -> passed to LockSystem (processed later)
         ActivateModuleCommand    -> prepared FittedSlot.is_active = true / apply_fitting()
         DeactivateModuleCommand  -> prepared FittedSlot.is_active = false / apply_fitting()
         JumpCommand              -> validates ownership/routing; reliable Raft proposal
                                    is a post-commit outbox obligation when required
         ApproachCommand          -> prepared ApproachComp
         OrbitCommand             -> prepared OrbitComp
         KeepAtRangeCommand       -> prepared KeepAtRangeComp
         WarpCommand              -> prepared WarpComp
         InTransit guards and mutual-exclusion rules are unchanged.

Step 2.5: Approach System
         SimulationNode::process_approach()
         -> updates prepared thrust/Approach state
         -> may emit no DomainEvent; RecoveryDelta still records changed authority

Step 2.55: Orbit System
         SimulationNode::process_orbit()
         -> updates prepared steering state

Step 2.56: Keep at Range System
         SimulationNode::process_keep_at_range()
         -> updates prepared steering state

Step 2.6: Warp System
         SimulationNode::process_warp(prepared_tick)
         -> updates prepared Warp/position/velocity/anchor state
         -> may produce VelocityChanged public facts
         -> if an auto_jump Warp completes, prepare an AutoJumpProposalIntent in
            the durable outbox. `pending_auto_jumps` may be a post-commit in-memory
            projection only; it is never the sole durable obligation.

Step 3: Movement System
         MovementSystem::run(prepared_world, prepared_tick)
         -> prepared final positions/velocities
         -> public VelocityChanged facts where defined

Step 4: Capacitor System
         CapacitorSystem::run(prepared_world, prepared_tick)
         -> recharge/drain/cycle state becomes RecoveryDelta authority
         -> public ModuleDeactivated facts when relevant

Step 4.5: Tackle System
         SimulationNode::process_tackle(prepared_tick)
         -> prepared TackledComp state + public facts

Step 5: Lock System
         LockSystem::run(prepared_world, prepared_tick, lock_commands)
         -> prepared LockComp state + public TargetLocked/LockLost facts
         Pending bot lock commands consumed here are authoritative queue input.

Step 5.5: Range Gate System
         SimulationNode::process_range_gate(prepared_tick)
         -> prepared fitting/cycle/target changes + public ModuleDeactivated facts

Step 6: Combat System
         CombatSystem::run(prepared_world, prepared_tick, cycles, anchors)
         -> prepared hull/destruction/reward state
         -> public WeaponFired / DamageTaken / ShipDestroyed facts

Step 6.5: Repair System
         RepairSystem::run(prepared_world, prepared_tick, repair_cycles)
         -> prepared hull-layer state + public RepairApplied facts

Step 7: Bot System
         SimulationNode::process_bots()
         -> bot decisions that affect a later Tick are represented in prepared
            authoritative queue/component state. In particular the pending bot
            lock-command queue is included in RecoveryDelta/checkpoints.

Step 8: Build one DurableTransitionBatch
         RecoveryDelta = every authoritative final-value change from Steps 1–7
         DomainEvents  = ordered public/business facts, possibly empty
         Outbox        = reliable post-commit intents, including auto-jump Raft
                         proposal when produced

Step 8.5: Atomically commit the complete logical transition envelope
          The configured durability profile (ADR-0049) must be satisfied before
          this transition can be acknowledged. A public EventStore append alone
          is NOT the commit boundary.

Step 9: Apply the prepared mutation / RecoveryDelta to committed live state
        and required local projections
         -> current_tick becomes prepared_tick here
         -> Station SQLite projection applies idempotently by transition identity
         -> any post-append apply failure fences/fail-stops the Sector

Step 10: Publish committed outputs
         -> replication receives committed recovery ranges only
         -> DomainEvent delivery starts from durable output records/cursors
         -> reliable outbox workers may attempt effects
         -> consumer cursor advances only after downstream acknowledgement

Step 11: Runtime/consensus pacing after local commit
         raft.tick()
         -> reliable Raft proposals are executed/retried from outbox intents
         -> completed-warp presentation outputs may be drained when classified
            as lossy presentation-only state
         -> authoritative operation acknowledgement is emitted only after its
            durability profile and required local projections are satisfied
```

### Commit means durable transition envelope, not public-event append

The old rule **"EventStore append completion = Commit" is superseded by ADR-0049**.
A Tick may mutate authoritative position, capacitor, lock/module/queue state without emitting
any `DomainEvent`; therefore a public-event append cannot represent the whole commit.

The commit boundary is the durable visibility of the complete logical transition envelope
(`RecoveryDelta` + public events + reliable outbox intents) under the selected durability
profile. Replication, publication, reliable effects, and acknowledgement cannot precede it.

### Auto-jump ordering

A Warp arrival with `auto_jump = true` commits the Warp state and an auto-jump outbox intent
in the same transition. The subsequent `raft.propose` may happen after local state commit.
If the process crashes before or ambiguously during that proposal, recovery retries the intent
using its stable transition/idempotency identity. Silent loss of the old in-memory
`pending_auto_jumps` queue is not permitted.

---

## 4. Tick-Event Correspondence Rules

### `tick` field is mandatory on public DomainEvents

Every domain event includes a `tick: Tick` field (INV-005).

```rust
VelocityChanged { ship_id, velocity, tick: Tick(42) }
```

A missing event `tick` loses the public fact's causal position. However, exact recovery ordering
uses the committed recovery transition position; a `DomainEvent` is not the exact-state reducer.

### Eventless Tick

A Tick that emits zero public events still produces a `RecoveryDelta` transition containing its
Tick advancement and every authoritative final-value change. `events_emitted == 0` never means
"nothing durable happened".

### Multiple moves of the same Ship within one Tick

```text
Current design: a Ship moves at most once per Tick (MovementSystem applies
                Velocity exactly once).

Future: when the command queue supports it, multiple Commands targeting the
        same Ship will carry over to the next Tick (undecided).
```

### Client motion-track ordering (ADR-0043 / ADR-0045)

The client passes every `VelocityChanged.tick` to `dawn-client-core::ShipMotion` as a
`MotionCommand`. Because `VelocityChanged` contains velocity but no position, applying it
updates the track's authority-tick watermark and future integration velocity without rewinding
the already-rendered position or presentation tick. The owner reconciles authoritative position
through `MotionCorrection` at the same logical tick. A velocity event older than the watermark
is ignored.

Docked tracks reject velocity updates and remain at zero velocity until an authoritative undock
transition. Normal-frame position application, `PositionSnap`, dock/undock resets, and
floating-origin rebase all dispatch through the same `ShipMotion` surface and use the
`ShipController` adapter's single Node3D position writer. `MotionFrame` keeps authoritative and
predicted server positions separate; only its origin-relative render position is narrowed to
Godot `Vector3`. `main.gd` and `WorldPresentation` do not write ship positions directly.

---

## 5. Tick Monotonicity Guarantee

### Tick never goes backward

```text
Guarantee: tick.next() > tick always holds
Implementation: u64 overflow occurs after u64::MAX (~1.8 x 10^19) Ticks —
                not reachable within any realistic operating lifetime
```

### Tick across node restarts

The versioned checkpoint retains `tick`, and recovery applies every committed RecoveryDelta
after the checkpoint position. Tick continues from the exact last committed transition.

---

## 6. Performance Targets

| Metric | Target | Current measurement |
|---|---|---|
| Per-Tick time (10,000 ships) | <= 16,000 us | measured via `cargo run --release` |
| P95 Tick time | <= 12,000 us | — |
| Max Tick time | <= 16,000 us | — |

### Measurement boundaries

The target authoritative Tick latency is measured from immediately before preparation through
successful local apply after the durable transition commit. External replication/publication,
downstream delivery acknowledgement, and unrelated runtime effects are excluded.

Until #271/#272 implement the ADR-0049 transition envelope, the existing benchmark may still
measure through `EventStore::append_batch()` as an implementation proxy; that legacy boundary
must not be documented as the semantic commit point.

### Running the benchmark

```bash
cargo run -p dawn-simulation --bin simulate --release
```

---

## 7. Tick Loop Implementation Ownership

`run_phase4_server()` (single node), `run_cluster_server()` (3-node Raft), and the production
`dawn-sector-node` process drive their loops via a fixed-interval `tokio::time::interval`
(100 ms/tick). Every server path enters `transit::run_runtime_tick()` for the authoritative
frame order.

During the #272 migration, `SimulationNode::tick_with_lock_commands()` still mutates live state
before public-event append; that is **current implementation debt**, not the normative commit
contract. #272 must replace it with prepare -> durable envelope -> live apply while preserving
the system ordering specified above.

---

## 8. Load Control Design (Anti-TiDi first; TiDi is a local last resort)

### EVE Online's TiDi and its problem

EVE Online's Time Dilation (TiDi) slows simulation speed up to 10x when a Sector
(solar system) is overloaded, trading player experience (unresponsive controls,
prolonged fights) for consistency — long unpopular with its community.

### This system's approach: make TiDi rare, local, and brief (ADR-0018)

The earlier policy was to prevent TiDi entirely via admission control. But a single
dense battle is fundamentally unsplittable, and admission control alone would mean
locking players out of the climactic fight — worse than EVE's TiDi (eve-reference §11.1).
ADR-0018 instead defines a degradation hierarchy matched to the nature of the load:

| Situation | First resort | Second resort | Final backstop |
|---|---|---|---|
| Spatially separable | Dynamic Sector fission (zero degradation) | — | — |
| Single dense battle | LoD (thin out distant/non-combat updates) | Local TiDi (everyone stays) | Admission control |

The differentiation from EVE isn't "no TiDi" — it's a far higher TiDi threshold
(Rust + multi-core + spatial indexing/AoI), and when it does trigger: local to one
Sector, brief, auto-recovering, and observable.

### Sector Population Cap (admission control = final backstop)

Each Sector has an entity-count ceiling, `population_cap`. Since ADR-0018, admission
control is the final backstop rather than the primary tool — LoD and local TiDi are
tried first for single dense battles.

```text
population_cap : max Ships a Sector will admit
warning threshold : population_cap x 0.8 (80%) triggers an alert
reject threshold   : population_cap x 0.95 (95%) rejects SpawnCommand
```

**SpawnCommand admission control:**

```text
SpawnCommand received
    |
    v
check Sector's current population
    |
    +- population < reject threshold -> proceed normally
    |
    +- population >= reject threshold -> SpawnRejected { reason: SectorAtCapacity }
                                          (includes nearby-Sector routing info)
```

A rejected command is not a committed `DomainEvent` under INV-006 unless the project explicitly
models a separate durable audit fact. Rejection telemetry may be logged without treating the
rejected request as an authoritative state transition.

### Dynamic Sector Fission

Fission preparation begins once population_cap exceeds 80% — starting before the
threshold is actually exceeded matters.

```text
[Sector A: 4,000/5,000 ships]  <- 80% alert
         |
         | Sector Fission begins
         v
[Sector A1: 2,000 ships] + [Sector A2: 2,000 ships]
```

Splitting strategy: spatial bisection (split at the midpoint along the X or Y axis).
Closely related to the SectorTransit design (see ownership.md).

### Cross-boundary operations (Fission's ongoing concern; aggregated here)

Because Fission bisects space with a plane, handling operations that straddle the
boundary is an open question. The principles are settled; in-flight operation details
will be finalized when Fission work starts (roadmap 8B-2, requires an ADR). This
section aggregates related discussion scattered across ADR-0018 / ADR-0020 /
eve-reference §11 / ADR-0022.

**Principles**:
- Boundary crossings always go through **SectorTransit (Raft-based ownership transfer)** (INV-003). No grey areas or dual ownership at boundaries.
- Cross-boundary causality is synchronized via **logical Tick**; real-time pacing differences are absorbed by the presentation layer (INV-005 / ADR-0018).
- Fission is only for **spatially separable load** — a single dense battle is never split; it uses local TiDi rather than creating a high-interaction boundary.

**Per-operation handling and difficulty**:

| Operation | Handling | Status |
|---|---|---|
| Discrete crossing | SectorTransit hands off authoritative state through consensus (ADR-0014); recovery/outbox rules follow ADR-0049 | Implemented/migrating |
| In-flight Warp crossing | Each Sector computes its own segment locally via parametric warp; transfer representation must be integrated with RecoveryDelta when Fission is implemented | Not implemented |
| Cross-boundary combat | Requires high-frequency cross-Sector coordination and is intentionally avoided by boundary placement; dense battle uses local TiDi | Fundamentally hard -> avoided |

### Local Time Dilation (safety net for single dense battles)

If an unsplittable hotspot exceeds node capacity, TiDi activates for that Sector only,
subject to INV-TiDi's five conditions (local / observable / non-destructive /
auto-recovering / after split & LoD).

```text
dilation decision (real-time pacing only; logical Tick processing is unchanged):
  if sector.tick_cost > sector.budget && !sector.splittable() {
      sector.dilation = (sector.budget / sector.tick_cost).max(MIN_DILATION);
      metrics.tidi_active.set(sector.id, sector.dilation);
  } else if sector.dilation < 1.0 && sector.tick_cost <= sector.budget {
      sector.dilation = 1.0;
  }
```

TiDi never changes logical transition order, durable commit semantics, or authoritative outcomes;
it is pure real-time pacing.

### Tick SLA monitoring and response hierarchy

```text
Tick time <= 12ms : normal
Tick time <= 32ms : warning
Tick time > 32ms  : logged + metrics, then in order:
                    1. Splittable?            -> dynamic split
                    2. Single dense battle?   -> LoD -> local TiDi
                    3. Still beyond tolerance -> admission control
```

Silently slowing the Tick is forbidden — dilation must always be observable.

### Design invariant (INV-TiDi revision, ADR-0018)

```text
INV-TiDi: logical Tick rate is normally constant.
          Time Dilation is permitted only when an unsplittable single hotspot
          exceeds capacity, and only as a bounded last resort satisfying
          (a) local (b) observable (c) non-destructive (d) auto-recovering
          (e) after split/LoD.
          Durable transition ordering is unchanged by real-time dilation.
```
