#!/usr/bin/env bash
# setup-godot.sh
#
# Downloads the pinned Godot editor build (see .godot-version), prepares the
# local GdUnit4 addon for the pinned Godot version, and warms the project's
# import/script-class cache so CLI tests can run on a fresh checkout.
#
# Usage:
#   scripts/setup-godot.sh            # installs Godot and prepares GdUnit4
#   scripts/setup-godot.sh --run-tests # prepares the environment, then runs GdUnit4
#   scripts/setup-godot.sh --print    # prints the resolved binary path only
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$(tr -d '[:space:]' < "$repo_root/.godot-version")"
install_dir="$repo_root/.tools/godot/$version"
case "$(uname -s)" in
	Linux*)
		asset="Godot_v${version}_linux.x86_64.zip"
		exe_path="$install_dir/Godot_v${version}_linux.x86_64"
		;;
	Darwin*)
		asset="Godot_v${version}_macos.universal.zip"
		exe_path="$install_dir/Godot.app/Contents/MacOS/Godot"
		;;
	*)
		asset="Godot_v${version}_win64.exe.zip"
		exe_path="$install_dir/Godot_v${version}_win64.exe"
		;;
esac
client_dir="$repo_root/client"
gdunit_dir="$client_dir/addons/gdUnit4"
run_tests=0
skip_gdunit=0

for arg in "$@"; do
	case "$arg" in
		--print)
			echo "$exe_path"
			exit 0
			;;
		--run-tests)
			run_tests=1
			;;
		--skip-gdunit)
			skip_gdunit=1
			;;
		*)
			echo "Unknown argument: $arg" >&2
			exit 1
			;;
	esac
done

install_godot() {
	if [ -x "$exe_path" ]; then
		echo "Godot $version already installed: $exe_path"
		return
	fi

	mkdir -p "$install_dir"
	tmp_dir="$(mktemp -d)"
	trap 'rm -rf "$tmp_dir"' EXIT

	base_url="https://github.com/godotengine/godot/releases/download/$version"
	echo "Downloading $asset ($version) from godotengine/godot releases ..."
	curl --fail --silent --show-error --location "$base_url/$asset" -o "$tmp_dir/$asset"
	curl --fail --silent --show-error --location "$base_url/SHA512-SUMS.txt" -o "$tmp_dir/SHA512-SUMS.txt"

	expected_sum="$(grep " $asset\$" "$tmp_dir/SHA512-SUMS.txt" | awk '{print $1}')"
	if [ -z "$expected_sum" ]; then
		echo "Error: could not find a checksum for $asset in SHA512-SUMS.txt" >&2
		exit 1
	fi

	if command -v sha512sum >/dev/null 2>&1; then
		actual_sum="$(sha512sum "$tmp_dir/$asset" | awk '{print $1}')"
	elif command -v shasum >/dev/null 2>&1; then
		actual_sum="$(shasum -a 512 "$tmp_dir/$asset" | awk '{print $1}')"
	else
		echo "Error: neither sha512sum nor shasum is available" >&2
		exit 1
	fi
	if [ "$expected_sum" != "$actual_sum" ]; then
		echo "Error: SHA512 mismatch for $asset" >&2
		echo "  expected: $expected_sum" >&2
		echo "  actual:   $actual_sum" >&2
		exit 1
	fi

	echo "Checksum verified. Extracting ..."
	unzip -q -o "$tmp_dir/$asset" -d "$install_dir"

	echo "Installed: $exe_path"
}

replace_if_present() {
	local path="$1"
	local from="$2"
	local to="$3"
	if [ ! -f "$path" ]; then
		echo "Required GdUnit4 file is missing: $path" >&2
		exit 1
	fi
	if grep -Fq "$from" "$path"; then
		FROM="$from" TO="$to" perl -0pi -e 'BEGIN { $from = $ENV{"FROM"}; $to = $ENV{"TO"} } s/\Q$from\E/$to/g' "$path"
		echo "Patched: $path"
	fi
}

