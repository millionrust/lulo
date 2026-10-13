#!/usr/bin/env python3
"""The Windows installer's WiX sources must be well-formed XML.

The installer only builds in windows-preview.yml, so a malformed .wxs (for
example `--` inside an XML comment, which WiX rejects with WIX0104) would
otherwise pass every regular CI job and only fail when a preview is built.
"""

from __future__ import annotations

import importlib.util
import unittest
import xml.etree.ElementTree as ElementTree
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WINDOWS = ROOT / "packaging" / "windows"


class WixSourcesAreWellFormed(unittest.TestCase):
    def test_every_wxs_parses(self) -> None:
        sources = sorted(WINDOWS.glob("*.wxs"))
        self.assertTrue(sources, "expected at least one .wxs under packaging/windows")
        for source in sources:
            with self.subTest(source=source.name):
                ElementTree.parse(source)


def load_msi_version():
    spec = importlib.util.spec_from_file_location("msi_version", WINDOWS / "msi_version.py")
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


class MsiVersionRisesWithEveryBuild(unittest.TestCase):
    """WIN-OS-54: every build used to be MSI 0.9.0, so a newer MSI left the
    older exes in place."""

    def test_the_build_number_is_the_third_field(self) -> None:
        msi_version = load_msi_version().msi_version
        self.assertEqual(msi_version("0.9.0-beta.1", 3947), "0.9.3947")
        self.assertEqual(msi_version("0.9.1", 4100), "0.9.4100")
        self.assertEqual(msi_version("1.0.0+build", 1), "1.0.1")

    def test_versions_sort_by_build_and_then_by_release(self) -> None:
        msi_version = load_msi_version().msi_version

        def key(version: str) -> tuple[int, ...]:
            return tuple(int(part) for part in version.split("."))

        self.assertLess(key(msi_version("0.9.0-beta.1", 3947)), key(msi_version("0.9.0-beta.1", 3948)))
        self.assertLess(key(msi_version("0.9.0", 4000)), key(msi_version("0.10.0", 10)))
        # The old fixed version sorts below every new build.
        self.assertLess(key("0.9.0"), key(msi_version("0.9.0-beta.1", 1)))

    def test_out_of_range_numbers_are_refused(self) -> None:
        msi_version = load_msi_version().msi_version
        for semver, build in [("0.9.0", 0), ("0.9.0", 65536), ("256.0.0", 1), ("next", 1)]:
            with self.subTest(semver=semver, build=build):
                with self.assertRaises(ValueError):
                    msi_version(semver, build)

    def test_major_upgrade_replaces_a_same_version_build(self) -> None:
        namespace = {"wix": "http://wixtoolset.org/schemas/v4/wxs"}
        product = ElementTree.parse(WINDOWS / "Product.wxs")
        upgrade = product.find(".//wix:MajorUpgrade", namespace)
        self.assertIsNotNone(upgrade)
        self.assertEqual(upgrade.get("AllowSameVersionUpgrades"), "yes")


class InstallBringsLuloBack(unittest.TestCase):
    """An upgrade left Lulo stopped (2026-10-14): the MSI now stops a running
    Lulo before it replaces the files and starts it again once it is done,
    and no longer deletes the user's Start Lulo at Sign-In on install."""

    namespace = {"wix": "http://wixtoolset.org/schemas/v4/wxs"}

    def product(self):
        return ElementTree.parse(WINDOWS / "Product.wxs")

    def test_lulo_starts_after_install_finalize_without_waiting(self) -> None:
        product = self.product()
        action = product.find(".//wix:CustomAction[@Id='StartLulo']", self.namespace)
        self.assertIsNotNone(action)
        self.assertIn("--after-install", action.get("ExeCommand"))
        self.assertEqual(action.get("Return"), "asyncNoWait")
        self.assertEqual(action.get("Execute"), "immediate")
        step = product.find(".//wix:Custom[@Action='StartLulo']", self.namespace)
        self.assertEqual(step.get("After"), "InstallFinalize")
        self.assertIn('NOT REMOVE="ALL"', step.get("Condition"))

    def test_a_running_lulo_stops_before_files_are_checked(self) -> None:
        step = self.product().find(".//wix:Custom[@Action='StopLuloForUpgrade']", self.namespace)
        self.assertIsNotNone(step)
        self.assertEqual(step.get("Before"), "InstallValidate")

    def test_install_keeps_start_at_sign_in(self) -> None:
        removed = [
            value.get("Key")
            for value in self.product().iter("{http://wixtoolset.org/schemas/v4/wxs}RemoveRegistryValue")
        ]
        self.assertFalse([key for key in removed if key.endswith(r"CurrentVersion\Run")], removed)

    def test_upgrade_keeps_the_lulo_layer_settings(self) -> None:
        # A removeOnUninstall <RemoveRegistryKey> also runs while an upgrade
        # removes the old product; `lulo-session --uninstall` removes them.
        removed = [
            key.get("Key")
            for key in self.product().iter("{http://wixtoolset.org/schemas/v4/wxs}RemoveRegistryKey")
        ]
        self.assertNotIn(r"Software\Lulo\Shell", removed)


if __name__ == "__main__":
    unittest.main()
