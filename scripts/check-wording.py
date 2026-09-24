#!/usr/bin/env python3
"""Report user-facing wording that violates FEEL_SPEC.md §D.10.

Scans product Rust source for the string patterns the spec says must never
reach a user. It is a report, not yet a gate: run it, fix the strings, then
wire it into CI once the tree is clean.

Usage:
    python3 scripts/check-wording.py [--fail]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
SCAN_ROOTS = (
    REPO_ROOT / "crates",
    REPO_ROOT / "shell" / "bins",
    REPO_ROOT / "shell" / "crates",
)

# A user-visible string literal (single or double quoted, may span one line).
STRING = re.compile(r'"((?:[^"\\]|\\.)*)"|\'((?:[^\'\\]|\\.)*)\'')
FORBIDDEN = (
    re.compile(r"Error:"),
    re.compile(r"Failed to"),
    re.compile(r"Warning:"),
    re.compile(r"/home/"),
    re.compile(r"\bdbus\b"),
)


# A Rust line carrying this trailing comment holds a literal the user never
# sees (a program name to match, a word to screen error text for).
INTERNAL_MARKER = "// wording: internal"

# Desktop-entry fields that are shown to the user.
DESKTOP_KEY = re.compile(r"^(Name|GenericName|Comment|X-GNOME-FullName)=")


def rust_sources() -> list[Path]:
    files: list[Path] = []
    for root in SCAN_ROOTS:
        if not root.is_dir():
            continue
        for path in root.rglob("*.rs"):
            if "test" in path.name.lower():
                continue
            files.append(path)
    return sorted(files)


def desktop_sources() -> list[Path]:
    packaging = REPO_ROOT / "packaging"
    return sorted(packaging.rglob("*.desktop")) if packaging.is_dir() else []


def scan_literal(relative: Path, number: int, literal: str) -> str | None:
    if "org.freedesktop" in literal or literal.startswith("Error::"):
        return None
    for pattern in FORBIDDEN:
        if pattern.search(literal):
            return f"{relative}:{number}: {literal}"
    return None


def scan_rust_line(relative: Path, number: int, line: str) -> list[str]:
    stripped = line.lstrip()
    if stripped.startswith("//") or line.rstrip().endswith(INTERNAL_MARKER):
        return []
    found = []
    for match in STRING.finditer(line):
        literal = match.group(1) or match.group(2) or ""
        violation = scan_literal(relative, number, literal)
        if violation is not None:
            found.append(violation)
    return found


def main() -> int:
    violations: list[str] = []
    for path in rust_sources():
        # Inline unit tests are not user-facing; drop them before scanning.
        source = path.read_text(encoding="utf-8", errors="replace")
        source = source.split("#[cfg(test)]", 1)[0]
        for number, line in enumerate(source.splitlines(), 1):
            violations.extend(
                scan_rust_line(path.relative_to(REPO_ROOT), number, line)
            )
    for path in desktop_sources():
        for number, line in enumerate(
            path.read_text(encoding="utf-8", errors="replace").splitlines(), 1
        ):
            if not DESKTOP_KEY.match(line):
                continue
            _, _, value = line.partition("=")
            found = scan_literal(path.relative_to(REPO_ROOT), number, value)
            if found is not None:
                violations.append(found)
    for violation in violations:
        print(violation)
    print(f"\n{len(violations)} wording violation(s)")
    return 1 if violations and "--fail" in sys.argv else 0


if __name__ == "__main__":
    raise SystemExit(main())
