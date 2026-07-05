---
scope    : Complete specification of the simulation time model and the per-Tick processing order
audience : AI Agent / Human Developer
update   : When the Tick processing order changes / when performance targets change
related  : event-catalog.md, ownership.md
---

# Tick Model

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
Now:    comparable only within a single Node (all processing is single-process)
Future: causal order across Sectors will use a VectorClock (not yet implemented)
```

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

Exceeding 100 ms is logged as a system anomaly. Logical Tick monotonicity and determinism
are always preserved (no reordering, dropping, or skipping events).
Overload is handled in this order: split → LoD → local TiDi → admission control (ADR-0018).
Local TiDi is the last resort that only slows real-time pacing, never the logical Tick's determinism.
See §8 for details.

---

## 3. Per-Tick Processing Order (Normative)

**This order must not change without an ADR.**

```
Step 1: Increment the Tick counter
         current_tick = current_tick + 1

Step 2: Process the command queue
         MoveCommand              -> updates ThrustComp.direction (is_braking = false)
         StopCommand              -> ThrustComp.is_braking = true (reverse thrust to decelerate)
         LockOnCommand            -> passed to LockSystem (processed in a later step)
         ActivateModuleCommand    -> FittedSlot.is_active = true / apply_fitting()
         DeactivateModuleCommand  -> FittedSlot.is_active = false / apply_fitting()
         JumpCommand              -> after can_propose_jump() validation, proposes
                                    TransitOp::Request (with gate_id) to Raft (ADR-0009)
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
           Transit/Jump/Approach/Orbit/KeepAtRange/Warp are all rejected (ADR-0014 /
           AI_DEVELOPMENT_GUIDE.md "Event Workflow"). Approach/Orbit/KeepAtRange are mutually exclusive —
           a later command clears the previous flight mode (ADR-0031).

Step 2.5: Approach System (before Movement, ADR-0015)
         SimulationNode::process_approach()
         -> Ships with ApproachComp only: steers thrust toward the target (Ship/Jump Gate);
           once within arrival radius, sets is_braking = true to hold position.
           If the target Ship disappears, removes ApproachComp and brakes.
         -> No events emitted (Movement emits VelocityChanged in later Ticks)

Step 2.55: Orbit System (after Approach, before Movement, ADR-0031)
         SimulationNode::process_orbit()
         -> Ships with OrbitComp only: steers thrust toward a point on the orbit circle
           (radius away from target, slightly ahead along the tangent; a fixed UP axis
           keeps orbit direction consistent). If target disappears, removes OrbitComp and brakes.
         -> No events emitted

Step 2.56: Keep at Range System (after Orbit, before Movement, ADR-0031)
         SimulationNode::process_keep_at_range()
         -> Ships with KeepAtRangeComp only: if distance < range, thrusts directly away
           from target; if distance >= range, sets is_braking = true. If target
           disappears, removes KeepAtRangeComp and brakes.
         -> No events emitted

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
           Gate targets with auto_jump = true are pushed to pending_auto_jumps on arrival (ADR-0023).
         -> Warping ships skip Step 3 Movement (warp speed is not clamped there).
           Emits VelocityChanged (records warp movement; no new event type).

Step 3: Movement System (ECS batch processing; skips warping ships)
         MovementSystem::run(&mut world, tick)
         -> Emits: Vec<VelocityChanged> (only ships whose velocity changed)

Step 4: Capacitor System
         CapacitorSystem::run(&mut world, tick)
         -> Every Tick: recovers cap by cap_recharge_per_tick (clamped to cap_max)
         -> At cycle start (cycle_remaining == 0): consumes cap_cost_per_cycle and
           sets cycle_remaining = cycle_time_ticks
         -> Otherwise: decrements cycle_remaining by 1
         -> If cap is insufficient to start a cycle: force the module OFF
         -> Emits: Vec<ModuleDeactivated> (forced OFF due to cap exhaustion)
         Must run after Movement, before Lock.

Step 4.5: Tackle System (after Capacitor, before Lock, ADR-0024)
         SimulationNode::process_tackle(tick)
         -> Ships with an active Tackle module (ModuleKind::Tackle, cap ON) only.
           If the locked target is within tackle_range, adds the tackler to TackledComp.
           If out of range, lock lost, or tackler destroyed, removes the tackler and
           emits TackleReleased.
           Ships with TackledComp return false from can_propose_warp / can_propose_jump.
         -> Emits: Vec<TackleApplied | TackleReleased>

Step 5: Lock System
         LockSystem::run(&mut world, tick, &lock_commands)
         -> Emits: Vec<TargetLocked | LockLost>
         Must run after Movement (lock decisions need final positions).

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

Step 8: Append all events to the EventStore
         event_store.append_batch(warp_events + move_events + cap_events + tackle_events + lock_events + combat_events + repair_events)

Step 9: Notify the Replication Actor of the delta
         replication_tx.send(delta)

Step 10: Send TickElapsed to RaftActor (ADR-0014)
         raft.tick()
         -> advances election-timeout / heartbeat timers by 1 Tick (INV-005 / FBD-003)
         Both the actor path and the clustered serve path share
         `transit::run_runtime_tick` (7.5 apply -> node.tick -> Step 9 hook -> raft.tick
         -> transient outputs). `serve::runtime::run_cluster_runtime_tick` additionally
         handles clustered-serve auto-jump / ownership handoff / AoI delivery / scoped
         InitialState resend. `transit::step_cluster_node` is a thin entry point that
         drains transients for callers such as `dawn-sector-node`.

