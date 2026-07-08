# /architecture-review — Periodic maintenance review from a senior-architect viewpoint

This skill re-measures and re-evaluates the server and client architecture
reviews against the latest codebase, **updating those files in place**. Each
side is split into three files by purpose, so a reader (or a future review
pass) can jump straight to the kind of information they need instead of
scanning one long combined document:

| Purpose | Server file | Client file |
|---|---|---|
| Structural assessment (grades, file sizes, issue-ID registry) | `docs/architecture/architecture-review/server.md` | `docs/architecture/architecture-review/client.md` |
| Completed-work log (dated, append-only) | `docs/architecture/architecture-review/server-completed.md` | `docs/architecture/architecture-review/client-completed.md` |
| Pending items (open issues, deferrals, rejected approaches) | `docs/architecture/architecture-review/server-pending.md` | `docs/architecture/architecture-review/client-pending.md` |

> **This is an analysis skill. It changes no code.**
> Where `/simplify` actually fixes messy code, this skill takes stock of
> codebase health and records findings and a roadmap in the review documents.
> Fixes happen later, in separate PRs, via the roadmap items this review files
> (through `/simplify` or manual refactoring). The separation is deliberate:
> the review diff stays docs-only, so review and remediation can be separate
> PRs.

Arguments (optional):
- `server` = server side only (the three `architecture-review/server*.md` files)
- `client` = client side only (the three `architecture-review/client*.md` files)
- omitted = both

---

## Phase 0 — Re-measure file sizes and diff against the tables

Measure actual source line counts for the target scope and compare with the
"file size" tables in the **assessment** file (`architecture-review/server.md` /
`architecture-review/client.md`) — not the completed/pending files, which
carry no line-count tables.

Server:
```bash
find crates -name '*.rs' -not -path '*/target/*' | xargs wc -l | sort -rn | head -50
```
Client:
```bash
find client/scripts -name '*.gd' | xargs wc -l | sort -rn
```

Check:
- In the table but the file is gone → remove the row (track renames/merges).
- On disk but not in the table (new file) → add a row with a verdict.
- Line count drifted → update to the measured value. **This is the most
  common staleness** — counts creep up silently as features land.

> The table counts are hand-written, so they are almost always stale after
> feature work (e.g. ADR implementations). The measurement is the truth.

## Phase 1 — Re-evaluate health (green / yellow / red)

Re-grade every file. The verdict weighs **cohesion of responsibility first,
size second** — not a strict line cutoff:

- **Green — healthy**: single cohesive responsibility. Small, or large but
  "splitting would be unnatural" (a single geometry kernel, a state machine).
- **Yellow — watch**: growing (~500+ lines as a guide), responsibilities
  starting to mix, or a split candidate where "not yet" is the right call.
  Record the reason and the trigger for when to split.
- **Red — act**: clearly past the threshold (~700+ lines as a guide) AND
  multiple responsibilities cohabiting. File it on the pending file (see
  Phase 2) with a root cause, decision, and trigger.

Also refresh the overall grade table ("現状評価") in the assessment file —
server axes: crate structure / file size / type design / duplication /
Rust-specific / AI-development-induced; client axes: file split /
responsibility cohesion / duplication / coupling / test coverage.

### Grade scale (A–F with ±)

Grades measure **the amount of unmanaged debt (no decision, no trigger)** —
not the mere existence of debt. Debt that has a documented acceptance decision
or re-evaluation trigger does not lower the grade (it is managed).

| Grade | Meaning |
|---|---|
| **A** | As designed. Zero unmanaged debt. The ideal for that axis. |
| **A−** | Near-ideal. Only minor debt that is **accepted with a decision and trigger**. |
| **B+** | Good, but several items **under watch (yellow)** or large files mid-split. No harm yet. |
| **B** | Works, but structural debt is visible. Direction is set, journey incomplete. |
| **B−** | Multiple visible debts coexist. Remediation started but unfinished. |
| **C** | Clear need for improvement: naming pollution, large duplication, an unsplit god object, or red files. Needs pending-file entries. |
| **D / F** | Design is broken (DAG inversion, everything in one file, no tests). Not expected in this project, but defined as the floor. |

`±` within a band: top of the band → `+`, one foot in the next band down → `−`.
**The overall grade is dragged by the weakest axis** (bottleneck axis ±1, not
an average).

Justify each grade in the "reason" column with concrete file names, issue IDs,
and line counts as of this review (never just "good"). When a grade moves,
add one line on what got fixed/worse to move it. Keep the scale table itself
free of file names and issue IDs — those change with the code and belong only
in the reason column.

