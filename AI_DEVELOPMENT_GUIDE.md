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
event-backed simulation with append-only public facts and a separate
authoritative recovery journal. The technical work exists to support:

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
cargo run -p dawn-server --bin simulate
cargo run -p dawn-server --bin simulate --release -- --serve
cargo run -p dawn-server --bin simulate --release -- --serve --cluster
cargo run -p dawn-server --bin simulate --release -- --serve --duel
cargo run -p dawn-server --bin simulate --release -- --serve --duel --enemies 2
cargo run -p dawn-server --bin simulate --release -- --aoi-bench
```

**Playing the Godot client against a live server**: `client/dawn_client_gdext.gdextension`
loads `target/debug/dawn_client_gdext.dll` (or the platform equivalent), which
is only rebuilt by an explicit `cargo build`/`cargo test` touching that crate
-- starting the Godot editor does **not** rebuild it. After any change to
`dawn-protocol`, `dawn-client-core`, or `dawn-client-gdext`, run
`cargo build -p dawn-client-gdext` before opening/reloading the Godot editor,
or the client silently runs against a stale binary. The symptom is subtle:
the server logs a normal handshake and then a disconnect, while the client
shows no error on screen -- `ServerMessageDecoder.decode` logs a `Serde
Deserialization Error` to the Godot output console (not a crash), for
whichever `ServerMessage` variant's shape moved since the `.dll` was last
built. Check the Godot output console for that error before suspecting a
wire-protocol or decode bug.

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

- INV-001: Committed public `DomainEvent`s are append-only facts. Do not update,
  delete, truncate, or rewrite them in place. ADR-0049's authoritative recovery
  delta is a separate versioned stream with checkpoint-governed compaction.
- INV-002: Exact Sector world state is recovered from the newest complete compatible
  checkpoint set plus every committed authoritative `RecoveryDelta` after its
  covered position. `DomainEvent`s are durable public/business facts, not the
  complete exact-state reducer. Eventless Ticks still have recovery records.
- INV-003: Cross-sector ownership transfer must go through the consensus path.
- INV-004: Entity IDs are unique and must not be reused.
- INV-005: Determinism uses logical ticks and IDs, not wall-clock time.
- INV-006: Commands are requests and may be rejected. Events are facts and must
  not be rejected.
- INV-MOVE: Public movement facts use velocity/anchor events where specified;
  exact recovery of position/velocity/flight state uses ADR-0049 RecoveryDelta,
  not historical Tick re-execution.
- INV-TiDi: TiDi is local bounded pacing only; logical ticks remain
  deterministic.
- Tick processing order and its prepare -> durable commit -> live apply boundary
  are part of the design. Check `docs/architecture/tick-model.md` and
  `docs/architecture/recovery-contract.md` before changing them.
- Actor boundaries communicate through messages/mailboxes, not direct calls into
  actor internals.
- Ship ownership is sector-local. A ship must not be owned by two sectors at
  once.
- Cross-sector transit uses the consensus path. Do not shortcut ownership moves.
- Player ownership **and active-ship command routing** are authoritative Sector
  world state. A successful `SelectActiveShip`/`Disembark` routing change is
  recoverable even when it emits no public `DomainEvent`.
- Station inventory authority is the Sector recovery journal; SQLite/repository
  Station state is an idempotent projection/read model (ADR-0038 amended by
  ADR-0049).
- Prepared admission and resume-ticket lifecycle state are different: they may be
  authoritative protocol state in #277's durable admission/identity repositories
  before a Ship is materialized. They require explicit reconciliation/catch-up,
  not accidental treatment as Station projection rows.
- Recovery must preserve authoritative state, retained reliable outputs/retry
  state, and required projection/repository reconciliation state through failover.
- A reliable post-commit action must have durable retry/idempotency state. A
  generic outbox is one implementation; #276 may represent Transit continuation
  through its durable Saga. Auto-jump cannot live only in an in-memory queue.
- Generic `ClientRequest` currently has no stable request ID. Do not claim or
  implement transparent exactly-once retry of arbitrary non-idempotent client
  commands after an ambiguous disconnect. Refresh state and treat resubmission as
  a new request unless the protocol has its own stable operation identity.
- `ReplicatedDurable` may stage committed bytes on a durability quorum before the
  owner applies them. Staged bytes are not applied/promotable state; publication
  and promotion require successful recovery-reducer/projection/repository checks.
- #278 owns runtime selection of durability profile/replica set, quorum threshold,
  durability-receipt aggregation, owner-epoch/fencing validation, and final ack
  gating. #271 owns durable journal evidence and #280 owns transport.
- Ordinary WebSocket/AoI presentation clients are not durable consumers and must
  not hold public-event retention/compaction watermarks after disconnect.
- Coordinate math must respect anchors/floating origins. Never compare raw
  anchor-relative offsets with sector-absolute positions.

Forbidden change IDs are stable because docs and ADRs refer to them:

- FBD-001: No destructive in-place public-event-log operations.
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
- `docs/architecture/recovery-contract.md`

## Authoritative Transition Workflow

ADR-0049 supersedes the old "mutate live state, then append events" workflow.
Keep this ordering intact unless a later accepted ADR changes it:

1. Receive and validate a command/Tick/committed input. A rejected command that
   changes no authority creates no recovery transition.
2. Prepare a bounded mutation without changing committed live authoritative
   state. Produce `RecoveryDelta`, public `DomainEvent`s, and reliable/runtime
   obligations together.
3. Make the complete logical transition durable under the configured durability
   profile. If this fails, discard the prepared mutation and expose no success or
   reliable effect. #271 owns journal framing/local durable evidence; #278 owns
   runtime durability-profile/quorum/ack policy and #280 transports remote
   durability messages.
4. Apply the prepared mutation through the same recovery-reducer semantics used
   by restore/replica catch-up. Apply required local projections and repository
   reconciliations idempotently.
5. If post-durable live/projection/reconciliation application fails,
   fence/fail-stop the Sector or affected authoritative service; do not continue
   from old/partial state and do not acknowledge.
6. Publish public events and execute reliable effects only after successful local
   apply. Durable delivery/retry state advances only after downstream acknowledgement
   or equivalent idempotency proof; delivery is at-least-once unless the downstream
   protocol provides stronger semantics. Ephemeral WebSocket/AoI delivery does not
   create a durable retention cursor.
7. Acknowledge the authoritative operation only after the selected durability
   profile and required local apply/projection/reconciliation conditions are
   satisfied.

`LocalDurable` RPO 0 covers process/OS/power-loss failure with the durable medium
intact. Claiming RPO 0 for owner machine/storage loss requires the synchronous
durability-quorum semantics of `ReplicatedDurable`; quorum-staged bytes are not by
themselves applied/promotable replica state. Do not enable/advertise that stronger
profile before #271/#278/#280 define and test one coherent quorum/fencing model.

### Recovery work-package ownership

Do not make one sibling refactor silently redefine another's contract:

- #284 / ADR-0049: recovery authority, RPO/RTO semantics, checkpoint/delta content
- #271: journal mechanics, durable evidence, corruption/compaction implementation
- #272: storage-independent engine and prepare -> durable -> live-apply API
- #275: state-owner decomposition; consumes the #284 authority inventory
- #276: Transit Saga/attempt/receipt/retry persistence; consumes #284 semantics
- #277: Station projection plus authoritative admission/identity repository APIs/schema
- #278: runtime durability profile/quorum/fencing/reconciliation/ack orchestration
- #280: peer/snapshot/durability transport; carries the #284/#277 representations

Do not merge command and event types. Do not invent rejection events for facts
that never happened.

Use the `/add-event` skill when introducing a new event and `/remove-event`
when deleting a deprecated one — they cover every public-event pipeline touchpoint.

### Wire protocol (client<->server)

`ServerFact` (the typed client projection) and the re-exported `ClientRequest` authority in
`crates/dawn-protocol/src/` are the schema-of-record for the wire format and
are generated into `docs/architecture/wire-protocol.schema.json` /
`wire-protocol-commands.schema.json` (see `docs/architecture/wire-protocol.md`).
After changing either enum (or a type either references), regenerate with
`cargo run -p dawn-actor --example gen_wire_schema` and commit both updated
`.schema.json` files in the same PR — `cargo test -p dawn-protocol`
(`wire_schema_doc_is_up_to_date`) fails
otherwise (`wire_schema_doc_is_up_to_date`). Never hand-edit the `.schema.json` files. `dawn-core` keeps `schemars`
optional behind its `schema` feature; `dawn-protocol` enables that feature only to
generate the versioned Sector envelope schema containing the shared `ClientRequest` authority.

The current `ClientRequest` envelope intentionally has no generic idempotency key.
If a future feature adds transparent exactly-once retry, the request/wire schema and
#278 runtime must add a stable `RequestId` (or equivalent) and durable dedup/result
retention as one designed protocol change; never infer identity from payload equality.

Since ADR-0042, the actual runtime transport for `Welcome`/`Redirect`/
`Event`/`Hello`/`Command`/`InitialState`/`PlayerLoadout`/`AoiEnter`/
`AoiLeave`/`PositionSnap`/`MotionCorrection` is postcard binary
(`ServerMessage`/`ClientMessage` in `dawn-protocol`), not JSON text. The two
schema enums are externally tagged (`{"VariantName": {...}}`), since postcard
cannot deserialize `#[serde(tag = "type")]`.

