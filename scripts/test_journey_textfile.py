"""Pure-logic unit tests for scripts/linux/run-journey-textfile.py.

These run with plain `python3 -m pytest scripts/test_journey_textfile.py` on
macOS (no pyatspi, no niri, no live session required): they cover the
fixture-naming, JSON-parsing, and report-building logic only. The live
AT-SPI/niri orchestration in the script itself can only be exercised on the
reference Linux laptop; see docs/journey-suite.md.
"""

from __future__ import annotations

import importlib.util
import json
import sys
import unittest
from pathlib import Path


SCRIPT = Path(__file__).parent / "linux" / "run-journey-textfile.py"
SPEC = importlib.util.spec_from_file_location("run_journey_textfile", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
journey = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = journey
SPEC.loader.exec_module(journey)


class ModuleImportTests(unittest.TestCase):
    def test_imports_without_pyatspi(self):
        # macOS has no pyatspi; the module must still load.
        self.assertTrue(hasattr(journey, "pyatspi"))


class FixtureNamingTests(unittest.TestCase):
    def test_fixture_dirname_embeds_the_token(self):
        name = journey.fixture_dirname("ab12cd34")
        self.assertEqual(name, "lulo-journey-5-ab12cd34")

    def test_fixture_dirname_rejects_path_separators(self):
        for bad_token in ("../escape", "a/b", "a\\b", "a b", "a\tb", "a\nb", ""):
            with self.assertRaises(journey.JourneyError):
                journey.fixture_dirname(bad_token)


class FixtureContentTests(unittest.TestCase):
    def test_sample_content_is_deterministic_and_carries_the_token(self):
        first = journey.make_sample_content("tok123")
        second = journey.make_sample_content("tok123")
        self.assertEqual(first, second)
        self.assertIn(b"tok123", first)

    def test_sample_content_differs_for_different_tokens(self):
        self.assertNotEqual(
            journey.make_sample_content("tok-a"), journey.make_sample_content("tok-b")
        )

    def test_large_content_is_exactly_the_requested_size(self):
        content = journey.make_large_content("tok", 10_000)
        self.assertEqual(len(content), 10_000)
        self.assertTrue(content.startswith(b"rmac journey 5 large fixture tok\n"))

    def test_large_content_is_deterministic(self):
        self.assertEqual(
            journey.make_large_content("tok", 5_000),
            journey.make_large_content("tok", 5_000),
        )

    def test_large_content_handles_sizes_smaller_than_the_header(self):
        # Must not crash or produce a negative-length body.
        content = journey.make_large_content("a-fairly-long-token-value", 4)
        self.assertEqual(len(content), 4)


class AtomicWriteTempFileTests(unittest.TestCase):
    def test_temp_write_pattern_matches_rmac_storage(self):
        # crates/rmac-storage/src/write.rs's atomic_write names its sibling
        # temp file `.{name}.tmp-<pid>-<sequence>`.
        self.assertEqual(journey.temp_write_pattern("big.txt"), ".big.txt.tmp-")

    def test_find_orphaned_temp_files_matches_only_the_right_prefix(self):
        entries = [
            "big.txt",
            ".big.txt.tmp-4821-3",
            ".other.txt.tmp-4821-3",
            ".big.txt.tmp-9999-1",
            "big.txt.bak",
        ]
        found = journey.find_orphaned_temp_files(entries, "big.txt")
        self.assertEqual(sorted(found), [".big.txt.tmp-4821-3", ".big.txt.tmp-9999-1"])

    def test_find_orphaned_temp_files_empty_when_none_match(self):
        self.assertEqual(journey.find_orphaned_temp_files(["a", "b"], "big.txt"), [])


class NiriJsonParsingTests(unittest.TestCase):
    def test_parse_windows_accepts_an_array(self):
        windows = journey.parse_windows('[{"id": 1, "app_id": "org.rmac.TextEditor"}]')
        self.assertEqual(windows, [{"id": 1, "app_id": "org.rmac.TextEditor"}])

    def test_parse_windows_rejects_non_array(self):
        with self.assertRaises(journey.JourneyError):
            journey.parse_windows('{"not": "a list"}')

    def test_parse_windows_rejects_invalid_json(self):
        with self.assertRaises(journey.JourneyError):
            journey.parse_windows("not json")

    def test_find_window_by_app_id(self):
        windows = [
            {"id": 1, "app_id": "org.rmac.Notes"},
            {"id": 2, "app_id": "org.rmac.TextEditor"},
        ]
        found = journey.find_window_by_app_id(windows, "org.rmac.TextEditor")
        self.assertEqual(found["id"], 2)
        self.assertIsNone(journey.find_window_by_app_id(windows, "org.rmac.Missing"))


class HashTests(unittest.TestCase):
    def test_sha256_hex_is_stable_and_distinguishes_content(self):
        self.assertEqual(journey.sha256_hex(b"abc"), journey.sha256_hex(b"abc"))
        self.assertNotEqual(journey.sha256_hex(b"abc"), journey.sha256_hex(b"abd"))
        # Known SHA-256 of the empty byte string.
        self.assertEqual(
            journey.sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        )


class ReportShapeTests(unittest.TestCase):
    def test_overall_pass_requires_every_step(self):
        steps = [
            journey.make_step("a", True, "ok"),
            journey.make_step("b", False, "not ok"),
        ]
        report = journey.build_report(steps, [], 0)
        self.assertFalse(report["overall_pass"])

    def test_overall_pass_true_when_all_steps_pass(self):
        steps = [journey.make_step("a", True, "ok")]
        report = journey.build_report(steps, [], 0)
        self.assertTrue(report["overall_pass"])

    def test_report_matches_the_documented_schema(self):
        report = journey.build_report([], [], 12345)
        self.assertEqual(
            set(report),
            {
                "format",
                "journey",
                "journey_title",
                "started_at_unix_ms",
                "steps",
                "gaps",
                "overall_pass",
            },
        )
        self.assertEqual(report["journey"], 5)

    def test_report_is_json_serializable_and_privacy_safe(self):
        steps = [journey.make_step("save", True, "ok")]
        report = journey.build_report(steps, [], 0)
        text = json.dumps(report)
        for forbidden in ("/home/", "screencapture", ".png", ".jpg"):
            self.assertNotIn(forbidden, text)


class TextEditorConstantTests(unittest.TestCase):
    def test_text_editor_identity_is_well_formed(self):
        self.assertEqual(journey.TEXT_EDITOR["app_id"], "org.rmac.TextEditor")
        # Launched by desktop id (gtk-launch), never a hard-coded binary path
        # -- /usr/bin/rmac-* on the reference laptop is a stale package.
        self.assertEqual(journey.TEXT_EDITOR["desktop_id"], "org.rmac.TextEditor")


if __name__ == "__main__":
    unittest.main()
