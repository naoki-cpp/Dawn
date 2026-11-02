#!/usr/bin/env bash
# Compatibility entry point; the shared preparer owns the pinned addon install.
set -euo pipefail
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec "${PYTHON:-python3}" "$script_dir/godot_setup.py" --install-addon-only "$@"
