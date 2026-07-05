# /add-event — Add a new Command/Event pair end to end

**Argument:** the new event name, optionally with the triggering command
(e.g. `/add-event CargoJettisoned` or `/add-event CargoJettisoned JettisonCargoCommand`)

This skill walks a new domain event from design to docs so nothing in the
event pipeline is missed. It is the constructive counterpart of
`/remove-event`.

**Design rules that must hold (from the Event Workflow and event-catalog.md):**
- Events are facts and cannot be rejected; commands are requests and can be
  (INV-006). Never merge the two, and never invent a "rejection event" for
  something that did not happen.
- Every event carries `tick: Tick` (INV-005).
- All initial fields are required — never `Option` at creation. `Option<T>`
  is reserved for fields added post-release.
- Events live in `dawn-core` only (FBD-002 keeps dawn-core dependency-free).

---

## Steps

### Step 0: Decide whether this is really a new event

- Is it a **fact** (something that happened) rather than a wish or a request?
  If it's a request, it's a Command, and the event is what the command
  produces on success.
- Could an existing event carry the fact? Check the table in
  `docs/architecture/event-catalog.md` first — a new field on nothing
  (pre-release allows breaking changes) or an existing event may suffice.
- Derived/transient state (position integration, capacitor level, lock
  countdowns) is **not** evented — it lives in snapshots and is recomputed
  per tick (see event-catalog.md "Persistence model"). Do not add per-tick
  state events (INV-MOVE).

If in doubt, stop and ask before writing code.

### Step 1: Catalog first

Add the event (and command) to `docs/architecture/event-catalog.md`:
- a row in the event table with status
- a detail section: fields with types, emitter, and a **Replay** note
  (how authoritative state is reconstructed from this event)

Writing the catalog entry first forces the field design to be reviewed as a
contract before it hardens into code.

### Step 2: Types in dawn-core

`crates/dawn-core/src/events.rs`:
- `pub struct <EventName> { ..., tick: Tick }` — all fields required
- add the `DomainEvent::<EventName>(<EventName>)` variant
- extend the `ship_id()` and `tick()` match arms
- add a unit test in `events.rs` (serde round-trip at minimum)

`crates/dawn-core/src/commands.rs` (when the event is command-triggered):
- add the command type; validation failures reject the command and emit
  **no** event

### Step 3: Emit and apply in dawn-sector

- Command validation + domain logic + emission: the relevant module under
  `crates/dawn-sector/src/node/` (dispatch enters via
  `apply_client_command` in `node/commands.rs`; tick-driven emission goes in
  the correct step of `tick_with_lock_commands` — check
  `docs/architecture/tick-model.md` before touching the order)
- `crates/dawn-sector/src/node/apply_event.rs`: add the `apply_event()` match
  arm so replay reconstructs state from the event
- Ask: does snapshot restore + tail replay still produce the same state?
  If the event mutates state that snapshots capture, check
  `crates/dawn-sector/src/persistence/` and `node/snapshot_io.rs`

### Step 4: Client delivery (when the client must see it)

- `crates/dawn-sector/src/node/serialization.rs`: event→JSON conversion arm
- `client/scripts/`: handle the new message; pure logic gets a GdUnit4 test
  under `client/test/` (docs/process/godot-client-testing.md)

### Step 5: Tests

- Unit test in `events.rs` (Step 2)
- A seam test proving: valid command → event appended → state changed;
  invalid command → rejected → **no** event appended
- A replay test if the event participates in state reconstruction
  (apply_event arm from Step 3)
- Test names describe the guarantee, not the mechanism

### Step 6: Docs sweep

- `docs/architecture/event-catalog.md` — final check that the entry matches
  the shipped code (Step 1 was written before the code existed)
- `docs/architecture/tick-model.md` — if the event is emitted during the tick
  pipeline, add it to the right step
- The owning ADR — flip implementation-checklist items, or create the ADR via
  `/new-adr` if the event introduces a new mechanic

### Step 7: Verify and commit

```bash
cargo fmt --all
cargo test --workspace
```

Commit per `docs/process/commit-convention.md`, e.g.:

```
feat(dawn-core): add <EventName> event for <mechanic> (ADR-XXXX)
```

PR description lists: motivation, referenced ADRs, changed crates, the new
event/command, and the test method (see `/ai-change-checklist`).