Step 7.5: Apply committed Raft entries (ADR-0014 §7)
         transit::apply_committed_raft_entries()
         Shared via `transit::run_runtime_tick()` for actor / clustered serve;
         `dawn-sector-node` runs it via `transit::step_cluster_node()`.
         -> Applies committed TransitOp to ECS:
           TransitOp::Request -> owning node: marks InTransit, appends
             SectorTransitRequested, exports Ship state, proposes TransitOp::Commit to Raft
           TransitOp::Commit  -> destination node: imports at entry_pos, appends
             SectorTransitCompleted; if gate_id is Some, also appends JumpGateUsed; if
             from/to StarSystemId differ, also appends StarSystemChanged
             (ADR-0009 / SimulationNode::append_jump_events)
         Runs before node.tick (Step 1). In the actor path, emitted events propagate to
         the dawn-replication transport in the same Tick's flush.
```

### Why Step 9 must never run before Step 8

Propagating to other nodes before the EventStore Append completes would let a receiver
reference an event that doesn't exist yet. **Append completion = Commit**; pre-Commit data
is treated as not existing.

---

## 4. Tick-Event Correspondence Rules

### `tick` field is mandatory

Every domain event includes a `tick: Tick` field (INV-005).

```rust
// Correct: includes tick
VelocityChanged { ship_id, velocity, tick: Tick(42) }

// Forbidden: omits tick (INV-005 violation)
VelocityChanged { ship_id, velocity }  // design should make this a compile error
```

Without `tick`, an Event's causal order is unknown and replay ordering can't be guaranteed.

### Multiple moves of the same Ship within one Tick

```
Current design: a Ship moves at most once per Tick (MovementSystem applies
                Velocity exactly once).

Future: when the command queue supports it, multiple Commands targeting the
        same Ship will carry over to the next Tick (undecided).
```

---

## 5. Tick Monotonicity Guarantee

### Tick never goes backward

```
Guarantee: tick.next() > tick always holds
Implementation: u64 overflow occurs after u64::MAX (~1.8 x 10^19) Ticks —
                not reachable within any realistic operating lifetime
```

### Tick across node restarts

`StateSnapshot` retains `tick`, and `SimulationNode::restore_from` restores
it. Tick continues across restarts.

---

## 6. Performance Targets

| Metric | Target | Current measurement |
|---|---|---|
| Per-Tick time (10,000 ships) | <= 16,000 us | measured via `cargo run --release` |
| P95 Tick time | <= 12,000 us | — |
| Max Tick time | <= 16,000 us | — |

### Measurement boundaries

```
Start: immediately before incrementing the Tick counter
End:   immediately after EventStore::append_batch() completes

Step 9 (Replication) is excluded (it's async)
```

### Running the benchmark

```bash
cargo run -p dawn-simulation --bin simulate --release
```

---

## 7. Tick Loop Implementation Ownership

`run_phase4_server()` (single node) / `run_cluster_server()` (3-node Raft) in
`main.rs` drive the loop via a fixed-interval `tokio::time::interval`
(100 ms/tick). `SimulationNode::tick_with_lock_commands()` itself is
synchronous; the caller's interval controls pacing.

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

SpawnRejected is recorded as a domain event in the EventLog, preserving a history of
why a Sector became full.

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
- Boundary crossings always go through **SectorTransit (Raft-based ownership transfer)** (INV-003). No grey areas or dual ownership at boundaries (ADR-0017 §5).
- Cross-boundary causality is synchronized via **logical Tick**; real-time pacing differences are absorbed by the presentation layer (INV-005 / ADR-0018, "cross-boundary causality under differential TiDi").
- Fission is only for **spatially separable load** (multiple fronts, broad economy) — **a single dense battle is never split** (everyone ends up in the same neighborhood, so splitting doesn't help; ADR-0020 background / eve-reference §11.1). So Fission boundaries are placed at **low-interaction locations**.

**Per-operation handling and difficulty**:

| Operation | Handling | Status |
|---|---|---|
| Discrete crossing (a ship crosses the boundary) | SectorTransit hands off `entry_pos` + `velocity` (ADR-0014) | Implemented |
| In-flight Warp crossing the boundary | Each Sector computes its own segment locally via **parametric warp** (clipping entry->endpoint at the boundary). Transit carries "warp endpoint + committed"; the receiving side skips Align and continues. Position is recorded each segment via VelocityChanged (preserves INV-MOVE) | Not implemented — to be designed at Fission time ([ADR-0022](../adr/ADR-0022-intra-sector-warp.md)'s parametric-warp revision is the basis) |
| Cross-boundary combat (interaction spans the boundary) | The hard case: requires per-Tick state sync between both Sectors every Tick. Fission is for separable load only, so this is **assumed not to occur at boundaries**. If a single dense battle exceeds node capacity, it's diverted to **local TiDi** rather than split | Fundamentally hard -> avoided by not splitting (local TiDi) (ADR-0018 / eve-reference §11.1, §11.3) |

In short: **Fission boundaries sit at separable/low-interaction locations, so only crossings and warps need to cross them. Crossings are solved; warp will be designed at Fission time on top of parametric warp; dense cross-boundary combat is avoided entirely by not splitting (local TiDi).**

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

TiDi never breaks logical Tick determinism (orthogonal to INV-005) — it must never
reorder, drop, or change the outcome of events; it is pure real-time pacing.

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
          (Canonical definition: AI_DEVELOPMENT_GUIDE.md "Architecture Invariants", INV-TiDi)
```
