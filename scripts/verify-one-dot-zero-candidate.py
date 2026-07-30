#!/usr/bin/env python3
"""Verify the deterministic I10 rmac 1.0 release-candidate contract."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import stat
import sys


REPO_ROOT = Path(__file__).resolve().parents[1]
CONTRACT_PATH = REPO_ROOT / "scripts/one-dot-zero-candidate.json"
MAX_BYTES = 2 * 1024 * 1024
VERSION_PATTERN = r"1\.0\.0-rc\.[1-9][0-9]*"
ARTIFACTS = (
    "source-archive",
    "rmac-apps-amd64-deb",
    "rmac-session-amd64-deb",
    "rmac-apps-arm64-deb",
    "rmac-session-arm64-deb",
    "text-editor-flatpak-bundle",
    "apt-repository-metadata",
    "sha256sums",
    "sbom-spdx-json",
    "build-provenance",
)
TOP_FIVE = (
    "desktop-session",
    "text-document",
    "notes-library",
    "terminal-session",
    "file-management",
)
CHECKS = (
    "accessibility-release-audit",
    "apt-update-trust-and-rollback",
    "beta-candidate-complete",
    "chaos-and-seven-day-soak",
    "complete-recovery-matrix",
    "current-documentation-and-limitations",
    "install-upgrade-rollback-uninstall",
    "journey-95-percent-each-build",
    "performance-budgets-met",
    "security-review-zero-findings",
    "signed-artifacts-and-sbom",
    "stock-gnome-and-safe-mode-recovery",
    "top-five-journeys-zero-crash",
    "two-consecutive-candidate-builds",
    "visual-suite-reviewed",
    "zero-release-blockers",
)
SOURCES = {
    "accessibility": "scripts/accessibility-audit.json",
    "beta": "scripts/beta-candidate.json",
    "chaos": "scripts/chaos-soak.json",
    "documentation": "scripts/documentation-set.json",
    "hardware": "packaging/hardware-matrix.json",
    "journeys": "scripts/journey-suite.json",
    "performance": "scripts/performance-budgets.json",
    "security": "scripts/security-review.json",
    "update_trust": "packaging/apt/update-trust.json",
    "visual": "scripts/visual-suite.json",
}
RECOVERY_CHECKS = (
    "lock-provider-recovery",
    "package-rollback",
    "safe-mode",
    "stock-gnome-session",
    "uninstall-to-stock-session",
)


class CandidateError(RuntimeError):
    """A bounded 1.0-candidate verification failure."""


def _read_regular(path: Path) -> bytes:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise CandidateError(f"required candidate file is unavailable: {path.name}") from error
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode):
        raise CandidateError(f"required candidate path is not regular: {path.name}")
    if metadata.st_size > MAX_BYTES:
        raise CandidateError(f"required candidate file is too large: {path.name}")
    try:
        raw = path.read_bytes()
    except OSError as error:
        raise CandidateError(f"required candidate file cannot be read: {path.name}") from error
    if len(raw) != metadata.st_size:
        raise CandidateError(f"required candidate file changed while reading: {path.name}")
    return raw


def _load_json(path: Path) -> object:
    try:
        return json.loads(_read_regular(path))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise CandidateError(f"candidate JSON is invalid: {path.name}") from error


def _sha256(path: Path) -> str:
    return hashlib.sha256(_read_regular(path)).hexdigest()


def _source_inventory() -> tuple[tuple[str, ...], tuple[str, ...]]:
    for relative in SOURCES.values():
        _read_regular(REPO_ROOT / relative)
    hardware = _load_json(REPO_ROOT / SOURCES["hardware"])
    journeys = _load_json(REPO_ROOT / SOURCES["journeys"])
    stations = (
        tuple(hardware.get("release_tiers", {}).get("one-dot-zero", ()))
        if isinstance(hardware, dict)
        else ()
    )
    names = tuple(
        journey.get("name")
        for journey in journeys.get("journeys", [])
        if isinstance(journey, dict)
    ) if isinstance(journeys, dict) else ()
    if len(stations) != 5 or len(set(stations)) != 5:
        raise CandidateError("1.0 hardware source inventory is invalid")
    if len(names) != 10 or len(set(names)) != 10 or names[:5] != TOP_FIVE:
        raise CandidateError("1.0 journey source inventory is invalid")
    return stations, names


def load_contract(path: Path = CONTRACT_PATH) -> dict[str, object]:
    document = _load_json(path)
    expected = {
        "artifacts": list(ARTIFACTS),
        "build_count": 2,
        "format": 1,
        "journey_quality": {
            "minimum_attempts_each_build": 20,
            "minimum_pass_percent_each_journey": 95,
            "zero_crash_journeys": list(TOP_FIVE),
        },
        "required_checks": list(CHECKS),
        "source_manifests": SOURCES,
        "tier": "one-dot-zero",
        "version_pattern": VERSION_PATTERN,
    }
    if document != expected:
        raise CandidateError("1.0 contract differs from the reviewed release boundary")
    _source_inventory()
    return document


def _build_template(
    sequence: int,
    revision: str,
    stations: tuple[str, ...],
    journeys: tuple[str, ...],
) -> dict[str, object]:
    return {
        "build_evidence_sha256": "",
        "journeys": [
            {"attempts": 0, "name": journey, "passed": 0, "status": "pending"}
            for journey in journeys
        ],
        "performance_status": "pending",
        "revision": revision,
        "sequence": sequence,
        "stations": [{"id": station, "status": "pending"} for station in stations],
        "status": "pending",
        "top_five_crashes": {journey: 0 for journey in TOP_FIVE},
    }


def evidence_template(
    contract: dict[str, object],
    version: str,
    previous_revision: str,
    revision: str,
) -> dict[str, object]:
    if not re.fullmatch(VERSION_PATTERN, version):
        raise CandidateError("1.0 candidate version is invalid")
    if previous_revision == revision:
        raise CandidateError("candidate build revisions must be distinct")
    stations, journeys = _source_inventory()
    return {
        "artifacts": [
            {"id": artifact, "sha256": "", "size_bytes": 0, "status": "pending"}
            for artifact in ARTIFACTS
        ],
        "builds": [
            _build_template(1, previous_revision, stations, journeys),
            _build_template(2, revision, stations, journeys),
        ],
        "checks": [{"check": check, "status": "pending"} for check in CHECKS],
        "contract_sha256": _sha256(CONTRACT_PATH),
        "format": 1,
        "limitations": {
            "sha256": _sha256(REPO_ROOT / "docs/known-limitations.md"),
            "status": "pending",
        },
        "recovery": [
            {"check": check, "status": "pending"} for check in RECOVERY_CHECKS
        ],
        "release_blockers": [],
        "revision": revision,
        "source_sha256": {
            name: _sha256(REPO_ROOT / relative)
            for name, relative in SOURCES.items()
        },
        "version": version,
    }


def _verify_build(
    build: object,
    *,
    sequence: int,
    revision: str,
    stations: tuple[str, ...],
    journey_names: tuple[str, ...],
) -> None:
    if not isinstance(build, dict) or set(build) != {
        "build_evidence_sha256",
        "journeys",
        "performance_status",
        "revision",
        "sequence",
        "stations",
        "status",
        "top_five_crashes",
    }:
        raise CandidateError("candidate build fields are not exact")
    if (
        build.get("sequence") != sequence
        or build.get("revision") != revision
        or build.get("status") != "pass"
        or build.get("performance_status") != "pass"
        or not re.fullmatch(r"[0-9a-f]{64}", build.get("build_evidence_sha256", ""))
        or build.get("stations")
        != [{"id": station, "status": "pass"} for station in stations]
        or build.get("top_five_crashes") != {journey: 0 for journey in TOP_FIVE}
    ):
        raise CandidateError("candidate build identity, station, crash, or performance proof differs")
    journeys = build.get("journeys")
    if not isinstance(journeys, list) or len(journeys) != len(journey_names):
        raise CandidateError("candidate journey inventory is not exact")
    for result, name in zip(journeys, journey_names):
        if not isinstance(result, dict) or set(result) != {
            "attempts",
            "name",
            "passed",
            "status",
        }:
            raise CandidateError("candidate journey result fields are not exact")
        attempts = result.get("attempts")
        passed = result.get("passed")
        if (
            result.get("name") != name
            or result.get("status") != "pass"
            or type(attempts) is not int
            or type(passed) is not int
            or attempts < 20
            or passed < 0
            or passed > attempts
            or passed * 100 < attempts * 95
        ):
            raise CandidateError("candidate journey is below the 95 percent floor")


def verify_evidence(
    contract: dict[str, object],
    path: Path,
    *,
    version: str,
    previous_revision: str,
    revision: str,
) -> None:
    document = _load_json(path)
    template = evidence_template(
        contract, version, previous_revision, revision
    )
    if not isinstance(document, dict) or set(document) != set(template):
        raise CandidateError("1.0 evidence fields are not exact")
    for field in set(template) - {
        "artifacts",
        "builds",
        "checks",
        "limitations",
        "recovery",
    }:
        if document.get(field) != template[field]:
            raise CandidateError(f"1.0 evidence {field} differs")
    if document.get("checks") != [
        {"check": check, "status": "pass"} for check in CHECKS
    ]:
        raise CandidateError("1.0 evidence does not prove every release check")
    if document.get("limitations") != {
        "sha256": _sha256(REPO_ROOT / "docs/known-limitations.md"),
        "status": "reviewed",
    }:
        raise CandidateError("1.0 limitations are not current and reviewed")
    if document.get("recovery") != [
        {"check": check, "status": "pass"} for check in RECOVERY_CHECKS
    ]:
        raise CandidateError("1.0 recovery matrix is incomplete")

    stations, journey_names = _source_inventory()
    builds = document.get("builds")
    if not isinstance(builds, list) or len(builds) != 2:
        raise CandidateError("exactly two candidate builds are required")
    _verify_build(
        builds[0],
        sequence=1,
        revision=previous_revision,
        stations=stations,
        journey_names=journey_names,
    )
    _verify_build(
        builds[1],
        sequence=2,
        revision=revision,
        stations=stations,
        journey_names=journey_names,
    )

    artifacts = document.get("artifacts")
    if not isinstance(artifacts, list) or len(artifacts) != len(ARTIFACTS):
        raise CandidateError("1.0 artifact inventory is not exact")
    for artifact, expected_id in zip(artifacts, ARTIFACTS):
        if not isinstance(artifact, dict) or set(artifact) != {
            "id",
            "sha256",
            "size_bytes",
            "status",
        }:
            raise CandidateError("1.0 artifact fields are not exact")
        if (
            artifact.get("id") != expected_id
            or artifact.get("status") != "pass"
            or not re.fullmatch(r"[0-9a-f]{64}", artifact.get("sha256", ""))
            or type(artifact.get("size_bytes")) is not int
            or artifact["size_bytes"] <= 0
        ):
            raise CandidateError("1.0 artifact is missing verified identity")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--print-template", action="store_true")
    parser.add_argument("--version")
    parser.add_argument("--previous-revision")
    parser.add_argument("--revision")
    arguments = parser.parse_args()
    try:
        contract = load_contract()
        if arguments.evidence is not None or arguments.print_template:
            if not re.fullmatch(VERSION_PATTERN, arguments.version or ""):
                raise CandidateError("a valid --version is required")
            for name, value in (
                ("--previous-revision", arguments.previous_revision),
                ("--revision", arguments.revision),
            ):
                if not re.fullmatch(r"[0-9a-f]{40}", value or ""):
                    raise CandidateError(f"an exact 40-hex {name} is required")
            if arguments.previous_revision == arguments.revision:
                raise CandidateError("candidate build revisions must be distinct")
        if arguments.evidence is not None:
            verify_evidence(
                contract,
                arguments.evidence,
                version=arguments.version,
                previous_revision=arguments.previous_revision,
                revision=arguments.revision,
            )
        if arguments.print_template:
            json.dump(
                evidence_template(
                    contract,
                    arguments.version,
                    arguments.previous_revision,
                    arguments.revision,
                ),
                sys.stdout,
                indent=2,
            )
            sys.stdout.write("\n")
            return 0
    except CandidateError as error:
        parser.exit(4, f"verify-one-dot-zero-candidate: {error}\n")
    print(f"rmac 1.0 candidate contract verified ({len(CHECKS)} release checks)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
