"""Unit tests for the real-hardware GPUI A4 report contract."""

import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("a4-report.py")
SPEC = importlib.util.spec_from_file_location("a4_report", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
a4_report = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = a4_report
SPEC.loader.exec_module(a4_report)


def completed_report(status="pass"):
    return a4_report.report_template().replace("=pending\n", f"={status}\n")


class A4ReportTests(unittest.TestCase):
    def test_template_is_bounded_revision_bound_and_all_pending(self):
        report = a4_report.report_template()
        statuses = a4_report.parse_report(report)
        summary = a4_report.summarize(statuses)

        self.assertLessEqual(len(report.encode("ascii")), a4_report.MAX_REPORT_BYTES)
        self.assertIn(
            f"revision={a4_report.PINNED_UPSTREAM_REVISION}\n",
            report,
        )
        self.assertEqual(len(statuses), len(a4_report.REQUIRED_RESULTS))
        self.assertEqual(len(summary.pending), len(a4_report.REQUIRED_RESULTS))
        self.assertFalse(summary.recording_complete)
        self.assertFalse(summary.all_passed)
        self.assertEqual(a4_report.verification_exit_code(summary), 4)

    def test_complete_passing_report_is_distinct_from_complete_failure(self):
        passing = a4_report.summarize(a4_report.parse_report(completed_report()))
        self.assertTrue(passing.recording_complete)
        self.assertTrue(passing.all_passed)
        self.assertEqual(a4_report.verification_exit_code(passing), 0)

        failed_text = completed_report().replace(
            "result.niri.fullscreen-overview=pass\n",
            "result.niri.fullscreen-overview=fail\n",
        )
        failed = a4_report.summarize(a4_report.parse_report(failed_text))
        self.assertTrue(failed.recording_complete)
        self.assertFalse(failed.all_passed)
        self.assertEqual(failed.failed, ("niri.fullscreen-overview",))
        self.assertEqual(a4_report.verification_exit_code(failed), 5)

    def test_rejects_wrong_revision_missing_extra_duplicate_and_reordered_fields(self):
        report = a4_report.report_template()
        cases = (
            report.replace(a4_report.PINNED_UPSTREAM_REVISION, "0" * 40),
            report.replace("result.gnome.orca-nodes=pending\n", ""),
            report + "private.note=not-allowed\n",
            report.replace(
                "result.gnome.orca-nodes=pending\n",
                "result.gnome.orca-focus-order=pending\n",
            ),
            report.replace(
                "result.gnome.orca-nodes=pending\n"
                "result.gnome.orca-focus-order=pending\n",
                "result.gnome.orca-focus-order=pending\n"
                "result.gnome.orca-nodes=pending\n",
            ),
        )
        for case in cases:
            with self.subTest(case=case[-80:]):
                with self.assertRaises(a4_report.ReportError):
                    a4_report.parse_report(case)

    def test_rejects_unknown_status_non_ascii_missing_newline_and_oversize(self):
        report = a4_report.report_template()
        cases = (
            report.replace("result.environment.reviewed=pending", "result.environment.reviewed=yes"),
            report.replace("pending", "päss", 1),
            report.rstrip("\n"),
            report + ("x" * a4_report.MAX_REPORT_BYTES),
        )
        for case in cases:
            with self.subTest(case=case[-80:]):
                with self.assertRaises(a4_report.ReportError):
                    a4_report.parse_report(case)

    def test_report_contains_only_fixed_public_ids_not_evidence_content(self):
        report = a4_report.report_template()
        for private in (
            "/home/",
            "/Users/",
            "WAYLAND_DISPLAY",
            "XDG_SESSION_ID",
            "GPU0",
            "monitor serial",
        ):
            self.assertNotIn(private, report)

    def test_summary_rejects_wrong_cardinality(self):
        with self.assertRaises(a4_report.ReportError):
            a4_report.summarize(("pass",))

    def test_reader_accepts_regular_file_and_refuses_symlink_or_oversize(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            report = directory / "report.txt"
            report.write_text(completed_report(), encoding="ascii")
            self.assertEqual(
                a4_report.parse_report(a4_report.read_report(report)),
                ("pass",) * len(a4_report.REQUIRED_RESULTS),
            )

            link = directory / "report-link.txt"
            link.symlink_to(report)
            with self.assertRaises(a4_report.ReportError):
                a4_report.read_report(link)

            oversized = directory / "oversized.txt"
            oversized.write_bytes(b"x" * (a4_report.MAX_REPORT_BYTES + 1))
            with self.assertRaises(a4_report.ReportError):
                a4_report.read_report(oversized)


if __name__ == "__main__":
    unittest.main()
