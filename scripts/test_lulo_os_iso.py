"""Checks for the Lulo OS image definition (ADR 0031) that need no build."""

from __future__ import annotations

import importlib.util
import json
import re
import sys
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
IMAGE = REPO / "packaging" / "lulo-os-iso"
ROOTFS = IMAGE / "rootfs"

spec = importlib.util.spec_from_file_location("qemu_iso_test", REPO / "scripts" / "iso" / "qemu_iso_test.py")
assert spec and spec.loader
driver = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = driver
spec.loader.exec_module(driver)


def default_pinned_apps() -> list[str]:
    identities = {
        name: value
        for name, value in re.findall(
            r'pub const ([A-Z_]+): &str = "([^"]+)";',
            (REPO / "crates" / "rmac-apps" / "src" / "identity.rs").read_text(),
        )
    }
    model = (REPO / "crates" / "rmac-shell-settings" / "src" / "model.rs").read_text()
    block = re.search(r"pub const DEFAULT_PINNED_APPS: \[&str; \d+\] = \[(.*?)\];", model, re.S)
    assert block
    return [identities[name] for name in re.findall(r"identity::([A-Z_]+)", block.group(1))]


class LiveSessionTest(unittest.TestCase):
    def test_live_dock_is_the_default_dock_plus_the_installer(self) -> None:
        stored = json.loads((ROOTFS / "usr/share/lulo-live/shell.json").read_text())
        version = re.search(
            r"const CURRENT_VERSION: u32 = (\d+);",
            (REPO / "crates/rmac-shell-settings/src/lib.rs").read_text(),
        )
        assert version
        self.assertEqual(stored["version"], int(version.group(1)))
        pinned = stored["settings"]["pinned_apps"]
        self.assertIn("org.lulo.Installer", pinned)
        self.assertEqual([app for app in pinned if app != "org.lulo.Installer"], default_pinned_apps())

    def test_installer_entry_runs_the_launcher(self) -> None:
        entry = (ROOTFS / "usr/share/applications/org.lulo.Installer.desktop").read_text()
        self.assertIn("Name=Install Lulo OS\n", entry)
        exec_line = re.search(r"^Exec=(\S+)", entry, re.M)
        assert exec_line
        self.assertTrue((ROOTFS / exec_line.group(1).lstrip("/")).is_file())

    def test_live_session_service_is_live_only(self) -> None:
        unit = (ROOTFS / "usr/lib/systemd/system/lulo-live-session.service").read_text()
        self.assertIn("ConditionKernelCommandLine=boot=casper", unit)
        probe = (ROOTFS / "usr/lib/systemd/system/lulo-ci-probe.service").read_text()
        self.assertIn("ConditionVirtualization=vm", probe)
        self.assertIn("ExecCondition=/usr/libexec/lulo-ci-probe --check", probe)


class InstallerConfigTest(unittest.TestCase):
    def test_every_configured_instance_has_its_file(self) -> None:
        settings = (ROOTFS / "etc/calamares/settings.conf").read_text()
        modules = ROOTFS / "etc/calamares/modules"
        for config in re.findall(r"config: (\S+)", settings):
            self.assertTrue((modules / config).is_file(), config)
        for script in re.findall(r'"(/usr/libexec/lulo-installer/[^" ]+)"', "".join(
            path.read_text() for path in modules.glob("*.conf")
        )):
            self.assertTrue((ROOTFS / script.lstrip("/")).is_file(), script)

    def test_live_cleanup_runs_before_the_user_is_created(self) -> None:
        settings = (ROOTFS / "etc/calamares/settings.conf").read_text()
        execs = settings.split("- exec:")[1].split("- show:")[0]
        steps = [line.strip("- ").strip() for line in execs.splitlines() if line.strip()]
        self.assertLess(steps.index("shellprocess@lulo_cleanup"), steps.index("users"))
        self.assertLess(steps.index("contextualprocess@lulo_bootloader_packages"), steps.index("bootloader"))
        self.assertLess(steps.index("packages"), steps.index("shellprocess@lulo_finish"))
        self.assertEqual(steps[-1], "umount")

    def test_branding_points_at_rendered_images(self) -> None:
        desc = (ROOTFS / "etc/calamares/branding/lulo/branding.desc").read_text()
        renderer = (IMAGE / "make-branding.py").read_text()
        for image in re.findall(r'"([a-z0-9-]+\.png)"', desc):
            self.assertIn(f'"{image}"', renderer, image)


class OcrMatchingTest(unittest.TestCase):
    TSV = "\n".join(
        [
            "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext",
            "5\t1\t1\t1\t1\t1\t200\t100\t80\t24\t95\tErase",
            "5\t1\t2\t1\t1\t1\t290\t102\t60\t22\t93\tdisk",
            "5\t1\t3\t1\t1\t1\t1600\t1500\t90\t26\t96\tInstall",
            "5\t1\t4\t1\t1\t1\t60\t700\t90\t26\t90\tInstall",
            "5\t1\t5\t1\t1\t1\t900\t900\t40\t20\t10\tnoise",
            "5\t1\t6\t1\t1\t1\t600\t300\t110\t24\t91\tPassword",
            "5\t1\t7\t1\t1\t1\t900\t300\t90\t24\t92\tRepeat",
            "5\t1\t8\t1\t1\t1\t1000\t300\t110\t24\t92\tPassword",
        ]
    )

    def words(self):
        return driver.parse_tesseract_tsv(self.TSV, scale=2.0)

    def test_words_scale_back_to_screen_pixels_and_drop_low_confidence(self) -> None:
        words = self.words()
        self.assertEqual(words[0], driver.Word("Erase", 100, 50, 40, 12))
        self.assertNotIn("noise", [word.text for word in words])

    def test_phrases_match_across_tesseract_blocks(self) -> None:
        [match] = driver.find_phrase(self.words(), "Erase disk")
        self.assertEqual((match.left, match.right), (100, 175))

    def test_words_of_one_line_with_uneven_tops_stay_together(self) -> None:
        words = [
            driver.Word("is", 140, 79, 12, 14),
            driver.Word("What", 100, 81, 36, 13),
            driver.Word("your", 156, 82, 30, 12),
            driver.Word("name?", 190, 80, 44, 14),
            driver.Word("login", 100, 160, 40, 13),
        ]
        [match] = driver.find_phrase(words, "What is your name?")
        self.assertEqual((match.left, match.right), (100, 234))
        self.assertEqual(driver.find_phrase(words, "name login"), [])

    def test_lowest_match_is_the_button(self) -> None:
        matches = driver.find_phrase(self.words(), "Install")
        self.assertEqual(len(matches), 2)
        self.assertEqual(max(matches, key=lambda match: match.y).y, 756)

    def test_exact_line_skips_the_repeat_field(self) -> None:
        matches = driver.find_phrase(self.words(), "Password", exact_line=True)
        self.assertEqual([match.left for match in matches], [300])

    def test_keys_for_the_ci_account(self) -> None:
        self.assertEqual(driver.qcodes_for("L"), ["shift", "l"])
        self.assertEqual(driver.qcodes_for("-"), ["minus"])
        self.assertEqual(driver.qcodes_for("\n"), ["ret"])
        for char in driver.CI_USER_NAME + driver.CI_PASSWORD:
            driver.qcodes_for(char)


if __name__ == "__main__":
    unittest.main()
