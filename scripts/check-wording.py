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
    re.compile(r"\bdbus\b", re.IGNORECASE),
)


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


def main() -> int:
    violations: list[str] = []
    for path in rust_sources():
        # Inline unit tests are not user-facing; drop them before scanning.
        source = path.read_text(encoding="utf-8", errors="replace")
        source = source.split("#[cfg(test)]", 1)[0]
        for number, line in enumerate(source.splitlines(), 1):
            stripped = line.lstrip()
            if stripped.startswith("//"):
                continue
            for match in STRING.finditer(line):
                literal = match.group(1) or match.group(2) or ""
                # D-Bus interface names and Rust Debug shapes are protocol
                # constants, not user prose.
                if "org.freedesktop" in literal or literal.startswith("Error::"):
                    continue
                for pattern in FORBIDDEN:
                    if pattern.search(literal):
                        relative = path.relative_to(REPO_ROOT)
                        violations.append(f"{relative}:{number}: {literal}")
                        break
    for violation in violations:
        print(violation)
    print(f"\n{len(violations)} wording violation(s)")
    return 1 if violations and "--fail" in sys.argv else 0


if __name__ == "__main__":
    raise SystemExit(main())
