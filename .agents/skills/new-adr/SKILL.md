---
name: new-adr
description: Scaffold a new Architecture Decision Record for Dawn with the house front-matter format and register it in docs/adr/README.md. Use when told to write/create/file an ADR, or when a change to invariants, crate boundaries, event schemas, tick order, or AI steering files needs to be recorded.
---

# new-adr — Create an ADR and register it in the index

Takes a short topic description as input (e.g. "market orders and escrow").

This skill scaffolds a new Architecture Decision Record with the
repository's established format and wires it into the index so it cannot be
orphaned.

**When an ADR is required** (from AI_DEVELOPMENT_GUIDE.md): any change to
invariants, crate responsibilities, event schemas, tick order, or AI steering
files -- and any new crate. Get human approval on the decision itself before
writing code; the ADR records that decision.

---

## Steps

### Step 1: Take the next number

```bash
ls docs/adr/ | sort
```

The new ADR takes the highest existing number + 1, zero-padded to four
digits. Numbers are chronological by decision -- categories live only in the
index, never in the numbering. Never renumber or rename existing ADRs (links
from AI_DEVELOPMENT_GUIDE.md and other docs depend on stable filenames).

Filename: `ADR-XXXX-<kebab-case-topic>.md`.

### Step 2: Write the ADR

Front matter follows the house format (see ADR-0036 as a model):

```
---
id      : ADR-XXXX
title   : <English or short bilingual title>
status  : proposed | accepted | deferred
date    : <today, YYYY-MM-DD>
deciders: [human, ai-agent]
related : ADR-YYYY (one-line why it relates), ...
---
```

Body sections (Japanese prose is fine -- repo convention allows Japanese for
design notes; code snippets and identifiers stay English):

- `# ADR-XXXX — <title>`
- Background section -- the forces: what exists today, what breaks or is
  missing, and which roadmap phase / prior ADR sets this up. Cite files and
  ADRs.
- Decision section -- the decision, with the key type/API sketches. State
  the alternatives considered and **why they lost** (one paragraph each is
  enough).
- Implementation checklist section -- concrete `[ ]` items that the doc-sync
  skill can later verify against the code. Every checklist item should be
  observable in the codebase (a type exists, a test exists, a doc updated).
- Optional: consequences / rejected options / open questions sections when
  the decision warrants them.

Ground rules:
- One decision per ADR. Two decisions = two ADRs.
- The decision must not violate an Architecture Invariant or a Forbidden
  Change (FBD-001..009) without explicitly superseding it -- and superseding
  one is itself the decision to record.
- `status: accepted` only after the human has approved; scaffold as
  `proposed` when the discussion is still open.

### Step 3: Register in the index

Edit `docs/adr/README.md`:
- add a row `| [ADR-XXXX](ADR-XXXX-....md) | <title> | <Status> |` to the
  matching category table (Architecture / Client-Communication / Movement /
  Combat / Economy / UI -- add a new category section only if none fits)

### Step 4: Cross-link

- If the ADR supersedes or amends another ADR, note it in **both** files'
  `related` lines.
- If the ADR belongs to a roadmap phase, reference it from
  `docs/process/roadmap.md` where that phase is described.
- If the ADR changes an invariant or forbidden change, update
  AI_DEVELOPMENT_GUIDE.md and `docs/architecture/forbidden-changes.md` in the
  same PR (this needs explicit human approval).

### Step 5: Commit

Docs-only commit per the convention:

```
docs(adr): add ADR-XXXX <short topic>
```

If the ADR and its implementation land together, the ADR still gets listed
in the PR description under "changed/referenced ADRs".
