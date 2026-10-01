#!/usr/bin/env python3
"""Copy exactly the source-pinned third-party deb pair into a candidate."""

from __future__ import annotations

import argparse
from pathlib import Path
import shutil

from third_party_packages import load_pins


def copy_pair(source: Path, destination: Path, architecture: str) -> None:
    expected = {
        f"{pin.name}_{pin.debian_version}_{architecture}.deb"
        for pin in load_pins().values()
    }
    if {path.name for path in source.glob("*.deb")} != expected:
        raise ValueError("third-party deb inventory differs from the source pins")
    for name in sorted(expected):
        path = source / name
        if path.is_symlink() or not path.is_file() or (destination / name).exists():
            raise ValueError(f"third-party package path is invalid: {name}")
        shutil.copyfile(path, destination / name)
        (destination / name).chmod(0o644)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--destination", required=True, type=Path)
    parser.add_argument("--architecture", required=True, choices=("amd64", "arm64"))
    arguments = parser.parse_args()
    copy_pair(arguments.source, arguments.destination, arguments.architecture)


if __name__ == "__main__":
    main()
