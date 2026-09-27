#!/usr/bin/env python3
"""Audit recorded Mac/Lulo screenshot pairs without scoring visual similarity.

Usage: python3 scripts/audit-visual-comparisons.py [comparison-root]
The default root is target/evidence/visual-comparisons.
"""

from __future__ import annotations

import argparse
import hashlib
import re
import stat
import struct
import sys
from pathlib import Path


DEFAULT_ROOT = Path(__file__).resolve().parents[1] / "target/evidence/visual-comparisons"
PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
REQUIRED = {
    "Mac image": "mac.png",
    "Mac SHA-256": None,
    "Mac pixel dimensions": None,
    "Mac OS/build": None,
    "Mac display": None,
    "Mac appearance": None,
    "Mac locale/region": None,
    "Mac state": None,
    "Lulo image": "lulo.png",
    "Lulo SHA-256": None,
    "Lulo pixel dimensions": None,
    "Lulo revision": None,
    "Lulo display": None,
    "Lulo appearance": None,
    "Lulo locale/region": None,
    "Lulo state": None,
    "Review": None,
    "Findings": None,
    "Reviewer/date": None,
}
HASH_RE = re.compile(r"[0-9a-f]{64}\Z")
DIMENSIONS_RE = re.compile(r"([1-9][0-9]*)\s*[×xX]\s*([1-9][0-9]*)\Z")


class AuditError(ValueError):
    """A comparison record is missing, malformed, or inconsistent."""


def png_dimensions(path: Path) -> tuple[int, int]:
    try:
        with path.open("rb") as image:
            header = image.read(24)
    except OSError as error:
        raise AuditError("cannot read image") from error
    if len(header) != 24 or header[:8] != PNG_SIGNATURE or header[12:16] != b"IHDR":
        raise AuditError("not a PNG with a valid IHDR header")
    width, height = struct.unpack(">II", header[16:24])
    if width == 0 or height == 0:
        raise AuditError("PNG dimensions are zero")
    return width, height


def regular_file(path: Path) -> bool:
    try:
        info = path.lstat()
    except OSError:
        return False
    return not path.is_symlink() and stat.S_ISREG(info.st_mode)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as source:
            for block in iter(lambda: source.read(1024 * 1024), b""):
                digest.update(block)
    except OSError as error:
        raise AuditError("cannot read image") from error
    return digest.hexdigest()


def parse_record(path: Path) -> dict[str, str]:
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as error:
        raise AuditError("cannot read comparison.md as UTF-8") from error
    values: dict[str, str] = {}
    for line in text.splitlines():
        if ":" not in line:
            continue
        key, value = line.split(":", 1)
        key, value = key.strip(), value.strip()
        if key in REQUIRED:
            if key in values:
                raise AuditError(f"duplicate field: {key}")
            values[key] = value
    missing = [key for key in REQUIRED if not values.get(key)]
    if missing:
        raise AuditError("missing fields: " + ", ".join(missing))
    placeholders = [key for key, value in values.items() if value.startswith("<") and value.endswith(">")]
    if placeholders:
        raise AuditError("unfilled fields: " + ", ".join(placeholders))
    for key, expected in REQUIRED.items():
        if expected is not None and values[key] != expected:
            raise AuditError(f"{key} must be {expected}")
    if values["Review"] not in {"pass", "mismatch", "not comparable"}:
        raise AuditError("Review must be pass, mismatch, or not comparable")
    return values


def audit_pair(directory: Path) -> list[str]:
    errors = []
    if directory.is_symlink() or not directory.is_dir():
        return ["comparison path is not an ordinary directory"]
    images = {name: directory / name for name in ("mac.png", "lulo.png")}
    for name, path in images.items():
        if not regular_file(path):
            errors.append(f"missing or non-regular {name}")
    record = directory / "comparison.md"
    if not regular_file(record):
        errors.append("missing or non-regular comparison.md")
    if errors:
        return errors
    try:
        values = parse_record(record)
        for prefix, name in (("Mac", "mac.png"), ("Lulo", "lulo.png")):
            path = images[name]
            recorded_hash = values[f"{prefix} SHA-256"]
            if not HASH_RE.fullmatch(recorded_hash):
                raise AuditError(f"{prefix} SHA-256 is malformed")
            if sha256(path) != recorded_hash:
                raise AuditError(f"{prefix} SHA-256 does not match {name}")
            dimensions = png_dimensions(path)
            match = DIMENSIONS_RE.fullmatch(values[f"{prefix} pixel dimensions"])
            if not match or tuple(map(int, match.groups())) != dimensions:
                raise AuditError(f"{prefix} pixel dimensions do not match {name}")
    except AuditError as error:
        errors.append(str(error))
    return errors


def audit(root: Path) -> tuple[list[tuple[str, list[str]]], list[str]]:
    if not root.exists():
        return [], ["comparison root does not exist"]
    if root.is_symlink() or not root.is_dir():
        return [], ["comparison root is not an ordinary directory"]
    pairs = []
    for directory in sorted(root.iterdir(), key=lambda item: item.name):
        if directory.name.startswith("."):
            continue
        pairs.append((directory.name, audit_pair(directory)))
    if not pairs:
        return [], ["no comparison pairs found"]
    return pairs, []


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", nargs="?", type=Path, default=DEFAULT_ROOT)
    args = parser.parse_args(argv)
    pairs, root_errors = audit(args.root)
    for error in root_errors:
        print(f"ERROR  {error}")
    failures = bool(root_errors)
    for name, errors in pairs:
        if errors:
            failures = True
            print(f"FAIL   {name}: " + "; ".join(errors))
        else:
            print(f"OK     {name}")
    if pairs:
        print(f"\n{sum(not errors for _, errors in pairs)}/{len(pairs)} comparison pairs valid")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
