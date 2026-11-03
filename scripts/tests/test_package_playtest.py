"""Offline checks for release isolation and version validation."""

import importlib.util
import hashlib
import io
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile

sys.path.insert(0, str(Path(__file__).parents[1]))
SPEC = importlib.util.spec_from_file_location("package_playtest", Path(__file__).parents[1] / "package_playtest.py")
package = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(package)


class PackageTests(unittest.TestCase):
    def test_archive_checksum_and_build_identity_match_payload(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "playtest-1"
            output.mkdir()
            (output / "Dawn.exe").write_bytes(b"executable")
            archive = package.archive_package(output, "playtest-1", "commit-id")
            digest = hashlib.sha256(archive.read_bytes()).hexdigest()
            self.assertEqual(archive.with_suffix(".zip.sha256").read_text(), f"{digest}  {archive.name}\n")
            with zipfile.ZipFile(archive) as source:
                self.assertEqual(source.read("Dawn.exe"), b"executable")
                self.assertEqual(source.read("BUILD.txt").decode().splitlines(), ["playtest-1", "Commit: commit-id"])

    def test_template_download_is_verified_before_extracting_executable(self):
        for valid in (True, False):
            with self.subTest(valid=valid), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                (root / ".godot-version").write_text("4.7.2-stable")
                (root / ".tools/godot/4.7.2-stable").mkdir(parents=True)
                archive = io.BytesIO()
                with zipfile.ZipFile(archive, "w") as source:
                    source.writestr("templates/windows_release_x86_64.exe", b"verified template")
                data = archive.getvalue()
                digest = hashlib.sha512(data).hexdigest() if valid else "invalid"

                def download(url, path):
                    if url.endswith(".tpz"):
                        path.write_bytes(data)
                    else:
                        path.write_text(f"{digest}  Godot_v4.7.2-stable_export_templates.tpz\n")

                with patch.object(package, "download", side_effect=download):
                    if valid:
                        self.assertEqual(package.windows_template(root).read_bytes(), b"verified template")
                    else:
                        with self.assertRaises(RuntimeError):
                            package.windows_template(root)
                        self.assertFalse((root / ".tools/godot/4.7.2-stable/windows_release_x86_64.exe").exists())

    def test_version_cannot_escape_output_directory(self):
        self.assertEqual(package.validate_version("playtest-2026.11.03.1"), "playtest-2026.11.03.1")
        for value in ("../secret", "playtest-../../x", "playtest-", "v1", "playtest-1/a", "playtest-1\n"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                package.validate_version(value)

    def test_staging_uses_committed_resources_without_tests_or_local_credentials(self):
        archive = io.BytesIO()
        with zipfile.ZipFile(archive, "w") as output:
            output.writestr("client/project.godot", '[editor_plugins]\nenabled=PackedStringArray("test-plugin")\n')
            output.writestr("client/scripts/main.gd", "extends Node\n")
            output.writestr("client/test/unit.gd", "test")
            output.writestr("client/addons/gdUnit4/plugin.cfg", "test")
            output.writestr("client/addons/lens_effects/effect.gd", "runtime")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "client").mkdir()
            (root / "client/server_url.txt").write_text("private endpoint")
            (root / "client/export_presets.cfg").write_text("preset")
            stage = root / "stage"
            with patch.object(package.subprocess, "run") as run:
                run.return_value.stdout = archive.getvalue()
                package.stage_client(root, stage)
                self.assertIn("HEAD", run.call_args.args[0])
            self.assertFalse((stage / "server_url.txt").exists())
            self.assertFalse((stage / "test").exists())
            self.assertFalse((stage / "addons/gdUnit4").exists())
            self.assertTrue((stage / "addons/lens_effects/effect.gd").exists())
            self.assertIn("enabled=PackedStringArray()", (stage / "project.godot").read_text())
            self.assertNotIn("../target", (stage / "dawn_client_gdext.gdextension").read_text())


if __name__ == "__main__":
    unittest.main()
