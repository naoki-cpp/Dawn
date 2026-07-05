---
name: architecture-review
description: Periodic senior-architect-style health review of Dawn's Rust crates and Godot client. Re-measures file sizes, re-grades health (green/yellow/red), files/updates issue IDs with root cause and decision, and updates architecture-review-server.md / architecture-review-client.md in place. Docs-only -- never changes code. Use for periodic maintenance review, not for fixing code.
---

# architecture-review — Periodic maintenance review from a senior-architect viewpoint

This skill re-measures and re-evaluates
`docs/architecture/architecture-review-server.md` (Rust crates) and
`docs/architecture/architecture-review-client.md` (Godot client) against the
latest codebase, **updating those files in place**.

> **This is an analysis skill. It changes no code.**
> Fixes happen later, in separate PRs, via the roadmap items this review
> files (through manual refactoring or a simplify pass). The separation is
> deliberate: the review diff stays docs-only, so review and remediation can
> be separate PRs.

Arguments (optional, pass as free text): `server` = server side only,
`client` = client side only, omitted = both.

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
- In the table but the file is gone -> remove the row (track renames/merges).
- On disk but not in the table (new file) -> add a row with a verdict.
- Line count drifted -> update to the measured value. **This is the most
  common staleness** -- counts creep up silently as features land.

## Phase 1 — Re-evaluate health (green / yellow / red)

Re-grade every file. The verdict weighs **cohesion of responsibility first,
size second** -- not a strict line cutoff:

- **Green — healthy**: single cohesive responsibility. Small, or large but
  "splitting would be unnatural" (a single geometry kernel, a state machine).
- **Yellow — watch**: growing (~500+ lines as a guide), responsibilities
  starting to mix, or a split candidate where "not yet" is the right call.
  Record the reason and the trigger for when to split.
- **Red — act**: clearly past the threshold (~700+ lines as a guide) AND
  multiple responsibilities cohabiting. File it on the improvement roadmap in
  Phase 2.

Also refresh the overall grade table -- server axes: crate structure / file
size / type design / duplication / Rust-specific / AI-development-induced;
client axes: file split / responsibility cohesion / duplication / coupling /
test coverage.

### Grade scale (A–F with ±)

Grades measure **the amount of unmanaged debt (no decision, no trigger)** --
not the mere existence of debt. Debt with a documented acceptance decision or
re-evaluation trigger does not lower the grade (it is managed).

| Grade | Meaning |
|---|---|
| **A** | As designed. Zero unmanaged debt. |
| **A−** | Near-ideal. Only minor debt accepted with a decision and trigger. |
| **B+** | Good, but several items under watch (yellow) or large files mid-split. No harm yet. |
| **B** | Works, but structural debt is visible. Direction set, journey incomplete. |
| **B−** | Multiple visible debts coexist. Remediation started but unfinished. |
| **C** | Clear need for improvement: naming pollution, large duplication, an unsplit god object, or red files. Needs roadmap entries. |
| **D / F** | Design is broken (DAG inversion, everything in one file, no tests). Not expected here, but defined as the floor. |

`±` within a band: top of the band -> `+`, one foot in the next band down ->
`−`. **The overall grade is dragged by the weakest axis** (bottleneck axis
±1, not an average).

Justify each grade with concrete file names, issue IDs, and line counts as of
this review. Keep the scale table itself free of file names/IDs.

Evaluation axes used by past findings:
- **Crate DAG / dependency direction**: consistent with architecture.md's DAG?
- **Duplication**: copied logic of the same shape (two-binary glue,
  event<->JSON conversion, etc.).
- **Rust-specific**: needless `Box<dyn>` / `Mutex`, `inner()` escapes, large
  enum size asymmetry (`Box` candidates).
- **AI-development-induced**: naming pollution, excess thin wrappers, ad-hoc
  specialization.
- **Tests** (client): is scene-tree/network-free pure logic tested?

## Phase 2 — Issue inventory (IDs, root cause, decision, re-evaluation trigger)

Carry the existing issue lists forward (server: `M-`/`L-`/`R-`/`P-`; client:
`C-`). **Preserve the ID scheme and numbering continuity** -- new issues take
the next number.

Every issue must carry three things:
1. **Root cause**: structural, not surface.
2. **Decision**, one of:
   - **Fix** (file on the improvement roadmap for later execution)
   - **Defer** (must include a **re-evaluation trigger** -- a defer without a
     trigger is forbidden)
   - **Accept** (why the cost/benefit doesn't pay, consistent with past
     rejections)
3. **State transition** since last review: resolved issues move to the
   "improvement roadmap > completed" table with date and description.

> Decisions must not contradict AGENTS.md / AI_DEVELOPMENT_GUIDE.md policy.
> Improvements that change behavior get "file an ADR" written down -- this
> skill never writes the ADR itself.

## Phase 3 — Update the review document

Update the target `architecture-review-*.md` in place:
- Front-matter `date` -> today plus a one-line summary.
- Rewrite the grade table, file-size tables, issue list, and improvement
  roadmap (completed + deferred) from the Phase 0-2 results.
- Keep the existing style, table structure, verdict markers, and IDs -- do
  not invent a new format.

---

## Guardrails (mandatory)

- **Change zero lines of code.** Only `docs/architecture/architecture-review-*.md`
  may change. If remediation is needed, **file it** -- nothing more.
- **Do not edit AGENTS.md / AI_DEVELOPMENT_GUIDE.md / docs/adr/** (reading is
  fine).
- **Never break ID continuity.**
- Line counts and verdicts come from **actual measurement** (`wc -l`), never
  memory or previous values.
- Every defer carries a trigger.

## Completion report

- Summarize: measurement changes (gone / new / drifted counts), grade moves,
  issue IDs filed and resolved.
- Docs-only change, so no test run needed. Show the diff scope with
  `git diff --stat docs/architecture/`.
- Ask the user before committing (English, Conventional Commits, e.g.
  `docs(architecture): refresh server review -- recount sizes, file R-2`).
