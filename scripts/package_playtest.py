"""Build an isolated Windows playtest ZIP from committed client resources."""

import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import zipfile

from godot_setup import ROOT, download, install_godot


def validate_version(value):
    if not re.fullmatch(r"playtest-[0-9][A-Za-z0-9.-]{0,60}", value):
        raise ValueError("Version must be playtest- followed by a number and letters/dots/hyphens")
    return value


def stage_client(root, destination):
    # Only committed runtime resources enter the package, never local secrets/caches.
    archive = subprocess.run(
        ["git", "archive", "--format=zip", "HEAD", "client"],
        cwd=root, check=True, capture_output=True,
    ).stdout
    with zipfile.ZipFile(io.BytesIO(archive)) as source:
        for member in source.infolist():
            path = Path(member.filename)
            if member.is_dir() or path.parts[1:2] == ("test",):
                continue
            if path.parts[:3] == ("client", "addons", "gdUnit4"):
                continue
            target = destination / path.relative_to("client")
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(source.read(member))
    project = destination / "project.godot"
    text = project.read_text(encoding="utf-8")
    text = re.sub(r"(?m)^enabled=PackedStringArray\(.*\)$", "enabled=PackedStringArray()", text)
    project.write_text(text, encoding="utf-8")
    (destination / "dawn_client_gdext.gdextension").write_text(
        '[configuration]\nentry_symbol="gdext_rust_init"\ncompatibility_minimum="4.6"\n'
        '[libraries]\nwindows.release.x86_64="res://bin/dawn_client_gdext.dll"\n'
        'windows.debug.x86_64="res://bin/dawn_client_gdext.dll"\n', encoding="utf-8",
    )
    shutil.copy2(root / "client/export_presets.cfg", destination / "export_presets.cfg")


def windows_template(root):
    version = (root / ".godot-version").read_text().strip()
    template = root / ".tools/godot" / version / "windows_release_x86_64.exe"
    if not template.exists():
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            name = f"Godot_v{version}_export_templates.tpz"
            base = f"https://github.com/godotengine/godot/releases/download/{version}"
            download(f"{base}/{name}", directory / name)
            download(f"{base}/SHA512-SUMS.txt", directory / "sums")
            expected = [line.split()[0].lower() for line in (directory / "sums").read_text().splitlines()
                        if len(line.split()) == 2 and line.split()[1].lstrip("*") == name]
            with (directory / name).open("rb") as source:
                digest = hashlib.file_digest(source, "sha512").hexdigest()
            if expected != [digest]:
                raise RuntimeError("Godot export template checksum mismatch")
            with zipfile.ZipFile(directory / name) as source:
                with template.open("wb") as output:
                    output.write(source.read("templates/windows_release_x86_64.exe"))
    return template


