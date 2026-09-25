"""Unit tests for the behaviour-parity suite (scripts/behavior, tests/behavior).

They run anywhere: no compositor, no AT-SPI and no Mac are needed.
"""

from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE / "behavior"))

import compare  # noqa: E402
import run_lulo  # noqa: E402
import scenario as sc  # noqa: E402
import wlinput  # noqa: E402

NESTED = {"WAYLAND_DISPLAY": "wayland-2", "XDG_RUNTIME_DIR": "/tmp/lulo-behavior-x/runtime", "RMAC_BEHAVIOR_NESTED": "1"}


class KeyTests(unittest.TestCase):
    def test_chords_parse_to_evdev_codes(self):
        self.assertEqual(wlinput.parse_chord("cmd-shift-n"), (["shift", "cmd"], 49))
        self.assertEqual(wlinput.parse_chord("⇧⌘N"), (["shift", "cmd"], 49))
        self.assertEqual(wlinput.parse_chord("cmd-backspace"), (["cmd"], 14))
        self.assertEqual(wlinput.parse_chord("⌘⌫"), (["cmd"], 14))
        self.assertEqual(wlinput.parse_chord("return"), ([], 28))
        self.assertEqual(wlinput.parse_chord("cmd--"), (["cmd"], 12))

    def test_typing_uses_shift_for_capitals_and_symbols(self):
        self.assertEqual(wlinput.text_to_strokes("Hi+"), [(["shift"], 35), ([], 23), (["shift"], 13)])
        with self.assertRaises(wlinput.InjectorError):
            wlinput.text_to_strokes("é")

    def test_mac_keystrokes_match_the_lulo_chord(self):
        self.assertEqual(sc.mac_keystroke("cmd-shift-n"), ("n", None, ["shift down", "command down"]))
        self.assertEqual(sc.mac_keystroke("cmd-backspace"), (None, 51, ["command down"]))
        self.assertEqual(sc.mac_keystroke("escape"), (None, 53, []))
        self.assertEqual(sc.mac_keystroke("down"), (None, 125, []))
        self.assertEqual(sc.mac_keystroke("space"), (None, 49, []))


class GuardTests(unittest.TestCase):
    def test_injector_refuses_the_live_session(self):
        wlinput.assert_nested(NESTED)
        for override in ({"WAYLAND_DISPLAY": "wayland-1"}, {"XDG_RUNTIME_DIR": "/run/user/1000"},
                         {"XDG_RUNTIME_DIR": "/run/user/1001"}, {"RMAC_BEHAVIOR_NESTED": "0"},
                         {"WAYLAND_DISPLAY": ""}):
            with self.subTest(override=override), self.assertRaises(wlinput.InjectorError):
                wlinput.assert_nested({**NESTED, **override})

    def test_runner_refuses_the_live_session(self):
        run_lulo.refuse_live_session({"XDG_RUNTIME_DIR": "/tmp/x", "WAYLAND_DISPLAY": "wayland-2"})
        for env in ({"XDG_RUNTIME_DIR": "/run/user/1000"}, {"XDG_RUNTIME_DIR": "/tmp/x", "WAYLAND_DISPLAY": "wayland-1"}):
            with self.subTest(env=env), self.assertRaises(SystemExit):
                run_lulo.refuse_live_session(env)

    def test_isolated_environment_drops_the_live_session(self):
        import os
        import tempfile

        saved = dict(os.environ)
        os.environ.update({"WAYLAND_DISPLAY": "wayland-1", "XDG_RUNTIME_DIR": "/run/user/1000",
                           "DBUS_SESSION_BUS_ADDRESS": "unix:path=/run/user/1000/bus"})
        try:
            with tempfile.TemporaryDirectory() as work:
                env = run_lulo.isolated_environment(Path(work))
                self.assertNotIn("WAYLAND_DISPLAY", env)
                self.assertNotIn("DBUS_SESSION_BUS_ADDRESS", env)
                self.assertTrue(env["XDG_RUNTIME_DIR"].startswith(work))
                self.assertTrue(env["HOME"].startswith(work))
                self.assertEqual(env["GSETTINGS_BACKEND"], "memory")
        finally:
            os.environ.clear()
            os.environ.update(saved)


