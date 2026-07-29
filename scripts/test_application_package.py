#!/usr/bin/env python3
"""Fixture tests for package-owned rmac application integration."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import os
from pathlib import Path
import stat
import sys
import tempfile
import unittest
from unittest import mock


def load_script(name: str, filename: str):
    script = Path(__file__).parent / "linux" / filename
    spec = importlib.util.spec_from_file_location(name, script)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


stage_package = load_script(
    "stage_application_package", "stage-application-package.py"
)
verify_package = load_script(
    "verify_application_package", "verify-application-package.py"
)


def write_program(path: Path, body: str = "exit 0\n") -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("#!/bin/sh\n" + body, encoding="utf-8")
    path.chmod(0o755)


def rehash(root: Path, relative: Path) -> None:
    manifest_path = root / verify_package.MANIFEST
    document = json.loads(manifest_path.read_text(encoding="utf-8"))
    absolute = f"/{relative.as_posix()}"
    for entry in document["files"]:
        if entry["path"] == absolute:
            entry["sha256"] = hashlib.sha256((root / relative).read_bytes()).hexdigest()
            break
    else:
        raise AssertionError(f"manifest does not claim {absolute}")
    manifest_path.write_text(
        json.dumps(document, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


class ApplicationPackageTests(unittest.TestCase):
    def stage(self, parent: Path, name: str = "package-root") -> Path:
        root = parent / name
        stage_package.stage(root)
        return root

    def test_stages_and_verifies_exact_localized_metadata(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = self.stage(Path(temporary))
            verify_package.verify_tree(root)
            manifest = json.loads(
                (root / verify_package.MANIFEST).read_text(encoding="utf-8")
            )
            paths = [entry["path"] for entry in manifest["files"]]
            self.assertEqual(paths, sorted(paths))
            self.assertEqual(len(paths), len(verify_package.EXPECTED_PATHS))
            self.assertTrue(all(path.startswith("/usr/share/") for path in paths))
            self.assertFalse(any("/home/" in path or "/root/" in path for path in paths))
            for entry in manifest["files"]:
                self.assertEqual(entry["mode"], "0644")

            desktop = (
                root / "usr/share/applications/org.rmac.TextEditor.desktop"
            ).read_text(encoding="utf-8")
            self.assertIn("Exec=/usr/bin/rmac-text-editor %F\n", desktop)
            self.assertIn("Name[hi]=पाठ संपादक\n", desktop)
            self.assertNotIn("DBusActivatable", desktop)
            self.assertNotIn("StartupNotify", desktop)

    def test_refuses_live_root_relative_and_nonempty_destinations(self):
        with self.assertRaises(stage_package.PackageError):
            stage_package.stage(Path("/"))
        with self.assertRaises(stage_package.PackageError):
            stage_package.stage(Path("relative"))
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "package-root"
            root.mkdir()
            owned = root / "owned"
            owned.write_text("leave me alone", encoding="utf-8")
            with self.assertRaises(stage_package.PackageError):
                stage_package.stage(root)
            self.assertEqual(owned.read_text(encoding="utf-8"), "leave me alone")

    def test_verifier_rejects_tampering_symlinks_and_extra_paths(self):
        with tempfile.TemporaryDirectory() as temporary:
            parent = Path(temporary)
            root = self.stage(parent)
            desktop = root / "usr/share/applications/org.rmac.Files.desktop"
            desktop.write_text("[Desktop Entry]\nType=Application\n", encoding="utf-8")
            with self.assertRaisesRegex(
                verify_package.VerificationError, "installed content differs"
            ):
                verify_package.verify_tree(root)

            root = self.stage(parent, "second-root")
            icon = (
                root
                / "usr/share/icons/hicolor/scalable/apps/org.rmac.Files.svg"
            )
            icon.unlink()
            icon.symlink_to(root / "usr/share/doc/rmac-apps/copyright")
            with self.assertRaisesRegex(
                verify_package.VerificationError, "not a regular file"
            ):
                verify_package.verify_tree(root)

            root = self.stage(parent, "third-root")
            extra = root / "usr/share/rmac/unclaimed"
            extra.write_text("unexpected\n", encoding="utf-8")
            with self.assertRaisesRegex(
                verify_package.VerificationError, "unexpected path"
            ):
                verify_package.verify_tree(root)

    def test_desktop_claims_appstream_links_and_catalogs_are_semantic_gates(self):
        with tempfile.TemporaryDirectory() as temporary:
            parent = Path(temporary)
            root = self.stage(parent)
            desktop_path = Path(
                "usr/share/applications/org.rmac.Files.desktop"
            )
            desktop = root / desktop_path
            desktop.write_text(
                desktop.read_text(encoding="utf-8") + "DBusActivatable=true\n",
                encoding="utf-8",
            )
            rehash(root, desktop_path)
            with self.assertRaisesRegex(
                verify_package.VerificationError, "unproven claim"
            ):
                verify_package.verify_tree(root)

            root = self.stage(parent, "second-root")
            metainfo_path = Path(
                "usr/share/metainfo/org.rmac.Files.metainfo.xml"
            )
            metainfo = root / metainfo_path
            metainfo.write_text(
                metainfo.read_text(encoding="utf-8").replace(
                    "org.rmac.Files.desktop", "org.rmac.Terminal.desktop"
                ),
                encoding="utf-8",
            )
            rehash(root, metainfo_path)
            with self.assertRaisesRegex(
                verify_package.VerificationError, "integration fields"
            ):
                verify_package.verify_tree(root)

            root = self.stage(parent, "third-root")
            catalog_path = Path("usr/share/doc/rmac-apps/localization/hi.po")
            catalog = root / catalog_path
            catalog.write_text(
                catalog.read_text(encoding="utf-8").replace(
                    'msgstr "फ़ाइलें"', 'msgstr "बासी अनुवाद"'
                ),
                encoding="utf-8",
            )
            rehash(root, catalog_path)
            with self.assertRaisesRegex(
                verify_package.VerificationError, "catalog is stale"
            ):
                verify_package.verify_tree(root)

    def test_installed_gate_requires_apps_and_runs_both_standard_validators(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = self.stage(Path(temporary))
            with self.assertRaisesRegex(
                verify_package.VerificationError, "required application is missing"
            ):
                verify_package.verify_installed_host(root)

            for specification in verify_package.APPLICATIONS.values():
                write_program(root / "usr/bin" / str(specification["binary"]))
            with self.assertRaisesRegex(
                verify_package.VerificationError, "metadata validator is missing"
            ):
                verify_package.verify_installed_host(root)

            log = Path(temporary) / "validator.log"
            body = 'printf "%s|%s\\n" "$0" "$*" >>"$RMAC_VALIDATOR_LOG"\n'
            write_program(root / "usr/bin/desktop-file-validate", body)
            write_program(root / "usr/bin/appstreamcli", body)
            with mock.patch.dict(os.environ, {"RMAC_VALIDATOR_LOG": str(log)}):
                verify_package.verify_installed_host(root)
            calls = log.read_text(encoding="utf-8").splitlines()
            self.assertEqual(len(calls), 1 + len(verify_package.APPLICATIONS))
            self.assertIn("desktop-file-validate", calls[0])
            self.assertTrue(all("validate --no-net" in call for call in calls[1:]))

            write_program(root / "usr/bin/appstreamcli", "exit 2\n")
            with mock.patch.dict(os.environ, {"RMAC_VALIDATOR_LOG": str(log)}):
                with self.assertRaisesRegex(
                    verify_package.VerificationError, "appstreamcli rejected"
                ):
                    verify_package.verify_installed_host(root)

    def test_all_staged_files_are_regular_read_only_package_data(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = self.stage(Path(temporary))
            for path in root.rglob("*"):
                if not path.is_file():
                    continue
                self.assertFalse(path.is_symlink())
                self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o644)


if __name__ == "__main__":
    unittest.main()
