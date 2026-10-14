"""The Mac-vs-Lulo robot's pure rules (scripts/robot/mac/common.py and
diff.py): what it may activate, how labels compare, what a committed
summary keeps. Nothing here touches a GUI."""

import sys
import unittest
from pathlib import Path

import importlib.util

ROBOT = Path(__file__).resolve().parent / "robot" / "mac"
sys.path.insert(0, str(ROBOT))


def _load(name: str, file: str):
    # Loaded by path under a unique name: scripts/inventory has its own
    # `diff.py`, and the whole suite runs in one interpreter.
    spec = importlib.util.spec_from_file_location(name, ROBOT / file)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


cm = _load("mac_robot_common", "common.py")
diff = _load("mac_robot_diff", "diff.py")


class ActivationTests(unittest.TestCase):
    def test_panel_openers_are_allowed(self):
        for label in ("Print…", "Page Setup…", "Save As…", "Export as PDF…", "Go to Folder…", "Settings…",
                      "Show Fonts", "Get Info", "About Finder", "Connect to Server…", "Rename…"):
            self.assertTrue(cm.may_activate(label), label)

    def test_destructive_outward_and_account_items_are_refused(self):
        for label in ("Empty Bin…", "Erase…", "Send Signal to Process…", "Add Account…", "Accounts…",
                      "Restart…", "Shut Down…", "Log Out Jacob…", "Start Dictation…", "Rename Mailbox…",
                      "Import…", "Share…", "New Smart Folder", "Take Screenshot", "Attach File…", "Delete",
                      "Quit Process", "Run Spindump", "Install…", "App Store…", ""):
            self.assertFalse(cm.may_activate(label), label)

    def test_plain_commands_are_only_looked_at(self):
        for label in ("Bold", "New Finder Window", "Close", "Minimise", "Select All"):
            self.assertFalse(cm.may_activate(label), label)


class LabelTests(unittest.TestCase):
    def test_app_names_quotes_ellipsis_and_spelling_fold(self):
        self.assertEqual(cm.norm_label("About Finder", "Finder"), cm.norm_label("About Files", "Finder"))
        self.assertEqual(cm.norm_label("Show Colors…"), cm.norm_label("Show Colours"))
        self.assertEqual(cm.norm_label("Don’t Save"), cm.norm_label("Don't Save"))
        self.assertEqual(cm.norm_label("‎7"), "7")

    def test_shortcuts_compare_without_modifier_order_or_globe(self):
        self.assertEqual(diff.norm_key("⇧⌘N"), diff.norm_key("⌘⇧N"))
        self.assertEqual(diff.norm_key("🌐F"), diff.norm_key("F"))
        self.assertNotEqual(diff.norm_key("⌃F"), diff.norm_key("⌃⇧⌘F"))

    def test_window_list_is_cut_after_arrange_in_front(self):
        items = [{"label": "Minimise"}, {"label": "Bring All to Front"}, {"label": "Arrange in Front"},
                 {"label": "-"}, {"label": "Somebody's document"}]
        self.assertEqual([i["label"] for i in diff.strip_window_list(items)],
                         ["Minimise", "Bring All to Front", "Arrange in Front"])


class ScrubTests(unittest.TestCase):
    tokens = ["Jacob Samas", "Jacob", "Samas", "jake"]

    def test_names_and_addresses_are_removed(self):
        self.assertEqual(cm.scrub_text("Log Out Jacob Samas…", self.tokens), "Log Out ‹owner›…")
        self.assertEqual(cm.scrub_text("jake@example.com", self.tokens), "‹email›")
        self.assertEqual(cm.scrub_text("Air Quality Map", self.tokens), "Air Quality Map")

    def test_window_content_keeps_only_menu_wording(self):
        state = {"app": "Weather", "path": "window/main", "kind": "window", "window": [800, 600],
                 "controls": [{"role": "button", "label": "Hometown, 29°, Cloudy", "enabled": True, "f": [0, 0, 1, 1]},
                              {"role": "button", "label": "Search", "enabled": True, "f": [0, 0, 1, 1]}]}
        out = cm.summary_for_commit(state, self.tokens, {"Search"})
        self.assertEqual([c["label"] for c in out["controls"]], ["‹content›", "Search"])
        self.assertNotIn("title", out)

    def test_settings_drop_numbers_and_addresses(self):
        state = {"app": "Mail", "path": "settings/Accounts", "kind": "settings", "window": [600, 400],
                 "controls": [{"role": "text", "label": "Description:", "f": None},
                              {"role": "text", "label": "jake@example.com", "f": None},
                              {"role": "text", "label": "Port 993", "f": None}]}
        out = cm.summary_for_commit(state, self.tokens, set())
        self.assertEqual([c["label"] for c in out["controls"]], ["Description:", "‹content›", "‹content›"])


if __name__ == "__main__":
    unittest.main()
