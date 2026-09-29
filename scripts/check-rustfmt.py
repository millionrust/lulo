#!/usr/bin/env python3
"""Check formatting for root workspace packages, excluding local path dependencies."""

from __future__ import annotations

import json
from pathlib import Path
import subprocess


ROOT = Path(__file__).resolve().parents[1]


def workspace_packages(metadata: dict) -> list[str]:
    members = set(metadata["workspace_members"])
    packages = {
        package["id"]: package["name"]
        for package in metadata["packages"]
        if package["id"] in members
    }
    if not members or set(packages) != members or len(set(packages.values())) != len(members):
        raise ValueError("Cargo metadata has an invalid workspace package inventory")
    return sorted(packages.values())


def main() -> None:
    result = subprocess.run(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    packages = workspace_packages(json.loads(result.stdout))
    command = ["cargo", "fmt"]
    for package in packages:
        command.extend(("--package", package))
    subprocess.run([*command, "--", "--check"], cwd=ROOT, check=True)
    print(f"Formatting passed for {len(packages)} root workspace packages")


if __name__ == "__main__":
    main()