## Crate Boundaries

Keep dependencies one-way. If a change needs a new dependency, check the
workspace DAG and relevant ADR first.

- `dawn-core`: domain types, commands, events. No network, ECS, storage, or IO.
- `dawn-client-core`: Godot-independent client-side domain model (loadout,
  wire row types, WorldSession state, motion policy, and ClientInteraction
  input policy). It owns pure client state, simulation, and typed
  `ClientAction` construction; it depends only on `dawn-core` (ADR-0039,
  ADR-0041, ADR-0045, ADR-0046).
- `dawn-client-gdext`: GDExtension binding (cdylib) exposing `dawn-client-core`
  to the Godot client. Thin type-conversion adapter only (ADR-0040, ADR-0046).
- `dawn-protocol`: client<->server wire schema (`ClientRequest`/`ServerFact`,
  the `ServerMessage`/`ClientMessage` binary envelope). Depends only on
  `dawn-core` + serde + postcard -- no transport/runtime dependency, so
  `dawn-client-gdext` can depend on it directly (ADR-0041, ADR-0042).
- `dawn-ecs`: components and systems. No event store or network ownership.
- `dawn-storage`: public-fact storage plus the fallible/versioned atomic
  journal mechanics required by ADR-0049. It owns append/recovery evidence;
  Sector state and runtime orchestration consume that boundary but do not
  define a second journal implementation.
