---
scope    : Complete specification of the simulation time model and the per-Tick processing order
audience : AI Agent / Human Developer
update   : When the Tick processing order, recovery commit boundary, or performance targets change
related  : event-catalog.md, recovery-contract.md, ownership.md, ../adr/ADR-0049-sector-recovery-state-delta-wal.md
---

# Tick Model

> **ADR-0049 recovery amendment (2026-08-07):** The detailed system order and game
> mechanics in this document remain normative. What changes is the **mutation and
> durability boundary** around them. In the target #272 architecture, the logical
> effects of Steps 1–7 are prepared without exposing mutations to the committed live
> world, then one ADR-0049 recovery transition is made durable, then the prepared
> result is applied locally, projections are caught up, and only then are public/
> reliable effects published. The current `SimulationNode` still performs parts of
> the old mutate-then-append pipeline; those current call shapes are migration debt,
> not a competing commit contract.

The current #272 migration slice exposes `SectorEngine::prepare_tick` and
`SimulationNode::commit_tick_transition` for the logical Tick counter only. It
proves the durable counter boundary, including eventless append failure, but it
does not replace `tick_with_lock_commands`: the ECS movement, capacitor, combat,
and other system write sets remain the legacy path until they acquire bounded
prepared mutations of their own.

## 1. Tick Definition

```
Tick is a logical time unit, unrelated to wall-clock time.
It is a monotonically increasing u64 newtype.
```

Tick counts simulation steps, not elapsed real time.

### Why wall-clock time is forbidden

```
Problem 1: clocks across Nodes always drift (NTP precision is tens of ms)
Problem 2: NTP step correction can move time backward
Problem 3: test and production environments can't reproduce identical results
```

Using wall-clock time for causal ordering produces non-deterministic results.
**Using `std::time::SystemTime` instead of Tick is forbidden (INV-005).**

### Comparability range

```
Now:    comparable only within a single Node/Sector logical timeline
Future: causal order across Sectors may use an explicit distributed ordering model
```

ADR-0049 journal positions are recovery/commit positions for one Sector authority;
they do not make unrelated Sector-local `Tick` values globally comparable.

---

## 2. Tick vs. Wall-Clock Time

### Benchmark run (`simulate` binary): unbounded

The Tick loop runs as fast as possible; per-Tick time depends on hardware and entity count.

### Server run (`--serve`): fixed interval

```
Current target  : 100 ms / Tick (10 Tick/s)
Implementation   : async timer via tokio::time::interval (active in WsServer mode)
Future target    : 16 ms / Tick (62.5 Tick/s) — revisit at Phase 8+
```

Exceeding 100 ms is logged as a system anomaly. Logical Tick monotonicity and deterministic
ordering are preserved; real-time overload policy must not silently reorder/drop committed
transitions. Overload is handled in this order: split → LoD → local TiDi → admission control
(ADR-0018). Local TiDi is the last resort that only slows real-time pacing, never the logical
Tick's authoritative transition order. See §8 for details.

---

## 3. Per-Tick Processing Order (Normative)

**This system order and the ADR-0049 commit boundary must not change without an ADR.**

The step descriptions below preserve the existing gameplay mechanics. Pseudocode such as
`&mut world` describes the logical mutation each system computes. Under #272 these mutations
must be accumulated in a bounded prepared state/write set (or equivalent) until the durable
boundary in Step 8.5; current code may still mutate live structures earlier during migration.

A committed distributed input such as a Raft Transit operation is applied as authoritative
input at the appropriate pre-Tick boundary, but any resulting Sector state/public output must
also enter the ADR-0049 recovery model. #276 may reshape the Transit persistence/lifecycle
without changing the following simulation ordering.

