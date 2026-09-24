#!/usr/bin/env python3
"""Verify the committed H7 APT trust policy and client templates."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import stat
import sys


REPO_ROOT = Path(__file__).resolve().parents[2]
POLICY_PATH = REPO_ROOT / "packaging/apt/update-trust.json"
SOURCES_PATH = REPO_ROOT / "packaging/apt/rmac.sources.in"
PREFERENCES_PATH = REPO_ROOT / "packaging/apt/rmac.pref"
MAX_BYTES = 256 * 1024
EXPECTED_SOURCES = (
    "Types: deb deb-src\n"
    "URIs: @RMAC_REPOSITORY_URI@\n"
    "Suites: resolute\n"
    "Components: main\n"
    "Architectures: amd64 arm64\n"
    "Signed-By: /usr/share/keyrings/rmac-archive-keyring.gpg\n"
    "Check-Valid-Until: yes\n"
).encode()
# The second stanza is what stops the rmac origin replacing any other package
# (sudo, openssh-server, …): every package it does not name is never
# installed from it.
EXPECTED_PREFERENCES = (
    "Package: rmac-apps rmac-archive-keyring rmac-session\n"
    "Pin: release o=rmac,n=resolute,c=main\n"
    "Pin-Priority: 500\n"
    "\n"
    "Package: *\n"
    "Pin: release o=rmac\n"
    "Pin-Priority: -1\n"
).encode()


class VerificationError(RuntimeError):
    """A deterministic update-trust policy failure."""


def _regular_bytes(path: Path) -> bytes:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise VerificationError(f"required trust file is unavailable: {path.name}") from error
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode):
        raise VerificationError(f"required trust path is not regular: {path.name}")
    if metadata.st_size > MAX_BYTES:
        raise VerificationError(f"required trust file is too large: {path.name}")
    try:
        raw = path.read_bytes()
    except OSError as error:
        raise VerificationError(f"required trust file cannot be read: {path.name}") from error
    if len(raw) != metadata.st_size:
        raise VerificationError(f"required trust file changed while reading: {path.name}")
    return raw


def load_policy(path: Path = POLICY_PATH) -> dict[str, object]:
    raw = _regular_bytes(path)
    try:
        document = json.loads(raw)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise VerificationError("update trust policy is invalid JSON") from error
    if not isinstance(document, dict) or set(document) != {
        "client",
        "format",
        "release",
        "repository",
        "rollback",
        "rollout",
        "signing",
        "source",
    }:
        raise VerificationError("update trust policy sections are not exact")
    if document.get("format") != 1 or type(document.get("format")) is not int:
        raise VerificationError("update trust policy format is invalid")
    return document


def _require_exact(section: object, expected: dict[str, object], label: str) -> None:
    if not isinstance(section, dict) or section != expected:
        raise VerificationError(f"{label} policy differs from the reviewed contract")


def verify_policy(
    policy_path: Path = POLICY_PATH,
    sources_path: Path = SOURCES_PATH,
    preferences_path: Path = PREFERENCES_PATH,
) -> str:
    policy = load_policy(policy_path)
    _require_exact(
        policy["client"],
        {
            "allow_downgrade_to_insecure": False,
            "allow_insecure": False,
            "check_valid_until": True,
            "keyring_package": "rmac-archive-keyring",
            "keyring_path": "/usr/share/keyrings/rmac-archive-keyring.gpg",
            "preferences_path": "/etc/apt/preferences.d/rmac.pref",
            "signed_by_isolated": True,
            "sources_path": "/etc/apt/sources.list.d/rmac.sources",
            "types": ["deb", "deb-src"],
        },
        "client",
    )
    _require_exact(
        policy["release"],
        {
            "acquire_by_hash": True,
            "allow_detached_release": False,
            "architectures": ["amd64", "arm64"],
            "codename": "resolute",
            "component": "main",
            "label": "rmac",
            "maximum_future_skew_seconds": 300,
            "maximum_validity_seconds": 172800,
            "minimum_validity_seconds": 21600,
            "origin": "rmac",
            "require_inrelease": True,
            "require_monotonic_date": True,
            "require_monotonic_snapshot": True,
            "signed_by_transition": True,
            "snapshot_field": "X-Rmac-Snapshot",
            "strong_hashes": ["SHA256", "SHA512"],
            "suite": "stable",
            "weak_hashes_forbidden": ["MD5Sum", "SHA1"],
        },
        "release",
    )
    _require_exact(
        policy["repository"],
        {
            "binary_packages": [
                "rmac-apps",
                "rmac-archive-keyring",
                "rmac-session",
            ],
            "source_packages": ["rmac", "rmac-archive-keyring"],
            "uri_placeholder": "@RMAC_REPOSITORY_URI@",
        },
        "repository",
    )
    _require_exact(
        policy["rollback"],
        {
            "automatic_downgrade": False,
            "emergency_revert": "publish-higher-version",
            "minimum_retained_snapshots": 3,
            "previous_release_state_required": True,
            "version_comparison": "dpkg",
        },
        "rollback",
    )
    _require_exact(
        policy["rollout"],
        {
            "eligibility": "apt-machine-id-local",
            "halt_percentage": 0,
            "minimum_observation_hours": 24,
            "normal_percentages": [10, 25, 50, 100],
            "package_field": "Phased-Update-Percentage",
            "security_percentage": 100,
            "telemetry": False,
        },
        "rollout",
    )
    _require_exact(
        policy["signing"],
        {
            "fingerprint_hex_lengths": [40, 64],
            "keyring_format": "binary-openpgp",
            "minimum_overlap_days": 30,
            "offline_primary": True,
            "online_signing_subkey": True,
            "revoked_key_rejected": True,
            "rotation_package_first": True,
            "single_valid_signature": True,
        },
        "signing",
    )
    _require_exact(
        policy["source"],
        {
            "artifacts": [".dsc", "source-tar", ".buildinfo", ".changes"],
            "cargo_lock_bound": True,
            "copyright_inventory_required": True,
            "deb_src_required": True,
            "license": "MIT",
            "license_text_required": True,
            "source_hashes": ["SHA256", "SHA512"],
        },
        "source",
    )

    sources = _regular_bytes(sources_path)
    preferences = _regular_bytes(preferences_path)
    if sources != EXPECTED_SOURCES:
        raise VerificationError("APT source template differs from the reviewed contract")
    if preferences != EXPECTED_PREFERENCES:
        raise VerificationError("APT preference differs from the reviewed contract")
    forbidden = (
        b"trusted=yes",
        b"allow-insecure=yes",
        b"check-valid-until=no",
        b"apt-key",
        b"/etc/apt/trusted.gpg",
        b"/etc/apt/trusted.gpg.d",
    )
    combined = sources.lower() + b"\n" + preferences.lower()
    if any(token in combined for token in forbidden):
        raise VerificationError("APT client template weakens repository trust")
    return hashlib.sha256(_regular_bytes(policy_path)).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args()
    try:
        digest = verify_policy()
    except VerificationError as error:
        parser.exit(4, f"verify-update-trust: {error}\n")
    print(f"rmac update trust policy verified ({digest[:12]})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
