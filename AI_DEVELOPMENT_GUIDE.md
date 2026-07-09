# Dawn AI Development Guide

This file is the short, always-read operating guide for AI agents working in
this repository. Keep it concise. Long-lived design detail belongs in
`docs/adr/`, `docs/architecture/`, or `docs/process/`, not here.

## First Steps

1. Read this file before changing code.
2. Inspect the current code before proposing a design.
3. Check the relevant ADRs and architecture docs for the area being changed.
4. Prefer small, reversible changes with a tight verification loop.
5. If a rule here conflicts with an ADR or architecture invariant, stop and ask.

## Project North Star

Dawn is building an EVE-like single-shard space game with a distributed,
event-sourced simulation. The technical work exists to support:

- large real-time battles without EVE-style global TiDi
- player-driven territory, structures, economy, and risk
- intentional combat decisions instead of grind
- real loss, tackle, and dangerous space

For design intent, start with:

- `docs/adr/ADR-0016-game-vision.md`
- `docs/process/roadmap.md`
- `docs/architecture/architecture.md`

## Common Commands

```bash
cargo fmt --all -- --check
cargo fmt --all
cargo test --workspace
cargo test -p dawn-core
cargo test test_name_filter
cargo build --workspace
cargo build --workspace --release
cargo deny check                              # licenses/bans/advisories/sources (deny.toml)
cargo audit                                   # RUSTSEC vulnerability advisories
cargo machete                                 # unused dependencies
cargo semver-checks check-release --baseline-rev main   # public API breakage vs main
```

Simulation:

```bash
cargo run -p dawn-simulation --bin simulate
cargo run -p dawn-simulation --bin simulate --release -- --serve
cargo run -p dawn-simulation --bin simulate --release -- --serve --cluster
cargo run -p dawn-simulation --bin simulate --release -- --serve --duel
cargo run -p dawn-simulation --bin simulate --release -- --serve --duel --enemies 2
cargo run -p dawn-simulation --bin simulate --release -- --aoi-bench
```

Godot client tests:

- Follow `docs/process/godot-client-testing.md`.
- Use GdUnit4 for `client/scripts/` changes when the logic is testable without
  scene-tree or editor-only behavior.

Raspberry Pi cluster:

- Follow `docs/process/8d5-hardware-notes.md`.
- Use `scripts/setup-pi-cluster.sh`, `scripts/deploy-pi-cluster.sh`, and
  `scripts/run-pi-cluster.sh`.

## Architecture Invariants

Do not violate these. Details live in `docs/architecture/`.

- INV-001: Event log is append-only. Do not update, delete, truncate, or
  rewrite events.
- INV-002: Events and authoritative snapshots are the source of truth. State is
  derived, cached, or rebuilt from snapshot plus tail replay.
- INV-003: Cross-sector ownership transfer must go through the consensus path.
- INV-004: Entity IDs are unique and must not be reused.
- INV-005: Determinism uses logical ticks and IDs, not wall-clock time.
- INV-006: Commands are requests and may be rejected. Events are facts and must
  not be rejected.
- INV-MOVE: Movement is replayed from velocity changes and deterministic
  integration, not per-tick position events.
- INV-TiDi: TiDi is local bounded pacing only; logical ticks remain
  deterministic.
- Tick processing order is part of the design. Check
  `docs/architecture/tick-model.md` before changing it.
- Actor boundaries communicate through messages/mailboxes, not direct calls into
  actor internals.
- Ship ownership is sector-local. A ship must not be owned by two sectors at
  once.
- Cross-sector transit uses the consensus path. Do not shortcut ownership moves.
- Snapshot restore must preserve authoritative state and catch up from the hot
  event log.
- Coordinate math must respect anchors/floating origins. Never compare raw
  anchor-relative offsets with sector-absolute positions.

Forbidden change IDs are stable because docs and ADRs refer to them:

