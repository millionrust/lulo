#!/usr/bin/env python3
"""Verify the deterministic I4 performance release-audit contract."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import stat
import sys


REPO_ROOT = Path(__file__).resolve().parents[1]
BUDGET_PATH = REPO_ROOT / "scripts/performance-budgets.json"
JOURNEY_PATH = REPO_ROOT / "scripts/journey-suite.json"
HARDWARE_PATH = REPO_ROOT / "packaging/hardware-matrix.json"
MAX_BYTES = 1024 * 1024
APPLICATIONS = (
    ("rmac-clock", 500, 0.3, 12),
    ("rmac-activity-monitor", 500, 2.5, 60),
    ("rmac-app-drawer", 500, 0.3, 12),
    ("rmac-finder", 900, 0.3, 12),
    ("rmac-notes", 500, 0.3, 12),
    ("rmac-system-settings", 500, 0.3, 12),
    ("rmac-terminal", 900, 0.3, 12),
    ("rmac-text-editor", 500, 0.3, 12),
)
PROTOCOL = {
    "idle_seconds": 60,
    "interaction_seconds": 60,
    "soak_hours": 8,
    "startup_repetitions": 20,
    "warmups": 3,
}


class PerformanceError(RuntimeError):
    """A bounded performance-audit verification failure."""


def _read_regular(path: Path) -> bytes:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise PerformanceError(f"required performance file is unavailable: {path.name}") from error
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode):
        raise PerformanceError(f"required performance path is not regular: {path.name}")
    if metadata.st_size > MAX_BYTES:
        raise PerformanceError(f"required performance file is too large: {path.name}")
    try:
        raw = path.read_bytes()
    except OSError as error:
        raise PerformanceError(f"required performance file cannot be read: {path.name}") from error
    if len(raw) != metadata.st_size:
        raise PerformanceError(f"required performance file changed while reading: {path.name}")
    return raw


def _load_json(path: Path) -> object:
    try:
        return json.loads(_read_regular(path))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise PerformanceError(f"performance JSON is invalid: {path.name}") from error


def _sha256(path: Path) -> str:
    return hashlib.sha256(_read_regular(path)).hexdigest()


def _journey_names() -> tuple[str, ...]:
    document = _load_json(JOURNEY_PATH)
    if not isinstance(document, dict) or not isinstance(document.get("journeys"), list):
        raise PerformanceError("journey source manifest is invalid")
    names = tuple(
        journey.get("name") for journey in document["journeys"]
        if isinstance(journey, dict)
    )
    if (
        len(names) != 10
        or any(not isinstance(name, str) for name in names)
        or len(set(names)) != len(names)
    ):
        raise PerformanceError("journey source inventory is invalid")
    return names


def _hardware() -> tuple[dict[str, list[str]], set[str]]:
    document = _load_json(HARDWARE_PATH)
    if (
        not isinstance(document, dict)
        or not isinstance(document.get("release_tiers"), dict)
        or not isinstance(document.get("stations"), list)
    ):
        raise PerformanceError("hardware source manifest is invalid")
    station_ids = {
        station.get("id")
        for station in document["stations"]
        if isinstance(station, dict) and isinstance(station.get("id"), str)
    }
    tiers = document["release_tiers"]
    if (
        set(tiers) != {"alpha", "beta", "one-dot-zero"}
        or any(not isinstance(ids, list) for ids in tiers.values())
        or any(set(ids) - station_ids for ids in tiers.values())
    ):
        raise PerformanceError("hardware release tiers are invalid")
    return tiers, station_ids


def load_budgets(path: Path = BUDGET_PATH) -> dict[str, object]:
    document = _load_json(path)
    if not isinstance(document, dict) or set(document) != {
        "applications",
        "environment",
        "format",
        "journey_source",
        "memory",
        "protocol",
        "rendering",
        "shell",
    }:
        raise PerformanceError("performance budget sections are not exact")
    expected = {
        "applications": [
            {
                "id": app,
                "idle_cpu_percent": cpu,
                "idle_wakeups_per_minute": wakeups,
                "startup_p95_ms": startup,
            }
            for app, startup, cpu, wakeups in APPLICATIONS
        ],
        "environment": {
            "desktop": "rmac-niri",
            "profile": "release",
            "ubuntu": "26.04",
        },
        "format": 1,
        "journey_source": "scripts/journey-suite.json",
        "memory": {
            "application_rss_growth_mib_8h": 16,
            "application_rss_mib": 128,
            "combined_shell_rss_growth_mib_8h": 24,
            "combined_shell_rss_mib": 256,
        },
        "protocol": PROTOCOL,
        "rendering": [
            {"frame_p95_ms": 16, "missed_frames_percent": 1, "refresh_hz": 60},
            {"frame_p95_ms": 8, "missed_frames_percent": 1, "refresh_hz": 120},
        ],
        "shell": {"idle_cpu_percent": 0.5, "idle_wakeups_per_minute": 30},
    }
    if document != expected:
        raise PerformanceError("performance budgets differ from the reviewed contract")
    _journey_names()
    _hardware()
    return document


def result_specs(budgets: dict[str, object]) -> list[dict[str, object]]:
    memory = budgets["memory"]
    specs = []
    for app in budgets["applications"]:
        specs.extend(
            (
                {
                    "kind": "application",
                    "limit": app["idle_cpu_percent"],
                    "metric": "idle-cpu",
                    "subject": app["id"],
                    "unit": "percent",
                },
                {
                    "kind": "application",
                    "limit": app["idle_wakeups_per_minute"],
                    "metric": "idle-wakeups",
                    "subject": app["id"],
                    "unit": "per-minute",
                },
                {
                    "kind": "application",
                    "limit": memory["application_rss_mib"],
                    "metric": "idle-rss",
                    "subject": app["id"],
                    "unit": "MiB",
                },
                {
                    "kind": "application",
                    "limit": memory["application_rss_growth_mib_8h"],
                    "metric": "rss-growth-absolute-8h",
                    "subject": app["id"],
                    "unit": "MiB",
                },
                {
                    "kind": "application",
                    "limit": app["startup_p95_ms"],
                    "metric": "startup-p95",
                    "subject": app["id"],
                    "unit": "ms",
                },
            )
        )
    specs.extend(
        (
            {
                "kind": "shell",
                "limit": budgets["shell"]["idle_cpu_percent"],
                "metric": "idle-cpu",
                "subject": "combined-shell",
                "unit": "percent",
            },
            {
                "kind": "shell",
                "limit": budgets["shell"]["idle_wakeups_per_minute"],
                "metric": "idle-wakeups",
                "subject": "combined-shell",
                "unit": "per-minute",
            },
            {
                "kind": "shell",
                "limit": memory["combined_shell_rss_mib"],
                "metric": "idle-rss",
                "subject": "combined-shell",
                "unit": "MiB",
            },
            {
                "kind": "shell",
                "limit": memory["combined_shell_rss_growth_mib_8h"],
                "metric": "rss-growth-absolute-8h",
                "subject": "combined-shell",
                "unit": "MiB",
            },
        )
    )
    for journey in _journey_names():
        specs.append(
            {
                "kind": "journey",
                "limit": 50,
                "metric": "interaction-p95",
                "subject": journey,
                "unit": "ms",
            }
        )
        for rendering in budgets["rendering"]:
            refresh = rendering["refresh_hz"]
            specs.extend(
                (
                    {
                        "kind": "journey",
                        "limit": rendering["frame_p95_ms"],
                        "metric": f"frame-p95-{refresh}hz",
                        "subject": journey,
                        "unit": "ms",
                    },
                    {
                        "kind": "journey",
                        "limit": rendering["missed_frames_percent"],
                        "metric": f"missed-frames-{refresh}hz",
                        "subject": journey,
                        "unit": "percent",
                    },
                )
            )
    return sorted(specs, key=lambda item: (item["kind"], item["subject"], item["metric"]))


def evidence_template(
    budgets: dict[str, object], station: str, revision: str
) -> dict[str, object]:
    _, station_ids = _hardware()
    if station not in station_ids:
        raise PerformanceError("unknown hardware station")
    results = [
        {**spec, "status": "pending", "value": None}
        for spec in result_specs(budgets)
    ]
    return {
        "budget_manifest_sha256": _sha256(BUDGET_PATH),
        "environment": budgets["environment"],
        "format": 1,
        "hardware_manifest_sha256": _sha256(HARDWARE_PATH),
        "journey_manifest_sha256": _sha256(JOURNEY_PATH),
        "protocol": budgets["protocol"],
        "results": results,
        "revision": revision,
        "station": station,
    }


def _verify_station(
    budgets: dict[str, object], path: Path, station: str, revision: str
) -> None:
    document = _load_json(path)
    template = evidence_template(budgets, station, revision)
    if not isinstance(document, dict) or set(document) != set(template):
        raise PerformanceError("performance evidence fields are not exact")
    for field in set(template) - {"results"}:
        if document.get(field) != template[field]:
            raise PerformanceError(f"performance evidence {field} differs")
    results = document.get("results")
    specs = result_specs(budgets)
    if not isinstance(results, list) or len(results) != len(specs):
        raise PerformanceError("performance evidence inventory is not exact")
    for result, spec in zip(results, specs):
        if not isinstance(result, dict) or set(result) != {
            "kind",
            "limit",
            "metric",
            "status",
            "subject",
            "unit",
            "value",
        }:
            raise PerformanceError("performance result fields are not exact")
        for field, expected in spec.items():
            if result.get(field) != expected:
                raise PerformanceError("performance result identity differs")
        value = result.get("value")
        if (
            result.get("status") != "pass"
            or isinstance(value, bool)
            or not isinstance(value, (int, float))
            or not math.isfinite(value)
            or value < 0
            or value > spec["limit"]
        ):
            raise PerformanceError("performance result exceeds or does not prove its budget")


def verify_evidence_directory(
    budgets: dict[str, object],
    directory: Path,
    *,
    tier: str,
    revision: str,
) -> None:
    if not directory.is_absolute() or directory.is_symlink() or not directory.is_dir():
        raise PerformanceError("evidence path must be an absolute ordinary directory")
    tiers, _ = _hardware()
    if tier not in tiers:
        raise PerformanceError("unknown performance release tier")
    expected = {f"{station}.json" for station in tiers[tier]}
    try:
        actual = {path.name for path in directory.iterdir()}
    except OSError as error:
        raise PerformanceError("performance evidence directory cannot be read") from error
    if actual != expected:
        raise PerformanceError("performance evidence station inventory is not exact")
    for station in tiers[tier]:
        _verify_station(budgets, directory / f"{station}.json", station, revision)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence-dir", type=Path)
    parser.add_argument("--tier", choices=("alpha", "beta", "one-dot-zero"))
    parser.add_argument("--revision")
    parser.add_argument("--print-template", action="store_true")
    parser.add_argument("--station")
    arguments = parser.parse_args()
    try:
        budgets = load_budgets()
        needs_revision = arguments.evidence_dir is not None or arguments.print_template
        if needs_revision and not re.fullmatch(r"[0-9a-f]{40}", arguments.revision or ""):
            raise PerformanceError("an exact 40-hex --revision is required")
        if arguments.evidence_dir is not None:
            if arguments.tier is None:
                raise PerformanceError("--tier is required with --evidence-dir")
            verify_evidence_directory(
                budgets,
                arguments.evidence_dir,
                tier=arguments.tier,
                revision=arguments.revision,
            )
        if arguments.print_template:
            if arguments.station is None:
                raise PerformanceError("--station is required with --print-template")
            json.dump(
                evidence_template(budgets, arguments.station, arguments.revision),
                sys.stdout,
                indent=2,
            )
            sys.stdout.write("\n")
            return 0
    except PerformanceError as error:
        parser.exit(4, f"verify-performance-audit: {error}\n")
    print(f"rmac performance audit verified ({len(result_specs(budgets))} budgets)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
