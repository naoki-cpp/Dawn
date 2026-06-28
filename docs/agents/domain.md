# Domain Docs

How the engineering skills should consume this repo's domain documentation when exploring the codebase.

## Before exploring, read these

- **`CONTEXT.md`** at the repo root.
- **`docs/adr/`**; read ADRs that touch the area you're about to work in.

If `CONTEXT.md` does not exist, proceed silently. Don't flag its absence or suggest creating it upfront. The `domain-modeling` skill, reached via `grill-with-docs` and `improve-codebase-architecture`, creates it lazily when terms or decisions actually get resolved.

## File structure

Single-context repo:

```text
/
|-- CONTEXT.md
|-- docs/adr/
|   |-- ADR-0001-event-sourcing.md
|   `-- ADR-0002-actor-model.md
`-- src/
```

## Use the glossary's vocabulary

When your output names a domain concept, such as in an issue title, refactor proposal, hypothesis, or test name, use the term as defined in `CONTEXT.md`. Don't drift to synonyms the glossary explicitly avoids.

If the concept you need isn't in the glossary yet, either reconsider whether you're inventing language the project doesn't use, or note the gap for `domain-modeling`.

## Flag ADR conflicts

If your output contradicts an existing ADR, surface it explicitly rather than silently overriding.