```
Step 1: Prepare the Tick counter increment
         prepared_tick = current_tick + 1
         Target architecture: committed current_tick becomes prepared_tick only after
         the durable transition succeeds and local live apply occurs.

Step 2: Process the command queue
         MoveCommand              -> updates ThrustComp.direction (is_braking = false)
         StopCommand              -> ThrustComp.is_braking = true (reverse thrust to decelerate)
         LockOnCommand            -> passed to LockSystem (processed in a later step)
         ActivateModuleCommand    -> FittedSlot.is_active = true / apply_fitting()
         DeactivateModuleCommand  -> FittedSlot.is_active = false / apply_fitting()
         JumpCommand              -> after can_propose_jump() validation, creates/continues
                                    the reliable Transit proposal obligation required by
                                    ADR-0014 / ADR-0049; current code may propose directly,
                                    #276 owns the final Saga representation
         ApproachCommand          -> attaches ApproachComp (semi-automatic approach to a
                                    target Ship/Jump Gate; cleared by Move/Stop/other
                                    flight modes; ADR-0015)
         OrbitCommand             -> attaches OrbitComp (orbit target at a given radius,
                                    default = weapon range; cleared by Move/Stop/other
                                    flight modes; rejected while Warping; ADR-0031)
         KeepAtRangeCommand       -> attaches KeepAtRangeComp (hold at least the given
                                    distance from target, default = weapon range; cleared
                                    by Move/Stop/other flight modes; rejected while
                                    Warping; ADR-0031)
         WarpCommand              -> after can_propose_warp() validation, attaches WarpComp
                                    (intra-Sector short-range Fold = Warp, ADR-0022;
                                    Move/Stop only clear it during align, ignored while warping)
         Note: while a Ship is in Transit (TransitState::InTransit), Move/Stop/duplicate
           Transit/Jump/Approach/Orbit/KeepAtRange/Warp are all rejected (ADR-0014).
           Approach/Orbit/KeepAtRange are mutually exclusive — a later command clears
           the previous flight mode (ADR-0031).

Step 2.5: Approach System (before Movement, ADR-0015)
         SimulationNode::process_approach()
         -> Ships with ApproachComp only: steers thrust toward the target (Ship/Jump Gate);
           once within arrival radius, sets is_braking = true to hold position.
           If the target Ship disappears, removes ApproachComp and brakes.
         -> No public events emitted directly (Movement may emit VelocityChanged later).
           The steering/flight-state change is still part of RecoveryDelta.

Step 2.55: Orbit System (after Approach, before Movement, ADR-0031)
         SimulationNode::process_orbit()
         -> Ships with OrbitComp only: steers thrust toward a point on the orbit circle
           (radius away from target, slightly ahead along the tangent; a fixed UP axis
           keeps orbit direction consistent). If target disappears, removes OrbitComp and brakes.
         -> No public event is required; authoritative changes still enter RecoveryDelta.

Step 2.56: Keep at Range System (after Orbit, before Movement, ADR-0031)
         SimulationNode::process_keep_at_range()
         -> Ships with KeepAtRangeComp only: if distance < range, thrusts directly away
           from target; if distance >= range, sets is_braking = true. If target
           disappears, removes KeepAtRangeComp and brakes.
         -> No public event is required; authoritative changes still enter RecoveryDelta.

Step 2.6: Warp System (after Keep at Range, before Movement, ADR-0022 / ADR-0025)
         SimulationNode::process_warp(tick)
         -> Ships with WarpComp only. Aligning accelerates toward the target direction;
           once velocity along that direction reaches 75% of max_speed, transitions to
           Warping (EVE-style; align time depends on agility; interruptible; Tackle window).
           Warping flies straight at the target, decelerating proportionally to remaining
           distance, stopping at:
             Gate target: within activation_radius x 0.8 (ADR-0022)
             Body target: within body.radius x 1.5 (ADR-0025 BODY_WARP_ARRIVAL_FACTOR)
           If unreachable (e.g. target disappears), removes WarpComp and brakes.
           Gate targets with auto_jump = true currently feed pending_auto_jumps on arrival
           (ADR-0023).
         -> Warping ships skip Step 3 Movement (warp speed is not clamped there).
           Emits VelocityChanged where the current public-event policy requires it.
         -> ADR-0049: an auto_jump arrival cannot rely only on pending_auto_jumps.
           The same durable transition must create replayable/idempotent continuation state;
           #276 may represent that continuation as a Transit Saga attempt.

Step 3: Movement System (ECS batch processing; skips warping ships)
         MovementSystem::run(&mut world, tick)
         -> Emits: Vec<VelocityChanged> (only ships whose velocity changed)
         -> Exact final position/velocity belongs to RecoveryDelta even when no public
            VelocityChanged is emitted.

Step 4: Capacitor System
         CapacitorSystem::run(&mut world, tick)
         -> Every Tick: recovers cap by cap_recharge_per_tick (clamped to cap_max)
         -> At cycle start (cycle_remaining == 0): consumes cap_cost_per_cycle and
           sets cycle_remaining = cycle_time_ticks
         -> Otherwise: decrements cycle_remaining by 1
         -> If cap is insufficient to start a cycle: force the module OFF
         -> Emits: Vec<ModuleDeactivated> (forced OFF due to cap exhaustion)
         Must run after Movement, before Lock.
         Capacitor/cycle final values are RecoveryDelta authority on every changed Tick.

Step 4.5: Tackle System (after Capacitor, before Lock, ADR-0024)
         SimulationNode::process_tackle(tick)
         -> Ships with an active Tackle module (ModuleKind::Tackle, cap ON) only.
           If the locked target is within tackle_range, adds the tackler to TackledComp.
           If out of range, lock lost, or tackler destroyed, removes the tackler and
           emits TackleReleased.
           Ships with TackledComp return false from can_propose_warp / can_propose_jump.
         -> Emits: Vec<TackleApplied | TackleReleased>
         -> Exact tackle membership is also RecoveryDelta authority.

Step 5: Lock System
         LockSystem::run(&mut world, tick, &lock_commands)
         -> Emits: Vec<TargetLocked | LockLost>
         Must run after Movement (lock decisions need final positions).
         Lock countdown/state final values are RecoveryDelta authority, including
         eventless countdown Ticks.

Step 5.5: Range Gate System (ADR-0035)
         SimulationNode::process_range_gate(tick)
         -> For every Active, targeted slot (Weapon/Tackle, FittedSlot.target_ship_id
           set), computes distance to target and force-deactivates the module if
           beyond its effective range (Weapon: weapon_range + weapon_falloff;
           Tackle: tackle_range). Mirrors capacitor.rs::deactivate_modules() —
           clears is_active/cycle_remaining/target_ship_id and re-runs apply_fitting().
         -> Emits: Vec<ModuleDeactivated>
         Must run after Lock (Step 5, freshest lock state) and before Combat/Repair
         (Step 6+, so those only ever see modules still in range).

Step 6: Combat System
         CombatSystem::run(&mut world, tick, &cap.weapon_cycles_started)
         -> Only fires for ships in weapon_cycles_started (ADR-0012)
         -> EVE hit-chance formula: hit_chance = 0.5^((angular/(tracking*sig))^2 + (max(0,d-opt)/falloff)^2)
         -> Emits: Vec<WeaponFired | DamageTaken | ShipDestroyed>
         Must run after Lock (reads Locked state). Destroyed ships are removed from
         ECS and ship_index by the caller.
         Realized random outcomes, Hull state, deletion, and reward mutation are captured
         as committed outcomes in RecoveryDelta; recovery does not re-roll combat.

Step 6.5: Repair System (ADR-0033, ADR-0036)
         RepairSystem::run(&mut world, tick, &cap.repair_cycles_started)
         -> Resolves ShieldBooster/ArmorRepairer (self, ADR-0033) and
           RemoteShieldBooster/RemoteArmorRepairer (a Locked target, ADR-0036)
           in repair_cycles_started -- each RepairCycle carries a
           target_ship_id (self for Local Repair, the module's own target for
           Remote Repair), resolved once by the Capacitor System; Repair
           System itself does not distinguish self vs remote.
         -> Shield-layer kinds restore current_shield, Armor-layer kinds
           restore current_armor (each clamped to its layer max; Hull is not affected)
         -> Emits: Vec<RepairApplied>
         Must run after Combat (applies repair after this Tick's damage).

Step 7: Bot System (ships with IsBotComp only)
         SimulationNode::process_bots()
         -> Bots generate and execute commands through the same pipeline as human players
         -> No events emitted directly (command effects appear via later systems in
           subsequent Ticks)
         Must run after Combat (destruction is resolved before Bot AI runs).
         Bot commands go through the same apply_*_owned() pipeline as player commands.
         The pending bot lock-command queue is authoritative while it survives to the
         next Tick and must be included in RecoveryDelta/checkpoint until same-Tick
         consumption removes that cross-Tick state.

Step 8: Build one logical durable transition
         RecoveryDelta = every authoritative final-value change from this Tick
         DomainEvents  = ordered public/business facts, possibly empty
         Reliable work = retry/idempotency state for obligations that must survive crash
                         (generic outbox and/or #276 Saga state as appropriate)

Step 8.5: Make the transition durable under the selected ADR-0049 profile
         #271 owns physical framing, commit marker, fsync/quorum evidence, torn-write
         handling, and independent retention mechanics.
         A public EventStore append alone is NOT the semantic Tick commit boundary.
         Under ReplicatedDurable, remote quorum copies may be durable-staged here;
         staged bytes are not yet proof of remote reducer application/promotability.

Step 9: Apply the committed prepared mutation / RecoveryDelta locally
         -> committed current_tick becomes prepared_tick
         -> apply the same recovery semantics used by restart/replica catch-up
         -> apply required local projections (e.g. Station) idempotently
         -> any post-durable apply/projection failure fences/fail-stops the Sector

Step 10: Publish committed outputs / application state
         -> public DomainEvents become deliverable
         -> application/publication replication may expose only successfully-applied
            contiguous state
         -> reliable effects may execute/retry
         -> consumer delivery state advances only after downstream acknowledgement

Step 11: Runtime/consensus pacing after local commit
         raft.tick()
         -> advances election-timeout / heartbeat timers by 1 logical Tick
         -> current runtime may drain completed-warp presentation outputs
         -> Transit/reliable proposal work follows ADR-0014/ADR-0049 and migrates to
            #276 Saga semantics where applicable
```