class ScenarioFileTests(unittest.TestCase):
    def test_every_scenario_is_valid_and_recorded(self):
        paths = sc.scenario_paths()
        self.assertGreaterEqual(len(paths), 20)
        for path in paths:
            with self.subTest(path=path.name):
                scenario = sc.load(path)
                expected = sc.expectation_path(path)
                self.assertTrue(expected.exists(), f"{path.name} has no .mac.json")
                data = json.loads(expected.read_text())
                self.assertNotIn("error", data)
                names = {s["observe"] for s in scenario["steps"] if "observe" in s}
                self.assertEqual(set(data["observations"]), names)

    def test_recordings_hold_no_paths_or_captures(self):
        for path in sc.SCENARIO_ROOT.glob("*/*.mac.json"):
            text = path.read_text()
            with self.subTest(path=path.name):
                for needle in ("/Users/", "/home/", "/tmp/", "/private/", ".png", "base64"):
                    self.assertNotIn(needle, text)
        self.assertEqual(list(sc.SCENARIO_ROOT.rglob("*.png")), [])

    def test_validation_rejects_escaping_the_sandbox(self):
        with self.assertRaises(sc.ScenarioError):
            sc.validate({"title": "t", "app": "files", "setup": {"files": {"../x": ""}},
                         "steps": [{"observe": "o", "facts": ["files"]}]})
        with self.assertRaises(sc.ScenarioError):
            sc.validate({"title": "t", "app": "files", "steps": [{"key": "a", "type": "b"}]})
        with self.assertRaises(sc.ScenarioError):
            sc.validate({"title": "t", "app": "files", "steps": [{"observe": "o", "facts": ["pixels"]}]})


class CompareTests(unittest.TestCase):
    scenario = {"title": "New Folder", "app": "files", "steps": [], "tolerance": {"a.windows.titles": "ignore"}}

    def test_matching_facts_pass(self):
        mac = {"observations": {"a": {"focus": {"role": "text-field", "value": "untitled folder", "selection": [0, 15]},
                                      "files": {"entries": ["b/", "a.txt"]}}}}
        lulo = {"observations": {"a": {"focus": {"role": "text-area", "value": "untitled folder", "selection": [0, 15]},
                                       "files": {"entries": ["a.txt", "b/"]}}}}
        self.assertEqual(sc.compare(self.scenario, mac, lulo), [])

    def test_differences_are_reported_per_field(self):
        mac = {"observations": {"a": {"focus": {"role": "text-field", "selection": [0, 15]},
                                      "windows": {"count": 1, "titles": ["x"]}}}}
        lulo = {"observations": {"a": {"focus": {"role": "list", "selection": None},
                                       "windows": {"count": 2, "titles": ["y"]}}}}
        fields = {(m["fact"], m["field"]) for m in sc.compare(self.scenario, mac, lulo)}
        self.assertEqual(fields, {("focus", "role"), ("focus", "selection"), ("windows", "count")})

    def test_missing_observation_fails(self):
        mac = {"observations": {"a": {"files": {"entries": []}}}}
        result = sc.compare(self.scenario, mac, {"observations": {}, "error": "app exited"})
        self.assertEqual(result[0]["actual"], "app exited")

    def test_text_rule_normalizes_typography(self):
        self.assertTrue(sc.field_matches("text", "Don’t save “A”…", "Don't save \"A\"..."))
        self.assertFalse(sc.field_matches("text", "Move to Bin", "Move to Trash"))

    def test_selection_facts(self):
        self.assertEqual(sc.selection_facts("report.txt", 0, 6),
                         {"selection": [0, 6], "selected_text": "report", "selected_all": False})
        self.assertTrue(sc.selection_facts("Plans", 5, 0)["selected_all"])

    def test_finish_observation_omits_and_trims_menus(self):
        spec = {"omit": ["o.focus.value"], "menu_until": "Quick Actions"}
        facts = {"focus": {"value": "/private/tmp", "role": "text-field"},
                 "menu": {"present": True, "items": ["Open", "Quick Actions", "Keka"]}}
        out = sc.finish_observation(spec, "o", facts)
        self.assertEqual(out["focus"], {"role": "text-field"})
        self.assertEqual(out["menu"]["items"], ["Open", "Quick Actions"])

    def test_parity_rows_continue_the_section_numbering(self):
        text = "### Files\n\n| ID | Sev |\n|---|---|\n| FILES-09 | P1 |\n| FILES-10 | P2 |\n\n### Settings\n| SET-02 | P1 |\n"
        self.assertEqual(sc.next_ids(text, "files", 2), ["FILES-11", "FILES-12"])
        mismatch = [{"observation": "a", "fact": "focus", "field": "role", "expected": "text-field", "actual": "list"}]
        rows = sc.parity_rows(text, [("files/new-folder", {"title": "New Folder", "app": "files"}, mismatch)])
        self.assertIn("| FILES-11 | P1 | S | Missing |", rows["### Files"][0])
        self.assertIn("behavior:files/new-folder", rows["### Files"][0])
        cited = text + "| FILES-11 | x | behavior:files/new-folder |\n"
        self.assertEqual(sc.parity_rows(cited, [("files/new-folder", {"title": "t", "app": "files"}, mismatch)]), {})

    def test_compare_cli_recomputes_from_results(self):
        import tempfile

        path = sc.scenario_paths(only=["files/new-folder"])[0]
        expected = json.loads(sc.expectation_path(path).read_text())
        results = {"results": [{"scenario": "files/new-folder", "lulo": {"observations": expected["observations"]}}]}
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as handle:
            json.dump(results, handle)
        evaluated = compare.evaluate(json.loads(Path(handle.name).read_text()))
        Path(handle.name).unlink()
        self.assertEqual([e["status"] for e in evaluated], ["pass"])


if __name__ == "__main__":
    unittest.main()
