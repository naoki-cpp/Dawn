"""Pinned Godot test preparation shared by local shells and CI (Python 3.10+)."""

import argparse
import configparser
import hashlib
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import tempfile
import urllib.request
import zipfile


ROOT = Path(__file__).resolve().parent.parent
GODOT_MCP_VERSION = "2026.10.20.1"


def patch_text(text, old, new):
    text = text.replace("\r\n", "\n")
    if text.count(new) == 1 and old not in text:
        return text
    if text.count(old) != 1 or new in text:
        raise RuntimeError(f"Unrecognized GdUnit4 patch state: {old[:90]!r}")
    return text.replace(old, new)


def compatibility_patches():
    # GdUnit4 6.1.3 already fixes the old FileAccess/plugin/monitor issues.
    patches = []
    for name, binary, args, redirect, separator in [
        ("runtest.cmd", '"!godot_binary!"', "!filtered_args!", " > nul", "\\"),
        ("runtest.sh", '"$godot_binary"', "$filtered_args", " > /dev/null", "/"),
    ]:
        for headless, script, suffix, log in [
            ("", "-s -d --remote-debug tcp://127.0.0.1:0 res://addons/gdUnit4/bin/GdUnitCmdTool.gd", "", "gdunit.log"),
            (" --headless", "--quiet -s res://addons/gdUnit4/bin/GdUnitCopyLog.gd", redirect, "gdunit-copy.log"),
        ]:
            tail = f" --path . {script} {args}{suffix}"
            patched_tail = tail.replace(" --remote-debug tcp://127.0.0.1:0", "")
            patches.append((name, binary + headless + tail,
                            binary + headless + f" --log-file .godot-test-logs{separator}{log}" + patched_tail))
    return patches


def patch_addon(directory):
    # Validate every patch before modifying any addon file.
    changes = {}
    for relative, old, new in compatibility_patches():
        path = directory / relative
        text = changes.get(path)
        if text is None:
            text = path.read_text(encoding="utf-8-sig")
        changes[path] = patch_text(text, old, new)
    for path, text in changes.items():
        if path.read_text(encoding="utf-8-sig") != text:
            path.write_text(text, encoding="utf-8", newline="")


def download(url, destination):
    print(f"Downloading {url}", flush=True)
    with urllib.request.urlopen(url, timeout=180) as response, destination.open("wb") as output:
        shutil.copyfileobj(response, output)


def extract_zip(archive, destination):
    with zipfile.ZipFile(archive) as source:
        for member in source.infolist():
            target = (destination / member.filename).resolve()
            if not target.is_relative_to(destination.resolve()):
                raise RuntimeError("Archive contains an unsafe path")
        source.extractall(destination)
        if os.name != "nt":
            for member in source.infolist():
                mode = (member.external_attr >> 16) & 0o777
                if mode:
                    (destination / member.filename).chmod(mode)


def godot_location(root, system=None):
    version = (root / ".godot-version").read_text().strip()
    system = system or platform.system()
    names = {
        "Windows": ("win64.exe.zip", f"Godot_v{version}_win64_console.exe"),
        "Linux": ("linux.x86_64.zip", f"Godot_v{version}_linux.x86_64"),
        "Darwin": ("macos.universal.zip", "Godot.app/Contents/MacOS/Godot"),
    }
    if system not in names:
        raise RuntimeError(f"Unsupported platform: {system}")
    if system != "Darwin" and platform.machine().lower() not in ("amd64", "x86_64"):
        raise RuntimeError("Godot setup currently supports x86_64 on Windows/Linux")
    suffix, executable = names[system]
    return version, f"Godot_v{version}_{suffix}", root / ".tools/godot" / version / executable


def install_godot(root):
    version, asset, executable = godot_location(root)
    if not executable.is_file():
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            base = f"https://github.com/godotengine/godot/releases/download/{version}"
            download(f"{base}/{asset}", directory / asset)
            download(f"{base}/SHA512-SUMS.txt", directory / "sums")
            checksums = [line.split()[0].lower() for line in (directory / "sums").read_text().splitlines()
                         if len(line.split()) == 2 and line.split()[1].lstrip("*") == asset]
            with (directory / asset).open("rb") as source:
                digest = hashlib.sha512()
                for block in iter(lambda: source.read(1024 * 1024), b""):
                    digest.update(block)
            if checksums != [digest.hexdigest()]:
                raise RuntimeError(f"SHA512 mismatch or missing checksum for {asset}")
            extract_zip(directory / asset, root / ".tools/godot" / version)
    actual = subprocess.run([str(executable), "--version"], check=True, capture_output=True, text=True).stdout.strip()
    if not actual.startswith(version.replace("-", ".") + "."):
        raise RuntimeError(f"Godot version mismatch: expected {version}, found {actual}")
    return executable


