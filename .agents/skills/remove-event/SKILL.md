---
name: remove-event
description: Fully delete a deprecated DomainEvent variant from Dawn's codebase, tests, and docs (dawn-core, dawn-sector, dawn-replication, event-catalog.md, tick-model.md, ADRs). Pre-release only. Use when told to remove/delete a specific event, e.g. "remove the ShipMoved event".
---

# remove-event — Fully delete a deprecated event

Takes the event name to delete (e.g. `ShipMoved`) as input.

This skill removes the given `DomainEvent` variant and its struct from the
codebase, tests, and documentation completely.

**Precondition:** pre-release stage (no persisted external-user event logs
exist). After release, follow the post-release breaking-change procedure in
`docs/architecture/event-schema-evolution.md` instead.

---

## Steps

### Step 1: Confirm the deletion target

Read `crates/dawn-core/src/events.rs` and confirm:
- `pub struct <EventName> { ... }` exists
- the `DomainEvent::<EventName>(...)` variant exists
- it carries `#[deprecated]` or an `@deprecated` note (if not, confirm the
  intent to delete with the user first)

### Step 2: Enumerate every reference

Grep for the bare name across the whole repo -- **this grep is the source of
truth**, not the list below:

```
rg -l '<EventName>' crates docs client CLAUDE.md AGENTS.md AI_DEVELOPMENT_GUIDE.md
```

Typical reference sites (verify against the grep; the layout moves over time):
- `crates/dawn-core/src/events.rs` -- struct definition, enum variant, and
  the match arms in `ship_id()` / `tick()`
- `crates/dawn-sector/src/node/apply_event.rs` -- `apply_event()` match arm
- `crates/dawn-sector/src/node/serialization.rs` -- event<->JSON conversion
  arms
- `crates/dawn-sector/src/` and `crates/dawn-simulation/src/serve/` -- tests
  and assertions that spawn or expect the event
- `crates/dawn-event-store/src/file.rs`, `memory.rs` -- test helpers
- `crates/dawn-replication/src/` -- bus / replica test helpers
- `docs/architecture/event-catalog.md` -- event table row and detail section
- `docs/architecture/tick-model.md` -- step descriptions and example code
- `docs/adr/` -- the deprecation-procedure text in the owning ADR
- `AI_DEVELOPMENT_GUIDE.md` / `CLAUDE.md` / `AGENTS.md` -- example code, notes

### Step 3: Delete the core definitions

Edit `crates/dawn-core/src/events.rs`:

1. Delete `pub struct <EventName> { ... }`
2. Delete the `DomainEvent::<EventName>(<EventName>)` variant
3. Delete the `ship_id()` match arm for it
4. Delete the `tick()` match arm for it
5. Remove any now-unneeded `#[allow(deprecated)]` annotations

### Step 4: Delete the runtime arms

- `crates/dawn-sector/src/node/apply_event.rs`: remove the `apply_event()`
  match arm; fix any comments that reference the event
- `crates/dawn-sector/src/node/serialization.rs`: remove the JSON conversion
  arm(s)

### Step 5: Replace test-helper usages

Wherever a test helper constructs `<EventName>` just to have "some event"
(event-store and replication tests), replace it with an existing event --
usually `VelocityChanged` or `ShipSpawned`:

```rust
// Before
use dawn_core::events::<EventName>;
DomainEvent::<EventName>(<EventName> { ship_id: ..., ... })

// After (using VelocityChanged)
use dawn_core::{events::VelocityChanged, Velocity};
DomainEvent::VelocityChanged(VelocityChanged {
    ship_id : ShipId::new(NodeId(0), n),
    velocity: Velocity::new(1.0, 0.0, 0.0),
    tick    : Tick(tick),
})
```

These tests exercise log mechanics, not event semantics, so any event works
as a substitute.

### Step 6: Update the docs

`docs/architecture/event-catalog.md`:
- Delete the event's row from the event table
- Delete its `@deprecated` note block
- Delete its detail section if one exists

`docs/architecture/tick-model.md`:
- Remove or fix `@deprecated` notes and example code using the event

`docs/architecture/entity-model.md`:
- Fix any references to the deleted event name

Owning ADR (`docs/adr/ADR-XXXX-*.md`):
- Rewrite the "deprecation procedure" section to "deleted"
- Flip the implementation-checklist item to `[x]`

`AI_DEVELOPMENT_GUIDE.md` / `CLAUDE.md` / `AGENTS.md`:
- Fix any example code still naming the event

### Step 7: Verify the build

```bash
cargo test --workspace
```

Any error means Step 2 missed a reference. Fix what the error points at and
re-run.

### Step 8: Commit

```bash
git add -p   # stage while reviewing each hunk
git commit -m "refactor(dawn-core): remove deprecated <EventName> event (ADR-XXXX)"
```

---

## Notes

- Missing even one reference from the Step 2 grep produces a compile error --
  that is the safety net, not a failure of the procedure.
- A leftover `#[allow(deprecated)]` keeps emitting warnings; sweep them.
- Run the final `rg '<EventName>'` once more after Step 7 to catch doc-only
  stragglers that the compiler cannot see.
