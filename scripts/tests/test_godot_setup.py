"""Offline regression tests for the shared readiness interface."""

import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import zipfile


SPEC = importlib.util.spec_from_file_location("godot_setup", Path(__file__).parents[1] / "godot_setup.py")
setup = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(setup)


class PreparationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / ".gdunit4-version").write_text("6.1.3\n")
        (self.root / ".godot-version").write_text("4.7.2-stable\n")
        self.addon = self.root / "client/addons/gdUnit4"

    def fixture(self, newline="\n"):
        files = {}
        for name, old, _ in setup.compatibility_patches():
            files[name] = files.get(name, "") + old + "\n"
        files["plugin.cfg"] = '[plugin]\nversion="6.1.3"\n'
        for name, text in files.items():
            path = self.addon / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(text.replace("\n", newline).encode())

    def test_patches_accept_lf_crlf_and_repeat_without_changes(self):
        for newline in ("\n", "\r\n"):
            with self.subTest(newline=newline):
                self.fixture(newline)
                setup.patch_addon(self.addon)
                first = {p: p.read_bytes() for p in self.addon.rglob("*") if p.is_file()}
                setup.patch_addon(self.addon)
                self.assertEqual(first, {p: p.read_bytes() for p in first})
                for name, _, new in setup.compatibility_patches():
                    self.assertIn(new, (self.addon / name).read_text())

    def test_unknown_patch_does_not_partially_modify_addon(self):
        self.fixture()
        (self.addon / "runtest.sh").write_text("unexpected upstream version")
        before = (self.addon / "runtest.cmd").read_bytes()
        with self.assertRaisesRegex(RuntimeError, "Unrecognized"):
            setup.patch_addon(self.addon)
        self.assertEqual(before, (self.addon / "runtest.cmd").read_bytes())

    def test_wrong_installed_version_is_not_silently_overwritten(self):
        self.fixture()
        (self.addon / "plugin.cfg").write_text('[plugin]\nversion="0.0.0"\n')
        with self.assertRaisesRegex(RuntimeError, "version mismatch"), patch.object(setup, "download") as download:
            setup.install_addon(self.root, self.addon)
        download.assert_not_called()

    def test_missing_addon_installs_pinned_release_and_reuses_it(self):
        def download(_, destination):
            with zipfile.ZipFile(destination, "w") as archive:
                archive.writestr('gdUnit4-6.1.3/addons/gdUnit4/plugin.cfg', '[plugin]\nversion="6.1.3"\n')
        with patch.object(setup, "download", side_effect=download) as fetch:
            setup.install_addon(self.root, self.addon)
            setup.install_addon(self.root, self.addon)
        self.assertEqual(fetch.call_count, 1)

    def test_build_precedes_import_and_runs_even_without_existing_library(self):
        self.fixture()
        commands = []
        def run(command, **_):
            commands.append(command)
            if "--import" in command:
                Path(command[-1]).write_text("Import complete\n")
        setup.prepare(self.root, Path("godot"), run)
        self.assertEqual(commands[0], ["cargo", "build", "-p", "dawn-client-gdext"])
        self.assertIn("--import", commands[1])

    def test_failed_build_blocks_import(self):
        self.fixture()
        def run(command, **_):
            raise subprocess.CalledProcessError(1, command)
        with self.assertRaises(subprocess.CalledProcessError):
            setup.prepare(self.root, Path("godot"), run)
        self.assertFalse((self.root / "client/.godot-test-logs/import.log").exists())

    def test_import_error_even_with_zero_exit_is_not_ready(self):
        self.fixture()
        def run(command, **_):
            if "--import" in command:
                Path(command[-1]).write_text("SCRIPT ERROR: Parse Error: missing native class\n")
        with self.assertRaisesRegex(RuntimeError, "import reported errors"):
            setup.prepare(self.root, Path("godot"), run)

    def test_import_exit_failure_is_not_ready(self):
        self.fixture()
        def run(command, **_):
            if "--import" in command:
                raise subprocess.CalledProcessError(1, command)
        with self.assertRaises(subprocess.CalledProcessError):
            setup.prepare(self.root, Path("godot"), run)

    def test_archive_traversal_is_rejected(self):
        archive = self.root / "unsafe.zip"
        with zipfile.ZipFile(archive, "w") as source:
            source.writestr("../outside", "bad")
        with self.assertRaisesRegex(RuntimeError, "unsafe path"):
            setup.extract_zip(archive, self.root / "extract")

    def test_import_failure_blocks_requested_test_execution(self):
        with patch.object(setup, "ROOT", self.root), \
                patch.object(setup, "install_godot", return_value=Path("godot")), \
                patch.object(setup, "prepare", side_effect=RuntimeError("import failed")), \
                patch.object(setup.subprocess, "run") as run, \
                patch("sys.argv", ["godot_setup.py", "--run-tests"]):
            with self.assertRaisesRegex(RuntimeError, "import failed"):
                setup.main()
            run.assert_not_called()

    def test_download_checksum_failure_prevents_extraction(self):
        def download(url, destination):
            destination.write_bytes(b"not a valid checksum" if url.endswith("SHA512-SUMS.txt") else b"archive")
        with patch.object(setup, "download", side_effect=download), \
                patch.object(setup, "extract_zip") as extract:
            with self.assertRaisesRegex(RuntimeError, "SHA512"):
                setup.install_godot(self.root)
            extract.assert_not_called()

    def test_cached_editor_version_is_checked(self):
        _, _, executable = setup.godot_location(self.root)
        executable.parent.mkdir(parents=True)
        executable.touch()
        result = subprocess.CompletedProcess([], 0, stdout="4.6.3.stable.official\n")
        with patch.object(setup.subprocess, "run", return_value=result), \
                patch.object(setup, "download") as download:
            with self.assertRaisesRegex(RuntimeError, "version mismatch"):
                setup.install_godot(self.root)
            download.assert_not_called()


if __name__ == "__main__":
    unittest.main()
