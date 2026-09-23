#!/usr/bin/env python3
"""Verify the complete I7 user-documentation set and its local links."""

from __future__ import annotations

import json
from pathlib import Path
import re
import stat


REPO_ROOT = Path(__file__).resolve().parents[1]
MANIFEST_PATH = REPO_ROOT / "scripts/documentation-set.json"
MAX_BYTES = 512 * 1024
DOCUMENTS = (
    ("README.md", "# Lulo OS"),
    ("CONTRIBUTING.md", "# Contributing"),
    ("docs/install.md", "# Install rmac"),
    ("docs/hardware-support.md", "# Hardware support"),
    ("docs/user-guide.md", "# rmac user guide"),
    ("docs/settings-guide.md", "# System Settings guide"),
    ("docs/shortcuts.md", "# Keyboard shortcuts"),
    ("docs/privacy.md", "# Privacy"),
    ("docs/known-limitations.md", "# Known limitations"),
    ("docs/troubleshooting.md", "# Troubleshooting and recovery"),
    ("docs/update-and-remove.md", "# Updates, rollback, and removal"),
    ("docs/release-notes.md", "# Release notes"),
)
TOPICS = {
    "contribution": "CONTRIBUTING.md",
    "daily-use": "docs/user-guide.md",
    "hardware-support": "docs/hardware-support.md",
    "install": "docs/install.md",
    "limitations": "docs/known-limitations.md",
    "logs": "docs/troubleshooting.md",
    "privacy": "docs/privacy.md",
    "recovery": "docs/troubleshooting.md",
    "release-notes": "docs/release-notes.md",
    "rollback": "docs/update-and-remove.md",
    "safe-mode": "docs/troubleshooting.md",
    "settings": "docs/settings-guide.md",
    "shortcuts": "docs/shortcuts.md",
    "uninstall": "docs/update-and-remove.md",
    "update": "docs/update-and-remove.md",
}
LINK = re.compile(r"\[[^\]]+\]\(([^)]+)\)")
INCOMPLETE = re.compile(r"\b(?:TODO|TBD)\b")


class DocumentationError(RuntimeError):
    """A bounded documentation-set verification failure."""


def _read_regular(path: Path) -> str:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise DocumentationError(f"documentation file is unavailable: {path.name}") from error
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode):
        raise DocumentationError(f"documentation path is not regular: {path.name}")
    if metadata.st_size > MAX_BYTES:
        raise DocumentationError(f"documentation file is too large: {path.name}")
    try:
        raw = path.read_bytes()
    except OSError as error:
        raise DocumentationError(f"documentation file cannot be read: {path.name}") from error
    if len(raw) != metadata.st_size:
        raise DocumentationError(f"documentation file changed while reading: {path.name}")
    try:
        return raw.decode("utf-8")
    except UnicodeDecodeError as error:
        raise DocumentationError(f"documentation is not UTF-8: {path.name}") from error


def load_manifest(path: Path = MANIFEST_PATH) -> dict[str, object]:
    try:
        document = json.loads(_read_regular(path))
    except json.JSONDecodeError as error:
        raise DocumentationError("documentation manifest JSON is invalid") from error
    expected = {
        "documents": [
            {"path": path, "title": title} for path, title in DOCUMENTS
        ],
        "format": 1,
        "topics": TOPICS,
    }
    if document != expected:
        raise DocumentationError("documentation manifest differs from the required set")
    return document


def _validate_local_link(source: Path, destination: str) -> None:
    destination = destination.strip()
    if (
        not destination
        or destination.startswith("#")
        or re.match(r"^[a-zA-Z][a-zA-Z0-9+.-]*:", destination)
    ):
        return
    destination = destination.strip("<>").split("#", 1)[0]
    if not destination:
        return
    candidate = source.parent / destination
    if candidate.is_symlink():
        raise DocumentationError(
            f"documentation link from {source.name} is linked: {destination}"
        )
    target = candidate.resolve()
    try:
        target.relative_to(REPO_ROOT.resolve())
    except ValueError as error:
        raise DocumentationError("documentation link escapes the repository") from error
    if not target.exists() or target.is_symlink() or not target.is_file():
        raise DocumentationError(
            f"documentation link from {source.name} is unavailable: {destination}"
        )


def verify_documentation(manifest: dict[str, object]) -> None:
    document_paths = {entry["path"] for entry in manifest["documents"]}
    if set(manifest["topics"].values()) - document_paths:
        raise DocumentationError("documentation topic has no reviewed document")
    for entry in manifest["documents"]:
        path = REPO_ROOT / entry["path"]
        text = _read_regular(path)
        if not text.startswith(entry["title"] + "\n"):
            raise DocumentationError(f"documentation title differs: {entry['path']}")
        if INCOMPLETE.search(text):
            raise DocumentationError(f"documentation contains a placeholder: {entry['path']}")
        for destination in LINK.findall(text):
            _validate_local_link(path, destination)


def main() -> int:
    try:
        manifest = load_manifest()
        verify_documentation(manifest)
    except DocumentationError as error:
        raise SystemExit(f"verify-documentation: {error}") from error
    print(
        "rmac documentation set verified "
        f"({len(manifest['topics'])} required topics)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