### Current committed-Raft input path (legacy Step 7.5 label)

Current `transit::run_runtime_tick()` calls `transit::apply_committed_raft_entries()`
before `node.tick`. Today this may directly mutate ECS and append Transit events:

```
TransitOp::Request -> current owner marks InTransit, appends SectorTransitRequested,
                      exports handoff, proposes Commit
TransitOp::Commit  -> current destination imports at entry_pos, appends
                      SectorTransitCompleted / JumpGateUsed / StarSystemChanged as relevant
```

This describes the **current implementation baseline** preserved by ADR-0014. #272
must bring resulting Sector mutation under the same prepare/durable/live-apply contract,
and #276 replaces EventStore-scan retry/receipt authority with a durable Saga. The
relative rule that committed consensus input is handled before the ordinary simulation
Tick can remain unless a later ADR changes it.

### Commit means ADR-0049 durable transition, not public-event append

The old statement **"EventStore append completion = Commit" is superseded**.
A Tick can change position, capacitor, countdowns, queues, routing/flight state, and
logical Tick without a public event, so public-event append cannot represent the whole
commit.

No public output, reliable effect, or application/publication replication may expose
a transition before its ADR-0049 durable boundary and successful required local apply.
Under `ReplicatedDurable`, durability staging to a quorum can precede local live apply;
those staged bytes remain unapplied/non-promotable until the shared reducer/projections
succeed.

