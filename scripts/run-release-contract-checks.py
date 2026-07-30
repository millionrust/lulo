#!/usr/bin/env python3
"""Run every lightweight rmac release-contract check without a Rust build."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile


REPO_ROOT = Path(__file__).resolve().parents[1]
SUITE_PATH = REPO_ROOT / "scripts/release-contract-suite.json"
MAX_MANIFEST_BYTES = 256 * 1024
MAX_FAILURE_OUTPUT_BYTES = 64 * 1024
STAGES = (
    (
        "focused-fixtures",
        (
            "$PYTHON",
            "-m",
            "unittest",
            "scripts.test_reference_preflight",
            "scripts.test_application_icons",
            "scripts.test_application_package",
            "scripts.test_flatpak_package",
            "scripts.test_native_packages",
            "scripts.test_session_package",
            "scripts.test_update_trust",
            "scripts.test_apt_publisher",
            "scripts.test_hardware_matrix",
            "scripts.test_journey_suite",
            "scripts.test_measure_baseline",
            "scripts.test_foundation_evidence",
            "scripts.test_visual_suite",
            "scripts.test_accessibility_audit",
            "scripts.test_performance_audit",
            "scripts.test_chaos_soak",
            "scripts.test_security_review",
            "scripts.test_documentation",
            "scripts.test_release_contract_checks",
            "scripts.test_alpha_candidate",
            "scripts.test_beta_candidate",
            "scripts.test_one_dot_zero_candidate",
        ),
    ),
    (
        "a4-report-fixtures",
        ("$PYTHON", "experiments/gpui-upstream-lab/scripts/test_a4_report.py"),
    ),
    ("update-trust", ("$PYTHON", "scripts/linux/verify-update-trust.py")),
    (
        "apt-publisher",
        (
            "$PYTHON",
            "scripts/linux/publish-apt-snapshot.py",
            "--check-contract",
        ),
    ),
    ("hardware-matrix", ("$PYTHON", "scripts/linux/verify-hardware-matrix.py")),
    ("foundation-evidence", ("$PYTHON", "scripts/verify-foundation-evidence.py")),
    ("journey-manifest", ("$PYTHON", "scripts/run-journey-suite.py", "--list")),
    ("visual-suite", ("$PYTHON", "scripts/verify-visual-suite.py")),
    ("accessibility-audit", ("$PYTHON", "scripts/verify-accessibility-audit.py")),
    ("performance-audit", ("$PYTHON", "scripts/verify-performance-audit.py")),
    ("chaos-soak", ("$PYTHON", "scripts/verify-chaos-soak.py")),
    ("security-review", ("$PYTHON", "scripts/verify-security-review.py")),
    ("documentation", ("$PYTHON", "scripts/verify-documentation.py")),
    ("alpha-candidate", ("$PYTHON", "scripts/verify-alpha-candidate.py")),
    ("beta-candidate", ("$PYTHON", "scripts/verify-beta-candidate.py")),
    (
        "one-dot-zero-candidate",
        ("$PYTHON", "scripts/verify-one-dot-zero-candidate.py"),
    ),
)


class ContractSuiteError(RuntimeError):
    """A bounded release-contract suite failure."""


def _read_regular(path: Path) -> bytes:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise ContractSuiteError("release-contract suite manifest is unavailable") from error
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode):
        raise ContractSuiteError("release-contract suite manifest is not regular")
    if metadata.st_size > MAX_MANIFEST_BYTES:
        raise ContractSuiteError("release-contract suite manifest is too large")
    try:
        raw = path.read_bytes()
    except OSError as error:
        raise ContractSuiteError("release-contract suite manifest cannot be read") from error
    if len(raw) != metadata.st_size:
        raise ContractSuiteError("release-contract suite manifest changed while reading")
    return raw


def load_suite(path: Path = SUITE_PATH) -> dict[str, object]:
    try:
        document = json.loads(_read_regular(path))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ContractSuiteError("release-contract suite JSON is invalid") from error
    expected = {
        "format": 1,
        "minimum_free_gib": 15,
        "stages": [
            {"command": list(command), "id": stage}
            for stage, command in STAGES
        ],
        "timeout_seconds_per_stage": 60,
    }
    if document != expected:
        raise ContractSuiteError("release-contract suite differs from the reviewed inventory")
    return document


def _resolved_command(command: list[str]) -> list[str]:
    return [sys.executable if value == "$PYTHON" else value for value in command]


def run_stage(stage: dict[str, object], timeout_seconds: int) -> None:
    command = _resolved_command(stage["command"])
    environment = os.environ.copy()
    environment["PYTHONDONTWRITEBYTECODE"] = "1"
    with tempfile.TemporaryFile() as output:
        try:
            completed = subprocess.run(
                command,
                cwd=REPO_ROOT,
                env=environment,
                stdin=subprocess.DEVNULL,
                stdout=output,
                stderr=subprocess.STDOUT,
                timeout=timeout_seconds,
                check=False,
            )
        except (OSError, subprocess.TimeoutExpired) as error:
            raise ContractSuiteError(
                f"release-contract stage could not complete: {stage['id']}"
            ) from error
        if completed.returncode == 0:
            return
        size = output.tell()
        output.seek(max(0, size - MAX_FAILURE_OUTPUT_BYTES))
        failure = output.read(MAX_FAILURE_OUTPUT_BYTES).decode("utf-8", "replace").strip()
        suffix = f"\n{failure}" if failure else ""
        raise ContractSuiteError(
            f"release-contract stage failed: {stage['id']} "
            f"(status {completed.returncode}){suffix}"
        )


def run_suite(suite: dict[str, object]) -> None:
    minimum = suite["minimum_free_gib"] * 1024**3
    if shutil.disk_usage(REPO_ROOT).free < minimum:
        raise ContractSuiteError("release-contract suite stopped below the 15 GiB floor")
    for index, stage in enumerate(suite["stages"], start=1):
        print(f"[{index:02d}/{len(suite['stages']):02d}] {stage['id']}", flush=True)
        run_stage(stage, suite["timeout_seconds_per_stage"])


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--list", action="store_true")
    arguments = parser.parse_args()
    try:
        suite = load_suite()
        if arguments.list:
            for stage in suite["stages"]:
                print(stage["id"])
            return 0
        run_suite(suite)
    except ContractSuiteError as error:
        parser.exit(4, f"run-release-contract-checks: {error}\n")
    print(f"rmac release contracts passed ({len(suite['stages'])} stages)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
