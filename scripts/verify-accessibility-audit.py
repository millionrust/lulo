#!/usr/bin/env python3
"""Verify the deterministic I3 accessibility release-audit contract."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import stat
import sys


REPO_ROOT = Path(__file__).resolve().parents[1]
AUDIT_PATH = REPO_ROOT / "scripts/accessibility-audit.json"
JOURNEY_PATH = REPO_ROOT / "scripts/journey-suite.json"
VISUAL_PATH = REPO_ROOT / "scripts/visual-suite.json"
MAX_BYTES = 1024 * 1024
REQUIRED_DIMENSIONS = (
    "actions",
    "announcements",
    "contrast",
    "focus-order",
    "focus-restoration",
    "ime",
    "keyboard",
    "orca-reading-order",
    "reduced-motion",
    "roles-names",
    "scale-200",
    "states-values",
    "visible-focus",
)
STANDARD_CHECKS = tuple(check for check in REQUIRED_DIMENSIONS if check != "ime")
PASSIVE_CHECKS = (
    "contrast",
    "orca-reading-order",
    "reduced-motion",
    "roles-names",
    "scale-200",
    "states-values",
)
EXPECTED_ASSIGNMENTS = {
    "application": (
        "files-browser",
        "files-conflict",
        "notes-editor",
        "notes-library",
        "system-monitor",
        "terminal",
        "text-editor",
    ),
    "settings": (
        "settings-appearance",
        "settings-battery",
        "settings-bluetooth",
        "settings-displays",
        "settings-input",
        "settings-network-vpn",
        "settings-notifications-focus",
        "settings-privacy-security",
        "settings-software-update",
        "settings-sound",
        "settings-wifi",
    ),
    "shell-interactive": (
        "dock",
        "launcher",
        "lock-screen",
        "notification-banner",
        "notification-center",
        "quick-settings",
        "spotlight",
    ),
    "shell-passive": ("desktop", "top-bar"),
}
IME_SURFACES = (
    "files-browser",
    "launcher",
    "lock-screen",
    "notes-editor",
    "notes-library",
    "settings-network-vpn",
    "settings-wifi",
    "spotlight",
    "terminal",
    "text-editor",
)


class AuditError(RuntimeError):
    """A bounded accessibility-audit verification failure."""


def _read_regular(path: Path) -> bytes:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise AuditError(f"required audit file is unavailable: {path.name}") from error
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode):
        raise AuditError(f"required audit path is not regular: {path.name}")
    if metadata.st_size > MAX_BYTES:
        raise AuditError(f"required audit file is too large: {path.name}")
    try:
        raw = path.read_bytes()
    except OSError as error:
        raise AuditError(f"required audit file cannot be read: {path.name}") from error
    if len(raw) != metadata.st_size:
        raise AuditError(f"required audit file changed while reading: {path.name}")
    return raw


def _load_json(path: Path) -> object:
    try:
        return json.loads(_read_regular(path))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise AuditError(f"audit JSON is invalid: {path.name}") from error


def _sha256(path: Path) -> str:
    return hashlib.sha256(_read_regular(path)).hexdigest()


def _load_source_inventory() -> tuple[dict[str, str], tuple[str, ...]]:
    visual = _load_json(VISUAL_PATH)
    journeys = _load_json(JOURNEY_PATH)
    if not isinstance(visual, dict) or not isinstance(visual.get("screens"), list):
        raise AuditError("visual source manifest is invalid")
    if not isinstance(journeys, dict) or not isinstance(journeys.get("journeys"), list):
        raise AuditError("journey source manifest is invalid")
    screens: dict[str, str] = {}
    for screen in visual["screens"]:
        if (
            not isinstance(screen, dict)
            or not isinstance(screen.get("id"), str)
            or screen.get("group") not in {"app", "settings", "shell"}
            or screen["id"] in screens
        ):
            raise AuditError("visual source screen inventory is invalid")
        screens[screen["id"]] = screen["group"]
    journey_names = tuple(
        journey.get("name") for journey in journeys["journeys"]
        if isinstance(journey, dict)
    )
    if (
        len(journey_names) != 10
        or any(not isinstance(name, str) for name in journey_names)
        or len(set(journey_names)) != len(journey_names)
    ):
        raise AuditError("journey source inventory is invalid")
    return screens, journey_names


def load_audit(path: Path = AUDIT_PATH) -> dict[str, object]:
    document = _load_json(path)
    if not isinstance(document, dict) or set(document) != {
        "assistive_technology",
        "base_surface_profiles",
        "format",
        "journey_checks",
        "required_dimensions",
        "source_manifests",
        "surface_assignments",
        "surface_supplements",
    }:
        raise AuditError("accessibility audit sections are not exact")
    if document.get("format") != 1 or type(document.get("format")) is not int:
        raise AuditError("accessibility audit format is invalid")
    if document.get("assistive_technology") != {
        "name": "Orca",
        "session": "niri",
        "transport": "AT-SPI",
    }:
        raise AuditError("assistive-technology authority differs")
    if document.get("source_manifests") != {
        "journeys": "scripts/journey-suite.json",
        "surfaces": "scripts/visual-suite.json",
    }:
        raise AuditError("accessibility source manifests differ")
    if document.get("required_dimensions") != list(REQUIRED_DIMENSIONS):
        raise AuditError("accessibility audit dimensions are incomplete")
    expected_profiles = {
        "application": list(STANDARD_CHECKS),
        "settings": list(STANDARD_CHECKS),
        "shell-interactive": list(STANDARD_CHECKS),
        "shell-passive": list(PASSIVE_CHECKS),
    }
    if document.get("base_surface_profiles") != expected_profiles:
        raise AuditError("accessibility surface profiles differ")
    if document.get("journey_checks") != list(STANDARD_CHECKS):
        raise AuditError("accessibility journey checks differ")
    assignments = document.get("surface_assignments")
    if assignments != {
        profile: list(subjects)
        for profile, subjects in EXPECTED_ASSIGNMENTS.items()
    }:
        raise AuditError("accessibility surface assignments differ")
    if document.get("surface_supplements") != {"ime": list(IME_SURFACES)}:
        raise AuditError("accessibility IME surface inventory differs")

    screens, _ = _load_source_inventory()
    assigned = {
        subject
        for subjects in EXPECTED_ASSIGNMENTS.values()
        for subject in subjects
    }
    if assigned != set(screens):
        raise AuditError("accessibility audit does not cover every critical surface")
    expected_groups = {
        "application": "app",
        "settings": "settings",
        "shell-interactive": "shell",
        "shell-passive": "shell",
    }
    for profile, subjects in EXPECTED_ASSIGNMENTS.items():
        if any(screens[subject] != expected_groups[profile] for subject in subjects):
            raise AuditError("accessibility surface group differs from visual source")
    return document


def expected_results(audit: dict[str, object], status: str = "pass") -> list[dict[str, str]]:
    if status not in {"pass", "pending"}:
        raise AuditError("accessibility result status is invalid")
    _, journey_names = _load_source_inventory()
    checks_by_surface: dict[str, set[str]] = {}
    for profile, subjects in EXPECTED_ASSIGNMENTS.items():
        profile_checks = audit["base_surface_profiles"][profile]
        for subject in subjects:
            checks_by_surface[subject] = set(profile_checks)
    for subject in audit["surface_supplements"]["ime"]:
        checks_by_surface[subject].add("ime")
    results = [
        {
            "check": check,
            "kind": "surface",
            "status": status,
            "subject": subject,
        }
        for subject, checks in checks_by_surface.items()
        for check in checks
    ]
    results.extend(
        {
            "check": check,
            "kind": "journey",
            "status": status,
            "subject": subject,
        }
        for subject in journey_names
        for check in audit["journey_checks"]
    )
    return sorted(results, key=lambda item: (item["kind"], item["subject"], item["check"]))


def evidence_template(audit: dict[str, object], revision: str) -> dict[str, object]:
    return {
        "assistive_technology": "Orca",
        "audit_manifest_sha256": _sha256(AUDIT_PATH),
        "format": 1,
        "journey_manifest_sha256": _sha256(JOURNEY_PATH),
        "platform": {"desktop": "rmac-niri", "ubuntu": "26.04"},
        "results": expected_results(audit, "pending"),
        "revision": revision,
        "visual_manifest_sha256": _sha256(VISUAL_PATH),
    }


def verify_evidence(
    audit: dict[str, object],
    evidence_path: Path,
    *,
    revision: str,
) -> None:
    document = _load_json(evidence_path)
    if not isinstance(document, dict) or set(document) != {
        "assistive_technology",
        "audit_manifest_sha256",
        "format",
        "journey_manifest_sha256",
        "platform",
        "results",
        "revision",
        "visual_manifest_sha256",
    }:
        raise AuditError("accessibility evidence fields are not exact")
    expected_identity = {
        "assistive_technology": "Orca",
        "audit_manifest_sha256": _sha256(AUDIT_PATH),
        "format": 1,
        "journey_manifest_sha256": _sha256(JOURNEY_PATH),
        "platform": {"desktop": "rmac-niri", "ubuntu": "26.04"},
        "revision": revision,
        "visual_manifest_sha256": _sha256(VISUAL_PATH),
    }
    for field, expected in expected_identity.items():
        if document.get(field) != expected:
            raise AuditError(f"accessibility evidence {field} differs")
    if document["results"] != expected_results(audit):
        raise AuditError("accessibility evidence does not prove every exact check")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--revision")
    parser.add_argument("--print-template", action="store_true")
    arguments = parser.parse_args()
    try:
        audit = load_audit()
        if arguments.evidence is not None or arguments.print_template:
            if not re.fullmatch(r"[0-9a-f]{40}", arguments.revision or ""):
                raise AuditError("an exact 40-hex --revision is required")
        if arguments.evidence is not None:
            verify_evidence(audit, arguments.evidence, revision=arguments.revision)
        if arguments.print_template:
            json.dump(evidence_template(audit, arguments.revision), sys.stdout, indent=2)
            sys.stdout.write("\n")
            return 0
    except AuditError as error:
        parser.exit(4, f"verify-accessibility-audit: {error}\n")
    print(
        "rmac accessibility audit verified "
        f"({len(expected_results(audit))} checks)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
