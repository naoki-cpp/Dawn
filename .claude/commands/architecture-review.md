# /architecture-review — Periodic maintenance review from a senior-architect viewpoint

This skill re-measures and re-evaluates
`docs/architecture/architecture-review-server.md` (Rust crates) and
`docs/architecture/architecture-review-client.md` (Godot client) against the
latest codebase, **updating those files in place**.

> **This is an analysis skill. It changes no code.**
> Where `/simplify` actually fixes messy code, this skill takes stock of
> codebase health and records findings and a roadmap in the review documents.
> Fixes happen later, in separate PRs, via the roadmap items this review files
> (through `/simplify` or manual refactoring). The separation is deliberate:
> the review diff stays docs-only, so review and remediation can be separate
> PRs.

Arguments (optional):
- `server` = server side only (`architecture-review-server.md`)
- `client` = client side only (`architecture-review-client.md`)
- omitted = both

---

## Phase 0 — Re-measure file sizes and diff against the tables

Measure actual source line counts for the target scope and compare with the
"file size" tables in the review document.

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
  multiple responsibilities cohabiting. File it on the improvement roadmap in
  Phase 2.

Also refresh the overall grade table ("current assessment") — server axes:
crate structure / file size / type design / duplication / Rust-specific /
AI-development-induced; client axes: file split / responsibility cohesion /
duplication / coupling / test coverage.

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
| **C** | Clear need for improvement: naming pollution, large duplication, an unsplit god object, or red files. Needs roadmap entries. |
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
the next number.

Every issue must carry three things:
1. **Root cause**: structural, not surface ("no shared home for X exists",
   not "this file is big").
2. **Decision**, one of:
   - **Fix** (file on the improvement roadmap → executed later via
     `/simplify` or manual refactor)
   - **Defer** (must include a **re-evaluation trigger** — "what happening
     would make us start". A defer without a trigger is forbidden)
   - **Accept** (why the cost/benefit doesn't pay, consistent with past
     rejections such as the `dawn-proto` non-adoption or the P4-3 skip)
3. **State transition** since last review: resolved issues move to the
   "improvement roadmap > completed" table with date and description.

> Decisions must not contradict CLAUDE.md / AI_DEVELOPMENT_GUIDE.md policy
> (new-crate checklist, 8D minimization, anti-grind, etc.). Improvements that
> change behavior get "file an ADR" written down — this skill never writes
> the ADR itself.

## Phase 3 — Update the review document

Update the target `architecture-review-*.md` in place:
- Front-matter `date` → today plus a one-line summary
  (e.g. `2026-06-24 (filed R-2; main.gd 1255→1239 remeasured)`).
- Rewrite the grade table, file-size tables, issue list, and improvement
  roadmap (completed + deferred) from the Phase 0–2 results.
- Keep the existing style, table structure, verdict markers, and IDs — do not
  invent a new format.

---

## Guardrails (specific to this skill — mandatory)

- **Change zero lines of code.** Only `docs/architecture/architecture-review-*.md`
  may change. If remediation is needed, **file it** on the issue list /
  roadmap — nothing more.
- **Do not edit CLAUDE.md / AI_DEVELOPMENT_GUIDE.md / docs/adr/** (reading is
  fine).
- **Never break ID continuity.** Do not delete or renumber past `M-`/`L-`/
  `R-`/`P-`/`C-` entries.
- Line counts and verdicts come from **actual measurement** (`wc -l`), never
  from memory or the previous values.
- Every defer carries a trigger. Neither force a fix nor defer vaguely.

## Completion report

- Summarize: measurement changes (gone / new / drifted counts), grade moves,
  issue IDs filed and resolved.
- Docs-only change, so no test run needed. Show the diff scope with
  `git diff --stat docs/architecture/`.
- Ask the user before committing (English, Conventional Commits, e.g.
  `docs(architecture): refresh server review — recount sizes, file R-2`).
