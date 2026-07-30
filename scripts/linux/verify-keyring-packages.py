#!/usr/bin/env python3
"""Verify the standard rmac archive-keyring binary and source package set."""

from __future__ import annotations

import argparse
from pathlib import Path

from keyring_package_contract import (
    KeyringPackageError,
    load_contract,
    verify_directory,
)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check-contract", action="store_true")
    parser.add_argument("--directory", type=Path)
    parser.add_argument(
        "--build-architecture",
        choices=("amd64", "arm64"),
    )
    arguments = parser.parse_args()
    try:
        if arguments.check_contract:
            if arguments.directory is not None or arguments.build_architecture is not None:
                raise KeyringPackageError("--check-contract cannot verify artifacts")
            load_contract()
            print("rmac archive keyring package contract verified")
            return 0
        if arguments.directory is None or arguments.build_architecture is None:
            raise KeyringPackageError(
                "--directory and --build-architecture are required"
            )
        manifest = verify_directory(
            arguments.directory,
            expected_architecture=arguments.build_architecture,
            run_dpkg_source=True,
        )
    except KeyringPackageError as error:
        parser.exit(4, f"verify-keyring-packages: {error}\n")
    print(
        "rmac archive keyring packages verified "
        f"({manifest['version']}, {len(manifest['artifacts'])} artifacts)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
