# Common Design Violation Patterns

> Canonical source for AI_DEVELOPMENT_GUIDE.md §12. The guide keeps only a reference link (ADR-0030).

Anti-patterns AI assistants tend to fall into, and how to fix them.

## Pattern 1: Using State sync because it's "convenient"

```
Situation: a Position discrepancy appears between nodes, and the fix
directly overwrites State instead of going through events.

Violating code:
  // "Direct sync is faster than an Event" — wrong call
  node_b.update_position(ship_id, node_a.get_position(ship_id))

Correct approach:
  Propagate the Event via Gossip. State converges from Events automatically.
  Direct State sync breaks INV-001 and INV-002 simultaneously.
```

## Pattern 2: Skipping tests to "write them later"

```
Situation: implementation is complex, so tests get deferred.

Why this is dangerous:
  AI has no context across sessions. "Later" means "never."
  Untested code gets broken unintentionally in the next AI session.

Fix:
  For complex work, write the test first, then the minimal implementation
  that passes it. The test becomes the spec.
```

## Pattern 3: Bloating dawn-core for a new feature

```
Situation: adding new functionality by putting implementation logic into dawn-core.

Violating code (dawn-core/src/position.rs):
  impl Position {
      pub async fn broadcast_to_nodes(&self, nodes: &[NodeAddr]) { // network logic
          ...
      }
  }

Correct approach:
  dawn-core holds only data definitions.
  Network logic belongs in dawn-replication or dawn-sector-node.
```

## Pattern 4: "Aligning" Tick with wall-clock time as an optimization

```
Situation: using wall-clock time because "matching Tick to real time is easier to follow."

Why this is dangerous:
  Once Tick depends on wall-clock time, Tick order becomes non-deterministic
  across nodes. Test and production environments can diverge in Tick order.
  An NTP step correction that moves time backward can break the system.

Fix:
  Keep Tick as a logical counter. Use human-readable time only in the
  Observation Layer (logs, metrics). See INV-005.
```

## Pattern 5: "Optimizing" Sector Transit by skipping Raft

```
Situation: implementing Sector Transit without Raft "to cut latency."

Consequence of the violation:
  Two nodes claim ownership of the same Ship simultaneously (split brain)
  -> both Sectors process ShipMove independently
  -> the world diverges (breaks the Single Shard guarantee).

Fix:
  Sector Transit must always go through Raft. See INV-003.
  If latency is a concern, reduce Transit frequency instead.
  Raft is implemented (ADR-0014); Transit runs over the Raft log.
```

## Pattern 6: Recording only an ID instead of a FittingSnapshot in events

```
Situation: a ShipFitted event stores only a list of ModuleIds, reasoning that
"the registry can resolve them later."

Consequence of the violation:
  If the registry changes later (e.g. a module's stats are updated), replaying
  the old Event reproduces different stats than at the time it occurred.
  -> Violates INV-002 (Event replay must fully reproduce the world).

Correct implementation:
  ShipFitted must include the full FittingSnapshot (complete module definitions).
  Replay must be self-contained and never depend on the registry. See ADR-0006 §1.
```

## Pattern 8: Encoding state as a flag instead of as the event itself

```
Situation: a module on/off change is represented with an is_active flag.

Violating code:
  ModuleToggled { ship_id, module_id, is_active: bool, tick }
  // you can't tell what happened without reading is_active
  // this describes a state, not a fact

Correct implementation:
  ModuleActivated   { ship_id, module_id, slot, tick }
  ModuleDeactivated { ship_id, module_id, slot, tick }
  // the event name itself says what happened

Principle:
  An Event is a fact that already happened (INV-006).
  Name it as "this action occurred," not "the state became this."
  Use past-tense verbs (Activated, Fired, Destroyed).
  Never make is_*/has_* flags a key field of an event.
```
