#!/usr/bin/env bash
# setup-godot.sh
#
# Downloads the pinned Godot editor build (see .godot-version) from the
# official godotengine/godot GitHub releases into .tools/godot/<version>/,
# verifying it against the release's published SHA512 checksum. This keeps
# the Godot binary out of git (like a uv/pyenv-managed toolchain) while
# pinning an exact, reproducible version across machines.
#
# Usage:
#   scripts/setup-godot.sh            # installs the pinned version if missing
#   scripts/setup-godot.sh --print    # prints the resolved binary path only
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$(tr -d '[:space:]' < "$repo_root/.godot-version")"
install_dir="$repo_root/.tools/godot/$version"
asset="Godot_v${version}_win64.exe.zip"
exe_path="$install_dir/Godot_v${version}_win64.exe"

if [ "${1:-}" = "--print" ]; then
	echo "$exe_path"
	exit 0
fi

if [ -x "$exe_path" ]; then
	echo "Godot $version already installed: $exe_path"
	exit 0
fi

mkdir -p "$install_dir"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

base_url="https://github.com/godotengine/godot/releases/download/$version"
echo "Downloading $asset ($version) from godotengine/godot releases ..."
curl -sL "$base_url/$asset" -o "$tmp_dir/$asset"
curl -sL "$base_url/SHA512-SUMS.txt" -o "$tmp_dir/SHA512-SUMS.txt"

expected_sum="$(grep " $asset\$" "$tmp_dir/SHA512-SUMS.txt" | awk '{print $1}')"
if [ -z "$expected_sum" ]; then
	echo "Error: could not find a checksum for $asset in SHA512-SUMS.txt" >&2
	exit 1
fi

actual_sum="$(sha512sum "$tmp_dir/$asset" | awk '{print $1}')"
if [ "$expected_sum" != "$actual_sum" ]; then
	echo "Error: SHA512 mismatch for $asset" >&2
	echo "  expected: $expected_sum" >&2
	echo "  actual:   $actual_sum" >&2
	exit 1
fi

echo "Checksum verified. Extracting ..."
unzip -q -o "$tmp_dir/$asset" -d "$install_dir"

echo "Installed: $exe_path"
echo "Set GODOT_BIN to use it, e.g.:"
echo "  export GODOT_BIN=\"$exe_path\""
