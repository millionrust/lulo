#!/usr/bin/env python3
"""Safely inspect the live top-bar Shut Down menu row over AT-SPI.

The filename is retained for existing invocations. Opening the confirmation
starts a 60-second automatic shutdown countdown, so this probe stops at the
menu row and never activates it. Only the menu toggle can be clicked.
Run as the expected logged-in user over SSH or in that user's graphical shell.
"""
from __future__ import annotations

import getpass
import os
import platform
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Iterable

try:
    import pyatspi  # type: ignore[import-not-found]
except ImportError:  # pragma: no cover - available on the Linux reference host
    pyatspi = None


TIMEOUT_SECONDS = 8.0
POLL_SECONDS = 0.2
TOP_BAR_NAMES = {"rmac-top-bar", "top-bar", "rmac-menubar"}
EXPECTED_USER = "jacob"


class ProbeError(RuntimeError):
    pass


@dataclass(frozen=True)
class Session:
    user: str
    uid: int
    runtime_dir: Path
    wayland_display: str
    session_id: str


def _properties(lines: Iterable[str]) -> dict[str, str]:
    values: dict[str, str] = {}
    for line in lines:
        key, sep, value = line.partition("=")
        if sep:
            values[key] = value
    return values


def _read_command(argv: list[str], env: dict[str, str] | None = None) -> str:
    try:
        result = subprocess.run(argv, check=True, capture_output=True, text=True,
                                timeout=5, env=env)
    except (OSError, subprocess.SubprocessError) as error:
        raise ProbeError(f"read-only session check failed ({argv[0]}): {error}") from error
    return result.stdout


def discover_live_session(expected_user: str) -> tuple[Session, dict[str, str]]:
    """Require one active local Wayland session belonging to expected_user."""
    if platform.system() != "Linux":
        raise ProbeError("refusing: this probe requires the live Linux session")
    if getpass.getuser() != expected_user:
        raise ProbeError(f"refusing: expected user {expected_user!r}, got {getpass.getuser()!r}")
    uid = os.getuid()
    if uid == 0:
        raise ProbeError("refusing to run as root")
    runtime = Path(f"/run/user/{uid}")
    base_env = dict(os.environ)
    base_env["XDG_RUNTIME_DIR"] = str(runtime)
    base_env.setdefault("DBUS_SESSION_BUS_ADDRESS", f"unix:path={runtime}/bus")
    if not runtime.is_dir() or not (runtime / "bus").exists():
        raise ProbeError(f"refusing: expected live user bus is absent at {runtime}/bus")

    sessions = _read_command(["loginctl", "list-sessions", "--no-legend", "--no-pager"])
    matches: list[tuple[str, dict[str, str]]] = []
    for line in sessions.splitlines():
        fields = line.split()
        if len(fields) < 3 or fields[2] != expected_user:
            continue
        session_id = fields[0]
        props = _properties(_read_command([
            "loginctl", "show-session", session_id, "--no-pager",
            "-p", "Name", "-p", "Type", "-p", "Active", "-p", "Remote",
            "-p", "State", "-p", "Seat", "-p", "Class",
        ]).splitlines())
        if (props.get("Name") == expected_user and props.get("Type") == "wayland"
                and props.get("Active") == "yes" and props.get("Remote") == "no"
                and props.get("State") == "active" and props.get("Seat") == "seat0"
                and props.get("Class") == "user"):
            matches.append((session_id, props))
    if len(matches) != 1:
        raise ProbeError(f"refusing: expected exactly one active local Wayland session for {expected_user}; found {len(matches)}")

    manager_env = _read_command(["systemctl", "--user", "show-environment"], base_env)
    live_env = dict(base_env)
    live_env.update(_properties(manager_env.splitlines()))
    display = live_env.get("WAYLAND_DISPLAY", "")
    if display != "wayland-1":
        raise ProbeError(f"refusing: expected WAYLAND_DISPLAY=wayland-1, got {display!r}")
    if live_env.get("XDG_RUNTIME_DIR") != str(runtime):
        raise ProbeError("refusing: user manager runtime directory does not match this UID")
    bus = live_env.get("DBUS_SESSION_BUS_ADDRESS", "")
    if not bus.startswith(f"unix:path={runtime}/bus"):
        raise ProbeError("refusing: session bus address does not match the expected live user bus")
    if not (runtime / display).exists():
        raise ProbeError(f"refusing: expected Wayland socket is absent at {runtime / display}")
    live_env["XDG_SESSION_ID"] = matches[0][0]
    return Session(expected_user, uid, runtime, display, matches[0][0]), live_env


def role(node: object) -> str:
    try:
        return node.getRoleName()  # type: ignore[attr-defined,no-any-return]
    except Exception:  # noqa: BLE001
        return ""


def name(node: object) -> str:
    try:
        return node.name or ""  # type: ignore[attr-defined,no-any-return]
    except Exception:  # noqa: BLE001
        return ""