def addon_version(directory):
    parser = configparser.ConfigParser()
    parser.read(directory / "plugin.cfg", encoding="utf-8-sig")
    return parser.get("plugin", "version", fallback="").strip('"')


def install_addon(root, directory):
    version = (root / ".gdunit4-version").read_text().strip()
    if addon_version(directory) == version:
        return
    if directory.exists():
        raise RuntimeError(f"GdUnit4 version mismatch at {directory}; move the existing addon aside and rerun")
    with tempfile.TemporaryDirectory() as temporary:
        staging = Path(temporary)
        archive = staging / "addon.zip"
        download(f"https://github.com/godot-gdunit-labs/gdUnit4/archive/refs/tags/v{version}.zip", archive)
        extract_zip(archive, staging / "unpacked")
        candidates = list((staging / "unpacked").glob("*/addons/gdUnit4"))
        if len(candidates) != 1 or addon_version(candidates[0]) != version:
            raise RuntimeError("Downloaded GdUnit4 archive does not match the pinned version")
        directory.parent.mkdir(parents=True, exist_ok=True)
        shutil.copytree(candidates[0], directory)


def install_godot_mcp_addon(directory):
    if addon_version(directory) == GODOT_MCP_VERSION:
        return
    if directory.exists():
        raise RuntimeError(f"Godot MCP addon version mismatch at {directory}; move the existing addon aside and rerun")
    with tempfile.TemporaryDirectory() as temporary:
        staging = Path(temporary)
        archive = staging / "godot_mcp_addon.zip"
        url = (
            "https://github.com/hybridindie/godot-mcp/releases/download/"
            f"{GODOT_MCP_VERSION}/godot_mcp_addon.zip"
        )
        download(url, archive)
        extract_zip(archive, staging / "unpacked")
        candidate = staging / "unpacked/addons/godot_mcp"
        if not candidate.is_dir() or addon_version(candidate) != GODOT_MCP_VERSION:
            raise RuntimeError("Downloaded Godot MCP addon does not match the pinned version")
        directory.parent.mkdir(parents=True, exist_ok=True)
        shutil.copytree(candidate, directory)


def prepare(root, executable, run=subprocess.run):
    install_godot_mcp_addon(root / "client/addons/godot_mcp")
    addon = root / "client/addons/gdUnit4"
    install_addon(root, addon)
    patch_addon(addon)
    run(["cargo", "build", "-p", "dawn-client-gdext"], cwd=root, check=True)
    logs = root / "client/.godot-test-logs"
    logs.mkdir(parents=True, exist_ok=True)
    log = logs / "import.log"
    log.unlink(missing_ok=True)
    run([str(executable), "--headless", "--editor", "--import", "--path", str(root / "client"),
         "--log-file", str(log)], cwd=root, check=True)
    # Godot may report script/import failures while returning exit status zero.
    text = log.read_text(encoding="utf-8", errors="replace")
    if re.search(r"(?:SCRIPT ERROR:|ERROR:|Parse Error:)", text):
        raise RuntimeError(f"Godot import reported errors; inspect {log}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--print", action="store_true", dest="print_path")
    parser.add_argument("--run-tests", action="store_true")
    parser.add_argument("--skip-gdunit", action="store_true", help="Install/validate Godot only; not test readiness")
    parser.add_argument("--install-addon-only", action="store_true")
    args = parser.parse_args()
    if args.print_path:
        print(godot_location(ROOT)[2])
        return
    if args.run_tests and (args.skip_gdunit or args.install_addon_only):
        parser.error("--run-tests requires complete preparation")
    if args.install_addon_only:
        install_addon(ROOT, Path(os.environ.get("GDUNIT4_DIR", ROOT / "client/addons/gdUnit4")).resolve())
        return
    executable = install_godot(ROOT)
    install_godot_mcp_addon(ROOT / "client/addons/godot_mcp")
    if args.skip_gdunit:
        print(f"Godot installed: {executable} (test preparation skipped)")
        return
    prepare(ROOT, executable)
    print(f"Godot test environment is ready: {executable}", flush=True)
    if args.run_tests:
        runner = ROOT / "client/addons/gdUnit4" / ("runtest.cmd" if os.name == "nt" else "runtest.sh")
        command = [str(runner)] if os.name == "nt" else ["bash", str(runner)]
        subprocess.run(command + ["--godot_binary", str(executable), "-a", "test"], cwd=ROOT / "client", check=True)


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, configparser.Error, zipfile.BadZipFile, subprocess.CalledProcessError) as error:
        raise SystemExit(f"Godot preparation failed: {error}") from error
