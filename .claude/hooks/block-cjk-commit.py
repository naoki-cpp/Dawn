#!/usr/bin/env python3
"""PreToolUse hook: block `git commit` when the message contains CJK text.

AI_DEVELOPMENT_GUIDE.md §8 / docs/commit-convention.md require commit messages
to be written in English (Conventional Commits). This enforces that
deterministically at the tool boundary instead of relying on the model
remembering the rule -- per the Anthropic "steering Claude Code" guidance that
hard "never do X" constraints belong in a PreToolUse hook, not in always-on
context.

Contract: read the tool-call JSON on stdin. If the command is a `git commit`
that carries CJK characters, print an explanation to stderr and exit 2, which
tells Claude Code to block the call and surface the message. Otherwise exit 0.
"""
import json
import re
import sys

# Hiragana, Katakana, CJK Unified Ideographs, and fullwidth forms.
CJK = re.compile(r"[぀-ヿ㐀-鿿＀-￯]")


def main() -> int:
    try:
        payload = json.load(sys.stdin)
    except Exception:
        # Never block on a parse failure -- fail open so we don't wedge commits.
        return 0

    command = payload.get("tool_input", {}).get("command", "") or ""

    # Only police actual commits; `git log`, `git commit-graph`, etc. are fine.
    if not re.search(r"\bgit\s+commit\b", command):
        return 0

    if CJK.search(command):
        sys.stderr.write(
            "Blocked: this git commit message contains Japanese/CJK text.\n"
            "Commit messages must be in English (Conventional Commits) -- see "
            "docs/process/commit-convention.md and AI_DEVELOPMENT_GUIDE.md "
            "'Comments And Commits'.\n"
            "Rewrite the message in English and commit again.\n"
        )
        return 2

    return 0


if __name__ == "__main__":
    sys.exit(main())
