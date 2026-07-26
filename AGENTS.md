# AGENTS.md

This file delegates to the shared AI development guide for this repository.

Read and follow [AI_DEVELOPMENT_GUIDE.md](./AI_DEVELOPMENT_GUIDE.md) before making code changes.

Repo-specific skill adapters for this project live in `.agents/skills/`
(ai-change-checklist, doc-sync, architecture-review, remove-event, add-event,
new-adr, rust-api-audit, security-check). Their canonical project procedures live in
`.agents/commands/`.

`.claude/commands/` is kept only as a Claude Code compatibility layer, and
`.claude/settings.json` remains the Claude Code entry point for the shared
hooks. The hook implementations live in `.agents/hooks/`.

The upstream Matt Pocock engineering skills are pinned as a git submodule at
`.agents/vendor/mattpocock-skills/`. When a task invokes an upstream skill,
read the pinned source under
`.agents/vendor/mattpocock-skills/skills/engineering/<skill>/SKILL.md`.
Do not depend on a machine-local `~/.codex/skills` or `~/.claude/skills`
installation. See [docs/agents/skill-source.md](./docs/agents/skill-source.md)
for initialization and update instructions.
