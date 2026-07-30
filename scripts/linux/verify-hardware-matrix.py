#!/usr/bin/env python3
"""Verify the H8 hardware matrix and optional privacy-safe station results."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import stat


REPO_ROOT = Path(__file__).resolve().parents[2]
MATRIX_PATH = REPO_ROOT / "packaging/hardware-matrix.json"
MAX_BYTES = 256 * 1024
BASE_CHECKS = {
    "applications",
    "clipboard-ime",
    "gdm-login-logout",
    "graphics",
    "install-upgrade-uninstall",
    "keyboard-pointer",
    "lock-resume",
    "niri-restart",
    "orca-accessibility",
    "performance",
    "portals",
    "safe-mode-gnome-recovery",
    "service-restarts",
    "shell-surfaces",
}
DEVICE_CHECKS = {
    "bluetooth-audio": "bluetooth-audio-hotplug",
    "bluetooth-input": "bluetooth-input-hotplug",
    "internal-audio": "internal-audio",
    "touchpad": "touchpad",
    "usb-audio": "usb-audio-hotplug",
    "usb-input": "usb-input-hotplug",
}


class MatrixError(RuntimeError):
    """A bounded hardware-matrix verification failure."""


def _read_regular(path: Path) -> bytes:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise MatrixError(f"required matrix file is unavailable: {path.name}") from error
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode):
        raise MatrixError(f"required matrix path is not regular: {path.name}")
    if metadata.st_size > MAX_BYTES:
        raise MatrixError(f"required matrix file is too large: {path.name}")
    try:
        raw = path.read_bytes()
    except OSError as error:
        raise MatrixError(f"required matrix file cannot be read: {path.name}") from error
    if len(raw) != metadata.st_size:
        raise MatrixError(f"required matrix file changed while reading: {path.name}")
    return raw


def _load_json(path: Path) -> object:
    try:
        return json.loads(_read_regular(path))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise MatrixError(f"matrix JSON is invalid: {path.name}") from error


def required_checks(station: dict[str, object]) -> tuple[str, ...]:
    checks = set(BASE_CHECKS)
    checks.update(f"scale-{scale}" for scale in station["scales"])
    checks.update(DEVICE_CHECKS[device] for device in station["devices"])
    if station["monitors"] == "multi":
        checks.update({"display-hotplug", "mixed-scale-multi-monitor"})
    if "suspend-resume" in station["power"]:
        checks.add("suspend-resume")
    if "lid" in station["power"]:
        checks.add("lid-close-open")
    return tuple(sorted(checks))


def load_matrix(path: Path = MATRIX_PATH) -> dict[str, object]:
    document = _load_json(path)
    if not isinstance(document, dict) or set(document) != {
        "format",
        "release_tiers",
        "stations",
        "ubuntu",
    }:
        raise MatrixError("hardware matrix sections are not exact")
    if document.get("format") != 1 or type(document.get("format")) is not int:
        raise MatrixError("hardware matrix format is invalid")
    if document.get("ubuntu") != {
        "session": "rmac-niri",
        "version": "26.04",
    }:
        raise MatrixError("hardware matrix Ubuntu target differs")
    stations = document.get("stations")
    if not isinstance(stations, list) or len(stations) != 5:
        raise MatrixError("hardware station inventory is invalid")
    ids = []
    for station in stations:
        if not isinstance(station, dict) or set(station) != {
            "architecture",
            "devices",
            "gpu_driver",
            "gpu_vendor",
            "id",
            "monitors",
            "power",
            "scales",
        }:
            raise MatrixError("hardware station fields are invalid")
        station_id = station["id"]
        if not isinstance(station_id, str) or not re.fullmatch(
            r"[a-z0-9]+(?:-[a-z0-9]+)+", station_id
        ):
            raise MatrixError("hardware station identity is invalid")
        ids.append(station_id)
        if station["architecture"] not in {"amd64", "arm64"}:
            raise MatrixError("hardware station architecture is invalid")
        if station["monitors"] not in {"single", "multi"}:
            raise MatrixError("hardware monitor class is invalid")
        if (
            not isinstance(station["scales"], list)
            or not station["scales"]
            or station["scales"] != sorted(set(station["scales"]))
            or any(scale not in {100, 125, 150, 200} for scale in station["scales"])
        ):
            raise MatrixError("hardware scale inventory is invalid")
        if (
            not isinstance(station["devices"], list)
            or station["devices"] != sorted(set(station["devices"]))
            or any(device not in DEVICE_CHECKS for device in station["devices"])
        ):
            raise MatrixError("hardware device inventory is invalid")
        if (
            not isinstance(station["power"], list)
            or station["power"] != sorted(set(station["power"]))
            or any(value not in {"lid", "suspend-resume"} for value in station["power"])
        ):
            raise MatrixError("hardware power inventory is invalid")
    if len(ids) != len(set(ids)):
        raise MatrixError("hardware station identities are not unique")
    coverage = {
        "architectures": {station["architecture"] for station in stations},
        "vendors": {station["gpu_vendor"] for station in stations},
        "monitors": {station["monitors"] for station in stations},
        "scales": {scale for station in stations for scale in station["scales"]},
        "devices": {device for station in stations for device in station["devices"]},
    }
    if (
        coverage["architectures"] != {"amd64", "arm64"}
        or not {"intel", "amd", "nvidia"} <= coverage["vendors"]
        or coverage["monitors"] != {"single", "multi"}
        or coverage["scales"] != {100, 125, 150, 200}
        or coverage["devices"] != set(DEVICE_CHECKS)
    ):
        raise MatrixError("hardware matrix coverage is incomplete")
    tiers = document.get("release_tiers")
    expected_tiers = {
        "alpha": ["amd64-intel-laptop", "amd64-amd-desktop"],
        "beta": [
            "amd64-intel-laptop",
            "amd64-amd-desktop",
            "amd64-nvidia-desktop",
        ],
        "one-dot-zero": ids,
    }
    if tiers != expected_tiers:
        raise MatrixError("hardware release tiers differ from the reviewed contract")
    return document


def verify_evidence(
    matrix: dict[str, object],
    evidence_directory: Path,
    *,
    tier: str,
    revision: str,
) -> None:
    if not evidence_directory.is_absolute() or evidence_directory.is_symlink():
        raise MatrixError("evidence directory must be an absolute ordinary directory")
    tiers = matrix["release_tiers"]
    if tier not in tiers:
        raise MatrixError("unknown hardware release tier")
    required_ids = tiers[tier]
    try:
        actual = {path.name for path in evidence_directory.iterdir()}
    except OSError as error:
        raise MatrixError("hardware evidence directory cannot be inspected") from error
    expected = {f"{station_id}.json" for station_id in required_ids}
    if actual != expected:
        raise MatrixError("hardware evidence inventory is not exact")
    stations = {station["id"]: station for station in matrix["stations"]}
    for station_id in required_ids:
        document = _load_json(evidence_directory / f"{station_id}.json")
        if not isinstance(document, dict) or set(document) != {
            "format",
            "results",
            "revision",
            "station",
        }:
            raise MatrixError("hardware evidence fields are not exact")
        if (
            document["format"] != 1
            or type(document["format"]) is not int
            or document["station"] != station_id
            or document["revision"] != revision
        ):
            raise MatrixError("hardware evidence identity differs")
        results = document["results"]
        if not isinstance(results, list):
            raise MatrixError("hardware evidence results are invalid")
        expected_checks = required_checks(stations[station_id])
        if results != [
            {"check": check, "status": "pass"} for check in expected_checks
        ]:
            raise MatrixError("hardware evidence does not prove every required check")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence-dir", type=Path)
    parser.add_argument("--tier", choices=("alpha", "beta", "one-dot-zero"))
    parser.add_argument("--revision")
    arguments = parser.parse_args()
    try:
        matrix = load_matrix()
        if arguments.evidence_dir is not None:
            if arguments.tier is None or not re.fullmatch(
                r"[0-9a-f]{40}", arguments.revision or ""
            ):
                raise MatrixError(
                    "--tier and an exact 40-hex --revision are required with evidence"
                )
            verify_evidence(
                matrix,
                arguments.evidence_dir,
                tier=arguments.tier,
                revision=arguments.revision,
            )
    except MatrixError as error:
        parser.exit(4, f"verify-hardware-matrix: {error}\n")
    print("rmac hardware matrix verified")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
