# /ai-change-checklist — Pre-change checklist for code changes

Run through this checklist before changing code. This skill is the canonical
procedure delegated from `AI_DEVELOPMENT_GUIDE.md` (ADR-0030); the guide keeps
only a pointer to it.

**If any item cannot be answered "no problem", stop and ask the human before
changing anything.**

---

## Before any change

```
[ ] Identified which crate(s) the change touches
[ ] Checked that crate's responsibility in AI_DEVELOPMENT_GUIDE.md
    "Crate Boundaries" (and docs/architecture/architecture.md for the DAG)
[ ] Identified downstream crates affected via the dependency DAG
    (cargo tree if in doubt)
[ ] Confirmed the change is in scope for the current roadmap phase
    (docs/process/roadmap.md)
[ ] Confirmed the change does not violate any Architecture Invariant
    (AI_DEVELOPMENT_GUIDE.md "Architecture Invariants": INV-001..006,
    INV-MOVE, INV-TiDi, and the unnumbered invariants listed there)
[ ] Confirmed the change is not a Forbidden Change
    (docs/architecture/forbidden-changes.md, FBD-001..009)
```

## Extra checks when adding or changing an Event

Prefer running `/add-event` for new events — it walks the full workflow.
Minimum bar either way:

```
[ ] Planned the docs/architecture/event-catalog.md update (same PR as code)
[ ] New event lives in crates/dawn-core/src/events.rs (never another crate)
[ ] New event carries tick: Tick (INV-005)
[ ] All initial fields are required, not Option
    (Option is only for fields added after release — never at creation)
[ ] A corresponding Command exists in crates/dawn-core/src/commands.rs
    (commands and events never share a type — INV-006)
[ ] If changing an existing event: checked release status
    - Pre-release (current): direct breaking change is allowed, no upcaster
    - Post-release: follow docs/architecture/event-schema-evolution.md
      "post-release breaking change procedure"
```

## Extra checks when the wire protocol changes

If the change touches `ServerFact` or `ClientRequest` in
`crates/dawn-protocol/src/` (or a type either references —
`PosJson`, `VelJson`, `WarpTargetJson`):

```
[ ] Regenerated both schema files:
    cargo run -p dawn-protocol --example gen_wire_schema
[ ] Committed the updated docs/architecture/wire-protocol.schema.json and
    wire-protocol-commands.schema.json in the same PR as the code change
[ ] cargo test -p dawn-protocol passes (wire_schema_doc_is_up_to_date catches
    a forgotten regeneration)
[ ] If the set of "type" values or a documented quirk changed (e.g. a new
    command, a field becoming required), updated the prose in
    docs/architecture/wire-protocol.md to match — the schema files are
    generated, but that prose is hand-maintained
[ ] Did not add a new pub type to dawn-core just to reuse it in protocol.rs
    (FBD-002: dawn-core must not depend on schemars)
[ ] New/changed field is untrusted client input: ran /security-check (wire
    scope) — closed-enum validation for strings, is_finite() for floats that
    feed physics/geometry, no unbounded collection without a length cap
```

## Extra checks when adding a new crate

```
[ ] Confirmed the need cannot be met by re-splitting existing crate
    responsibilities
[ ] Decided the new crate's position in the dependency DAG
[ ] Verified no dependency cycle (cargo tree)
[ ] Updated AI_DEVELOPMENT_GUIDE.md "Crate Boundaries" and
    docs/architecture/architecture.md
[ ] Wrote an ADR in docs/adr/ (use /new-adr)
```

## Test checks

```
[ ] Every changed pub fn has a corresponding test (FBD-007)
[ ] Test names describe the guarantee, not the implementation step
[ ] cargo test --workspace passes with zero errors
[ ] If an ADR with an implementation checklist is involved, its invariant
    tests exist
[ ] client/scripts/ change with scene-tree-free pure logic: GdUnit4 test
    added under client/test/ (docs/process/godot-client-testing.md)
[ ] client/scripts/ change that depends on the scene tree: manual Godot
    editor verification (or its absence) is stated in the PR description
[ ] PR adds/changes a pub item: run /rust-api-audit before opening the PR
[ ] PR adds a new client-facing command, a new SQL call site, or a new
    `*_owned` handler: run /security-check before opening the PR
[ ] PR adds a new logic-bearing .rs file, or adds match arms to event
    replay (node/apply_event.rs) or wire conversion (dawn-protocol/src/): the
    new arms have direct tests, not just incidental
    coverage through other tests. These two files' failure modes only
    surface on restart / at the client, so gaps stay invisible to manual
    testing (see /coverage-audit for the full periodic audit)
```

## PR description checks

```
[ ] Motivation stated (why this change is needed)
[ ] ADRs referenced or changed are listed (e.g. "see ADR-0003")
[ ] Changed crates listed
[ ] Affected events listed (if any)
[ ] Test method described
```
