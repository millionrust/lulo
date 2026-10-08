"""The nested shell runners find top-bar controls by exact accessible name.

Renaming the Lulo mark from "menu" to "Lulo menu" silently broke
run_power_dialogs.py and run_menu_dismiss.py (every check that opens the
Lulo menu failed). Keep each runner's name in step with the top bar's.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
MENUBAR = REPO / "shell/bins/rmac-menubar/src/lib.rs"
RUNNERS = (
    "scripts/behavior/run_power_dialogs.py",
    "scripts/behavior/run_menu_dismiss.py",
    "scripts/behavior/run_shutdown.py",
    "scripts/linux/probe_live_shutdown_cancel.py",
)


def rust_label(name: str) -> str:
    match = re.search(rf'const {name}: &str = "([^"]+)";', MENUBAR.read_text())
    assert match, f"{name} is missing from {MENUBAR}"
    return match.group(1)


class ShellAccessibleNameTests(unittest.TestCase):
    def test_runners_find_the_lulo_mark_by_its_real_name(self):
        label = rust_label("LULO_MENU_LABEL")
        for runner in RUNNERS:
            source = (REPO / runner).read_text()
            match = re.search(r'^LULO_MENU = "([^"]+)"$', source, re.M)
            self.assertIsNotNone(match, f"{runner} has no LULO_MENU")
            self.assertEqual(match.group(1), label, runner)
            # A literal "menu" lookup is the old name and finds nothing.
            self.assertNotRegex(source, r'(find|click)_button\("menu"\)', runner)
            self.assertNotIn('"push button", "menu")', source, runner)

    def test_the_mark_is_named_by_the_constant_only(self):
        source = MENUBAR.read_text()
        label = rust_label("LULO_MENU_LABEL")
        self.assertEqual(source.count(f'"{label}"'), 1)
        self.assertNotRegex(source, r'aria_label\("menu"\)')

    def test_journeys_click_control_centre_by_its_label(self):
        source = MENUBAR.read_text()
        self.assertIn('.aria_label("Control Centre")', source)
        self.assertIn('"Control Centre"', (REPO / "tests/parallel/09-menu-bar.json").read_text())


if __name__ == "__main__":
    unittest.main()