initialize_gdunit() {
	if [ "$skip_gdunit" -eq 1 ]; then
		return
	fi
	if [ ! -f "$gdunit_dir/runtest.sh" ]; then
		echo "GdUnit4 is not installed under client/addons/gdUnit4. Install it from Godot AssetLib, then rerun this script." >&2
		exit 1
	fi

	replace_if_present \
		"$gdunit_dir/src/core/GdUnitFileAccess.gd" \
		"return file.get_as_text(true)" \
		"return file.get_as_text()"
	replace_if_present \
		"$gdunit_dir/plugin.gd" \
		'ProjectSettings.get_setting("debug/gdscript/warnings/exclude_addons")' \
		'ProjectSettings.get_setting("debug/gdscript/warnings/exclude_addons", false)'
	replace_if_present \
		"$gdunit_dir/src/monitor/GodotGdErrorMonitor.gd" \
		$'var _eof: int\n' \
		$'var _eof: int = 0\n'
	replace_if_present \
		"$gdunit_dir/src/monitor/GodotGdErrorMonitor.gd" \
		$'func collect_full_logs() -> PackedStringArray:\n\tawait (Engine.get_main_loop() as SceneTree).process_frame\n\tawait (Engine.get_main_loop() as SceneTree).physics_frame\n\n\tvar file := FileAccess.open(_godot_log_file, FileAccess.READ)\n\tfile.seek(_eof)' \
		$'func collect_full_logs() -> PackedStringArray:\n\tawait (Engine.get_main_loop() as SceneTree).process_frame\n\tawait (Engine.get_main_loop() as SceneTree).physics_frame\n\n\tvar file := FileAccess.open(_godot_log_file, FileAccess.READ)\n\tif file == null:\n\t\treturn PackedStringArray()\n\tfile.seek(_eof)'
	replace_if_present \
		"$gdunit_dir/src/monitor/GodotGdErrorMonitor.gd" \
		$'func _collect_log_entries(force_collect_reports: bool) -> Array[ErrorLogEntry]:\n\tvar file := FileAccess.open(_godot_log_file, FileAccess.READ)\n\tfile.seek(_eof)' \
		$'func _collect_log_entries(force_collect_reports: bool) -> Array[ErrorLogEntry]:\n\tvar file := FileAccess.open(_godot_log_file, FileAccess.READ)\n\tif file == null:\n\t\treturn []\n\tfile.seek(_eof)'

	mkdir -p "$client_dir/.godot-test-logs"

	replace_if_present \
		"$gdunit_dir/runtest.cmd" \
		'"!godot_binary!" --path . -s -d res://addons/gdUnit4/bin/GdUnitCmdTool.gd !filtered_args!' \
		'"!godot_binary!" --log-file .godot-test-logs\gdunit.log --path . -s -d res://addons/gdUnit4/bin/GdUnitCmdTool.gd !filtered_args!'
	replace_if_present \
		"$gdunit_dir/runtest.cmd" \
		'"!godot_binary!" --headless --path . --quiet -s res://addons/gdUnit4/bin/GdUnitCopyLog.gd !filtered_args! > nul' \
		'"!godot_binary!" --headless --log-file .godot-test-logs\gdunit-copy.log --path . --quiet -s res://addons/gdUnit4/bin/GdUnitCopyLog.gd !filtered_args! > nul'
	replace_if_present \
		"$gdunit_dir/runtest.sh" \
		'"$godot_binary" --path . -s -d res://addons/gdUnit4/bin/GdUnitCmdTool.gd $filtered_args' \
		'"$godot_binary" --log-file .godot-test-logs/gdunit.log --path . -s -d res://addons/gdUnit4/bin/GdUnitCmdTool.gd $filtered_args'
	replace_if_present \
		"$gdunit_dir/runtest.sh" \
		'"$godot_binary" --headless --path . --quiet -s res://addons/gdUnit4/bin/GdUnitCopyLog.gd $filtered_args > /dev/null' \
		'"$godot_binary" --headless --log-file .godot-test-logs/gdunit-copy.log --path . --quiet -s res://addons/gdUnit4/bin/GdUnitCopyLog.gd $filtered_args > /dev/null'

	echo "Importing Godot project and warming script-class cache ..."
	"$exe_path" --headless --editor --quit-after 3 --path "$client_dir"

	if [ "$run_tests" -eq 1 ]; then
		echo "Running GdUnit4 tests ..."
		(
			cd "$client_dir"
			bash addons/gdUnit4/runtest.sh --godot_binary "$exe_path" -a test
		)
	fi
}

install_godot
initialize_gdunit

echo "Godot test environment is ready."
echo "Godot binary: $exe_path"
echo "Run tests with:"
echo "  scripts/setup-godot.sh --run-tests"
