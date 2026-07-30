#!/usr/bin/env python3
"""Verify the privacy-safe A1–A4 Linux foundation evidence bundle."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import stat
import sys


REPO_ROOT = Path(__file__).resolve().parents[1]
CONTRACT_PATH = REPO_ROOT / "scripts/foundation-evidence.json"
MAX_BYTES = 1024 * 1024
UPSTREAM_REVISION = "76c93968da5b8b8809bdd72e4ad9e7d0e946bad0"
BUNDLE_FILES = (
    "a4-upstream-report.txt",
    "foundation-summary.json",
    "gnome-platform-lab.txt",
    "niri-platform-lab.txt",
)
PHASE_CHECKS = {
    "a1": (
        "gnome-wayland-session",
        "hardware-vulkan",
        "multi-monitor-and-scale",
        "platform-lab-launch",
        "preflight-pass",
        "privacy-review",
        "reviewed-environment-evidence",
        "suspend-resume",
    ),
    "a3": (
        "four-hour-soak",
        "gnome-session-retained",
        "niri-enabled-output",
        "niri-portal-routing",
        "niri-preflight-pass",
        "privacy-review",
        "rmac-session-services",
        "stable-platform-lab-pass",
    ),
}
PROBES = (
    "window",
    "input",
    "clipboard",
    "file-dialog",
    "file-drop",
    "scroll-scale",
    "display-lifecycle",
    "keyboard-focus",
    "suspend-resume",
    "idle",
)
BLOCKERS = ("accessibility", "layer-shell")
A4_RESULTS = (
    "environment.reviewed",
    "automation.wayland-clippy",
    "automation.nested-smoke-live-revision",
    "gnome.orca-nodes",
    "gnome.orca-focus-order",
    "gnome.orca-actions-state",
    "gnome.scale-100",
    "gnome.scale-125",
    "gnome.scale-150",
    "gnome.scale-200",
    "gnome.mixed-scale-move",
    "niri.layer-placement",
    "niri.layer-exclusive-zone",
    "niri.layer-keyboard-noninterference",
    "niri.topbar-per-output",
    "niri.topbar-orca-semantics",
    "niri.output-hotplug",
    "niri.fractional-scale",
    "niri.fullscreen-overview",
    "niri.idle-redraw",
    "niri.interaction-30m",
    "niri.soak-4h",
    "privacy.reviewed-evidence",
)
SUPPORTING_EVIDENCE = (
    "a1-gnome",
    "a2-gnome",
    "a2-niri",
    "a3-niri",
    "a4-upstream",
)


class FoundationError(RuntimeError):
    """A bounded Linux-foundation evidence failure."""


def _read_regular(path: Path) -> bytes:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise FoundationError(f"required foundation file is unavailable: {path.name}") from error
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode):
        raise FoundationError(f"required foundation path is not regular: {path.name}")
    if metadata.st_size > MAX_BYTES:
        raise FoundationError(f"required foundation file is too large: {path.name}")
    try:
        raw = path.read_bytes()
    except OSError as error:
        raise FoundationError(f"required foundation file cannot be read: {path.name}") from error
    if len(raw) != metadata.st_size:
        raise FoundationError(f"required foundation file changed while reading: {path.name}")
    return raw


def _load_json(path: Path) -> object:
    try:
        return json.loads(_read_regular(path))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise FoundationError(f"foundation JSON is invalid: {path.name}") from error


def _sha256(path: Path) -> str:
    return hashlib.sha256(_read_regular(path)).hexdigest()


def load_contract(path: Path = CONTRACT_PATH) -> dict[str, object]:
    document = _load_json(path)
    expected = {
        "bundle_files": list(BUNDLE_FILES),
        "durations_seconds": {
            "niri-stable-soak": 14_400,
            "upstream-soak": 14_400,
        },
        "format": 1,
        "phase_checks": {
            phase: list(checks) for phase, checks in PHASE_CHECKS.items()
        },
        "supported_architectures": ["aarch64", "x86_64"],
        "upstream_revision": UPSTREAM_REVISION,
    }
    if document != expected:
        raise FoundationError("foundation contract differs from the reviewed boundary")
    return document


def parse_platform_report(path: Path) -> str:
    try:
        text = _read_regular(path).decode("ascii")
    except UnicodeDecodeError as error:
        raise FoundationError("platform-lab report must be ASCII") from error
    lines = text.splitlines()
    expected_keys = (
        "rmac-platform-lab-report",
        "os",
        "arch",
        "recording_complete",
        "exercisable_probes_passed",
        "expected_blockers_confirmed",
        "recorded",
        *(f"probe.{probe}" for probe in PROBES),
        *(f"blocker.{blocker}" for blocker in BLOCKERS),
    )
    if not text.endswith("\n") or len(lines) != len(expected_keys):
        raise FoundationError("platform-lab report inventory is not exact")
    values = {}
    for line, expected_key in zip(lines, expected_keys):
        key, separator, value = line.partition("=")
        if not separator or key != expected_key or not value:
            raise FoundationError(f"platform-lab report expected {expected_key}")
        values[key] = value
    if (
        values["rmac-platform-lab-report"] != "1"
        or values["os"] != "linux"
        or values["arch"] not in {"aarch64", "x86_64"}
        or values["recording_complete"] != "true"
        or values["exercisable_probes_passed"] != "true"
        or values["expected_blockers_confirmed"] != "true"
        or values["recorded"] != "12/12"
        or any(values[f"probe.{probe}"] != "pass" for probe in PROBES)
        or any(
            values[f"blocker.{blocker}"] != "blocker-confirmed"
            for blocker in BLOCKERS
        )
    ):
        raise FoundationError("platform-lab report does not prove the stable gate")
    return values["arch"]


def parse_a4_report(path: Path) -> None:
    try:
        text = _read_regular(path).decode("ascii")
    except UnicodeDecodeError as error:
        raise FoundationError("A4 report must be ASCII") from error
    lines = text.splitlines()
    expected_keys = (
        "rmac-upstream-a4-report",
        "revision",
        *(f"result.{result}" for result in A4_RESULTS),
    )
    if not text.endswith("\n") or len(lines) != len(expected_keys):
        raise FoundationError("A4 report inventory is not exact")
    for index, (line, expected_key) in enumerate(zip(lines, expected_keys)):
        key, separator, value = line.partition("=")
        if not separator or key != expected_key:
            raise FoundationError(f"A4 report expected {expected_key}")
        expected_value = (
            "1" if index == 0
            else UPSTREAM_REVISION if index == 1
            else "pass"
        )
        if value != expected_value:
            raise FoundationError("A4 report is pending, failed, or has wrong identity")


def expected_results(status: str = "pass") -> list[dict[str, str]]:
    if status not in {"pass", "pending"}:
        raise FoundationError("foundation result status is invalid")
    return [
        {"check": check, "phase": phase, "status": status}
        for phase, checks in PHASE_CHECKS.items()
        for check in checks
    ]


def summary_template(bundle: Path, revision: str) -> dict[str, object]:
    report_names = (
        "a4-upstream-report.txt",
        "gnome-platform-lab.txt",
        "niri-platform-lab.txt",
    )
    return {
        "durations_seconds": {
            "niri-stable-soak": 0,
            "upstream-soak": 0,
        },
        "format": 1,
        "product_revision": revision,
        "reports_sha256": {
            name: _sha256(bundle / name) for name in report_names
        },
        "results": expected_results("pending"),
        "supporting_evidence_sha256": {
            name: "" for name in SUPPORTING_EVIDENCE
        },
        "upstream_revision": UPSTREAM_REVISION,
    }


def verify_bundle(
    contract: dict[str, object], bundle: Path, *, revision: str
) -> None:
    if not bundle.is_absolute() or bundle.is_symlink() or not bundle.is_dir():
        raise FoundationError("foundation bundle must be an absolute ordinary directory")
    try:
        actual = {path.name for path in bundle.iterdir()}
    except OSError as error:
        raise FoundationError("foundation bundle cannot be inspected") from error
    if actual != set(BUNDLE_FILES):
        raise FoundationError("foundation bundle file inventory is not exact")
    gnome_arch = parse_platform_report(bundle / "gnome-platform-lab.txt")
    niri_arch = parse_platform_report(bundle / "niri-platform-lab.txt")
    if gnome_arch != niri_arch:
        raise FoundationError("GNOME and niri platform reports use different architectures")
    parse_a4_report(bundle / "a4-upstream-report.txt")

    document = _load_json(bundle / "foundation-summary.json")
    template = summary_template(bundle, revision)
    if not isinstance(document, dict) or set(document) != set(template):
        raise FoundationError("foundation summary fields are not exact")
    for field in set(template) - {
        "durations_seconds",
        "results",
        "supporting_evidence_sha256",
    }:
        if document.get(field) != template[field]:
            raise FoundationError(f"foundation summary {field} differs")
    if document.get("results") != expected_results():
        raise FoundationError("foundation summary does not prove every A1/A3 check")
    durations = document.get("durations_seconds")
    if (
        not isinstance(durations, dict)
        or set(durations) != set(contract["durations_seconds"])
        or any(type(value) is not int for value in durations.values())
        or any(
            durations[name] < minimum
            for name, minimum in contract["durations_seconds"].items()
        )
    ):
        raise FoundationError("foundation soak duration is incomplete")
    supporting = document.get("supporting_evidence_sha256")
    if (
        not isinstance(supporting, dict)
        or set(supporting) != set(SUPPORTING_EVIDENCE)
        or any(
            not isinstance(value, str)
            or not re.fullmatch(r"[0-9a-f]{64}", value)
            for value in supporting.values()
        )
    ):
        raise FoundationError("foundation supporting evidence identity is incomplete")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle-dir", type=Path)
    parser.add_argument("--revision")
    parser.add_argument("--print-summary", action="store_true")
    arguments = parser.parse_args()
    try:
        contract = load_contract()
        if arguments.bundle_dir is not None or arguments.print_summary:
            if arguments.bundle_dir is None or not arguments.bundle_dir.is_absolute():
                raise FoundationError("an absolute --bundle-dir is required")
            if not re.fullmatch(r"[0-9a-f]{40}", arguments.revision or ""):
                raise FoundationError("an exact 40-hex --revision is required")
        if arguments.print_summary:
            json.dump(
                summary_template(arguments.bundle_dir, arguments.revision),
                sys.stdout,
                indent=2,
            )
            sys.stdout.write("\n")
            return 0
        if arguments.bundle_dir is not None:
            verify_bundle(
                contract, arguments.bundle_dir, revision=arguments.revision
            )
    except FoundationError as error:
        parser.exit(4, f"verify-foundation-evidence: {error}\n")
    print(
        "rmac Linux foundation contract verified "
        f"({len(expected_results())} reviewed checks + 47 report results)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