Evaluation axes used by past findings:
- **Crate DAG / dependency direction**: consistent with architecture.md's DAG?
  No upper context depending on a lower one?
- **Duplication**: copied logic of the same shape (two-binary glue,
  event↔JSON conversion, etc.).
- **Rust-specific**: needless `Box<dyn>` / `Mutex`, `inner()` escapes, large
  enum size asymmetry (`Box` candidates).
- **AI-development-induced**: naming pollution, excess thin wrappers, ad-hoc
  specialization.
- **Tests** (client): is scene-tree/network-free pure logic tested?

## Phase 2 — Issue inventory (IDs, root cause, decision, re-evaluation trigger)

Carry the existing issue lists forward (server: `M-`/`L-`/`R-`/`P-`; client:
`C-`). **Preserve the ID scheme and numbering continuity** — new issues take
the next number. Live (open) issues, in-progress refactor-roadmap entries, and
"deliberately not doing this" decisions all go in the **pending** file
(`architecture-review/server-pending.md` / `architecture-review/client-pending.md`).

Every issue must carry three things:
1. **Root cause**: structural, not surface ("no shared home for X exists",
   not "this file is big").
2. **Decision**, one of:
   - **Fix** (file on the improvement roadmap in the pending file → executed
     later via `/simplify` or manual refactor)
   - **Defer** (must include a **re-evaluation trigger** — "what happening
     would make us start". A defer without a trigger is forbidden)
   - **Accept** (why the cost/benefit doesn't pay, consistent with past
     rejections such as the `dawn-proto` non-adoption or the P4-3 skip)
3. **State transition** since last review: an issue that resolves this pass
   moves out of the pending file into the **completed** file
   (`architecture-review/server-completed.md` /
   `architecture-review/client-completed.md`), dated, with a short
   description of what changed. Leave a one-line strikethrough pointer behind
   in the pending file (`~~M-7~~ resolved — see completed.md`) so the ID
   registry and any code comments referencing it (client `.gd` files cite
   issue IDs like `architecture-review/client.md C-1`) still resolve.

> Decisions must not contradict CLAUDE.md / AI_DEVELOPMENT_GUIDE.md policy
> (new-crate checklist, 8D minimization, anti-grind, etc.). Improvements that
> change behavior get "file an ADR" written down — this skill never writes
> the ADR itself.

## Phase 3 — Update the review documents

Update the three target files in place, each staying within its own lane:

- **Assessment** (`architecture-review/server.md` / `-client.md`): front-matter
  `date` → today plus a one-line summary (e.g. `2026-06-24 (filed R-2;
  main.gd 1255→1239 remeasured)`); rewrite the grade table and file-size
  tables from Phase 0–1. Keep the client-side issue-ID registry table here
  even for fully-resolved IDs (code comments point at this file by ID).
- **Completed** (`*-completed.md`): append newly-resolved issues/roadmap items
  with the date and a short description of what changed. Never delete or
  rewrite past entries — this file is an append-only log.
- **Pending** (`*-pending.md`): rewrite the open-issue list, refactor
  roadmap, and "won't do" decisions from Phase 2. Remove entries that moved to
  completed this pass, leaving only the strikethrough pointer if useful for
  continuity.

Keep the existing style, table structure, verdict markers, and IDs across all
three files — do not invent a new format.

---

## Guardrails (specific to this skill — mandatory)

- **Change zero lines of code.** Only the six
  `docs/architecture/architecture-review/{server,client}[-completed|-pending].md`
  files may change. If remediation is needed, **file it** in the pending
  file — nothing more.
- **Do not edit CLAUDE.md / AI_DEVELOPMENT_GUIDE.md / docs/adr/** (reading is
  fine).
- **Never break ID continuity.** Do not delete or renumber past `M-`/`L-`/
  `R-`/`P-`/`C-` entries, even when they move from pending to completed.
- **Never delete history from the completed file.** It is an append-only
  audit log; correct forward, don't rewrite the past.
- Line counts and verdicts come from **actual measurement** (`wc -l`), never
  from memory or the previous values.
- Every defer carries a trigger. Neither force a fix nor defer vaguely.

## Completion report

- Summarize: measurement changes (gone / new / drifted counts), grade moves,
  issue IDs filed (now in pending) and resolved (now in completed).
- Docs-only change, so no test run needed. Show the diff scope with
  `git diff --stat docs/architecture/`.
- Ask the user before committing (English, Conventional Commits, e.g.
  `docs(architecture): refresh server review — recount sizes, file R-2`).
