---
title: Agent skill source
audience: AI agents and contributors
---

# Agent Skill Source

Dawn keeps the upstream Matt Pocock engineering skills as a pinned git
submodule. This makes the skill source reproducible across workspaces and
avoids depending on a user's global Codex or Claude installation.

## Source Layout

| Kind | Source | Ownership |
|---|---|---|
| Upstream engineering skill | `.agents/vendor/mattpocock-skills/skills/engineering/<skill>/SKILL.md` | Matt Pocock's submodule; do not edit in Dawn |
| Dawn-specific adapter | `.agents/skills/<skill>/SKILL.md` | Dawn; frontmatter and discovery metadata |
| Dawn-specific procedure | `.agents/commands/<skill>.md` | Dawn; project workflow and invariants |
| Claude Code compatibility shim | `.claude/commands/<skill>.md` | Dawn; forwards to the `.agents/commands/` procedure |
| Claude Code settings | `.claude/settings.json` | Dawn; auto-discovered settings that point at `.agents/hooks/` |

The submodule is pinned by the gitlink recorded in the superproject. A normal
checkout therefore uses the same upstream revision as every other workspace.

## Initialize

For a fresh clone:

```text
git clone --recurse-submodules <repository-url>
```

For an existing clone:

```text
git submodule update --init --recursive
```

If the submodule directory is present but empty, the second command is the
repair step. No symlink, global agent install, or machine-specific path is
required.

## Update Deliberately

Upstream updates are repository changes, not implicit workstation updates:

```text
git -C .agents/vendor/mattpocock-skills fetch --depth=1 origin main
git -C .agents/vendor/mattpocock-skills checkout FETCH_HEAD
git add .agents/vendor/mattpocock-skills
git commit -m "chore(skills): update Matt Pocock skill source"
```

Review the upstream diff before committing the gitlink update. Project
adapters and procedures remain in Dawn so local domain decisions are not
overwritten when the upstream skill set changes.

## Selection Rule

Use the pinned upstream source for general engineering skills such as
`improve-codebase-architecture`, `codebase-design`, `diagnosing-bugs`,
`grilling`, and `setup-matt-pocock-skills`. Use the Dawn adapter and
`.agents/commands/` procedure for Dawn-specific workflows such as `doc-sync`,
`architecture-review`, `add-event`, and `new-adr`. Claude Code users reach the
same procedures through the thin `.claude/commands/` shims.
