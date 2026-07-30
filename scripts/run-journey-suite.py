#!/usr/bin/env python3
"""Run the ten GOAL product-journey fixture suites sequentially."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time


REPO_ROOT = Path(__file__).resolve().parents[1]
MANIFEST_PATH = Path(__file__).with_name("journey-suite.json")
MAX_MANIFEST_BYTES = 256 * 1024
COVERAGE = {
    "accessibility",
    "failure",
    "happy-path",
    "keyboard",
    "persistence",
    "recovery",
    "scale",
    "service-fixture",
}


class JourneyError(RuntimeError):
    """A bounded journey-suite failure."""


def workspace_packages(root: Path = REPO_ROOT) -> set[str]:
    packages = set()
    for manifest in (root / "crates").glob("*/Cargo.toml"):
        try:
            raw = manifest.read_text(encoding="utf-8")
        except OSError as error:
            raise JourneyError("workspace package inventory is unavailable") from error
        match = re.search(r"(?m)^name\s*=\s*\"([a-z0-9][a-z0-9_-]*)\"\s*$", raw)
        if match is None:
            raise JourneyError("workspace package identity is invalid")
        packages.add(match.group(1))
    return packages


def load_manifest(path: Path = MANIFEST_PATH) -> dict[str, object]:
    try:
        metadata = path.lstat()
        raw = path.read_bytes()
    except OSError as error:
        raise JourneyError("journey manifest is unavailable") from error
    if path.is_symlink() or not path.is_file() or metadata.st_size > MAX_MANIFEST_BYTES:
        raise JourneyError("journey manifest is not a bounded regular file")
    if len(raw) != metadata.st_size:
        raise JourneyError("journey manifest changed while reading")
    try:
        document = json.loads(raw)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise JourneyError("journey manifest is invalid JSON") from error
    if not isinstance(document, dict) or set(document) != {
        "format",
        "journeys",
        "minimum_free_gib",
        "result_format",
    }:
        raise JourneyError("journey manifest fields are not exact")
    if (
        document["format"] != 1
        or type(document["format"]) is not int
        or document["result_format"] != 1
        or document["minimum_free_gib"] != 25
    ):
        raise JourneyError("journey manifest contract is invalid")
    journeys = document["journeys"]
    if not isinstance(journeys, list) or len(journeys) != 10:
        raise JourneyError("journey inventory is invalid")
    available = workspace_packages()
    expected_ids = list(range(1, 11))
    for expected_id, journey in zip(expected_ids, journeys, strict=True):
        if not isinstance(journey, dict) or set(journey) != {
            "coverage",
            "id",
            "name",
            "packages",
            "python_tests",
        }:
            raise JourneyError("journey fields are not exact")
        if (
            journey["id"] != expected_id
            or type(journey["id"]) is not int
            or not isinstance(journey["name"], str)
            or not re.fullmatch(r"[a-z]+(?:-[a-z]+)*", journey["name"])
        ):
            raise JourneyError("journey identity is invalid")
        packages = journey["packages"]
        if (
            not isinstance(packages, list)
            or not packages
            or packages != sorted(set(packages))
            or not set(packages) <= available
        ):
            raise JourneyError("journey package inventory is invalid")
        coverage = journey["coverage"]
        if (
            not isinstance(coverage, list)
            or coverage != sorted(set(coverage))
            or not set(coverage) <= COVERAGE
            or not {"failure", "happy-path", "recovery"} <= set(coverage)
        ):
            raise JourneyError("journey coverage is incomplete")
        tests = journey["python_tests"]
        if (
            not isinstance(tests, list)
            or tests != sorted(set(tests))
            or any(
                not isinstance(test, str)
                or not re.fullmatch(r"scripts\.test_[a-z_]+", test)
                for test in tests
            )
        ):
            raise JourneyError("journey Python test inventory is invalid")
    if (
        "accessibility" not in journeys[9]["coverage"]
        or "scale" not in journeys[9]["coverage"]
        or any("keyboard" not in journey["coverage"] for journey in journeys)
    ):
        raise JourneyError("cross-journey keyboard/accessibility coverage is incomplete")
    return document


def commands_for(journey: dict[str, object]) -> list[list[str]]:
    cargo = ["cargo", "test", "--locked"]
    for package in journey["packages"]:
        cargo.extend(["-p", package])
    commands = [cargo]
    if journey["python_tests"]:
        commands.append(
            [sys.executable, "-m", "unittest", *journey["python_tests"]]
        )
    return commands


def _run_command(command: list[str]) -> tuple[bool, int]:
    started = time.monotonic()
    try:
        result = subprocess.run(
            command,
            cwd=REPO_ROOT,
            env={**os.environ, "CARGO_TERM_COLOR": "never"},
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=3600,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired):
        return False, int((time.monotonic() - started) * 1000)
    return result.returncode == 0, int((time.monotonic() - started) * 1000)


def _revision() -> str:
    try:
        result = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=REPO_ROOT,
            check=False,
            capture_output=True,
            text=True,
            timeout=10,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise JourneyError("repository revision cannot be read") from error
    value = result.stdout.strip()
    if result.returncode != 0 or not re.fullmatch(r"[0-9a-f]{40}", value):
        raise JourneyError("repository revision is invalid")
    return value


def _clean_tracked_worktree() -> bool:
    return (
        subprocess.run(
            ["git", "diff", "--quiet", "--ignore-submodules", "--"],
            cwd=REPO_ROOT,
            check=False,
        ).returncode
        == 0
        and subprocess.run(
            ["git", "diff", "--cached", "--quiet", "--ignore-submodules", "--"],
            cwd=REPO_ROOT,
            check=False,
        ).returncode
        == 0
    )


def _publish(path: Path, document: dict[str, object]) -> None:
    if not path.is_absolute() or path == Path("/"):
        raise JourneyError("result path must be absolute")
    if path.exists() and (path.is_symlink() or not path.is_file()):
        raise JourneyError("result path must be an ordinary file")
    raw = (json.dumps(document, indent=2, sort_keys=True) + "\n").encode()
    temporary = None
    try:
        descriptor, name = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
        temporary = Path(name)
        with os.fdopen(descriptor, "wb") as output:
            output.write(raw)
            output.flush()
            os.fsync(output.fileno())
        temporary.chmod(0o644)
        os.replace(temporary, path)
    except OSError as error:
        raise JourneyError("journey result cannot be published") from error
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def _previous_passes(path: Path, revision: str) -> dict[int, dict[str, object]]:
    if not path.exists():
        return {}
    try:
        metadata = path.lstat()
        if (
            path.is_symlink()
            or not path.is_file()
            or metadata.st_size > MAX_MANIFEST_BYTES
        ):
            raise JourneyError("existing journey result is not a bounded regular file")
        raw = path.read_bytes()
        document = json.loads(raw)
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise JourneyError("existing journey result is invalid") from error
    if (
        len(raw) > MAX_MANIFEST_BYTES
        or not isinstance(document, dict)
        or set(document) != {"format", "results", "revision"}
        or document["format"] != 1
        or document["revision"] != revision
        or not isinstance(document["results"], list)
    ):
        raise JourneyError("existing journey result does not match this revision")
    passes = {}
    for result in document["results"]:
        if (
            not isinstance(result, dict)
            or set(result) != {"duration_ms", "id", "name", "status"}
            or result["status"] not in {"pass", "fail"}
            or type(result["id"]) is not int
            or type(result["duration_ms"]) is not int
            or result["duration_ms"] < 0
            or not isinstance(result["name"], str)
        ):
            raise JourneyError("existing journey result record is invalid")
        if result["status"] == "pass":
            if result["id"] in passes:
                raise JourneyError("existing journey result identity is duplicated")
            passes[result["id"]] = result
    return passes


def run(
    manifest: dict[str, object],
    *,
    selected: set[int],
    output: Path,
    fail_fast: bool,
    resume: bool,
) -> bool:
    minimum = manifest["minimum_free_gib"] * 1024**3
    if shutil.disk_usage(REPO_ROOT).free < minimum:
        raise JourneyError("journey suite requires at least 25 GiB free")
    if not _clean_tracked_worktree():
        raise JourneyError("journey suite requires a clean tracked worktree")
    revision = _revision()
    previous = _previous_passes(output, revision) if resume else {}
    results = []
    passed = True
    for journey in manifest["journeys"]:
        if journey["id"] not in selected:
            continue
        if journey["id"] in previous:
            result = previous[journey["id"]]
            if result["name"] != journey["name"]:
                raise JourneyError("existing journey result identity differs")
            results.append(result)
            continue
        duration = 0
        status = "pass"
        for command in commands_for(journey):
            succeeded, elapsed = _run_command(command)
            duration += elapsed
            if not succeeded:
                status = "fail"
                passed = False
                break
        results.append(
            {
                "duration_ms": duration,
                "id": journey["id"],
                "name": journey["name"],
                "status": status,
            }
        )
        _publish(
            output,
            {
                "format": manifest["result_format"],
                "results": results,
                "revision": revision,
            },
        )
        if fail_fast and status == "fail":
            break
    return passed


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--journey", type=int, action="append")
    parser.add_argument("--list", action="store_true")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--fail-fast", action="store_true")
    parser.add_argument("--resume", action="store_true")
    arguments = parser.parse_args()
    try:
        manifest = load_manifest()
        if arguments.list:
            for journey in manifest["journeys"]:
                print(f"{journey['id']}: {journey['name']}")
            return 0
        selected = set(arguments.journey or range(1, 11))
        if not selected or not selected <= set(range(1, 11)):
            raise JourneyError("selected journey is invalid")
        if arguments.output is None:
            raise JourneyError("--output is required when running journeys")
        passed = run(
            manifest,
            selected=selected,
            output=arguments.output,
            fail_fast=arguments.fail_fast,
            resume=arguments.resume,
        )
    except JourneyError as error:
        parser.exit(4, f"run-journey-suite: {error}\n")
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
