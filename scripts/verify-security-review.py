#!/usr/bin/env python3
"""Verify the deterministic I6 security and privacy review contract."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import stat
import sys


REPO_ROOT = Path(__file__).resolve().parents[1]
CONTRACT_PATH = REPO_ROOT / "scripts/security-review.json"
HARDWARE_PATH = REPO_ROOT / "packaging/hardware-matrix.json"
JOURNEY_PATH = REPO_ROOT / "scripts/journey-suite.json"
MAX_BYTES = 4 * 1024 * 1024
DOMAINS = (
    (
        "desktop-entry-execution",
        (
            "exact-field-code-expansion",
            "executable-boundary-preserved",
            "hidden-tryexec-precedence",
            "launch-diagnostics-redacted",
            "no-shell-interpolation",
            "terminal-wrapper-argument-boundary",
            "working-directory-validated",
        ),
        ("docs/application-launch.md", "crates/rmac-apps/src/lib.rs"),
    ),
    (
        "dbus-polkit",
        (
            "broadcasts-contain-no-secrets",
            "bounded-call-time-and-output",
            "denial-and-cancel-distinct",
            "interactive-authorization-only-from-user-action",
            "mutation-requires-authoritative-readback",
            "no-credential-collection",
            "system-bus-callers-treated-untrusted",
            "unique-owner-revalidated",
        ),
        (
            "docs/system-settings-audit.md",
            "docs/network.md",
            "docs/software-update.md",
        ),
    ),
    (
        "portals",
        (
            "backend-routing-verified",
            "document-access-remains-scoped",
            "permissionstore-does-not-overclaim",
            "portal-mediates-user-consent",
            "request-cancel-and-close-bounded",
            "response-handle-correlated",
            "selected-uri-revalidated",
        ),
        (
            "docs/flatpak-packaging.md",
            "docs/privacy-security.md",
            "crates/rmac-portal/src/lib.rs",
        ),
    ),
    (
        "file-operations",
        (
            "atomic-save-and-authoritative-readback",
            "cancellation-preserves-recoverable-destination",
            "conflict-refuses-stale-overwrite",
            "mount-disappearance-recovers",
            "private-path-diagnostics-redacted",
            "symlink-and-root-boundaries-enforced",
            "trash-and-destructive-actions-confirmed",
            "untrusted-content-never-executed",
        ),
        ("crates/finder/SPEC.md", "crates/text-editor/SPEC.md", "crates/notes/SPEC.md"),
    ),
    (
        "lock-boundary",
        (
            "compositor-exclusive-lock-proven",
            "mfa-conversation-bounded",
            "no-password-or-keycode-logging",
            "pam-is-sole-unlock-authority",
            "provider-crash-fails-closed",
            "secret-lifetime-and-zeroization-reviewed",
            "suspend-waits-for-lock-readiness",
            "tty-recovery-proven",
            "wrong-password-and-cancel-remain-locked",
        ),
        (
            "docs/decisions/0004-secure-lock-boundary.md",
            "docs/decisions/0005-rmac-lock-provider-state-machine.md",
            "docs/rmac-pam-wrapper-audit.md",
            "docs/secure-lock.md",
        ),
    ),
    (
        "notifications",
        (
            "action-target-bound-to-notification",
            "diagnostics-redacted",
            "focus-suppression-authoritative",
            "history-and-payload-bounded",
            "lock-screen-content-redacted",
            "markup-treated-as-untrusted",
            "sender-attribution-not-invented",
        ),
        ("docs/notifications.md", "crates/rmac-notifications/src/lib.rs"),
    ),
    (
        "search-indexing",
        (
            "allowed-roots-only",
            "cancellation-rejects-stale-results",
            "excluded-roots-never-indexed",
            "private-path-diagnostics-redacted",
            "query-and-content-bounded",
            "result-count-bounded",
            "stored-index-does-not-expand-authority",
        ),
        (
            "docs/launcher.md",
            "crates/rmac-launcher-providers/src/lib.rs",
            "crates/rmac-notes-runtime/src/search.rs",
        ),
    ),
    (
        "packages",
        (
            "architecture-and-file-inventory-exact",
            "artifact-contains-no-build-host-data",
            "dependency-and-advisory-policy-passes",
            "license-inventory-complete",
            "native-and-sandbox-boundaries-explicit",
            "rollback-and-uninstall-tested",
            "signature-and-origin-claims-bounded",
            "unpackaged-executables-rejected",
        ),
        (
            "docs/dependency-policy.md",
            "docs/native-packaging.md",
            "docs/flatpak-packaging.md",
            "scripts/linux/verify-native-packages.py",
        ),
    ),
    (
        "updates",
        (
            "apt-key-scope-isolated",
            "atomic-inrelease-last-and-monotonic",
            "backend-failure-recovers-authoritatively",
            "cancellation-and-restart-readback",
            "immutable-pool-and-by-hash",
            "keyring-public-only-and-package-scoped",
            "packagekit-invoked-without-shell",
            "polkit-interaction-is-user-initiated",
            "signature-failure-fails-closed",
            "trusted-only-install-enforced",
            "update-error-states-privacy-safe",
        ),
        (
            "docs/keyring-packaging.md",
            "docs/software-update.md",
            "docs/update-trust.md",
            "packaging/apt/keyring-package.json",
            "packaging/apt/publisher.json",
            "packaging/apt/update-trust.json",
            "scripts/linux/build-keyring-packages.py",
            "scripts/linux/keyring_package_contract.py",
            "scripts/linux/publish-apt-snapshot.py",
            "scripts/linux/verify-keyring-packages.py",
            "scripts/linux/verify-update-trust.py",
        ),
    ),
    (
        "logs-diagnostics",
        (
            "bus-peers-and-session-identities-redacted",
            "control-characters-normalized",
            "debug-implementations-redact-private-fields",
            "evidence-uses-synthetic-data",
            "failure-text-and-output-bounded",
            "log-growth-and-retention-bounded",
            "no-private-paths-or-content",
            "no-secrets-credentials-or-tokens",
        ),
        ("docs/about.md", "docs/privacy-security.md", "docs/chaos-soak.md"),
    ),
)


class SecurityError(RuntimeError):
    """A bounded security-review verification failure."""


def _read_regular(path: Path) -> bytes:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise SecurityError(f"required security file is unavailable: {path.name}") from error
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode):
        raise SecurityError(f"required security path is not regular: {path.name}")
    if metadata.st_size > MAX_BYTES:
        raise SecurityError(f"required security file is too large: {path.name}")
    try:
        raw = path.read_bytes()
    except OSError as error:
        raise SecurityError(f"required security file cannot be read: {path.name}") from error
    if len(raw) != metadata.st_size:
        raise SecurityError(f"required security file changed while reading: {path.name}")
    return raw


def _load_json(path: Path) -> object:
    try:
        return json.loads(_read_regular(path))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise SecurityError(f"security JSON is invalid: {path.name}") from error


def _sha256(path: Path) -> str:
    return hashlib.sha256(_read_regular(path)).hexdigest()


def _source_inventory() -> tuple[dict[str, list[str]], set[str]]:
    hardware = _load_json(HARDWARE_PATH)
    journeys = _load_json(JOURNEY_PATH)
    if (
        not isinstance(hardware, dict)
        or not isinstance(hardware.get("release_tiers"), dict)
        or not isinstance(hardware.get("stations"), list)
    ):
        raise SecurityError("hardware source manifest is invalid")
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
        raise SecurityError("hardware release tiers are invalid")
    if not isinstance(journeys, dict) or len(journeys.get("journeys", [])) != 10:
        raise SecurityError("journey source manifest is invalid")
    return tiers, stations


def load_contract(path: Path = CONTRACT_PATH) -> dict[str, object]:
    document = _load_json(path)
    if not isinstance(document, dict) or set(document) != {
        "domains",
        "environment",
        "format",
        "source_manifests",
    }:
        raise SecurityError("security review sections are not exact")
    expected = {
        "domains": [
            {"checks": list(checks), "id": domain, "sources": list(sources)}
            for domain, checks, sources in DOMAINS
        ],
        "environment": {"desktop": "rmac-niri", "ubuntu": "26.04"},
        "format": 1,
        "source_manifests": {
            "hardware": "packaging/hardware-matrix.json",
            "journeys": "scripts/journey-suite.json",
        },
    }
    if document != expected:
        raise SecurityError("security review differs from the reviewed threat boundary")
    for _, _, sources in DOMAINS:
        for relative in sources:
            source = (REPO_ROOT / relative).resolve()
            try:
                source.relative_to(REPO_ROOT.resolve())
            except ValueError as error:
                raise SecurityError("security review source escapes the repository") from error
            _read_regular(source)
    _source_inventory()
    return document


def expected_results(
    contract: dict[str, object], status: str = "pass"
) -> list[dict[str, str]]:
    if status not in {"pass", "pending"}:
        raise SecurityError("security result status is invalid")
    return [
        {"check": check, "domain": domain["id"], "status": status}
        for domain in contract["domains"]
        for check in domain["checks"]
    ]


def evidence_template(
    contract: dict[str, object], tier: str, revision: str
) -> dict[str, object]:
    tiers, _ = _source_inventory()
    if tier not in tiers:
        raise SecurityError("unknown security release tier")
    return {
        "contract_sha256": _sha256(CONTRACT_PATH),
        "environment": contract["environment"],
        "format": 1,
        "hardware_manifest_sha256": _sha256(HARDWARE_PATH),
        "journey_manifest_sha256": _sha256(JOURNEY_PATH),
        "open_findings": [],
        "results": expected_results(contract, "pending"),
        "revision": revision,
        "stations": [
            {"id": station, "status": "pending"} for station in tiers[tier]
        ],
        "tier": tier,
    }


def verify_evidence(
    contract: dict[str, object],
    evidence_path: Path,
    *,
    tier: str,
    revision: str,
) -> None:
    document = _load_json(evidence_path)
    template = evidence_template(contract, tier, revision)
    if not isinstance(document, dict) or set(document) != set(template):
        raise SecurityError("security evidence fields are not exact")
    for field in set(template) - {"results", "stations"}:
        if document.get(field) != template[field]:
            raise SecurityError(f"security evidence {field} differs")
    if document.get("results") != expected_results(contract):
        raise SecurityError("security evidence does not prove every exact check")
    tiers, _ = _source_inventory()
    expected_stations = [
        {"id": station, "status": "pass"} for station in tiers[tier]
    ]
    if document.get("stations") != expected_stations:
        raise SecurityError("security evidence does not prove every required station")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--tier", choices=("alpha", "beta", "one-dot-zero"))
    parser.add_argument("--revision")
    parser.add_argument("--print-template", action="store_true")
    arguments = parser.parse_args()
    try:
        contract = load_contract()
        needs_identity = arguments.evidence is not None or arguments.print_template
        if needs_identity:
            if arguments.tier is None:
                raise SecurityError("--tier is required")
            if not re.fullmatch(r"[0-9a-f]{40}", arguments.revision or ""):
                raise SecurityError("an exact 40-hex --revision is required")
        if arguments.evidence is not None:
            verify_evidence(
                contract,
                arguments.evidence,
                tier=arguments.tier,
                revision=arguments.revision,
            )
        if arguments.print_template:
            json.dump(
                evidence_template(contract, arguments.tier, arguments.revision),
                sys.stdout,
                indent=2,
            )
            sys.stdout.write("\n")
            return 0
    except SecurityError as error:
        parser.exit(4, f"verify-security-review: {error}\n")
    print(f"rmac security review verified ({len(expected_results(contract))} checks)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
