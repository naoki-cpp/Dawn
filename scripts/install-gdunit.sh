#!/usr/bin/env bash
# install-gdunit.sh
#
# Restores the pinned GdUnit4 addon into client/addons/. The addon is ignored
# by Git so local AssetLib installs remain possible, while CI can use the same
# version on every runner.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$(tr -d '[:space:]' < "$repo_root/.gdunit4-version")"
ref="v$version"
client_dir="$repo_root/client"
gdunit_dir="${GDUNIT4_DIR:-$client_dir/addons/gdUnit4}"

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

archive="$tmp_dir/gdUnit4.tar.gz"
url="https://github.com/godot-gdunit-labs/gdUnit4/archive/refs/tags/$ref.tar.gz"

echo "Downloading GdUnit4 $version ..."
curl --fail --silent --show-error --location "$url" -o "$archive"
tar -xzf "$archive" -C "$tmp_dir"

plugin_file="$(find "$tmp_dir" -type f -path '*/addons/gdUnit4/plugin.cfg' -print -quit)"
if [ -z "$plugin_file" ]; then
	echo "Could not find addons/gdUnit4 in the GdUnit4 $ref archive." >&2
	exit 1
fi

actual_version="$(sed -n 's/^version="\([^"]*\)"/\1/p' "$plugin_file")"
if [ "$actual_version" != "$version" ]; then
	echo "GdUnit4 version mismatch: expected $version, found $actual_version" >&2
	exit 1
fi

source_dir="$(dirname "$plugin_file")"
mkdir -p "$(dirname "$gdunit_dir")"
rm -rf "$gdunit_dir"
cp -a "$source_dir" "$gdunit_dir"

echo "Installed GdUnit4 $version at $gdunit_dir"