- FBD-001: No destructive event-log operations.
- FBD-002: No external dependencies in `dawn-core`.
- FBD-003: No wall-clock time for causal order.
- FBD-004: No direct actor-to-actor method calls.
- FBD-005: No ship/entity ID reuse.
- FBD-006: No sector transit that bypasses consensus.
- FBD-007: No untested public behavior.
- FBD-008: Retired by ADR-0016; new contexts still need ADR/DAG review.
- FBD-009: No skill-point growth, passive growth, pay-to-win, or AFK mining.

Reference docs:

- `docs/architecture/forbidden-changes.md`
- `docs/architecture/design-violations.md`
- `docs/architecture/event-catalog.md`
- `docs/architecture/event-schema-evolution.md`
- `docs/architecture/ownership.md`
- `docs/architecture/tick-model.md`

## Event Workflow

Keep this flow intact unless an ADR changes it:

1. Receive a command from a client, script, simulation, or actor message.
2. Validate the command. Rejected commands do not emit domain events.
3. Apply domain logic to the authoritative in-memory model.
4. Generate domain events that describe facts that happened.
5. Append events to the event store. If append fails, do not treat the state
   change as committed.
6. Replicate or gossip the append-only log through the relevant runtime path.
7. Update projections, read models, and client snapshots from committed facts.

Do not merge command and event types. Do not invent rejection events for facts
that never happened.

Use the `/add-event` skill when introducing a new event and `/remove-event`
when deleting a deprecated one — they cover every pipeline touchpoint.

### Wire protocol (client<->server JSON)

`EventJson` and `ClientCommandJson` in `crates/dawn-actor/src/protocol.rs` are
the schema-of-record for the wire format and are generated into
`docs/architecture/wire-protocol.schema.json` /
`wire-protocol-commands.schema.json` (see `docs/architecture/wire-protocol.md`).
After changing either enum (or a type either references), regenerate with
`cargo run -p dawn-actor --example gen_wire_schema` and commit both updated
`.schema.json` files in the same PR — `cargo test -p dawn-actor` fails
otherwise (`wire_schema_doc_is_up_to_date`). Never hand-edit the `.schema.json`
files; never add a new domain type to `dawn-core` just to reuse it here
(FBD-002 keeps `dawn-core` free of the `schemars` dependency).

## Crate Boundaries

Keep dependencies one-way. If a change needs a new dependency, check the
workspace DAG and relevant ADR first.

- `dawn-core`: domain types, commands, events. No network, ECS, storage, or IO.
- `dawn-ecs`: components and systems. No event store or network ownership.
- `dawn-event-store`: append-only persistence and snapshots.
- `dawn-consensus`: Raft and consensus transport.
- `dawn-replication`: sector-local replication and anti-entropy.
- `dawn-sector`: sector game logic, ownership, transit, warp, AoI, snapshots.
- `dawn-actor`: client/server protocol and connection boundary.
- `dawn-simulation`: runnable simulation wiring and demos.
- `dawn-sector-node`: real hardware node binary and TOML config loading.

## Change Workflow

Use the same discipline for features and bugs:

1. Build a tight feedback loop first: a test, script, replay, or manual command
   that can catch the exact behavior.
2. Make the smallest implementation change.
3. Add or update tests at the seam that actually exercises the behavior.
4. Run the narrowest useful check, then broader checks if the change crosses
   crate or runtime boundaries.
5. Remove temporary logs, debug scripts, and probes before committing.

Before changing invariants, crate responsibilities, event schemas, tick order,
or AI steering files, check the ADR index and get human approval. If the change
is architectural, record the decision in `docs/adr/` (use the `/new-adr` skill).

For hard bugs, follow the `diagnosing-bugs` skill:

- reproduce
- minimize
- rank falsifiable hypotheses
- instrument one variable at a time
- fix with a regression test
- clean up and record the root cause

## Testing Rules

- Tests are required for behavior changes unless no correct seam exists.
- If no correct seam exists, say so explicitly and document the residual risk.
- Test names should describe the guarantee, not the implementation step.
- Actor tests should use messages/mailboxes, not direct access to actor internals.
- Client tests should extract pure logic where possible; scene-tree and editor
  behavior can remain manual if documented.
