"""Pure-logic unit tests for scripts/behavior/monkey.py.

These run with plain `python3 -m pytest scripts/test_monkey.py` on macOS (no
/proc, no niri, no AT-SPI, no live compositor required): they cover the
sandbox fixture, the shortcut-inventory flattening, random text generation,
the shrink bisection algorithm, and the markdown/JSON report writer. The
live monkey session (launching an app in the nested niri+shell and driving
it) can only be exercised on the reference Linux laptop; see
docs/behavior-suite.md and AGENT-BRIEF.md.
"""

from __future__ import annotations

import importlib.util
import json
import sys
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

sys.path.insert(0, str(Path(__file__).parent / "behavior"))
import wlinput  # noqa: E402

SCRIPT = Path(__file__).parent / "behavior" / "monkey.py"
SPEC = importlib.util.spec_from_file_location("monkey", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
monkey = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = monkey
SPEC.loader.exec_module(monkey)


class FindBinaryTests(unittest.TestCase):
    def test_finds_first_matching_name_in_first_directory(self):
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "a").mkdir()
            (root / "b").mkdir()
            target = root / "b" / "rmac-files"
            target.write_text("#!/bin/sh\n")
            target.chmod(0o755)
            found = monkey.find_binary([root / "a", root / "b"], ["rmac-files"])
            self.assertEqual(found, target.resolve())

    def test_returns_none_when_missing(self):
        with TemporaryDirectory() as tmp:
            self.assertIsNone(monkey.find_binary([Path(tmp)], ["nope"]))

    def test_skips_non_executable(self):
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            target = root / "rmac-files"
            target.write_text("not executable\n")
            self.assertIsNone(monkey.find_binary([root], ["rmac-files"]))


class SandboxTests(unittest.TestCase):
    def test_builds_expected_fixture(self):
        with TemporaryDirectory() as tmp:
            root = Path(tmp) / "sandbox"
            monkey.build_sandbox(root)
            self.assertTrue((root / "notes.txt").read_text())
            self.assertEqual((root / "empty.txt").stat().st_size, 0)
            self.assertGreater((root / "large.bin").stat().st_size, 10 * 1024 * 1024)
            self.assertTrue((root / "picture.png").read_bytes().startswith(b"\x89PNG"))
            self.assertTrue((root / "document.pdf").read_bytes().startswith(b"%PDF"))
            self.assertTrue((root / "Folder" / "Nested Folder").is_dir())
            self.assertTrue((root / "Folder" / "inside.txt").is_file())
            long_names = [p for p in root.iterdir() if len(p.name) > 100]
            self.assertTrue(long_names, "expected a long file name in the fixture")
            unicode_names = [p for p in root.iterdir() if "café" in p.name]
            self.assertTrue(unicode_names, "expected a unicode file name in the fixture")

    def test_is_deterministic_across_calls(self):
        # Shrink replay relies on the fixture not depending on the seed.
        with TemporaryDirectory() as tmp:
            first, second = Path(tmp) / "one", Path(tmp) / "two"
            monkey.build_sandbox(first)
            monkey.build_sandbox(second)
            self.assertEqual(sorted(p.name for p in first.iterdir()), sorted(p.name for p in second.iterdir()))


class ShortcutInventoryTests(unittest.TestCase):
    def test_loads_calculator_shortcuts(self):
        shortcuts = monkey.load_shortcuts("calculator")
        self.assertTrue(shortcuts, "Calculator.json should yield at least one shortcut")
        labels = [label for label, _chord in shortcuts]
        self.assertTrue(any("Quit" in label for label in labels))
        chords = [chord for _label, chord in shortcuts]
        self.assertIn("⌘Q", chords)

    def test_unknown_app_returns_empty(self):
        self.assertEqual(monkey.load_shortcuts("shell"), [])

    def test_system_monitor_never_uses_quit_process_shortcut(self):
        labels = [label for label, _shortcut in monkey.load_shortcuts("system-monitor")]
        self.assertFalse(any("Quit Process" in label for label in labels))

    def test_shortcut_to_chord_parses_mac_glyphs(self):
        self.assertEqual(monkey.shortcut_to_chord("⌘Q"), "⌘Q")

    def test_shortcut_to_chord_rejects_garbage(self):
        self.assertIsNone(monkey.shortcut_to_chord("not-a-real-shortcut-!@#"))


