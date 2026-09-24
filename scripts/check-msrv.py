#!/usr/bin/env python3
"""Verify that the recorded MSRV agrees with the pinned toolchain.

Both workspaces pin an exact Rust toolchain in `rust-toolchain.toml` and
also record `rust-version` in `[workspace.package]`. Those two numbers
must always match: `rust-version` is what crates.io, other tooling, and
downstream consumers see, and it must not silently drift from what the
project actually builds with.

This performs no network access and no build; it only reads TOML files,
so it is cheap enough to run on every PR.
"""

from __future__ import annotations

import sys
import tomllib
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

WORKSPACES = [
    ("root", REPO_ROOT / "rust-toolchain.toml", REPO_ROOT / "Cargo.toml"),
    ("shell", REPO_ROOT / "shell" / "rust-toolchain.toml", REPO_ROOT / "shell" / "Cargo.toml"),
]


def toolchain_channel(path: Path) -> str:
    data = tomllib.loads(path.read_text())
    return data["toolchain"]["channel"]


def workspace_rust_version(path: Path) -> str:
    data = tomllib.loads(path.read_text())
    return data["workspace"]["package"]["rust-version"]


def main() -> int:
    status = 0
    for name, toolchain_path, cargo_path in WORKSPACES:
        channel = toolchain_channel(toolchain_path)
        rust_version = workspace_rust_version(cargo_path)
        if channel != rust_version:
            print(
                f"{name}: {toolchain_path} pins {channel!r} but "
                f"{cargo_path} declares rust-version {rust_version!r}",
                file=sys.stderr,
            )
            status = 1
        else:
            print(f"{name}: rust-version {rust_version} matches the pinned toolchain")
    return status


if __name__ == "__main__":
    raise SystemExit(main())