def write_notices(root, output):
    metadata = json.loads(subprocess.run(
        ["cargo", "metadata", "--locked", "--format-version=1", "--filter-platform", "x86_64-pc-windows-msvc"], cwd=root,
        check=True, capture_output=True, text=True,
    ).stdout)
    nodes = {node["id"]: node["dependencies"] for node in metadata["resolve"]["nodes"]}
    pending = [p["id"] for p in metadata["packages"] if p["name"] == "dawn-client-gdext"]
    dependencies = set()
    while pending:
        identity = pending.pop()
        if identity not in dependencies:
            dependencies.add(identity)
            pending.extend(nodes[identity])
    notices = [(root / "THIRD-PARTY-LICENSES.md").read_text(encoding="utf-8")]
    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        if package["id"] not in dependencies or not package["source"]:
            continue
        notices.append(f'\n## {package["name"]} {package["version"]} ({package["license"]})\n'
                       f'Source: https://crates.io/crates/{package["name"]}/{package["version"]}\n')
        directory = Path(package["manifest_path"]).parent
        licenses = [p for p in directory.iterdir() if p.is_file()
                    and p.name.lower().startswith(("license", "licence", "copying", "notice", "copyright"))]
        if not licenses and package["license"] == "MPL-2.0" and package["repository"] == "https://github.com/godot-rust/gdext":
            # The published gdext crates omit License.txt; use the pinned upstream text.
            path = output / "GODOT-RUST-License.txt"
            if not path.exists():
                version = next(p["version"] for p in metadata["packages"] if p["name"] == "godot")
                download(f"https://raw.githubusercontent.com/godot-rust/gdext/v{version}/License.txt", path)
            licenses = [path]
        if not licenses and package["name"] == "nanoserde-derive" and package["license"] == "MIT":
            sibling = next(p for p in metadata["packages"]
                           if p["name"] == "nanoserde" and p["version"] == package["version"])
            licenses = [Path(sibling["manifest_path"]).parent / "LICENSE-MIT"]
        if not licenses:
            raise RuntimeError(f"No license text found for {package['name']}")
        notices.extend(p.read_text(encoding="utf-8", errors="replace") for p in sorted(licenses))
    notices.append((root / "client/addons/lens_effects/LICENSE").read_text(encoding="utf-8"))
    version = (root / ".godot-version").read_text().strip()
    for name in ("LICENSE.txt", "COPYRIGHT.txt"):
        path = output / ("GODOT-" + name)
        download(f"https://raw.githubusercontent.com/godotengine/godot/{version}/{name}", path)
    (output / "THIRD-PARTY-NOTICES.txt").write_text("\n".join(notices), encoding="utf-8")


def archive_package(output, version, commit):
    (output / "BUILD.txt").write_text(f"{version}\nCommit: {commit}\n", encoding="utf-8")
    archive = Path(shutil.make_archive(str(output) + "-windows-x86_64", "zip", output))
    with archive.open("rb") as source:
        checksum = hashlib.file_digest(source, "sha256").hexdigest()
    archive.with_suffix(".zip.sha256").write_text(f"{checksum}  {archive.name}\n", encoding="utf-8")
    print(f"Playtest package: {archive}")
    return archive


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True, type=validate_version)
    args = parser.parse_args()
    if os.name != "nt":
        parser.error("Build on Windows x86_64 (including GitHub Actions windows-latest)")
    subprocess.run(["git", "diff", "--quiet", "HEAD", "--", "Cargo.toml", "Cargo.lock",
                    "rust-toolchain.toml", "crates"], cwd=ROOT, check=True)
    godot = install_godot(ROOT)
    template = windows_template(ROOT)
    subprocess.run(["cargo", "build", "--locked", "--release", "-p", "dawn-client-gdext"], cwd=ROOT, check=True)
    target = json.loads(subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version=1"], cwd=ROOT,
        check=True, capture_output=True, text=True,
    ).stdout)["target_directory"]
    output = ROOT / "client/export" / args.version
    if output.exists():
        raise RuntimeError(f"Output already exists; use a new version: {output}")
    output.mkdir(parents=True)
    with tempfile.TemporaryDirectory() as temporary:
        stage = Path(temporary) / "client"
        stage_client(ROOT, stage)
        (stage / "bin").mkdir()
        shutil.copy2(Path(target) / "release/dawn_client_gdext.dll", stage / "bin")
        preset = stage / "export_presets.cfg"
        preset.write_text(preset.read_text().replace('custom_template/release=""',
                          f'custom_template/release="{template.as_posix()}"'), encoding="utf-8")
        subprocess.run([str(godot), "--headless", "--path", str(stage), "--editor", "--import"], check=True)
        subprocess.run([str(godot), "--headless", "--path", str(stage), "--export-release",
                        "Windows Playtest", str(output / "Dawn.exe")], check=True)
    for name in ("Dawn.exe", "Dawn.pck", "dawn_client_gdext.dll"):
        if not (output / name).is_file():
            raise RuntimeError(f"Export is missing {name}")
    shutil.copy2(ROOT / "docs/process/playtest-download.md", output / "README.md")
    write_notices(ROOT, output)
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    archive_package(output, args.version, commit)


if __name__ == "__main__":
    main()
