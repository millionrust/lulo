#!/usr/bin/env python3
"""Download and verify the latest successful candidate for one Git commit."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tempfile

from native_package_contract import native_version


REPO_ROOT = Path(__file__).resolve().parents[2]
SHA = re.compile(r"[0-9a-f]{40}\Z")
CHECKSUM_LINE = re.compile(r"([0-9a-f]{64})  ([A-Za-z0-9+_.~-]+\.deb)\Z")


class FetchError(RuntimeError):
    """A candidate fetch or validation failure."""


def command(*arguments: str, cwd: Path = REPO_ROOT) -> str:
    try:
        result = subprocess.run(
            arguments, cwd=cwd, check=True, text=True, capture_output=True
        )
    except (OSError, subprocess.CalledProcessError) as error:
        raise FetchError(f"command failed: {arguments[0]} {arguments[1]}") from error
    return result.stdout.strip()


def selected_run(raw: str, sha: str) -> int:
    try:
        runs = json.loads(raw)
    except json.JSONDecodeError as error:
        raise FetchError("gh returned invalid run data") from error
    if not isinstance(runs, list):
        raise FetchError("gh returned invalid run data")
    matching = [
        run
        for run in runs
        if isinstance(run, dict)
        and run.get("headSha") == sha
        and run.get("conclusion") == "success"
        and type(run.get("databaseId")) is int
        and run["databaseId"] > 0
        and isinstance(run.get("createdAt"), str)
    ]
    if not matching:
        raise FetchError(f"no successful candidate run exists for {sha}")
    return max(matching, key=lambda run: (run["createdAt"], run["databaseId"]))[
        "databaseId"
    ]


def verify_checksums(directory: Path) -> None:
    checksum_path = directory / "SHA256SUMS"
    if checksum_path.is_symlink() or not checksum_path.is_file():
        raise FetchError("candidate SHA256SUMS is missing or invalid")
    try:
        lines = checksum_path.read_text(encoding="ascii").splitlines()
    except (OSError, UnicodeError) as error:
        raise FetchError("candidate SHA256SUMS cannot be read") from error
    if len(lines) != 4:
        raise FetchError("candidate must contain checksums for exactly four debs")
    seen: set[str] = set()
    for line in lines:
        match = CHECKSUM_LINE.fullmatch(line)
        if match is None or match[2] in seen:
            raise FetchError("candidate SHA256SUMS has an invalid or duplicate entry")
        seen.add(match[2])
        archive = directory / match[2]
        if archive.is_symlink() or not archive.is_file():
            raise FetchError(f"candidate deb is missing or invalid: {match[2]}")
        digest_hash = hashlib.sha256()
        with archive.open("rb") as source:
            while chunk := source.read(1024 * 1024):
                digest_hash.update(chunk)
        digest = digest_hash.hexdigest()
        if digest != match[1]:
            raise FetchError(f"candidate checksum differs: {match[2]}")
    actual = {path.name for path in directory.glob("*.deb")}
    if actual != seen:
        raise FetchError("candidate deb inventory differs from SHA256SUMS")


def candidate_profile(directory: Path) -> str:
    manifest = directory / "native-packages.json"
    if manifest.is_symlink() or not manifest.is_file():
        raise FetchError("candidate native-packages.json is missing or invalid")
    try:
        version = json.loads(manifest.read_text(encoding="utf-8"))["version"]
    except (OSError, UnicodeError, ValueError, KeyError, TypeError) as error:
        raise FetchError("candidate manifest has no readable version") from error
    for profile, metadata in (("release", None), ("iterate", "iterate")):
        if version == native_version(REPO_ROOT, build_metadata=metadata):
            return profile
    raise FetchError("candidate version does not match the checked-out commit")


def fetch(commit: str) -> tuple[Path, int, str]:
    sha = command("git", "rev-parse", "--verify", f"{commit}^{{commit}}")
    if not SHA.fullmatch(sha) or command("git", "rev-parse", "HEAD") != sha:
        raise FetchError("check out the requested full commit before fetching")
    destination = Path.home() / "rmac-release" / f"packages-ci-{sha}"
    if destination.exists() or destination.is_symlink():
        raise FetchError(f"candidate destination already exists: {destination}")
    run_id = selected_run(
        command(
            "gh", "run", "list", "--workflow", "candidate.yml", "--commit", sha,
            "--status", "success", "--json",
            "databaseId,headSha,conclusion,createdAt", "--limit", "100"
        ),
        sha,
    )
    destination.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix=f".packages-ci-{sha}.", dir=destination.parent))
    try:
        command(
            "gh", "run", "download", str(run_id), "--name", f"lulo-candidate-{sha}",
            "--dir", str(staging)
        )
        verify_checksums(staging)
        profile = candidate_profile(staging)
        verify = [
            sys.executable,
            str(REPO_ROOT / "scripts/linux/verify-native-packages.py"),
            "--directory", str(staging), "--architecture", "amd64",
        ]
        if profile == "iterate":
            verify.extend(("--build-metadata", "iterate"))
        command(*verify)
        os.replace(staging, destination)
    finally:
        if staging.exists():
            shutil.rmtree(staging)
    return destination, run_id, profile


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("commit", help="the exact checked-out candidate commit")
    arguments = parser.parse_args()
    try:
        destination, run_id, profile = fetch(arguments.commit)
    except FetchError as error:
        parser.exit(2, f"fetch-candidate: {error}\n")
    installer = REPO_ROOT / "scripts/linux/install-native-candidate.sh"
    args = ["bash", str(installer), "--check", "--directory", str(destination)]
    if profile == "iterate":
        args.extend(("--build-metadata", "iterate"))
    print(f"Verified candidate from successful run {run_id}: {destination}")
    print("Install preflight: " + shlex.join(args))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
