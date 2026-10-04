#!/usr/bin/env python3
"""Verify the deterministic I9 invited daily-driver Beta contract."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import stat
import subprocess
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
# Owner-recorded station waivers for this tier. The owner has neither an AMD
# nor an NVIDIA desktop, so Beta 1 ships without either H8 desktop station
# (decision of 2026-10-04, docs/known-limitations.md). A waived station is
# written as {"id", "status": "waived", "waiver": <id>} and needs no cohort
# participants. A waiver never passes a check, a journey, a defect class or
# another station.
STATION_WAIVER = "owner-2026-10-04-beta1-without-amd-nvidia-desktops"
WAIVED_STATIONS = ("amd64-amd-desktop", "amd64-nvidia-desktop")


def _template_station(station: str) -> dict[str, str]:
    if station in WAIVED_STATIONS:
        return {"id": station, "status": "waived", "waiver": STATION_WAIVER}
    return {"id": station, "status": "pending"}


def effective_minimum(station: str) -> int:
    return 0 if station in WAIVED_STATIONS else STATION_MINIMUMS[station]


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


def _checkout_revision() -> str:
    try:
        result = subprocess.run(
            ["git", "-C", str(REPO_ROOT), "rev-parse", "--verify", "HEAD"],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            check=False,
            timeout=5,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise BetaError("Beta checkout revision cannot be verified") from error
    revision = result.stdout.decode("ascii", "replace").strip()
    if result.returncode != 0 or not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise BetaError("Beta checkout revision cannot be verified")
    return revision


def _require_clean_checkout() -> None:
    try:
        result = subprocess.run(
            ["git", "-C", str(REPO_ROOT), "status", "--porcelain", "--untracked-files=all"],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            check=False,
            timeout=5,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise BetaError("Beta checkout cleanliness cannot be verified") from error
    if result.returncode != 0:
        raise BetaError("Beta checkout cleanliness cannot be verified")
    if result.stdout:
        raise BetaError("Beta evidence requires a clean checkout")


def _verify_checkout(revision: str) -> None:
    if _checkout_revision() != revision:
        raise BetaError("Beta evidence revision differs from the checkout")
    _require_clean_checkout()


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
    journey_entries = journeys.get("journeys") if isinstance(journeys, dict) else None
    if not isinstance(journey_entries, list) or len(journey_entries) != 10:
        raise BetaError("Beta journey source inventory is invalid")
    names_list: list[str] = []
    for journey in journey_entries:
        if not isinstance(journey, dict):
            raise BetaError("Beta journey source inventory is invalid")
        name = journey.get("name")
        if not isinstance(name, str) or not name.strip():
            raise BetaError("Beta journey source inventory is invalid")
        names_list.append(name)
    names = tuple(names_list)
    if len(set(names)) != 10:
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
            "minimum_days_per_participant": 0,
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
        # Evidence format 2 adds the minimum individual participation duration.
        "format": 2,
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
        "stations": [_template_station(station) for station in stations],
        "version": version,
    }


def verify_evidence(
    contract: dict[str, object],
    path: Path,
    *,
    version: str,
    revision: str,
) -> None:
    _verify_checkout(revision)
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
    recorded = document.get("stations")
    if not isinstance(recorded, list) or len(recorded) != len(stations):
        raise BetaError("Beta evidence does not prove every station")
    for entry, station in zip(recorded, stations):
        accepted = [{"id": station, "status": "pass"}]
        if station in WAIVED_STATIONS:
            accepted.append(
                {"id": station, "status": "waived", "waiver": STATION_WAIVER}
            )
        if entry not in accepted:
            raise BetaError("Beta evidence does not prove every station")

    cohort = document.get("cohort")
    if not isinstance(cohort, dict) or set(cohort) != {
        "duration_days",
        "minimum_days_per_participant",
        "participant_days",
        "participants",
        "station_participants",
        "status",
    }:
        raise BetaError("Beta cohort fields are not exact")
    numeric = (
        "duration_days",
        "minimum_days_per_participant",
        "participant_days",
        "participants",
    )
    if any(type(cohort.get(field)) is not int for field in numeric):
        raise BetaError("Beta cohort measurements are invalid")
    station_participants = cohort.get("station_participants")
    if (
        cohort.get("status") != "pass"
        or cohort["duration_days"] < 14
        or cohort["minimum_days_per_participant"] < 14
        or cohort["participants"] < 20
        or cohort["participant_days"] < cohort["participants"] * 14
        or not isinstance(station_participants, dict)
        or set(station_participants) != set(STATION_MINIMUMS)
        or any(type(value) is not int for value in station_participants.values())
        or any(
            station_participants[station] < effective_minimum(station)
            for station in STATION_MINIMUMS
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
    print(
        "rmac Beta contract inventory verified "
        f"({len(CHECKS)} promotion checks; "
        f"{'candidate evidence verified' if arguments.evidence is not None else 'candidate evidence not supplied'})"
    )
    for station in WAIVED_STATIONS:
        print(f"station {station} is waived for Beta: {STATION_WAIVER}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