- `dawn-distributed`: one distributed-systems boundary containing Raft,
  versioned peer lifecycle/transport, replication, and anti-entropy. Its
  modules keep policy direction explicit: Raft and replication adapt the shared
  peer transport, while the transport knows no domain message semantics. It
  carries #278 ownership fencing and #284 recovery ranges without redefining
  their authority (ADR-0027, ADR-0050, #280).
- `dawn-market`: Market order book (bid/ask) + `PlayerId` Currency ledger,
  its own SQLite authority independent of Sector tick determinism. The SQLite
  layer is an adapter; the private matching policy owns crossing, price-time
  priority, partial fills, maker-price settlement, and Bid price-improvement
  refunds. Depends only on `dawn-core` + serde + rusqlite -- no
  transport/runtime dependency, same DAG position as `dawn-protocol` (ADR-0034
  §4/§5/§6). Only constructs
  `RemoveItemCommand`/`ReturnItemCommand`/`CreditItemCommand`; never applies
  them to a `SimulationNode` itself, so `dawn-sector` never depends on it.
- `dawn-sector`: current Sector game logic and broad `SimulationNode` composition.
  #272 removes persistence ownership from the pure engine; #275 splits state
  owners. Depends on `dawn-protocol` today to build typed wire messages it hands to
  `dawn-actor` (e.g. `PlayerLoadoutWire`).
- `dawn-actor`: client/server protocol and connection boundary.
- `dawn-server`: production/local server composition, runnable simulation
  modes, and demos. #278 now shares runtime orchestration so durability
  profile, repository reconciliation, ack, retry, and effect policy have one
  implementation.
  Depends on `dawn-market` to route Market-domain requests and bridge commands
  before they reach the owning `SimulationNode` (ADR-0034 §4, roadmap.md §12
  9D-4/5).
- `dawn-actor`: low-level WebSocket client transport used by `dawn-server`;
  it owns no Sector or runtime policy.

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
  invisible to manual play — recovery reducer/checkpoint-tail equivalence,
  public-event projection (`node/apply_event.rs`), and wire conversion
  (`dawn-protocol` conversion modules) first. New match arms in public replay/wire
  conversion need direct tests in the same PR. Deliberately uncovered code is
  named in the PR description with the reason, never silently skipped. See
  #112 for the audit pattern.

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
- Recovery contract: `docs/architecture/recovery-contract.md`
- Recovery ADR: `docs/adr/ADR-0049-sector-recovery-state-delta-wal.md`
- Forbidden changes: `docs/architecture/forbidden-changes.md`
- Event catalog: `docs/architecture/event-catalog.md`
- Wire protocol (postcard binary plus the remaining JSON frames at the
  client/server boundary): `docs/architecture/wire-protocol.md`
- Tick model: `docs/architecture/tick-model.md`
- Client testing: `docs/process/godot-client-testing.md`
- Raspberry Pi hardware flow: `docs/process/8d5-hardware-notes.md`
- Commit convention: `docs/process/commit-convention.md`
- Third-party license notes (non-permissive dependencies): `THIRD-PARTY-LICENSES.md`
- EVE Online research notes: `docs/reference/eve-reference.md`
- Carbon Engine comparison: `docs/reference/carbon-engine-comparison.md`

## Agent Configuration

This repo uses Matt Pocock's engineering skills configuration:

- pinned upstream skill source: `.agents/vendor/mattpocock-skills/`
- Dawn-specific procedures: `.agents/commands/`
- Claude Code compatibility shims: `.claude/commands/` and `.claude/settings.json`
- skill source and initialization: `docs/agents/skill-source.md`
- issue tracker: `docs/agents/issue-tracker.md`
- triage labels: `docs/agents/triage-labels.md`
- domain docs layout: `docs/agents/domain.md`

When a skill applies, use the pinned source or the Dawn-specific adapter named
in `AGENTS.md`; do not assume the skill is installed globally. Keep this file
short; add detailed project memory to the correct doc instead of expanding
this guide.

---

Last updated: 2026-08-11 / Covers ADR-0001 through ADR-0052
