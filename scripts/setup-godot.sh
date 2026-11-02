#!/usr/bin/env bash
# Python owns preparation; preserve the existing Bash entry point.
set -euo pipefail
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec "${PYTHON:-python3}" "$script_dir/godot_setup.py" "$@"
