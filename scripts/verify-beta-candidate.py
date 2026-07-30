#!/usr/bin/env python3
"""Verify the deterministic I9 invited daily-driver Beta contract."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import stat
import sys


REPO_ROOT = Path(__file__).resolve().parents[1]
CONTRACT_PATH = REPO_ROOT / "scripts/beta-candidate.json"
MAX_BYTES = 1024 * 1024
VERSION_PATTERN = r"0\.[0-9]+\.[0-9]+-beta\.[1-9][0-9]*"
STATION_MINIMUMS = {
    "amd64-amd-desktop": 8,
    "amd64-intel-laptop": 8,
    "amd64-nvidia-desktop": 4,
}
DEFECT_CLASSES = (
    "critical-accessibility",
    "data-loss",
    "privilege-escalation",
    "session-lockout",
)
CHECKS = (
    "accessibility-release-audit",
    "alpha-candidate-complete",
    "beta-artifacts-signed",
    "beta-hardware-matrix",
    "chaos-soak-beta-stations",
    "clean-install-upgrade-rollback",
    "cohort-consent-and-exit",
    "cohort-duration-complete",
    "cohort-feedback-privacy-reviewed",
    "journey-quality-floor",
    "limitations-and-release-notes-current",
    "performance-release-audit",
    "recovery-and-safe-mode-tested",
    "security-review-zero-findings",
    "update-channel-staged-rollback",
)
SOURCES = {
    "accessibility": "scripts/accessibility-audit.json",
    "alpha": "scripts/alpha-candidate.json",
    "chaos": "scripts/chaos-soak.json",
    "hardware": "packaging/hardware-matrix.json",
    "journeys": "scripts/journey-suite.json",
    "performance": "scripts/performance-budgets.json",
    "security": "scripts/security-review.json",
}


class BetaError(RuntimeError):
    """A bounded Beta-candidate verification failure."""


def _read_regular(path: Path) -> bytes:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise BetaError(f"required Beta file is unavailable: {path.name}") from error
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode):
        raise BetaError(f"required Beta path is not regular: {path.name}")
    if metadata.st_size > MAX_BYTES:
        raise BetaError(f"required Beta file is too large: {path.name}")
    try:
        raw = path.read_bytes()
    except OSError as error:
        raise BetaError(f"required Beta file cannot be read: {path.name}") from error
    if len(raw) != metadata.st_size:
        raise BetaError(f"required Beta file changed while reading: {path.name}")
    return raw


def _load_json(path: Path) -> object:
    try:
        return json.loads(_read_regular(path))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise BetaError(f"Beta JSON is invalid: {path.name}") from error


def _sha256(path: Path) -> str:
    return hashlib.sha256(_read_regular(path)).hexdigest()


def _source_inventory() -> tuple[tuple[str, ...], tuple[str, ...]]:
    for relative in SOURCES.values():
        _read_regular(REPO_ROOT / relative)
    hardware = _load_json(REPO_ROOT / SOURCES["hardware"])
    journeys = _load_json(REPO_ROOT / SOURCES["journeys"])
    if (
        not isinstance(hardware, dict)
        or hardware.get("release_tiers", {}).get("beta")
        != [
            "amd64-intel-laptop",
            "amd64-amd-desktop",
            "amd64-nvidia-desktop",
        ]
    ):
        raise BetaError("Beta hardware source inventory is invalid")
    names = tuple(
        journey.get("name")
        for journey in journeys.get("journeys", [])
        if isinstance(journey, dict)
    ) if isinstance(journeys, dict) else ()
    if len(names) != 10 or len(set(names)) != 10:
        raise BetaError("Beta journey source inventory is invalid")
    return tuple(hardware["release_tiers"]["beta"]), names


def load_contract(path: Path = CONTRACT_PATH) -> dict[str, object]:
    document = _load_json(path)
    expected = {
        "audience": "invited-daily-driver-cohort",
        "cohort": {
            "minimum_duration_days": 14,
            "minimum_participant_days": 280,
            "minimum_participants": 20,
            "station_minimums": STATION_MINIMUMS,
        },
        "defect_classes": list(DEFECT_CLASSES),
        "format": 1,
        "journey_quality": {
            "minimum_attempts_each": 20,
            "minimum_pass_percent": 90,
        },
        "required_checks": list(CHECKS),
        "source_manifests": SOURCES,
        "tier": "beta",
        "version_pattern": VERSION_PATTERN,
    }
    if document != expected:
        raise BetaError("Beta contract differs from the reviewed promotion boundary")
    _source_inventory()
    return document


def evidence_template(
    contract: dict[str, object], version: str, revision: str
) -> dict[str, object]:
    if not re.fullmatch(VERSION_PATTERN, version):
        raise BetaError("Beta version is invalid")
    stations, journeys = _source_inventory()
    return {
        "checks": [{"check": check, "status": "pending"} for check in CHECKS],
        "cohort": {
            "duration_days": 0,
            "participant_days": 0,
            "participants": 0,
            "station_participants": {
                station: 0 for station in STATION_MINIMUMS
            },
            "status": "pending",
        },
        "contract_sha256": _sha256(CONTRACT_PATH),
        "defects": [
            {"class": defect, "open_count": 0, "status": "pending"}
            for defect in DEFECT_CLASSES
        ],
        "format": 1,
        "journeys": [
            {"attempts": 0, "name": journey, "passed": 0, "status": "pending"}
            for journey in journeys
        ],
        "release_blockers": [],
        "revision": revision,
        "source_sha256": {
            name: _sha256(REPO_ROOT / relative)
            for name, relative in SOURCES.items()
        },
        "stations": [{"id": station, "status": "pending"} for station in stations],
        "version": version,
    }


def verify_evidence(
    contract: dict[str, object],
    path: Path,
    *,
    version: str,
    revision: str,
) -> None:
    document = _load_json(path)
    template = evidence_template(contract, version, revision)
    if not isinstance(document, dict) or set(document) != set(template):
        raise BetaError("Beta evidence fields are not exact")
    for field in set(template) - {
        "checks",
        "cohort",
        "defects",
        "journeys",
        "stations",
    }:
        if document.get(field) != template[field]:
            raise BetaError(f"Beta evidence {field} differs")
    if document.get("checks") != [
        {"check": check, "status": "pass"} for check in CHECKS
    ]:
        raise BetaError("Beta evidence does not prove every promotion check")
    if document.get("defects") != [
        {"class": defect, "open_count": 0, "status": "pass"}
        for defect in DEFECT_CLASSES
    ]:
        raise BetaError("Beta safety defect floor is not zero")
    stations, journey_names = _source_inventory()
    if document.get("stations") != [
        {"id": station, "status": "pass"} for station in stations
    ]:
        raise BetaError("Beta evidence does not prove every station")

    cohort = document.get("cohort")
    if not isinstance(cohort, dict) or set(cohort) != {
        "duration_days",
        "participant_days",
        "participants",
        "station_participants",
        "status",
    }:
        raise BetaError("Beta cohort fields are not exact")
    numeric = ("duration_days", "participant_days", "participants")
    if any(type(cohort.get(field)) is not int for field in numeric):
        raise BetaError("Beta cohort measurements are invalid")
    station_participants = cohort.get("station_participants")
    if (
        cohort.get("status") != "pass"
        or cohort["duration_days"] < 14
        or cohort["participants"] < 20
        or cohort["participant_days"] < cohort["participants"] * 14
        or not isinstance(station_participants, dict)
        or set(station_participants) != set(STATION_MINIMUMS)
        or any(type(value) is not int for value in station_participants.values())
        or any(
            station_participants[station] < minimum
            for station, minimum in STATION_MINIMUMS.items()
        )
        or sum(station_participants.values()) != cohort["participants"]
    ):
        raise BetaError("Beta cohort duration or coverage is incomplete")

    journeys = document.get("journeys")
    if not isinstance(journeys, list) or len(journeys) != len(journey_names):
        raise BetaError("Beta journey inventory is not exact")
    for result, name in zip(journeys, journey_names):
        if not isinstance(result, dict) or set(result) != {
            "attempts",
            "name",
            "passed",
            "status",
        }:
            raise BetaError("Beta journey result fields are not exact")
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
            or passed * 100 < attempts * 90
        ):
            raise BetaError("Beta journey quality floor is incomplete")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--print-template", action="store_true")
    parser.add_argument("--version")
    parser.add_argument("--revision")
    arguments = parser.parse_args()
    try:
        contract = load_contract()
        if arguments.evidence is not None or arguments.print_template:
            if not re.fullmatch(VERSION_PATTERN, arguments.version or ""):
                raise BetaError("a valid --version is required")
            if not re.fullmatch(r"[0-9a-f]{40}", arguments.revision or ""):
                raise BetaError("an exact 40-hex --revision is required")
        if arguments.evidence is not None:
            verify_evidence(
                contract,
                arguments.evidence,
                version=arguments.version,
                revision=arguments.revision,
            )
        if arguments.print_template:
            json.dump(
                evidence_template(contract, arguments.version, arguments.revision),
                sys.stdout,
                indent=2,
            )
            sys.stdout.write("\n")
            return 0
    except BetaError as error:
        parser.exit(4, f"verify-beta-candidate: {error}\n")
    print(f"rmac Beta contract verified ({len(CHECKS)} promotion checks)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
