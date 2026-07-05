# Forbidden Changes Catalog

> Canonical reference behind the Forbidden Changes list in AI_DEVELOPMENT_GUIDE.md
> ("Architecture Invariants"). The guide keeps only the
> FBD-00x ID list and one-line summaries; this file holds the details and code
> examples (ADR-0030). FBD-00x IDs are stable and must not be renumbered.

The changes below must **never be made, for any reason**, even if a technical
justification is offered. If a change like this is genuinely needed, propose
an ADR revision and get human approval first.

## FBD-001: Destructive operations on the Event Log

```rust
// Do not add methods with these signatures to the EventStore trait:
fn update(&self, id: EventId, payload: Bytes) -> Result<()>;
fn delete(&self, id: EventId) -> Result<()>;
fn truncate(&self, from_index: u64) -> Result<()>;
fn rewrite(&self, index: u64, event: Event) -> Result<()>;
```

Protects INV-001 (event log is append-only). Per ADR-0017, log compaction is
handled outside the trait as an operational process (move segments behind a
verified snapshot to cold storage, then atomically swap the hot log via
write-new-then-swap) — events within a segment are never rewritten.

## FBD-002: External dependencies in `dawn-core`

```toml
# Do not add dependencies like these to dawn-core/Cargo.toml:
tokio      = ...  # async runtime
tonic      = ...  # gRPC
reqwest    = ...  # HTTP client
sqlx       = ...  # database
serde_json = ...  # JSON serializer (only the serde feature is allowed)
```

Keeps `dawn-core` a dependency-free, deterministic simulation core.

## FBD-003: Wall-clock time for causal ordering

```rust
// Do not use these for causal ordering:
use std::time::SystemTime;
SystemTime::now()

use chrono::Utc;
Utc::now()

// Use the logical tick counter instead:
self.tick_counter.fetch_add(1, Ordering::SeqCst)
```

Wall-clock time is non-deterministic across nodes; only logical Tick order is reproducible.

## FBD-004: Direct method calls between Actors

```rust
// Forbidden: Actor A calls Actor B's methods directly
struct SectorSimulatorActor {
    replication_actor: Arc<ReplicationActor>, // must not hold an Arc directly
}

impl SectorSimulatorActor {
    async fn on_tick_complete(&self) {
        self.replication_actor.sync(delta).await; // direct call forbidden
    }
}

// Correct: send a message through the mailbox
struct SectorSimulatorActor {
    replication_tx: mpsc::Sender<ReplicationMessage>, // hold only the Sender
}

impl SectorSimulatorActor {
    async fn on_tick_complete(&self, delta: Delta) {
        let _ = self.replication_tx.send(ReplicationMessage::Sync(delta)).await;
    }
}
```

Preserves actor isolation and the message-passing concurrency model.

## FBD-005: Reusing a Ship's EntityId

```rust
// Forbidden: pooling and reassigning despawned IDs
struct IdPool {
    recycled: VecDeque<ShipId>,
}

impl IdPool {
    fn next_id(&mut self) -> ShipId {
        self.recycled.pop_front().unwrap_or_else(|| self.generate_new())
        // ^ popping from `recycled` is forbidden
    }
}
```

Reused IDs break event-log identity guarantees and can resurrect stale references.

## FBD-006: Sector Transit that bypasses consensus

```rust
// Forbidden: direct cross-sector state transfer that bypasses Raft
async fn teleport_ship_between_sectors(
    &self,
    ship_id: ShipId,
    from: SectorId,
    to: SectorId,
) {
    self.sector_nodes[from].remove_ship(ship_id).await; // no Raft
    self.sector_nodes[to].add_ship(ship_id).await;       // no Raft
}
```

Every cross-Node Ship transfer must go through consensus to keep World state consistent.

## FBD-007: Adding `pub fn` without tests

```
CI auto-rejects a PR if:
  - a pub fn was added with no corresponding test
  - coverage drops below 80%

No exceptions. If a test can't be written, use pub(crate) or pub(super) instead.
```

## FBD-008: ~~Implementation outside MVP scope~~ — repealed (ADR-0016)

Repealed following the gamification decision (ADR-0016). These crates may now
be created, subject to ADR approval:

```
crates/dawn-economy/   — economy systems
crates/dawn-character/ — character entity (growth/progression still banned, see FBD-009)
crates/dawn-inventory/ — inventory
crates/dawn-ui/        — UI-only crate
crates/dawn-graphics/  — graphics-only crate
```

New crates still require the normal process: file an ADR and get human
approval (§9), place the crate correctly in the Dependency DAG (§3) without
creating cycles, and update the crate responsibility table (§11). Combat and
Fitting logic stay inside `dawn-ecs` / `dawn-core` (an ADR is required to
split them into a separate crate).

## FBD-009: Skill-point growth / passive growth / AFK mining

> Stays in force after gamification (ADR-0016). Anti-grind is core to
> "surpassing EVE" and was the most-disliked element family observed in §6
> (18k-document/forum survey; forum sentiment is not proof — mind selection
> bias, see eve-reference §11.5).

**Skill points / passive growth** — do not implement any form of:
- abilities that unlock over time
- passive growth proportional to play time
- progression that can be accelerated with real money (pay-to-win)

Reason: performance would scale with time/money spent rather than player
skill, undermining perceived fairness. (Having a Character as an *entity* is
allowed per ADR-0016 — only growth/progression tied to time or payment is
banned, not the entity's existence.)

**AFK mining** — do not implement content where a player activates a mining
laser and walks away.

Reason: it removes the moment of deliberate player decision-making (the core
design question is "does this feature increase opportunities for deliberate
player decisions?" — AFK mining answers no). In EVE, miners function as
"helpless targets" for pirates; the miner themself isn't really playing.
(Active-decision resource gathering, or resource sinks that force decisions
via scarcity, remain open designs — see ADR-0016 §5, eve-reference §7.4.3.
Only unattended, idle-progress mining is banned.)

See `docs/design/game-design.md` §5.
