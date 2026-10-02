"""Name normalisation shared by diff.py and its tests.

macOS and Lulo call some things by different names. `APP_NAMES` maps a
Mac app's display name to the Lulo app it corresponds to; `LABEL_ALIASES`
does the same for menu/control labels that mean the same thing but are
spelled differently (the Mac's "Trash" is Lulo's "Bin", and so on).
"""

from __future__ import annotations

import re

# Mac app display name -> Lulo app display name.
APP_NAMES: dict[str, str] = {
    "Finder": "Finder",
    "TextEdit": "Text Editor",
    "Preview": "Preview",
    "Notes": "Notes",
    "Terminal": "Terminal",
    "Calculator": "Calculator",
    "Activity Monitor": "System Monitor",
    "System Settings": "System Settings",
    "Clock": "Clock",
    "Weather": "Weather",
}

# A label that means the same thing on both sides, spelled differently.
# Matched case-sensitively after trimming trailing "…"/ellipsis variants,
# so list both the Mac and the Lulo spelling once each, in either order.
LABEL_ALIASES: list[tuple[str, str]] = [
    ("Trash", "Bin"),
    ("Move to Trash", "Move to Bin"),
    ("Empty Trash", "Empty Bin"),
    ("Empty Trash…", "Empty Bin…"),
]

_ALIAS_TO_CANON: dict[str, str] = {}
for _a, _b in LABEL_ALIASES:
    canon = _a
    _ALIAS_TO_CANON[_a] = canon
    _ALIAS_TO_CANON[_b] = canon


def normalize_app_name(name: str) -> str:
    """Map a Mac app's display name to Lulo's for it; unknown names pass
    through unchanged (so a new app shows up as a visible gap, not a
    silent no-op)."""
    return APP_NAMES.get(name, name)


def _strip_ellipsis(label: str) -> tuple[str, bool]:
    had = label.endswith("…") or label.endswith("...")
    stripped = label[:-1] if label.endswith("…") else label[:-3] if label.endswith("...") else label
    return stripped, had


def normalize_label(label: str | None) -> str | None:
    """Fold Mac/Lulo spelling differences and whitespace so the same
    command compares equal regardless of side."""
    if label is None:
        return None
    text = label.strip()
    text = re.sub(r"\s+", " ", text)
    # AX reports the nonbreaking hyphen in Wi‑Fi while Linux labels use
    # ASCII hyphen; they name the same control.
    text = text.replace("‑", "-")
    # Finder validates these labels against the current selection at runtime.
    # The source inventory contains their unselected base names, while the
    # recorded Mac inventory contains the selected item's name.
    if re.fullmatch(r"Copy [“\"].+[”\"] as Pathname", text):
        text = "Copy as Pathname"
    elif re.fullmatch(r"Quick Look [“\"].+[”\"]", text):
        text = "Quick Look"
    elif re.fullmatch(r"Compress [“\"].+[”\"]", text):
        text = "Compress"
    elif re.fullmatch(r"Undo (?:Move|Copy|New Folder|Replace|Restore).+", text):
        text = "Undo"
    base, had_ellipsis = _strip_ellipsis(text)
    canon = _ALIAS_TO_CANON.get(base, base)
    if canon != base:
        text = canon + ("…" if had_ellipsis else "")
    else:
        text = base + ("…" if had_ellipsis else "")
    return text


def normalize_shortcut(shortcut: str | None) -> str:
    """Canonicalize a shortcut glyph string for comparison: normalise the
    hyphen/minus-sign used for "-" keys and drop surrounding whitespace.
    Modifier order is assumed already canonical (⌃⌥⇧⌘) on both sides."""
    if not shortcut:
        return ""
    text = shortcut.strip()
    # AX uses a private-use glyph for the Up Arrow key in Finder's Go menu.
    text = text.replace("", "↑")
    text = text.replace("-", "−")  # ASCII hyphen -> U+2212 MINUS SIGN
    return text
