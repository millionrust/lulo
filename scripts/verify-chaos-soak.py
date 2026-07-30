#!/usr/bin/env python3
"""Verify the deterministic I5 chaos and soak release contract."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import stat
import sys


REPO_ROOT = Path(__file__).resolve().parents[1]
CONTRACT_PATH = REPO_ROOT / "scripts/chaos-soak.json"
HARDWARE_PATH = REPO_ROOT / "packaging/hardware-matrix.json"
JOURNEY_PATH = REPO_ROOT / "scripts/journey-suite.json"
MAX_BYTES = 1024 * 1024
SCENARIOS = (
    ("desktop-service-restarts", "system-and-session-dbus"),
    ("niri-restart", "niri-session"),
    ("shell-component-crash", "systemd-user-components"),
    ("shell-supervisor-crash", "rmac-session-supervisor"),
    ("isolated-low-disk", "dedicated-test-volume"),
    ("malformed-niri-config", "niri-config-validator"),
    ("malformed-rmac-config", "versioned-rmac-settings"),
    ("mount-disappearance", "mount-namespace"),
    ("display-hotplug", "niri-output-state"),
    ("peripheral-hotplug", "bluez-pipewire-libinput"),
    ("suspend-resume", "systemd-logind"),
    ("update-backend-restart", "packagekit"),
    ("update-network-loss", "packagekit-networkmanager"),
    ("update-install-interruption", "packagekit-apt-recovery"),
)
SCENARIO_ASSERTIONS = (
    "authoritative-recovery",
    "degraded-state-visible",
    "last-known-good-or-safe-empty",
    "no-crash-loop",
    "no-data-loss",
    "operation-bounded",
    "privacy-safe-diagnostics",
)
SOAK_ASSERTIONS = (
    "all-components-responsive",
    "all-journeys-recoverable",
    "bounded-log-growth",
    "idle-budget-maintained",
    "memory-growth-budget-maintained",
    "no-crash-loop",
    "no-data-loss",
    "no-stuck-operation",
    "post-run-restart-clean",
)
SOAKS = (
    {
        "checkpoint_seconds": 300,
        "id": "eight-hour",
        "minimum_checkpoints": 96,
        "minimum_duration_seconds": 28_800,
    },
    {
        "checkpoint_seconds": 900,
        "id": "seven-day",
        "minimum_checkpoints": 672,
        "minimum_duration_seconds": 604_800,
    },
)


class ChaosError(RuntimeError):
    """A bounded chaos/soak verification failure."""


def _read_regular(path: Path) -> bytes:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise ChaosError(f"required chaos file is unavailable: {path.name}") from error
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode):
        raise ChaosError(f"required chaos path is not regular: {path.name}")
    if metadata.st_size > MAX_BYTES:
        raise ChaosError(f"required chaos file is too large: {path.name}")
    try:
        raw = path.read_bytes()
    except OSError as error:
        raise ChaosError(f"required chaos file cannot be read: {path.name}") from error
    if len(raw) != metadata.st_size:
        raise ChaosError(f"required chaos file changed while reading: {path.name}")
    return raw


def _load_json(path: Path) -> object:
    try:
        return json.loads(_read_regular(path))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ChaosError(f"chaos JSON is invalid: {path.name}") from error


def _sha256(path: Path) -> str:
    return hashlib.sha256(_read_regular(path)).hexdigest()


def _sources() -> tuple[dict[str, list[str]], set[str], tuple[str, ...]]:
    hardware = _load_json(HARDWARE_PATH)
    journeys = _load_json(JOURNEY_PATH)
    if (
        not isinstance(hardware, dict)
        or not isinstance(hardware.get("release_tiers"), dict)
        or not isinstance(hardware.get("stations"), list)
    ):
        raise ChaosError("hardware source manifest is invalid")
    stations = {
        station.get("id")
        for station in hardware["stations"]
        if isinstance(station, dict) and isinstance(station.get("id"), str)
    }
    tiers = hardware["release_tiers"]
    if (
        set(tiers) != {"alpha", "beta", "one-dot-zero"}
        or any(not isinstance(ids, list) for ids in tiers.values())
        or any(set(ids) - stations for ids in tiers.values())
    ):
        raise ChaosError("hardware release tiers are invalid")
    if not isinstance(journeys, dict) or not isinstance(journeys.get("journeys"), list):
        raise ChaosError("journey source manifest is invalid")
    names = tuple(
        journey.get("name")
        for journey in journeys["journeys"]
        if isinstance(journey, dict)
    )
    if (
        len(names) != 10
        or any(not isinstance(name, str) for name in names)
        or len(set(names)) != len(names)
    ):
        raise ChaosError("journey source inventory is invalid")
    return tiers, stations, names


def load_contract(path: Path = CONTRACT_PATH) -> dict[str, object]:
    document = _load_json(path)
    if not isinstance(document, dict) or set(document) != {
        "environment",
        "format",
        "scenario_assertions",
        "scenarios",
        "soak_assertions",
        "soaks",
        "source_manifests",
        "storage_safety",
    }:
        raise ChaosError("chaos contract sections are not exact")
    expected = {
        "environment": {"desktop": "rmac-niri", "ubuntu": "26.04"},
        "format": 1,
        "scenario_assertions": list(SCENARIO_ASSERTIONS),
        "scenarios": [
            {"authority": authority, "id": scenario}
            for scenario, authority in SCENARIOS
        ],
        "soak_assertions": list(SOAK_ASSERTIONS),
        "soaks": list(SOAKS),
        "source_manifests": {
            "hardware": "packaging/hardware-matrix.json",
            "journeys": "scripts/journey-suite.json",
        },
        "storage_safety": {
            "host_minimum_free_gib": 15,
            "low_disk_requires_disposable_volume": True,
            "real_user_data_forbidden": True,
            "update_interruption_requires_disposable_station": True,
        },
    }
    if document != expected:
        raise ChaosError("chaos contract differs from the reviewed release boundary")
    _sources()
    return document


def expected_results(
    contract: dict[str, object], status: str = "pass"
) -> list[dict[str, str]]:
    if status not in {"pass", "pending"}:
        raise ChaosError("chaos result status is invalid")
    _, _, journeys = _sources()
    results = [
        {
            "check": check,
            "kind": "scenario",
            "phase": "fault-injection",
            "status": status,
            "subject": scenario["id"],
        }
        for scenario in contract["scenarios"]
        for check in contract["scenario_assertions"]
    ]
    results.extend(
        {
            "check": check,
            "kind": "soak",
            "phase": soak["id"],
            "status": status,
            "subject": soak["id"],
        }
        for soak in contract["soaks"]
        for check in contract["soak_assertions"]
    )
    results.extend(
        {
            "check": "post-soak-journey",
            "kind": "journey",
            "phase": soak["id"],
            "status": status,
            "subject": journey,
        }
        for soak in contract["soaks"]
        for journey in journeys
    )
    return sorted(
        results,
        key=lambda item: (
            item["phase"],
            item["kind"],
            item["subject"],
            item["check"],
        ),
    )


def evidence_template(
    contract: dict[str, object], station: str, revision: str
) -> dict[str, object]:
    _, stations, _ = _sources()
    if station not in stations:
        raise ChaosError("unknown hardware station")
    return {
        "contract_sha256": _sha256(CONTRACT_PATH),
        "environment": contract["environment"],
        "format": 1,
        "hardware_manifest_sha256": _sha256(HARDWARE_PATH),
        "journey_manifest_sha256": _sha256(JOURNEY_PATH),
        "results": expected_results(contract, "pending"),
        "revision": revision,
        "runs": [
            {
                "checkpoints": soak["minimum_checkpoints"],
                "duration_seconds": soak["minimum_duration_seconds"],
                "phase": soak["id"],
                "status": "pending",
            }
            for soak in contract["soaks"]
        ],
        "station": station,
    }


def _verify_station(
    contract: dict[str, object], path: Path, station: str, revision: str
) -> None:
    document = _load_json(path)
    template = evidence_template(contract, station, revision)
    if not isinstance(document, dict) or set(document) != set(template):
        raise ChaosError("chaos evidence fields are not exact")
    for field in set(template) - {"results", "runs"}:
        if document.get(field) != template[field]:
            raise ChaosError(f"chaos evidence {field} differs")
    if document.get("results") != expected_results(contract):
        raise ChaosError("chaos evidence does not prove every exact result")
    runs = document.get("runs")
    if not isinstance(runs, list) or len(runs) != len(SOAKS):
        raise ChaosError("chaos soak run inventory is not exact")
    for run, soak in zip(runs, SOAKS):
        if not isinstance(run, dict) or set(run) != {
            "checkpoints",
            "duration_seconds",
            "phase",
            "status",
        }:
            raise ChaosError("chaos soak run fields are not exact")
        if (
            run.get("phase") != soak["id"]
            or run.get("status") != "pass"
            or type(run.get("duration_seconds")) is not int
            or type(run.get("checkpoints")) is not int
            or run["duration_seconds"] < soak["minimum_duration_seconds"]
            or run["checkpoints"] < soak["minimum_checkpoints"]
        ):
            raise ChaosError("chaos soak duration or checkpoints are incomplete")


def verify_evidence_directory(
    contract: dict[str, object],
    directory: Path,
    *,
    tier: str,
    revision: str,
) -> None:
    if not directory.is_absolute() or directory.is_symlink() or not directory.is_dir():
        raise ChaosError("evidence path must be an absolute ordinary directory")
    tiers, _, _ = _sources()
    if tier not in tiers:
        raise ChaosError("unknown chaos release tier")
    expected = {f"{station}.json" for station in tiers[tier]}
    try:
        actual = {path.name for path in directory.iterdir()}
    except OSError as error:
        raise ChaosError("chaos evidence directory cannot be read") from error
    if actual != expected:
        raise ChaosError("chaos evidence station inventory is not exact")
    for station in tiers[tier]:
        _verify_station(contract, directory / f"{station}.json", station, revision)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence-dir", type=Path)
    parser.add_argument("--tier", choices=("alpha", "beta", "one-dot-zero"))
    parser.add_argument("--revision")
    parser.add_argument("--print-template", action="store_true")
    parser.add_argument("--station")
    arguments = parser.parse_args()
    try:
        contract = load_contract()
        needs_revision = arguments.evidence_dir is not None or arguments.print_template
        if needs_revision and not re.fullmatch(r"[0-9a-f]{40}", arguments.revision or ""):
            raise ChaosError("an exact 40-hex --revision is required")
        if arguments.evidence_dir is not None:
            if arguments.tier is None:
                raise ChaosError("--tier is required with --evidence-dir")
            verify_evidence_directory(
                contract,
                arguments.evidence_dir,
                tier=arguments.tier,
                revision=arguments.revision,
            )
        if arguments.print_template:
            if arguments.station is None:
                raise ChaosError("--station is required with --print-template")
            json.dump(
                evidence_template(contract, arguments.station, arguments.revision),
                sys.stdout,
                indent=2,
            )
            sys.stdout.write("\n")
            return 0
    except ChaosError as error:
        parser.exit(4, f"verify-chaos-soak: {error}\n")
    print(f"rmac chaos/soak contract verified ({len(expected_results(contract))} results)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
