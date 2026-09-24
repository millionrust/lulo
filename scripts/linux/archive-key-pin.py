#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Read and check the archive-key pin in packaging/apt/archive-key.json.

The pin is the single reviewed record of which OpenPGP primary key(s) the
rmac APT archive uses. scripts/release/create-archive-key.sh writes it
together with packaging/apt/archive-keyring.asc and the fingerprint in
scripts/linux/install.sh; the release workflow and the publisher read it.

  --check                     the file is well formed (an empty list is allowed:
                              no key exists yet)
  --require FPR               FPR is one of the pinned primary fingerprints
  --fingerprint-arguments     print "--fingerprint FPR ..." for
                              build-keyring-packages.py
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import sys
from typing import Tuple


REPO_ROOT = Path(__file__).resolve().parents[2]
PIN_PATH = REPO_ROOT / "packaging/apt/archive-key.json"
FINGERPRINT_RE = re.compile(r"(?:[0-9A-F]{40}|[0-9A-F]{64})")
KEYRING = "packaging/apt/archive-keyring.asc"


class PinError(RuntimeError):
    pass


def load(path: Path = PIN_PATH) -> Tuple[str, ...]:
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise PinError("packaging/apt/archive-key.json is unreadable") from error
    if not isinstance(document, dict) or set(document) != {
        "format",
        "primary_fingerprints",
        "public_keyring",
    }:
        raise PinError("archive-key.json fields are not exact")
    values = document["primary_fingerprints"]
    if (
        document["format"] != 1
        or document["public_keyring"] != KEYRING
        or not isinstance(values, list)
        or len(values) > 2
        or values != sorted(set(values))
        or any(not isinstance(value, str) or not FINGERPRINT_RE.fullmatch(value) for value in values)
    ):
        raise PinError("archive-key.json must list at most two sorted uppercase primary fingerprints")
    if values and not (path.parent.parent.parent / KEYRING).is_file():
        raise PinError(f"a key is pinned but {KEYRING} is missing")
    return tuple(values)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--require")
    parser.add_argument("--fingerprint-arguments", action="store_true")
    arguments = parser.parse_args()
    try:
        pinned = load()
        if arguments.require is not None and arguments.require not in pinned:
            raise PinError("the fingerprint is not pinned in packaging/apt/archive-key.json")
        if arguments.fingerprint_arguments:
            if not pinned:
                raise PinError("no archive key is pinned yet; run scripts/release/create-archive-key.sh")
            print(" ".join(f"--fingerprint {value}" for value in pinned))
    except PinError as error:
        parser.exit(2, f"archive-key-pin: {error}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