class RandomTextTests(unittest.TestCase):
    def test_deterministic_for_a_given_seed(self):
        import random

        a = monkey.random_text(random.Random(1))
        b = monkey.random_text(random.Random(1))
        self.assertEqual(a, b)

    def test_sanitize_drops_unmappable_glyphs(self):
        text = monkey.sanitize_for_typing("hi \U0001F389 bye")
        self.assertNotIn("\U0001F389", text)
        self.assertIn("hi", text)
        self.assertIn("bye", text)

    def test_sanitize_drops_tab_even_though_it_is_ascii(self):
        # A regression check: plain `char.isascii()` is true for tab and
        # other C0 controls, but wlinput's text_to_strokes has no keycode
        # for a literal "\t" and raises -- sanitize must filter it too.
        text = monkey.sanitize_for_typing("a\tb")
        self.assertNotIn("\t", text)
        self.assertEqual(text, "ab")

    def test_every_character_sanitize_keeps_is_actually_typeable(self):
        # Every character that survives sanitize_for_typing must be one
        # wlinput.text_to_strokes accepts -- the whole point of sanitizing.
        sample = "".join(chr(c) for c in range(0, 1200)) + monkey._UNICODE_BITS + "\n"
        kept = monkey.sanitize_for_typing(sample)
        for char in kept:
            if char == "\n":
                continue
            wlinput.text_to_strokes(char)  # must not raise


class ShrinkBisectionTests(unittest.TestCase):
    """The bisection itself, with `reproduces` stubbed out: a real shrink
    run needs a live app and compositor, but the search algorithm (find the
    shortest prefix that still reproduces the finding) is pure and must be
    tested without one."""

    def test_finds_minimal_failing_prefix(self):
        actions = [monkey.ActionRecord(index=i, kind="click-point", params={}) for i in range(20)]
        threshold = 7  # any prefix >= 7 actions "reproduces"

        original = monkey.reproduces
        monkey.reproduces = lambda _monkey, _actions, count, _kind, _since: count >= threshold
        try:
            minimal = monkey.shrink(None, actions, len(actions), "crash", 0.0, lambda _msg: None)
        finally:
            monkey.reproduces = original
        self.assertEqual(minimal, threshold)

    def test_already_minimal(self):
        actions = [monkey.ActionRecord(index=0, kind="click-point", params={})]
        original = monkey.reproduces
        monkey.reproduces = lambda *_args, **_kwargs: True
        try:
            minimal = monkey.shrink(None, actions, 1, "crash", 0.0, lambda _msg: None)
        finally:
            monkey.reproduces = original
        self.assertEqual(minimal, 1)

    def test_does_not_claim_minimal_repro_when_full_log_fails(self):
        actions = [monkey.ActionRecord(index=0, kind="click-point", params={})]
        original = monkey.reproduces
        monkey.reproduces = lambda *_args, **_kwargs: False
        try:
            minimal = monkey.shrink(None, actions, 1, "crash", 0.0, lambda _msg: None)
        finally:
            monkey.reproduces = original
        self.assertIsNone(minimal)


class JsonableTests(unittest.TestCase):
    def test_rounds_floats_and_recurses(self):
        value = monkey._jsonable({"a": 1.23456, "b": [1.0, {"c": 2.0001}], "d": "text"})
        self.assertEqual(value, {"a": 1.235, "b": [1.0, {"c": 2.0}], "d": "text"})


