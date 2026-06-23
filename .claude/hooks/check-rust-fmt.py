#!/usr/bin/env python3
"""PostToolUse hook: warn when an edited Rust file isn't rustfmt-clean.

Fires after Edit/Write. Runs `rustfmt --check` on just the file that changed
(not the whole workspace, so it stays fast enough for an every-edit hook) and,
if it would reformat, nudges Claude to run formatting. This is advisory: it
never edits the file and never blocks -- it only reports, so a formatting nit
can't wedge the session. The workspace was brought to a clean `cargo fmt --all`
baseline on 2026-06-23, so this should normally stay quiet; if it fires, the
edit drifted from rustfmt's formatting and should be re-run through `cargo fmt`.

Contract: read the tool-call JSON on stdin. On a non-clean .rs file, print a
note to stderr and exit 2 (PostToolUse surfaces stderr back to Claude).
Otherwise exit 0.
"""
import json
import subprocess
import sys


def main() -> int:
    try:
        payload = json.load(sys.stdin)
    except Exception:
        return 0

    path = payload.get("tool_input", {}).get("file_path", "") or ""
    if not path.endswith(".rs"):
        return 0

    try:
        # Discard output: we only need the exit code, and rustfmt echoes the
        # file (which may contain non-ASCII comments) -- decoding that as text on
        # a cp932 Windows console would crash the capture thread.
        result = subprocess.run(
            ["rustfmt", "--edition", "2021", "--check", path],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    except FileNotFoundError:
        # rustfmt not installed -- silently skip rather than nag.
        return 0

    if result.returncode != 0:
        sys.stderr.write(
            f"Note: {path} is not rustfmt-clean. Run `cargo fmt` before committing "
            "(AI_DEVELOPMENT_GUIDE.md §8).\n"
        )
        return 2

    return 0


if __name__ == "__main__":
    sys.exit(main())
