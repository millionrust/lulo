"""What the live robot may and may not do (docs/behavior-suite.md, "Live robot").

Pure functions only, so ``scripts/test_robot_safety.py`` covers them.

The owner granted keyboard and mouse access to the live session on
2026-10-13 for robot testing, under strict rules: never confirm a
destructive or outward action, never touch personal files, never change
connectivity or account settings, one app instance at a time, and leave
the session as it was found. Everything the robot activates passes
``item_allowed`` first; anything unknown is refused, never guessed.
"""

from __future__ import annotations

import re
from typing import Optional

# Labels (case-insensitive word matches) the robot never activates, in any
# app or shell surface. Destructive, outward, session-ending, data-changing
# or connectivity/account-changing commands.
DENY_WORDS = (
    "delete", "remove", "erase", "format", "empty", "trash", "bin",
    "send", "reply", "forward", "redirect", "bounce", "junk", "archive",
    "shut", "restart", "log out", "logout", "sign out", "sleep", "lock",
    "uninstall", "install", "update", "upgrade", "reset", "quit", "force",
    "discard", "revert", "rename", "move", "duplicate", "alias", "compress",
    "eject", "burn", "cut", "paste", "clear", "undo", "redo", "import",
    "export", "share", "sync", "connect", "disconnect", "account", "password",
    "wi-fi", "wifi", "bluetooth", "network", "airplane", "login",
    "sharing", "users", "print", "save", "close all", "flag", "mark",
    "unread", "read", "new event", "new calendar", "new reminder",
    "new alarm", "new timer", "new mailbox", "new folder", "new smart",
    "new message", "new note", "new tab with", "subscribe", "accept",
    "decline", "invite", "attach", "pin", "unpin", "trust", "screen reader",
    "voiceover", "voice", "dictation", "kill", "terminate", "sample",
    "run", "execute", "inspect", "end process", "signal", "rotate",
    "crop", "markup", "annotate", "sign", "permanently", "replace",
    "transform", "convert", "find and replace", "spelling", "substitution",
    "get mail", "rebuild", "offline", "secure", "encrypt", "password",
    "open recent", "recent", "go to folder", "network", "computer",
    "airdrop", "icloud", "server", "add to", "favourite", "favorite",
    "location", "default", "reveal", "show in", "enclosing", "lap",
    "start", "stop", "set ", "keep",
)

# Action identifiers refused regardless of label (exact or prefix match).
DENY_ACTIONS = (
    "app::quit", "app::hide", "app::hide-others", "app::quit-keep-windows",
    "system::sleep", "system::restart", "system::shutdown", "system::lock",
    "system::logout", "system::force-quit", "system::software-center",
    "system::recents",
)

# Apps whose menus only get read-only view commands activated: they show
# the owner's mail, notes and calendar, so selection-dependent commands
# could change the owner's data.
VIEW_ONLY_APPS = {"Mail", "Notes", "Calendar", "Files"}

# Menus whose commands are safe to activate in a view-only app.
VIEW_ONLY_MENUS = {"View", "Go", "Window", "Help", "Application"}

# Chords the robot never presses: screen reader toggles, the power key,
# session and lock shortcuts.
DENY_CHORDS = {
    "cmd-f5", "cmd-alt-s", "ctrl-cmd-q", "cmd-shift-q", "cmd-alt-escape",
    "cmd-q-all", "cmd-backspace", "cmd-delete", "cmd-alt-backspace",
    "cmd-shift-backspace", "cmd-alt-shift-backspace", "cmd-alt-d",
}


def _normal(label: str) -> str:
    return re.sub(r"\s+", " ", label.replace("…", "").replace("...", "")).strip().lower()


def deny_reason(label: str, action: str = "") -> Optional[str]:
    """Why the robot must not activate this item, or ``None`` if it may."""

    for denied in DENY_ACTIONS:
        if action == denied:
            return f"action {denied} is denied"
    text = _normal(label)
    if not text:
        return "an unlabelled item"
    for word in DENY_WORDS:
        stripped = word.strip()
        if word.endswith(" "):
            if text.startswith(stripped + " "):
                return f"label starts with {stripped!r}"
            continue
        if re.search(r"(^|[^a-z])" + re.escape(stripped) + r"($|[^a-z])", text):
            return f"label contains {stripped!r}"
    return None


def item_allowed(app: str, menu: str, label: str, action: str, enabled: bool) -> tuple[bool, str]:
    """Whether the robot may activate one menu item, and why not."""

    if not enabled:
        return False, "disabled"
    reason = deny_reason(label, action)
    if reason:
        return False, reason
    if app in VIEW_ONLY_APPS and menu not in VIEW_ONLY_MENUS:
        return False, f"{app} is view-only outside {sorted(VIEW_ONLY_MENUS)}"
    return True, ""


def chord_allowed(chord: str) -> bool:
    return chord.strip().lower() not in DENY_CHORDS


# Panels/dialogs the robot may dismiss: it presses Esc, which every Lulo
# sheet and dialog maps to Cancel. It never presses Return in one.
DISMISS_KEYS = ("escape",)