class ReportTests(unittest.TestCase):
    def test_writes_markdown_and_action_log(self):
        with TemporaryDirectory() as tmp:
            findings_dir = Path(tmp) / "findings"
            actions = [
                monkey.ActionRecord(index=0, kind="click-point", params={"xy": (1.0, 2.0)}),
                monkey.ActionRecord(index=1, kind="shortcut", params={"chord": "⌘Q"}, note="Application > Quit"),
            ]
            finding = monkey.Finding(kind="crash", detail="process exited with 134", action_count=2,
                                     evidence={"log_tail": "panicked at 'x'"})
            report = monkey.write_report(findings_dir, "calculator", 42, finding, actions, 2,
                                         "panicked at 'x'\n", None)
            self.assertTrue(report.exists())
            text = report.read_text()
            self.assertIn("calculator", text)
            self.assertIn("crash", text)
            self.assertIn("Seed: 42", text)
            self.assertIn("--replay", text)
            actions_path = findings_dir / f"{report.stem}.actions.json"
            self.assertTrue(actions_path.exists())
            logged = json.loads(actions_path.read_text())
            self.assertEqual(len(logged), 2)
            self.assertEqual(logged[1]["note"], "Application > Quit")
            # Never written inside the git-tracked tree.
            repo_root = Path(__file__).resolve().parents[1]
            self.assertFalse(str(findings_dir.resolve()).startswith(str(repo_root)))


class FindingClassificationTests(unittest.TestCase):
    def test_replay_resets_home_but_normal_relaunch_preserves_it(self):
        with TemporaryDirectory() as tmp:
            subject = monkey.Monkey.__new__(monkey.Monkey)
            subject.home = Path(tmp) / "home"
            subject.home.mkdir()
            subject.sandbox = subject.home / "sandbox"
            subject.sandbox.mkdir()
            marker = subject.home / "note-state.db"
            marker.write_text("saved")
            subject._mem_samples = []
            subject.stop = lambda: None
            subject.launch = lambda: None
            subject.relaunch()
            self.assertTrue(marker.exists())
            subject.relaunch(reset_state=True)
            self.assertFalse(marker.exists())

    def test_startup_work_is_not_classified_as_idle_cpu(self):
        import time

        subject = monkey.Monkey.__new__(monkey.Monkey)
        subject._launched_at = time.monotonic()
        subject.sample = lambda: self.fail("sampled startup as idle")
        self.assertIsNone(subject.check_idle_cpu())

    def test_clean_quit_is_not_a_crash(self):
        class Process:
            returncode = 0

            def poll(self):
                return 0

        subject = monkey.Monkey.__new__(monkey.Monkey)
        subject.app = "notes"
        subject.process = Process()
        self.assertIsNone(subject.check_crashed())

    def test_abnormal_exit_is_a_crash(self):
        class Process:
            returncode = 134

            def poll(self):
                return 134

        subject = monkey.Monkey.__new__(monkey.Monkey)
        subject.app = "notes"
        subject.process = Process()
        subject.tail_log = lambda: "panicked at example"
        finding = subject.check_crashed()
        self.assertEqual(finding.kind, "crash")
        self.assertIn("134", finding.detail)

    def test_hidden_window_with_stale_showing_frame_is_not_stuck(self):
        class Process:
            def poll(self):
                return None

        subject = monkey.Monkey.__new__(monkey.Monkey)
        subject.app = "notes"
        subject.process = Process()
        subject._window_missing_since = None
        subject.app_window = lambda: None
        subject.frames = lambda: [object()]
        original_atspi, original_has_state = monkey.run_lulo.atspi, monkey.run_lulo.has_state
        monkey.run_lulo.atspi = lambda: type("States", (), {"STATE_SHOWING": 1})()
        monkey.run_lulo.has_state = lambda _frame, _state: True
        try:
            self.assertIsNone(subject.check_stuck_window())
        finally:
            monkey.run_lulo.atspi, monkey.run_lulo.has_state = original_atspi, original_has_state


if __name__ == "__main__":
    unittest.main()