---

## 4. Tick-Event Correspondence Rules

### `tick` field is mandatory on public DomainEvents

Every domain event includes a `tick: Tick` field (INV-005).

```rust
// Correct: includes tick
VelocityChanged { ship_id, velocity, tick: Tick(42) }

// Forbidden public-event shape: omits tick
VelocityChanged { ship_id, velocity }
```

Without `tick`, a public Event's local causal Tick is unknown. Exact recovery ordering
also has the ADR-0049 authoritative journal position; the Event itself is not the exact
state reducer.

### Eventless Tick

A Tick with zero `DomainEvent`s still produces an ADR-0049 recovery transition containing
its Tick advancement and all authoritative final-value changes. `events_emitted == 0` does
not mean "nothing durable happened".

### Multiple moves of the same Ship within one Tick

```
Current design: a Ship moves at most once per Tick (MovementSystem applies
                Velocity exactly once).

Future: when the command queue supports it, multiple Commands targeting the
        same Ship will carry over to the next Tick (undecided).
```

### Client motion-track ordering (ADR-0043 / ADR-0045)

The client passes every `VelocityChanged.tick` to
`dawn-client-core::ShipMotion` as a `MotionCommand`. Because `VelocityChanged`
contains velocity but no position, applying it updates the track's
authority-tick watermark and future integration velocity without rewinding the
already-rendered position or presentation tick. The owner then reconciles the
authoritative position through `MotionCorrection` at the same logical tick. A
velocity event older than the watermark is ignored.

