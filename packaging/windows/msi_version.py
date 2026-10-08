#!/usr/bin/env python3
"""The MSI ProductVersion for a Lulo build (ADR 0023 "Installer", WIN-OS-54).

Windows Installer compares only the first three fields of a ProductVersion
(major.minor.build; major and minor at most 255, build at most 65535) and
ignores any fourth. Every Lulo build used to be "0.9.0" (the numeric part of
0.9.0-beta.1), so installing a newer MSI over an older one was a same-version
install: msiexec exited 0 and left the old exes in place.

The mapping: ProductVersion = <major>.<minor>.<build>, where <build> is the
number of commits in the build's history (`git rev-list --count HEAD`), which
only grows along the branch builds are made from. The semver patch and
pre-release tag are not in the MSI version (Windows has no place for them);
each exe's own VERSIONINFO strings and the MSI's file name keep the full
version. A later release always has more commits, so 0.9.0-beta.1 at commit
3947 installs as 0.9.3947 and 0.9.1 at commit 4100 as 0.9.4100; a new minor
or major version sorts above every build of the one before. MajorUpgrade
with AllowSameVersionUpgrades (Product.wxs) also replaces a build re-stamped
with the same number.

Usage: msi_version.py <semver> <build-number>   -> prints e.g. 0.9.3947
"""

from __future__ import annotations

import re
import sys

MAX_MAJOR_MINOR = 255
MAX_BUILD = 65535


def msi_version(semver: str, build: int) -> str:
    match = re.match(r"^(\d+)\.(\d+)\.(\d+)(?:[-+].*)?$", semver.strip())
    if not match:
        raise ValueError(f"not a semver version: {semver!r}")
    major, minor = int(match.group(1)), int(match.group(2))
    if major > MAX_MAJOR_MINOR or minor > MAX_MAJOR_MINOR:
        raise ValueError(f"{semver}: MSI major and minor must be at most {MAX_MAJOR_MINOR}")
    if not 1 <= build <= MAX_BUILD:
        raise ValueError(f"build number {build} must be between 1 and {MAX_BUILD}")
    return f"{major}.{minor}.{build}"


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        sys.stderr.write(__doc__ or "")
        return 2
    try:
        print(msi_version(argv[1], int(argv[2])))
    except ValueError as error:
        sys.stderr.write(f"msi_version.py: {error}\n")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
