#!/usr/bin/env python3
"""Verify the deterministic I8 contributor Alpha candidate contract."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import stat
import sys


REPO_ROOT = Path(__file__).resolve().parents[1]
CONTRACT_PATH = REPO_ROOT / "scripts/alpha-candidate.json"
HARDWARE_PATH = REPO_ROOT / "packaging/hardware-matrix.json"
JOURNEY_PATH = REPO_ROOT / "scripts/journey-suite.json"
ISSUE_FORM_PATH = REPO_ROOT / ".github/ISSUE_TEMPLATE/01-alpha-bug.yml"
MAX_BYTES = 1024 * 1024
ARTIFACTS = (
    "source-archive",
    "rmac-apps-amd64-deb",
    "rmac-session-amd64-deb",
    "sha256sums",
    "sbom-spdx-json",
    "build-provenance",
)
LIMITATIONS = (
    "accessibility-and-layer-shell-evidence-incomplete",
    "hardware-support-limited-to-alpha-stations",
    "general-user-support-unavailable",
    "public-apt-channel-unavailable",
)
CHECKS = (
    "accessibility-status-explicit",
    "alpha-hardware-matrix",
    "clean-install",
    "clean-source-revision",
    "dependency-policy",
    "gdm-rmac-login-logout",
    "issue-intake-privacy-reviewed",
    "known-limitations-reviewed",
    "linux-amd64-reproducible-build",
    "native-package-artifact",
    "package-signature-checksum-sbom",
    "release-notes-reviewed",
    "stock-gnome-recovery",
    "ten-product-journeys",
    "upgrade-rollback-uninstall",
    "zero-open-data-loss",
    "zero-open-privilege-escalation",
    "zero-open-session-lockout",
)
VERSION_PATTERN = r"0\.[0-9]+\.[0-9]+-alpha\.[1-9][0-9]*"
ISSUE_IDS = (
    "revision",
    "station",
    "gpu",
    "compositor",
    "scale",
    "portals",
    "journey",
    "reproduction",
    "expected",
    "actual",
    "recovery",
    "severity",
    "logs",
    "confirmations",
)


class AlphaError(RuntimeError):
    """A bounded Alpha-candidate verification failure."""


def _read_regular(path: Path) -> bytes:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise AlphaError(f"required Alpha file is unavailable: {path.name}") from error
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode):
        raise AlphaError(f"required Alpha path is not regular: {path.name}")
    if metadata.st_size > MAX_BYTES:
        raise AlphaError(f"required Alpha file is too large: {path.name}")
    try:
        raw = path.read_bytes()
    except OSError as error:
        raise AlphaError(f"required Alpha file cannot be read: {path.name}") from error
    if len(raw) != metadata.st_size:
        raise AlphaError(f"required Alpha file changed while reading: {path.name}")
    return raw


def _load_json(path: Path) -> object:
    try:
        return json.loads(_read_regular(path))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise AlphaError(f"Alpha JSON is invalid: {path.name}") from error


def _sha256(path: Path) -> str:
    return hashlib.sha256(_read_regular(path)).hexdigest()


def _alpha_stations() -> tuple[str, ...]:
    hardware = _load_json(HARDWARE_PATH)
    journeys = _load_json(JOURNEY_PATH)
    if (
        not isinstance(hardware, dict)
        or not isinstance(hardware.get("release_tiers"), dict)
        or hardware["release_tiers"].get("alpha")
        != ["amd64-intel-laptop", "amd64-amd-desktop"]
    ):
        raise AlphaError("Alpha hardware source inventory is invalid")
    if not isinstance(journeys, dict) or len(journeys.get("journeys", [])) != 10:
        raise AlphaError("Alpha journey source inventory is invalid")
    return tuple(hardware["release_tiers"]["alpha"])


def verify_issue_form(path: Path = ISSUE_FORM_PATH) -> None:
    try:
        text = _read_regular(path).decode("utf-8")
    except UnicodeDecodeError as error:
        raise AlphaError("Alpha issue form is not UTF-8") from error
    for marker in (
        "name: Alpha bug report\n",
        "description:",
        'labels: ["alpha", "bug", "needs-triage"]',
        "body:",
        "Use the repository Security tab to report it privately.",
        "Do not include usernames, hostnames, serials, machine IDs",
        "Never paste a complete journal or environment dump.",
    ):
        if marker not in text:
            raise AlphaError("Alpha issue form is missing a required boundary")
    ids = tuple(re.findall(r"^    id: ([a-z][a-z0-9-]*)$", text, re.MULTILINE))
    if ids != ISSUE_IDS or len(ids) != len(set(ids)):
        raise AlphaError("Alpha issue form field inventory differs")
    for required_id in ISSUE_IDS[:-2]:
        block = text.split(f"    id: {required_id}\n", 1)[1].split("\n  - type:", 1)[0]
        if "required: true" not in block:
            raise AlphaError("Alpha issue form required field became optional")


def load_contract(path: Path = CONTRACT_PATH) -> dict[str, object]:
    document = _load_json(path)
    expected = {
        "artifacts": list(ARTIFACTS),
        "audience": "contributors",
        "format": 1,
        "issue_form": ".github/ISSUE_TEMPLATE/01-alpha-bug.yml",
        "known_limitations": list(LIMITATIONS),
        "required_checks": list(CHECKS),
        "sources": {
            "hardware": "packaging/hardware-matrix.json",
            "journeys": "scripts/journey-suite.json",
            "limitations": "docs/known-limitations.md",
            "release_notes": "docs/release-notes.md",
            "security_policy": "SECURITY.md",
        },
        "tier": "alpha",
        "version_pattern": VERSION_PATTERN,
    }
    if document != expected:
        raise AlphaError("Alpha contract differs from the reviewed publish boundary")
    for relative in document["sources"].values():
        _read_regular(REPO_ROOT / relative)
    verify_issue_form()
    _alpha_stations()
    return document


def evidence_template(
    contract: dict[str, object], version: str, revision: str
) -> dict[str, object]:
    if not re.fullmatch(VERSION_PATTERN, version):
        raise AlphaError("Alpha version is invalid")
    return {
        "artifacts": [
            {"id": artifact, "sha256": "", "size_bytes": 0, "status": "pending"}
            for artifact in contract["artifacts"]
        ],
        "checks": [
            {"check": check, "status": "pending"}
            for check in contract["required_checks"]
        ],
        "contract_sha256": _sha256(CONTRACT_PATH),
        "format": 1,
        "hardware_manifest_sha256": _sha256(HARDWARE_PATH),
        "journey_manifest_sha256": _sha256(JOURNEY_PATH),
        "known_limitations": [
            {"id": limitation, "status": "pending"}
            for limitation in contract["known_limitations"]
        ],
        "release_blockers": [],
        "revision": revision,
        "stations": [
            {"id": station, "status": "pending"} for station in _alpha_stations()
        ],
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
        raise AlphaError("Alpha evidence fields are not exact")
    for field in set(template) - {
        "artifacts",
        "checks",
        "known_limitations",
        "stations",
    }:
        if document.get(field) != template[field]:
            raise AlphaError(f"Alpha evidence {field} differs")
    expected_checks = [
        {"check": check, "status": "pass"} for check in CHECKS
    ]
    if document.get("checks") != expected_checks:
        raise AlphaError("Alpha evidence does not prove every required check")
    expected_limitations = [
        {"id": limitation, "status": "disclosed"} for limitation in LIMITATIONS
    ]
    if document.get("known_limitations") != expected_limitations:
        raise AlphaError("Alpha limitations are not all disclosed")
    expected_stations = [
        {"id": station, "status": "pass"} for station in _alpha_stations()
    ]
    if document.get("stations") != expected_stations:
        raise AlphaError("Alpha evidence does not prove both stations")
    artifacts = document.get("artifacts")
    if not isinstance(artifacts, list) or len(artifacts) != len(ARTIFACTS):
        raise AlphaError("Alpha artifact inventory is not exact")
    for artifact, expected_id in zip(artifacts, ARTIFACTS):
        if not isinstance(artifact, dict) or set(artifact) != {
            "id",
            "sha256",
            "size_bytes",
            "status",
        }:
            raise AlphaError("Alpha artifact fields are not exact")
        if (
            artifact.get("id") != expected_id
            or artifact.get("status") != "pass"
            or not re.fullmatch(r"[0-9a-f]{64}", artifact.get("sha256", ""))
            or type(artifact.get("size_bytes")) is not int
            or artifact["size_bytes"] <= 0
        ):
            raise AlphaError("Alpha artifact is missing verified identity")


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
                raise AlphaError("a valid --version is required")
            if not re.fullmatch(r"[0-9a-f]{40}", arguments.revision or ""):
                raise AlphaError("an exact 40-hex --revision is required")
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
    except AlphaError as error:
        parser.exit(4, f"verify-alpha-candidate: {error}\n")
    print(f"rmac Alpha contract verified ({len(CHECKS)} publish checks)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