Docked tracks reject velocity updates and remain at zero velocity until an
authoritative undock transition. Normal-frame position application,
`PositionSnap`, dock/undock resets, and floating-origin rebase all dispatch
through the same `ShipMotion` surface and use the `ShipController` adapter's
single Node3D position writer. `MotionFrame` keeps authoritative and predicted
server positions separate; only its origin-relative render position is narrowed
to Godot `Vector3`. `main.gd` and `WorldPresentation` do not write ship
positions directly.

This client prediction/reconciliation path is presentation logic. It does not replace
server RecoveryDelta/checkpoint authority.

---

## 5. Tick Monotonicity Guarantee

### Tick never goes backward

```
Guarantee: tick.next() > tick always holds
Implementation: u64 overflow occurs after u64::MAX (~1.8 x 10^19) Ticks —
                not reachable within any realistic operating lifetime
```

### Tick across node restarts

Current `StateSnapshot` retains `tick`, but ADR-0049's target guarantee is stronger:
a versioned checkpoint retains the covered Tick/state and recovery applies every
contiguous committed `RecoveryDelta` after the checkpoint. Restart continues from
the exact last committed authoritative transition, not from public-event-tail inference.

---

## 6. Performance Targets

| Metric | Target | Current measurement |
|---|---|---|
| Per-Tick time (10,000 ships) | <= 16,000 us | measured via `cargo run --release` |
| P95 Tick time | <= 12,000 us | — |
| Max Tick time | <= 16,000 us | — |

### Measurement boundaries

The target authoritative Tick latency for the #272 architecture is:

```
Start: immediately before Tick preparation
End:   immediately after durable transition + successful required local live apply
```

External public delivery acknowledgements, bulk catch-up, and unrelated asynchronous
runtime effects are not part of the local simulation-compute benchmark. A future
`ReplicatedDurable` production SLA must separately account for its synchronous durability
quorum if that profile is used for acknowledgement.

Until #271/#272 land, existing benchmarks that stop after `EventStore::append_batch()` are
only **legacy implementation proxies**. They must not be interpreted as the semantic commit
boundary or numeric recovery RTO.

### Running the benchmark

```bash
cargo run -p dawn-simulation --bin simulate --release
```

---

## 7. Tick Loop Implementation Ownership

`run_phase4_server()` (single node), `run_cluster_server()` (3-node Raft),
and the production `dawn-sector-node` process drive their loops via a
fixed-interval `tokio::time::interval` (100 ms/tick). Every server path currently
enters `transit::run_runtime_tick()` for the frame order;
`SimulationNode::tick_with_lock_commands()` remains synchronous.

This is current implementation topology, not a permanent storage/API constraint:

- #272 moves persistence ownership outside the pure Sector engine and introduces the
  explicit prepared transition boundary. Its current vertical slices cover AoI,
  Stop, and the logical Tick counter; the full ECS Tick remains a later slice;
- #275 splits heterogeneous `SimulationNode` state authority;
- #276 replaces current Transit scan/retry state with a durable Saga;
- #280 may replace replication/snapshot transport wiring while preserving the Tick/
  recovery ordering defined here.

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

```
population_cap : max Ships a Sector will admit
warning threshold : population_cap x 0.8 (80%) triggers an alert
reject threshold   : population_cap x 0.95 (95%) rejects SpawnCommand
```

**SpawnCommand admission control:**

