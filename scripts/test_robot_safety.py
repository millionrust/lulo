"""The live robot's safety rules (scripts/robot/safety.py) and its pure
helpers in scripts/robot/live.py. None of this touches a session."""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent / "robot"))

import live  # noqa: E402
import safety  # noqa: E402


class DenyListTests(unittest.TestCase):
    def test_destructive_and_outward_items_are_refused(self):
        for label in ("Delete", "Move to Bin", "Empty Bin…", "Erase…", "Send", "Reply All", "Forward",
                      "Shut Down…", "Restart…", "Log Out Jacob…", "Lock Screen", "Sleep", "Uninstall",
                      "Remove from Dock", "Reset…", "Force Quit…", "Discard Changes", "Quit Terminal",
                      "Delete Immediately…", "Close Window", "Close All", "Minimise", "Save As…",
                      "Turn Wi-Fi Off", "Bluetooth Settings…", "Paste", "Undo Typing"):
            with self.subTest(label=label):
                self.assertIsNotNone(safety.deny_reason(label), label)

    def test_ordinary_view_items_are_allowed(self):
        for label in ("About Calculator", "Scientific", "Show Sidebar", "Hide Toolbar", "as Columns",
                      "Bigger", "Zoom In", "Show Tab Bar", "Settings…", "New Window", "Find…"):
            with self.subTest(label=label):
                self.assertIsNone(safety.deny_reason(label), label)

    def test_words_match_whole_words_only(self):
        # "Clock" contains "lock", "Binary" contains "bin": both are fine.
        self.assertIsNone(safety.deny_reason("Clock Settings"))
        self.assertIsNone(safety.deny_reason("Binary"))

    def test_actions_are_refused_whatever_their_label(self):
        self.assertIsNotNone(safety.deny_reason("Harmless", "system::shutdown"))
        self.assertIsNotNone(safety.deny_reason("Harmless", "app::quit"))

    def test_view_only_apps_only_run_view_menus(self):
        self.assertEqual(safety.item_allowed("Mail", "Message", "Show Headers", "mail::X", True)[0], False)
        self.assertEqual(safety.item_allowed("Mail", "View", "Show Headers", "mail::X", True)[0], True)
        self.assertEqual(safety.item_allowed("Notes", "Format", "Title", "notes::Title", True)[0], False)
        self.assertEqual(safety.item_allowed("Calculator", "View", "Scientific", "c::S", False)[0], False)

    def test_screen_reader_and_session_chords_are_never_pressed(self):
        for chord in ("cmd-f5", "cmd-alt-s", "ctrl-cmd-q"):
            self.assertFalse(safety.chord_allowed(chord))
        self.assertTrue(safety.chord_allowed("cmd-w"))


class LayoutTests(unittest.TestCase):
    def test_busctl_json_layout_decodes_with_submenu_paths(self):
        data = {"type": "(ua(sa(sssuy)))", "data": [7, [
            ["View", [["Decimal Places", "", "", live.ENABLED | live.SUBMENU, 0],
                      ["0", "calculator::DecimalPlaces0", "", live.ENABLED, 1],
                      ["8", "calculator::DecimalPlaces8", "", live.ENABLED | live.CHECKED, 1],
                      ["RPN Mode", "calculator::ToggleRpnMode", "⌘R", live.ENABLED | live.SEPARATOR, 0]]],
        ]]}
        revision, menus = live.parse_layout(data)
        self.assertEqual(revision, 7)
        items = menus[0]["items"]
        self.assertEqual(items[1]["path"], ["View", "Decimal Places", "0"])
        self.assertTrue(items[2]["checked"])
        self.assertEqual(items[3]["path"], ["View", "RPN Mode"])
        self.assertTrue(items[0]["submenu"])


class DockRecentsTests(unittest.TestCase):
    W, C, M, K, P, T, S = (f"org.rmac.{n}.desktop" for n in
                           ("Weather", "Calculator", "Mail", "Clock", "Preview", "Terminal", "SystemMonitor"))

    def replay(self, current, order):
        state = list(current)
        for app in order:
            state = live.simulate_recent_launch(state, app)
        return state

    def test_a_replay_restores_a_shifted_list(self):
        original, current = [self.W, self.C, self.M], [self.C, self.M, self.K]
        order = live.recents_launch_order(original, current)
        self.assertEqual(self.replay(current, order), original)

    def test_fillers_push_out_apps_listed_in_the_wrong_place(self):
        original, current = [self.W, self.C, self.M], [self.K, self.W, self.P]
        self.assertEqual(live.recents_launch_order(original, current), [])
        order = live.recents_launch_order(original, current, [self.K, self.T, self.S, self.C, self.P])
        self.assertEqual(self.replay(current, order), original)
        self.assertNotIn(self.K, order[:3])

    def test_nothing_to_do_when_unchanged(self):
        self.assertEqual(live.recents_launch_order([self.W], [self.W]), [])


class JournalTests(unittest.TestCase):
    def test_lulo_panics_and_errors_are_classified(self):
        lines = [
            "2026-10-13T19:00:00 Jake rmac-terminal[1]: thread 'main' panicked at src/x.rs:1:1",
            "2026-10-13T19:00:01 Jake rmac-preview[2]: ERROR could not open",
            "2026-10-13T19:00:02 Jake firefox[3]: ERROR unrelated",
        ]
        kinds = [kind for kind, _line in live.classify_journal(lines)]
        self.assertEqual(kinds, ["panic", "error"])


if __name__ == "__main__":
    unittest.main()