def descendants(root: object, limit: int = 32):
    stack = [(root, 0)]
    while stack:
        node, depth = stack.pop()
        yield node
        if depth >= limit:
            continue
        try:
            children = [node.getChildAtIndex(i) for i in range(node.childCount)]  # type: ignore[attr-defined]
        except Exception:  # noqa: BLE001
            continue
        stack.extend((child, depth + 1) for child in reversed(children) if child is not None)


class AtspiBackend:
    def __init__(self, api: object):
        self.api = api
        self.desktop = api.Registry.getDesktop(0)  # type: ignore[attr-defined]
        self.top_bar = self._find_top_bar()

    def _find_top_bar(self):
        apps = []
        try:
            apps = [self.desktop.getChildAtIndex(i) for i in range(self.desktop.childCount)]
        except Exception as error:  # noqa: BLE001
            raise ProbeError(f"cannot read AT-SPI desktop: {error}") from error
        matches = [app for app in apps if app is not None and name(app) in TOP_BAR_NAMES]
        if len(matches) != 1:
            raise ProbeError(f"expected one top-bar AT-SPI app {sorted(TOP_BAR_NAMES)}, found {[name(app) for app in matches]}")
        return matches[0]

    def find(self, wanted_role: str, wanted_name: str):
        return next((node for node in descendants(self.top_bar)
                     if role(node) in {wanted_role, "button"} and name(node) == wanted_name), None)

    def has_confirmation(self) -> bool:
        return (self.find("push button", "Shut Down") is not None
                and self.find("push button", "Cancel") is not None)

    def activate_click(self, node: object) -> None:
        identity = (role(node), name(node))
        allowed = {
            ("push button", "menu"),
            ("button", "menu"),
        }
        if identity not in allowed:
            raise ProbeError(f"refusing AT-SPI activation outside the safe allowlist: {identity!r}")
        action = node.queryAction()  # type: ignore[attr-defined]
        for index in range(action.nActions):
            if action.getName(index) == "click":
                action.doAction(index)
                return
        raise ProbeError(f"AT-SPI node {role(node)} {name(node)!r} has no click action")

    def wait(self, predicate: Callable[[], object], label: str, timeout: float = TIMEOUT_SECONDS):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            result = predicate()
            if result:
                return result
            time.sleep(POLL_SECONDS)
        raise ProbeError(f"timed out waiting for {label}")


def run_probe(backend: AtspiBackend, report: Callable[[str], None] = print) -> None:
    """Check the live menu row without starting its auto-shutdown countdown."""
    if backend.has_confirmation() or backend.find("menu item", "Shut Down…") is not None:
        raise ProbeError("refusing: the system menu or confirmation was already open")
    menu_may_be_open = False
    try:
        logo = backend.wait(lambda: backend.find("push button", "menu"), "top-bar menu button")
        report("state: live top-bar menu button found")
        menu_may_be_open = True
        backend.activate_click(logo)
        menu_item = backend.wait(lambda: backend.find("menu item", "Shut Down…"),
                                 "Shut Down… menu item")
        if menu_item is None:
            raise ProbeError("Shut Down… row is unavailable")
        report("state: system menu open; Shut Down… row found; confirmation not opened")
    finally:
        try:
            confirmation_still_open = backend.has_confirmation()
        except BaseException as error:  # noqa: BLE001
            confirmation_still_open = True
            report(f"cleanup: could not verify confirmation state: {error}")
        if menu_may_be_open and not confirmation_still_open:
            try:
                item = backend.find("menu item", "Shut Down…")
                if item is not None:
                    logo = backend.find("push button", "menu")
                    if logo is None:
                        report("cleanup: system menu remains open; menu toggle not found")
                    else:
                        backend.activate_click(logo)
                        backend.wait(lambda: backend.find("menu item", "Shut Down…") is None,
                                     "system menu to close", 3.0)
                        report("cleanup: system menu closed")
                else:
                    report("cleanup: system menu already closed")
            except BaseException as error:  # noqa: BLE001
                report(f"cleanup: system menu may remain open: {error}")


def main(argv: list[str] | None = None) -> int:
    if argv:
        print("refusing: this probe accepts no command-line overrides", file=sys.stderr)
        return 2
    try:
        session, live_env = discover_live_session(EXPECTED_USER)
        if pyatspi is None:
            raise ProbeError("pyatspi is required on the live Linux host")
        os.environ.update(live_env)
        print(f"session: user={session.user} uid={session.uid} session={session.session_id} "
              f"display={session.wayland_display} runtime={session.runtime_dir}")
        backend = AtspiBackend(pyatspi)
        run_probe(backend)
    except ProbeError as error:
        print(f"refused/failed safely: {error}", file=sys.stderr)
        return 2
    except KeyboardInterrupt:
        print("interrupted; menu cleanup attempted", file=sys.stderr)
        return 130
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
