#!/usr/bin/env python3
"""Focused tests for retiring source-install artifacts safely."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).parent / "linux/archive-development-install.py"
SPEC = importlib.util.spec_from_file_location("archive_development_install", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class ArchiveDevelopmentInstallTests(unittest.TestCase):
    def roots(self, temporary: str):
        home = Path(temporary) / "home"
        home.mkdir()
        return MODULE.Roots(
            home=home,
            config=home / ".config",
            data=home / ".local/share",
            state=home / ".local/state",
        ).validated()

    @staticmethod
    def write(path: Path, value: bytes = b"generated\n") -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(value)

    def test_no_development_install_is_a_noop(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            roots = self.roots(temporary)
            self.assertEqual(MODULE.discover(roots), ())
            self.assertIsNone(MODULE.archive(roots, ()))

    def test_exact_generated_namespace_moves_and_user_settings_remain(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            roots = self.roots(temporary)
            unit = roots.config / "systemd/user/rmac-lock.service"
            dbus = roots.data / "dbus-1/services/org.rmac.Focus1.service"
            portal = roots.data / "xdg-desktop-portal/portals/rmac.portal"
            launcher = roots.home / ".local/bin/rmac-session-start"
            binary = roots.home / ".local/libexec/rmac/rmac-locker"
            setting = roots.config / "rmac/lock-policy.json"
            for path in (unit, dbus, portal, launcher, binary, setting):
                self.write(path)

            artifacts = MODULE.discover(roots)
            self.assertEqual(len(artifacts), 5)
            destination = MODULE.archive(roots, artifacts)
            self.assertEqual(
                destination,
                roots.state / "rmac/migrations/development-install-v1",
            )
            self.assertTrue(setting.is_file())
            self.assertFalse(unit.exists())
            self.assertTrue(
                (destination / "config/systemd/user/rmac-lock.service").is_file()
            )
            self.assertTrue(
                (destination / "home-local/libexec/rmac/rmac-locker").is_file()
            )
            self.assertEqual(MODULE.discover(roots), ())

    def test_unknown_development_binary_refuses_without_moving(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            roots = self.roots(temporary)
            unit = roots.config / "systemd/user/rmac-lock.service"
            unknown = roots.home / ".local/libexec/rmac/personal-tool"
            self.write(unit)
            self.write(unknown)
            with self.assertRaisesRegex(MODULE.MigrationError, "unknown entry"):
                MODULE.discover(roots)
            self.assertTrue(unit.is_file())
            self.assertTrue(unknown.is_file())

    def test_symlinked_generated_artifact_refuses(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            roots = self.roots(temporary)
            target = roots.home / "target"
            self.write(target)
            unit = roots.config / "systemd/user/rmac-lock.service"
            unit.parent.mkdir(parents=True)
            unit.symlink_to(target)
            with self.assertRaisesRegex(MODULE.MigrationError, "not a regular file"):
                MODULE.discover(roots)
            self.assertTrue(unit.is_symlink())


if __name__ == "__main__":
    unittest.main()