- Run `cargo fmt --all -- --check` before committing Rust changes.
- Any PR that adds or changes a `pub` item in a Rust crate (new type, new
  constructor, changed error enum, new crate) must be checked against the
  [Rust API Guidelines checklist](https://rust-lang.github.io/api-guidelines/checklist.html)
  before opening the PR. Focus on the categories most likely to regress here:
  `C-DEBUG` (public types implement `Debug`), `C-VALIDATE` (constructors
  reject invariant-breaking input), `C-GOOD-ERR` (error variants carry
  context), and `C-CRATE-DOC`/`C-EXAMPLE` (new crates get a crate-level doc
  example). Note any deliberately-skipped item in the PR description rather
  than silently ignoring it. See #82 and #83 for the audit pattern. The
  `/rust-api-audit` skill runs this audit end to end.
- Coverage is audited periodically with `cargo llvm-cov` (the
  `/coverage-audit` skill runs the procedure end to end), not gated per PR.
  Wiring/binary files at 0% are intentional (covered by manual/hardware
  verification); the gaps worth closing are logic whose failure mode is
  invisible to manual play — event replay (`node/apply_event.rs`, INV-002)
  and wire conversion (`dawn-actor/src/protocol.rs`) first. New match arms
  in those two files need direct tests in the same PR. Deliberately
  uncovered code is named in the PR description with the reason, never
  silently skipped. See #112 for the audit pattern.

## Encoding Rules

All repository text files are UTF-8.

Reading:

- In PowerShell, use `Get-Content -Encoding UTF8 <path>`.
- Prefer `rg` for search.
- Prefer `git show` and `git diff` for repository content inspection.

Writing:

- Prefer `apply_patch` for manual edits.
- Do not rewrite source/docs with PowerShell redirection (`>` / `>>`) unless
  encoding is explicit.
- Scripts that write text must write UTF-8 explicitly.

If Japanese appears as mojibake such as `笏` or `繧`, do not infer the content
and edit it. Re-read as UTF-8 first. If the file itself is corrupted, fix that
as a separate change.

## Comments And Commits

- New code comments must be English.
- Commit messages must be English and follow `docs/process/commit-convention.md`.
- Do not bulk-convert old comments. Convert nearby comments only when touching
  the relevant code.
- Docs may be Japanese when they are user-facing process notes or design notes.

## Git Safety

- Do not overwrite user work.
- Do not run destructive commands such as `git reset --hard`, `git clean`, or
  `git checkout -- <path>` unless the user explicitly asks or approves.
- If the worktree has unrelated changes, stage only the intended files.
- Prefer small branches and draft PRs for AI-authored work.

## Documentation Map

Use this guide as the router, then read the relevant long-form doc:

- ADR index: `docs/adr/README.md`
- Domain context and vocabulary: `CONTEXT.md`
- Game vision: `docs/adr/ADR-0016-game-vision.md`
- Architecture overview: `docs/architecture/architecture.md`
- Forbidden changes: `docs/architecture/forbidden-changes.md`
- Event catalog: `docs/architecture/event-catalog.md`
- Wire protocol (client<->server JSON over WebSocket): `docs/architecture/wire-protocol.md`
- Tick model: `docs/architecture/tick-model.md`
- Client testing: `docs/process/godot-client-testing.md`
- Raspberry Pi hardware flow: `docs/process/8d5-hardware-notes.md`
- Commit convention: `docs/process/commit-convention.md`
- EVE Online research notes: `docs/reference/eve-reference.md`
- Carbon Engine comparison: `docs/reference/carbon-engine-comparison.md`

## Agent Configuration

This repo uses Matt Pocock's engineering skills configuration:

- issue tracker: `docs/agents/issue-tracker.md`
- triage labels: `docs/agents/triage-labels.md`
- domain docs layout: `docs/agents/domain.md`

When a skill applies, use it. Keep this file short; add detailed project memory
to the correct doc instead of expanding this guide.

---

Last updated: 2026-07-10 / Covers ADR-0001 through ADR-0038