```
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

A validation-stage rejection does not become an authoritative committed `DomainEvent`
under INV-006 merely because it is useful telemetry. If `SpawnRejected` is represented
as a separate durable audit fact in the future, that must be explicit rather than
conflated with a state transition.

### Dynamic Sector Fission

Fission preparation begins once population_cap exceeds 80% — starting before the
threshold is actually exceeded matters.

```
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
- Boundary crossings always go through **SectorTransit (Raft-based ownership transfer)** (INV-003). No grey areas or dual active ownership at boundaries (ADR-0014 / ownership.md).
- Cross-boundary causality is synchronized via **logical Tick/explicit consensus state** as defined by the operation; real-time pacing differences are absorbed by the presentation layer (INV-005 / ADR-0018, "cross-boundary causality under differential TiDi").
- Fission is only for **spatially separable load** (multiple fronts, broad economy) — **a single dense battle is never split** (everyone ends up in the same neighborhood, so splitting doesn't help; ADR-0020 background / eve-reference §11.1). So Fission boundaries are placed at **low-interaction locations**.

**Per-operation handling and difficulty**:

| Operation | Handling | Status |
|---|---|---|
| Discrete crossing (a ship crosses the boundary) | SectorTransit hands off `entry_pos` + `velocity` through consensus (ADR-0014); final durable attempt state migrates under #276 | Implemented baseline / persistence migrating |
| In-flight Warp crossing the boundary | Each Sector computes its own segment locally via **parametric warp** (clipping entry->endpoint at the boundary). Transit carries "warp endpoint + committed"; the receiving side skips Align and continues. Position/public motion is recorded as specified by ADR-0022/INV-MOVE; exact state would enter RecoveryDelta | Not implemented — to be designed at Fission time ([ADR-0022](../adr/ADR-0022-intra-sector-warp.md)'s parametric-warp revision is the basis) |
| Cross-boundary combat (interaction spans the boundary) | The hard case: requires per-Tick state sync between both Sectors every Tick. Fission is for separable load only, so this is **assumed not to occur at boundaries**. If a single dense battle exceeds node capacity, it's diverted to **local TiDi** rather than split | Fundamentally hard -> avoided by not splitting (local TiDi) (ADR-0018 / eve-reference §11.1, §11.3) |

In short: **Fission boundaries sit at separable/low-interaction locations, so only crossings and warps need to cross them. Crossings are solved behaviorally; warp will be designed at Fission time on top of parametric warp; dense cross-boundary combat is avoided entirely by not splitting (local TiDi).**

Related: [ADR-0018](../adr/ADR-0018-tidi-graceful-degradation.md) (cross-boundary causality), [ADR-0020](../adr/ADR-0020-simulation-lod.md) (Fission doesn't help dense battles), [ADR-0022](../adr/ADR-0022-intra-sector-warp.md) (parametric warp), [roadmap.md](../process/roadmap.md) §10 (8B-2 Fission / 8B-8 cross-boundary TiDi), [eve-reference.md](../reference/eve-reference.md) §11.1/§11.3.

### Local Time Dilation (safety net for single dense battles)

If an unsplittable hotspot exceeds node capacity, TiDi activates for that Sector only,
subject to INV-TiDi's five conditions (local / observable / non-destructive /
auto-recovering / after split & LoD).

```
dilation decision (real-time pacing only; logical Tick processing is unchanged):
  if sector.tick_cost > sector.budget && !sector.splittable() {
      sector.dilation = (sector.budget / sector.tick_cost).max(MIN_DILATION);
      metrics.tidi_active.set(sector.id, sector.dilation);   // observability
  } else if sector.dilation < 1.0 && sector.tick_cost <= sector.budget {
      sector.dilation = 1.0;                                  // auto-recovery
  }
```

TiDi never breaks logical Tick determinism or ADR-0049 transition ordering — it must
never reorder, drop, or change the authoritative outcome; it is pure real-time pacing.

### Tick SLA monitoring and response hierarchy

When Tick processing time exceeds target, it is logged (never silently slowed) and
handled via the hierarchy:

```
Tick time <= 12ms : normal
Tick time <= 32ms : warning (warn! log)
Tick time > 32ms  : logged (error! log + metrics), then in order:
                    1. Splittable?           -> dynamic split (zero degradation)
                    2. Single dense battle?  -> LoD -> local TiDi (activated observably)
                    3. Still beyond tolerance -> admission control (final backstop)
```

Silently slowing the Tick is forbidden — dilation must always be observable.
This is what distinguishes dawn's local/observable/auto-recovering TiDi from EVE's
global TiDi.

### Design invariant (INV-TiDi revision, ADR-0018)

```
INV-TiDi: logical Tick rate is normally constant.
          Time Dilation is permitted only when an unsplittable single hotspot
          exceeds capacity, and only as a bounded last resort satisfying
          (a) local (b) observable (c) non-destructive (d) auto-recovering
          (e) after split/LoD.
          Durable logical transition ordering is unchanged by real-time dilation.
```
