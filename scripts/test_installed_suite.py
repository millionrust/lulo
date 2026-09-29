"""Focused checks for installed-suite report accounting."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).parent / "behavior" / "run_installed_suite.py"
SPEC = importlib.util.spec_from_file_location("run_installed_suite", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
suite = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = suite
SPEC.loader.exec_module(suite)


class InstalledSuiteReportTests(unittest.TestCase):
    def test_installed_inventory_requires_dpkg_owned_package_locations(self):
        app_dir = Path("/usr/bin").resolve()
        shell_dir = Path("/usr/libexec/rmac").resolve()
        rows = [
            {"path": str(app_dir / "rmac-files"), "sha256": "a" * 64,
             "package_owner": f"rmac-apps: {app_dir / 'rmac-files'}"},
            {"path": str(shell_dir / "rmac-top-bar"), "sha256": "b" * 64,
             "package_owner": f"rmac-session: {shell_dir / 'rmac-top-bar'}"},
        ]
        self.assertTrue(suite.installed_inventory_valid(rows, [app_dir], [shell_dir]))
        self.assertFalse(suite.installed_inventory_valid(rows, [Path("/tmp/build")], [shell_dir]))
        self.assertFalse(suite.installed_inventory_valid(rows, [app_dir], [Path("/tmp/build")]))
        self.assertFalse(suite.installed_inventory_valid(
            [{**rows[0], "package_owner": "unowned"}, rows[1]], [app_dir], [shell_dir]
        ))

    def test_package_versions_identify_installed_build_not_runner_source(self):
        def dpkg_version(argv, **_kwargs):
            return subprocess.CompletedProcess(argv, 0, "0.9.0~beta.1-38", "")

        with patch.object(suite.subprocess, "run", side_effect=dpkg_version) as run:
            self.assertEqual(suite.installed_package_versions(), {
                "rmac-apps": "0.9.0~beta.1-38",
                "rmac-session": "0.9.0~beta.1-38",
            })
        self.assertEqual(run.call_count, 2)
        with patch.object(suite.subprocess, "run", return_value=subprocess.CompletedProcess(
            ["dpkg-query"], 1, "", "not installed"
        )):
            with self.assertRaisesRegex(RuntimeError, "version unavailable"):
                suite.installed_package_versions()

    def test_installed_package_files_must_pass_dpkg_verification(self):
        command = ["dpkg", "--verify", "rmac-apps", "rmac-session"]
        with patch.object(suite.subprocess, "run", return_value=subprocess.CompletedProcess(
            command, 0, "", ""
        )) as run:
            self.assertTrue(suite.installed_packages_verified())
            run.assert_called_once_with(command, text=True, capture_output=True, check=False)
        for status, output in ((1, "??5?????? /usr/bin/rmac-files\n"), (0, "unexpected output")):
            with self.subTest(status=status, output=output):
                with patch.object(suite.subprocess, "run", return_value=subprocess.CompletedProcess(
                    command, status, output, ""
                )):
                    self.assertFalse(suite.installed_packages_verified())

    def test_startup_artifact_requires_every_app_and_installed_binary_hash(self):
        expected = {"archive-utility": "rmac-archive-utility", "files": "rmac-files"}
        inventory = [
            {"path": "/usr/bin/rmac-archive-utility", "sha256": "a" * 64},
            {"path": "/usr/bin/rmac-files", "sha256": "b" * 64},
        ]
        rows = [
            {"app": "archive-utility", "binary": "rmac-archive-utility", "binary_sha256": "a" * 64,
             "outcome": "passed", "readiness": "fixture_extracted"},
            {"app": "files", "binary": "rmac-files", "binary_sha256": "b" * 64,
             "outcome": "passed", "readiness": "mapped_and_accessible"},
        ]
        report = {"results": rows, "summary": {"passed": 2, "failed": 0, "skipped": 0, "total": 2}}
        self.assertTrue(suite.startup_report_complete(
            report, inventory, Path("/usr/bin"), expected
        ))
        for bad_rows in (
            [rows[0]], [rows[0], rows[0]],
            [rows[0], {**rows[1], "binary_sha256": "c" * 64}],
            [rows[0], {**rows[1], "readiness": "window_mapped"}],
        ):
            with self.subTest(rows=bad_rows):
                self.assertFalse(suite.startup_report_complete(
                    {**report, "results": bad_rows}, inventory, Path("/usr/bin"), expected
                ))
        self.assertFalse(suite.startup_report_complete(
            {**report, "summary": {"passed": 2, "failed": 0, "skipped": 1, "total": 2}},
            inventory, Path("/usr/bin"), expected,
        ))

    def test_startup_hash_must_match_the_directory_used_by_smoke(self):
        expected = {"files": "rmac-files"}
        inventory = [
            {"path": "/opt/apps/rmac-files", "sha256": "a" * 64},
            {"path": "/usr/libexec/rmac/rmac-files", "sha256": "b" * 64},
        ]
        report = {
            "results": [{
                "app": "files", "binary": "rmac-files", "binary_sha256": "b" * 64,
                "outcome": "passed", "readiness": "mapped_and_accessible",
            }],
            "summary": {"passed": 1, "failed": 0, "skipped": 0, "total": 1},
        }
        self.assertFalse(suite.startup_report_complete(
            report, inventory, Path("/opt/apps"), expected
        ))
        self.assertTrue(suite.startup_report_complete(
            report, inventory, Path("/usr/libexec/rmac"), expected
        ))

    def test_behavior_artifact_requires_exact_recorded_scenario_ids(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for identifier in ("files/open", "notes/edit"):
                path = root / f"{identifier}.json"
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("{}", encoding="utf-8")
                path.with_name(f"{path.stem}.mac.json").write_text("{}", encoding="utf-8")
            good = {"results": [
                {"scenario": "files/open", "status": "pass", "mismatches": [], "lulo": {"scenario": "files/open"}},
                {"scenario": "notes/edit", "status": "pass", "mismatches": [], "lulo": {"scenario": "notes/edit"}},
            ]}
            self.assertTrue(suite.behavior_report_complete(good, root, 2))
            duplicate = {"results": [good["results"][0], good["results"][0]]}
            self.assertFalse(suite.behavior_report_complete(duplicate, root, 2))
            missing = {"results": [good["results"][0]]}
            self.assertFalse(suite.behavior_report_complete(missing, root, 2))
            unknown = {"results": [good["results"][0], {"scenario": "other", "status": "pass", "mismatches": [], "lulo": {"scenario": "other"}}]}
            self.assertFalse(suite.behavior_report_complete(unknown, root, 2))
            failed = {"results": [good["results"][0], {"scenario": "notes/edit", "status": "fail", "mismatches": [], "lulo": {"scenario": "notes/edit"}}]}
            self.assertFalse(suite.behavior_report_complete(failed, root, 2))
            mislabeled = {"results": [good["results"][0], {"scenario": "notes/edit", "status": "pass", "mismatches": [], "lulo": {"scenario": "files/open"}}]}
            self.assertFalse(suite.behavior_report_complete(mislabeled, root, 2))
            contradictory = {"results": [good["results"][0], {**good["results"][1], "mismatches": ["difference"]}]}
            self.assertFalse(suite.behavior_report_complete(contradictory, root, 2))
            self.assertFalse(suite.behavior_report_complete(good, root, 27))

    def test_source_fingerprint_covers_terminal_probe_helper(self):
        self.assertIn("scripts/linux/run-journey-terminal.py", suite.SOURCE_INPUTS)

    def test_run_step_captures_output_and_reports_success(self):
        with tempfile.TemporaryDirectory() as directory:
            result = suite.run_step(
                "startup-smoke", [sys.executable, "-c", "print('ready')"],
                Path(directory), timeout_seconds=2,
            )
            self.assertEqual(result["exit_code"], 0)
            self.assertFalse(result["timed_out"])
            self.assertEqual(
                (Path(directory) / "startup-smoke.stdout.txt").read_text().strip(),
                "ready",
            )

    def test_run_step_times_out_and_stops_its_child_group(self):
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / "late-child-marker"
            child = f"import time, pathlib; time.sleep(0.5); pathlib.Path({str(marker)!r}).touch()"
            parent = (
                "import subprocess, sys, time; "
                f"subprocess.Popen([sys.executable, '-c', {child!r}]); "
                "time.sleep(60)"
            )
            result = suite.run_step(
                "notes-recovery", [sys.executable, "-c", parent],
                Path(directory), timeout_seconds=0.2,
            )
            self.assertEqual(result["exit_code"], 124)
            self.assertTrue(result["timed_out"])
            time.sleep(0.6)
            self.assertFalse(marker.exists())

    def test_run_step_stops_children_left_after_successful_parent_exit(self):
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / "late-child-marker"
            child = f"import time, pathlib; time.sleep(0.5); pathlib.Path({str(marker)!r}).touch()"
            parent = (
                "import subprocess, sys; "
                f"subprocess.Popen([sys.executable, '-c', {child!r}])"
            )
            result = suite.run_step(
                "notes-recovery", [sys.executable, "-c", parent],
                Path(directory), timeout_seconds=2,
            )
            self.assertEqual(result["exit_code"], 0)
            self.assertFalse(result["timed_out"])
            time.sleep(0.6)
            self.assertFalse(marker.exists())

    def test_partial_single_phase_report_counts_other_phases_as_not_run(self):
        results = [
            {"name": "behavior-27", "exit_code": 1},
            {"name": "shutdown-completion", "exit_code": 0},
        ]
        self.assertEqual(suite.count_not_run(results), 4)

    def test_cumulative_report_counts_each_phase_once(self):
        results = [{"name": name, "exit_code": 0} for name in suite.PHASE_NAMES]
        results.append({"name": "shutdown-completion", "exit_code": 0})
        self.assertEqual(suite.count_not_run(results), 0)

    def test_skipped_shell_phases_remain_unrun(self):
        results = [
            {"name": "behavior-27", "exit_code": 0},
            {"name": "startup-smoke", "exit_code": 0},
            {"name": "terminal-roundtrip", "exit_code": 0},
        ]
        self.assertEqual(suite.count_not_run(results), 3)

    def test_notes_recovery_is_required_for_complete_coverage(self):
        results = [
            {"name": name, "exit_code": 0}
            for name in suite.PHASE_NAMES if name != "notes-recovery"
        ]
        self.assertEqual(suite.count_not_run(results), 1)

    def test_source_fingerprint_changes_when_scenario_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "tests" / "behavior" / "example.json"
            source.parent.mkdir(parents=True)
            source.write_text('{"steps": []}\n', encoding="utf-8")
            baseline = suite.source_inputs_sha256(root, ("tests/behavior",))

            source.write_text('{"steps": [{"key": "return"}]}\n', encoding="utf-8")
            changed = suite.source_inputs_sha256(root, ("tests/behavior",))
            self.assertNotEqual(baseline, changed)

    def test_source_fingerprint_includes_shell_configuration(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "packaging/rmac-session/shell.kdl"
            source.parent.mkdir(parents=True)
            source.write_text("output * scale 1\n", encoding="utf-8")
            baseline = suite.source_inputs_sha256(root, ("packaging/rmac-session/shell.kdl",))
            source.write_text("output * scale 2\n", encoding="utf-8")
            changed = suite.source_inputs_sha256(root, ("packaging/rmac-session/shell.kdl",))
            self.assertNotEqual(baseline, changed)

    def test_inventory_covers_every_rmac_executable_and_detects_mutation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            app_dir = root / "bin"
            shell_dir = root / "libexec"
            app_dir.mkdir()
            shell_dir.mkdir()
            for path in (app_dir / "rmac-files", shell_dir / "rmac-mission-control",
                         shell_dir / "rmac-notification-center"):
                path.write_bytes(b"binary")
                path.chmod(0o755)
            ignored = shell_dir / "rmac-unowned"
            ignored.write_bytes(b"other")
            ignored.chmod(0o755)
            (shell_dir / "rmac-not-executable").write_bytes(b"data")

            def owner_for(command, **_kwargs):
                path = command[-1]
                package = "rmac-apps" if path.endswith("/bin/rmac-files") else "rmac-session"
                if path.endswith("rmac-unowned"):
                    return type("Result", (), {"returncode": 1, "stdout": ""})()
                return type("Result", (), {"returncode": 0, "stdout": f"{package}: {path}\n"})()

            with patch.object(suite.subprocess, "run", side_effect=owner_for):
                inventory = suite.binary_inventory([app_dir, shell_dir])
            self.assertEqual(
                {Path(item["path"]).name for item in inventory},
                {"rmac-files", "rmac-mission-control", "rmac-notification-center", "rmac-unowned"},
            )
            unowned = next(item for item in inventory if Path(item["path"]).name == "rmac-unowned")
            self.assertEqual(unowned["package_owner"], "unowned")
            before = {item["path"]: item["sha256"] for item in inventory}
            (shell_dir / "rmac-mission-control").write_bytes(b"mutated binary")
            with patch.object(suite.subprocess, "run", side_effect=owner_for):
                after_inventory = suite.binary_inventory([app_dir, shell_dir])
            after = {item["path"]: item["sha256"] for item in after_inventory}
            self.assertNotEqual(before, after)

    def test_unknown_and_duplicate_phase_rows_fail_accounting(self):
        results = [
            {"name": "behavior-27", "exit_code": 0},
            {"name": "behavior-27", "exit_code": 0},
            {"name": "invented-phase", "exit_code": 0},
        ]
        self.assertEqual(suite.report_accounting(results), (0, 2, 5))

    def test_single_failed_phase_is_counted_as_failed(self):
        results = [{"name": "notes-recovery", "exit_code": 1}]
        self.assertEqual(suite.report_accounting(results), (0, 1, 5))

    def test_boolean_exit_code_cannot_claim_a_pass(self):
        results = [{"name": "notes-recovery", "exit_code": False}]
        self.assertEqual(suite.report_accounting(results), (0, 1, 5))


if __name__ == "__main__":
    unittest.main()
