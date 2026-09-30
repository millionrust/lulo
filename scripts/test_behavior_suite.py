"""Unit tests for the behaviour-parity suite (scripts/behavior, tests/behavior).

They run anywhere: no compositor, no AT-SPI and no Mac are needed.
"""

from __future__ import annotations

import json
import sys
import time
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE / "behavior"))

import compare  # noqa: E402
import record_mac  # noqa: E402
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
        self.assertEqual(sc.mac_keystroke("+"), (None, 69, []))


class ClickTests(unittest.TestCase):
    """wlinput.Wayland.click, without a real compositor: build a bare
    instance (skip __init__, which needs a live socket) and record the
    wire-protocol sends `click()` makes."""

    def _client(self):
        client = object.__new__(wlinput.Wayland)
        client.pointer = 501
        client.keyboard = 502
        client.started = time.monotonic()
        client.calls: list[tuple[int, int]] = []
        client._send = lambda object_id, opcode, payload, fds=None: client.calls.append((object_id, opcode))
        client.roundtrip = lambda: None
        return client

    def test_plain_click_is_unchanged_a_single_unmodified_left_click(self):
        client = self._client()
        client.click(10, 20, 100, 100)
        # motion+frame (move), then one button-down+frame, one button-up+frame; no keyboard traffic.
        self.assertEqual(client.calls, [(client.pointer, 1), (client.pointer, 4),
                                         (client.pointer, 2), (client.pointer, 4),
                                         (client.pointer, 2), (client.pointer, 4)])

    def test_double_click_sends_two_button_press_release_pairs(self):
        client = self._client()
        client.click(10, 20, 100, 100, count=2)
        button_events = [opcode for object_id, opcode in client.calls if object_id == client.pointer]
        self.assertEqual(button_events.count(2), 4)  # two down + two up

    def test_shift_click_holds_shift_around_the_pointer_click(self):
        client = self._client()
        client.click(10, 20, 100, 100, modifiers=["shift"])
        keyboard_calls = [i for i, (object_id, _opcode) in enumerate(client.calls) if object_id == client.keyboard]
        pointer_buttons = [i for i, (object_id, opcode) in enumerate(client.calls)
                            if object_id == client.pointer and opcode == 2]
        self.assertTrue(keyboard_calls and pointer_buttons)
        # Shift goes down before the first click and up after the last.
        self.assertLess(keyboard_calls[0], pointer_buttons[0])
        self.assertGreater(keyboard_calls[-1], pointer_buttons[-1])
        # Exactly one press and one release of the modifier key itself (plus one wl_keyboard.modifiers each).
        self.assertEqual(len(keyboard_calls), 4)

    def test_cmd_click_uses_the_super_key(self):
        client = self._client()
        client.click(10, 20, 100, 100, modifiers=["cmd"])
        keyboard_calls = [i for i, (object_id, _opcode) in enumerate(client.calls) if object_id == client.keyboard]
        self.assertEqual(len(keyboard_calls), 4)

    def test_no_modifiers_means_no_keyboard_traffic(self):
        client = self._client()
        client.click(10, 20, 100, 100, modifiers=[])
        self.assertTrue(all(object_id == client.pointer for object_id, _opcode in client.calls))


class SelectStepTests(unittest.TestCase):
    """run_lulo.LuloRun.run_steps' "select" branch threads modifiers/double
    through to click_item without needing a live AT-SPI tree or compositor."""

    def _run(self, step):
        run = object.__new__(run_lulo.LuloRun)
        run.scenario = {"steps": [{**step, "settle": 0}]}
        run.settle = 0
        run.ensure_alive = lambda: None
        calls = []
        run.click_item = lambda label, button, count=1, modifiers=None: calls.append((label, button, count, modifiers))
        run.run_steps(limit=1)
        return calls

    def test_plain_select_is_unchanged_a_single_left_click_no_modifiers(self):
        self.assertEqual(self._run({"select": "a.txt"}), [("a.txt", "left", 1, None)])

    def test_shift_click_select_passes_modifiers_through(self):
        self.assertEqual(self._run({"select": "c.txt", "modifiers": ["shift"]}),
                          [("c.txt", "left", 1, ["shift"])])

    def test_cmd_click_select_passes_modifiers_through(self):
        self.assertEqual(self._run({"select": "c.txt", "modifiers": ["cmd"]}),
                          [("c.txt", "left", 1, ["cmd"])])

    def test_double_select_sends_count_two(self):
        self.assertEqual(self._run({"select": "Projects", "double": True}),
                          [("Projects", "left", 2, None)])


