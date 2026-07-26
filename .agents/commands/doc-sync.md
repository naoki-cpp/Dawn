# /doc-sync — Detect and fix drift between docs and implementation

This skill cross-checks the documentation against the current codebase, finds
mismatches / stale entries / typos, and fixes them in one pass.

Run it at phase completion, after large refactors, or as periodic session-start
maintenance.

---

## Steps

### Step 1: Event definitions

Read the `DomainEvent` enum in `crates/dawn-core/src/events.rs` and compare it
with the event list in `docs/architecture/event-catalog.md`. Do the same for
commands in `crates/dawn-core/src/commands.rs`.

Check:
- Event in code but missing from the catalog → add it to the catalog
- Event in the catalog but not in code → remove it (or mark it deleted)
- Field mismatches (type / field name) → fix the catalog to match the code
- Status column (implemented / not implemented / @deprecated) wrong → fix

### Step 2: Wire protocol

Run `cargo test -p dawn-actor wire_schema_doc_is_up_to_date` to check the two
generated schema files (`docs/architecture/wire-protocol.schema.json` /
`wire-protocol-commands.schema.json`) against `EventJson` /
`ClientCommandJson` in `crates/dawn-actor/src/protocol.rs`. If it fails,
regenerate with `cargo run -p dawn-actor --example gen_wire_schema` and
commit the updated files — do not hand-edit them.

The schema files are machine-checked; the prose in
`docs/architecture/wire-protocol.md` is not. Check it separately:

- The `"type"` value lists for both halves match the current enum variants
- The documented quirks (`WarpCommand`'s legacy `gate_id` vs `target` form,
  the `gate_id`/`target_id` selection on `ApproachCommand`/`OrbitCommand`/
  `KeepAtRangeCommand`) still match `client_command_from_json`
- The list of `DomainEvent` variants that never reach the wire (return `None`
  from `domain_event_to_json`) is still accurate

### Step 3: ADR implementation checklists

Only ADRs that contain an implementation checklist section are in scope.
Grep `docs/adr/` for `実装チェックリスト` (the section heading used in the
Japanese ADR bodies) first — do not bulk-read every ADR.

Check:
- Items still `[ ]` that are actually implemented → flip to `[x]`
- Prose that contradicts the current code → fix
- References to types / fields / methods that no longer exist → fix

### Step 4: Tick processing order

Read the step order in `tick_with_lock_commands()` in
`crates/dawn-sector/src/node/tick.rs` and compare it with
`docs/architecture/tick-model.md` ("Tick processing steps" section).

Check:
- Step order, count, and content match the implementation
- The events emitted at each step are listed correctly

### Step 5: Roadmap

Read `docs/process/roadmap.md` and verify completion flags (`[x]` / done
markers) against reality. Completed-phase details live in
`docs/process/roadmap-history.md`; roadmap.md keeps only the completed-phase
summary plus the in-progress phase.

Check:
- Tasks implemented but still `[ ]` → flip to `[x]`
- Completed-phase summaries match what was actually built
- No unmet prerequisites listed for the next phase
- When a phase completes: add one summary line to roadmap.md and move the
  detailed record to roadmap-history.md (not into roadmap.md)

### Step 6: AI_DEVELOPMENT_GUIDE.md

Read `AI_DEVELOPMENT_GUIDE.md` (CLAUDE.md only delegates to it) and verify:

- "Crate Boundaries" lists every crate in `crates/` (compare with `ls crates/`)
  and the one-way dependency rules still hold
- "Common Commands" all still work as written (spot-check anything suspicious)
- "Architecture Invariants" and the FBD-001..009 list match
  `docs/architecture/forbidden-changes.md`
- Every link in "Documentation Map" and "Reference docs" resolves to an
  existing file
- The footer "Last updated / Covers ADR-XXXX through ADR-YYYY" matches the
  highest ADR number in `docs/adr/`
- The guide starts with a single H1 and no bare `#` lines outside code fences

### Step 7: Player-facing docs

Read `docs/process/playtest-guide.md` and verify it matches current controls
and features.

Check:
- Keybindings match the implementation in `client/scripts/main.gd`
- No documented-but-removed features; no implemented-but-undocumented ones

### Step 8: Design docs

Read `docs/architecture/architecture.md`, `docs/architecture/entity-model.md`,
`docs/architecture/ownership.md`, and `docs/design/game-design.md`, and verify
implementation-status statements.

Check:
- architecture.md: crate list and dependency DAG cover every crate
  (compare with `ls crates/`); transport description does not contradict
  ADR-0007
- architecture-review/server.md / architecture-review/client.md: line counts
  in the file-size tables match reality (`wc -l` on the biggest files — these
  go stale fastest; full refresh belongs to /architecture-review)
- entity-model.md: ECS component list matches `crates/dawn-ecs/src/components/`;
  nothing marked "future / unimplemented" is actually implemented
- ownership.md: the status table and phase labels match the current phase;
  no state-transition entries still marked unimplemented that now exist
- game-design.md: §4.1 (implemented, with ADR references) vs §4.2 (future,
  unimplemented) are correctly separated; promote items from 4.2 to 4.1 as
  they land

---

## Report format

After each step report either:

```
### Step N: <target>
OK — no drift
```

or

```
### Step N: <target>
Drift — N issue(s)
  - <file>: <what>
  - ...
-> fixed
```

After all steps, commit any changes together:
`docs: sync documentation with current implementation`