class GuardTests(unittest.TestCase):
    def test_mac_background_context_point_requires_viewport_inside_our_window(self):
        self.assertEqual(record_mac.background_context_point((20, 30, 800, 600), (40, 100, 740, 500)), (756, 576))
        self.assertEqual(record_mac.background_context_point((20, 30, 800, 600), (40, 100, 800, 500)), (796, 576))
        with self.assertRaises(record_mac.Stop):
            record_mac.background_context_point((20, 30, 800, 600), (900, 100, 800, 500))

    def test_background_context_point_stays_inside_viewport(self):
        self.assertEqual(run_lulo.empty_viewport_point((100, 80, 600, 400), (20, 30)), (696, 486))
        with self.assertRaises(run_lulo.StepFailed):
            run_lulo.empty_viewport_point((0, 0, 48, 100), (0, 0))

    def test_background_context_targets_inner_folder_list(self):
        sidebar = (9, 53, 146, 738)
        outer_with_status_bar = (156, 52, 1124, 748)
        inner_folder_list = (156, 80, 1124, 692)
        self.assertEqual(
            run_lulo.content_viewport([sidebar, outer_with_status_bar, inner_folder_list]),
            inner_folder_list,
        )

    def test_calculator_sway_frame_normalization(self):
        self.assertEqual(run_lulo.calculator_visible_size(254, 432), (230, 408))
        self.assertEqual(run_lulo.calculator_visible_size(698, 432), (674, 408))
        self.assertEqual(run_lulo.calculator_visible_size(230, 406), (230, 406))
        self.assertEqual(run_lulo.calculator_visible_size(674, 406), (674, 406))
        self.assertEqual(run_lulo.calculator_visible_size(1280, 800), (1280, 800))

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
                           "DBUS_SESSION_BUS_ADDRESS": "unix:path=/run/user/1000/bus",
                           "LC_ALL": "C.UTF-8"})
        try:
            with tempfile.TemporaryDirectory() as work:
                env = run_lulo.isolated_environment(Path(work))
                self.assertNotIn("WAYLAND_DISPLAY", env)
                self.assertNotIn("DBUS_SESSION_BUS_ADDRESS", env)
                self.assertNotIn("LC_ALL", env)
                self.assertEqual(env["LANG"], "en_GB.UTF-8")
                self.assertTrue(env["XDG_RUNTIME_DIR"].startswith(work))
                self.assertTrue(env["HOME"].startswith(work))
                self.assertEqual(env["GSETTINGS_BACKEND"], "memory")
        finally:
            os.environ.clear()
            os.environ.update(saved)


class ScenarioFileTests(unittest.TestCase):
    def test_get_info_background_targets_empty_content_and_current_folder(self):
        path = sc.SCENARIO_ROOT / "files" / "get-info-background.json"
        scenario = sc.load(path)
        self.assertEqual(scenario["launch"], {"folder": "."})
        self.assertEqual(
            [(step.get("context"), step.get("select")) for step in scenario["steps"] if "context" in step or "select" in step],
            [("background", None), (None, "Get Info")],
        )
        self.assertEqual(scenario["steps"][-3], {"observe": "menu", "facts": ["menu"]})
        self.assertEqual(scenario["steps"][-1], {"observe": "info", "facts": ["dialog", "windows"]})
        expected = json.loads(sc.expectation_path(path).read_text())
        self.assertTrue(expected["observations"]["menu"]["menu"]["present"])
        self.assertIn("Get Info", expected["observations"]["menu"]["menu"]["items"])
        self.assertFalse(expected["observations"]["info"]["dialog"]["present"])
        self.assertEqual(expected["observations"]["info"]["windows"]["front"], "sandbox Info")

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

    def test_file_chooser_can_live_in_either_binary_directory(self):
        import tempfile

        with tempfile.TemporaryDirectory() as temporary:
            app_bins = Path(temporary) / "apps"
            helper_bins = Path(temporary) / "helpers"
            helper_bins.mkdir()
            binary = helper_bins / "rmac-file-chooser"
            binary.write_text("#!/bin/sh\nexit 0\n")
            binary.chmod(0o755)
            self.assertEqual(
                run_lulo.find_file_chooser_binary([app_bins, helper_bins]),
                binary.resolve(),
            )
            binary.chmod(0o644)
            self.assertIsNone(run_lulo.find_file_chooser_binary([app_bins, helper_bins]))

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

    def test_window_size_tolerance_allows_only_compositor_rounding(self):
        self.assertTrue(sc.field_matches("within-2", 408, 406))
        self.assertTrue(sc.field_matches("within-2", 408, 410))
        self.assertFalse(sc.field_matches("within-2", 408, 405))
        self.assertFalse(sc.field_matches("within-2", 408, None))

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

    def test_lulo_titles_drop_the_app_name_and_edited_mark(self):
        self.assertEqual(sc.lulo_window_title("sandbox — Files"), "sandbox")
        self.assertEqual(sc.lulo_window_title("Untitled — Edited — Text Editor"), "Untitled")
        self.assertEqual(sc.lulo_window_title("Quick Look"), "Quick Look")

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

    def test_compare_does_not_drop_results_without_scenarios_or_expectations(self):
        import tempfile

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "area").mkdir()
            (root / "area" / "no-expectation.json").write_text(json.dumps({
                "title": "No expectation", "app": "files",
                "steps": [{"observe": "window", "facts": ["windows"]}],
            }))
            evaluated = compare.evaluate({"results": [
                {"scenario": "area/unknown", "lulo": {}},
                {"scenario": "area/no-expectation", "lulo": {}},
            ]}, root)

        self.assertEqual([entry["status"] for entry in evaluated], ["fail", "fail"])
        self.assertEqual([entry["mismatches"][0]["actual"] for entry in evaluated], ["missing", "missing"])

    def test_compare_cli_rejects_an_empty_result_set(self):
        import contextlib
        import io
        import tempfile

        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "empty.json"
            path.write_text('{"results": []}', encoding="utf-8")
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(compare.main([str(path)]), 1)


if __name__ == "__main__":
    unittest.main()
