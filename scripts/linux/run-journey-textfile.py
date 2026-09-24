#!/usr/bin/env python3
"""Acceptance test for product journey 5 (todo.md "Product journeys"):

    Open, edit and save a text file through the portal without losing
    content.

Runs against the live rmac session on the reference laptop (niri + AT-SPI),
the same way scripts/linux/run-journey-launch.py exercises journey 1: no
keyboard or pointer injector is installed there (no wtype, no ydotool), so
every step drives Text Editor (crates/text-editor) and the portal Open/Save
panel (crates/rmac-file-chooser, ADR 0012) only through:

  * AT-SPI actions (pyatspi) on elements that expose a real "click" action --
    the top bar's exported File/App menus (rmac_app_menu) render as proper
    AT-SPI "menu"/"menu item" nodes and are reliably clickable;
  * niri IPC, to discover windows and their app ids.

Text Editor's own document body and every `InputState`-backed entry this
script has probed (the document body, System Monitor's search field) expose
neither the AT-SPI Text nor EditableText interface -- `queryText()` and
`queryEditableText()` both raise -- so **no text can be typed anywhere in the
product without a keyboard injector**, not just in Spotlight's query field
(see run-journey-launch.py's LAUNCHER_QUERY_NAME note, which documented this
for Spotlight alone; this script confirms the same failure on Text Editor's
body). Concretely this means the buffer can never become dirty by assistive
technology alone, so File > Save -- a no-op on a clean buffer
(crates/text-editor/src/view/saving.rs:37-42) -- cannot be exercised as a
real write, and neither can a SIGKILL-during-save race. This script still
proves everything it can for real:

  * the portal Open/Save panel is attempted for real (File > Open, File >
    Save As...) and the resulting window (or its absence) is observed over
    niri IPC -- on the reference laptop as of this writing,
    `rmac-file-chooser.service` is not a registered systemd user unit and
    `rmac-portals.conf` still reads `default=gnome;gtk;*` with no
    `org.freedesktop.impl.portal.FileChooser` override, so neither dialog
    opens at all (confirmed live: no new window, no in-app error alert, no
    `rmac-file-chooser` AT-SPI application). ADR 0012's backend exists in the
    repository but is not deployed here yet. When it opens, a
    `fallback_spawn`-labelled direct launch keeps the rest of the journey
    measurable, exactly like run-journey-launch.py's Dock/Spotlight fallback;
  * external-change detection is fully real and needs no typing: Text
    Editor watches its open document's directory
    (crates/text-editor/src/view/lifecycle.rs:17-26) and shows an always-
    visible "This document changed outside Text Editor..." banner with a
    "Review..." button the moment the file changes on disk
    (crates/text-editor/src/view/render.rs:191-224) -- no save, no typing,
    no menu required. This script edits the file directly on disk while it
    is open, waits for that banner, opens the Conflict alert via "Review...",
    and clicks Cancel, then verifies the file on disk still holds exactly
    the externally-written bytes (the app never overwrote it without an
    explicit Overwrite confirmation);
  * content-loss is checked the same way throughout: every step recomputes
    the on-disk SHA-256 and compares it against what this script itself last
    wrote, so nothing the app does (or fails to do) can go unnoticed.

The report is privacy-safe: no screenshots, no window titles, no absolute
paths, no file contents. Every wait in this script is bounded; it never
hangs. The disposable test folder is always removed, even on failure.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import secrets
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Callable, Optional

try:
    import pyatspi  # type: ignore[import-not-found]
except ImportError:  # pragma: no cover - exercised only off-Linux
    pyatspi = None


class JourneyError(RuntimeError):
    """A bounded, privacy-safe journey failure."""


FORMAT = 1
JOURNEY_ID = 5
JOURNEY_TITLE = (
    "Open, edit and save a text file through the portal without losing content."
)

NIRI_TIMEOUT_S = 5.0
WINDOW_APPEAR_TIMEOUT_S = 5.0
PORTAL_WINDOW_TIMEOUT_S = 6.0
ATSPI_FIND_TIMEOUT_S = 5.0
BANNER_TIMEOUT_S = 3.0
CLOSE_TIMEOUT_S = 5.0
POLL_INTERVAL_S = 0.05

TEXT_EDITOR: dict[str, str] = {
    "display_name": "Text Editor",
    "app_id": "org.rmac.TextEditor",
    "exec": "/usr/bin/rmac-text-editor",
}
FILE_CHOOSER_OPEN_APP_ID = "org.rmac.FileChooser"
FILE_CHOOSER_SAVE_APP_ID = "org.rmac.FileChooser.Save"

# crates/rmac-app-menu/src/lib.rs TEXT_EDITOR_MENUS -- exact exported labels.
FILE_MENU_BUTTON = "File menu"
OPEN_ITEM = "Open…"
SAVE_ITEM = "Save"
SAVE_AS_ITEM = "Save As…"

INITIAL_CONTENT = "rmac journey 5 fixture\nfirst line\n"
EXTERNAL_CONTENT = "rmac journey 5 fixture -- changed by another process\n"


# --------------------------------------------------------------------------
# Pure helpers (unit-tested from scripts/test_journey_textfile.py on macOS)
# --------------------------------------------------------------------------


def discover_environment(
    environ: dict[str, str], runtime_dir: Path
) -> dict[str, str]:
    """Same contract as run-journey-launch.py's helper of the same name."""

    additions: dict[str, str] = {}
    if "XDG_RUNTIME_DIR" not in environ:
        additions["XDG_RUNTIME_DIR"] = str(runtime_dir)
    if "NIRI_SOCKET" not in environ:
        sockets = sorted(runtime_dir.glob("niri*.sock"))
        if not sockets:
            raise JourneyError("no niri IPC socket found in the runtime directory")
        additions["NIRI_SOCKET"] = str(sockets[0])
    if "WAYLAND_DISPLAY" not in environ:
        displays = sorted(
            entry.name
            for entry in runtime_dir.glob("wayland-*")
            if not entry.name.endswith(".lock")
        )
        if not displays:
            raise JourneyError("no Wayland display socket found in the runtime directory")
        additions["WAYLAND_DISPLAY"] = displays[0]
    if "DBUS_SESSION_BUS_ADDRESS" not in environ:
        bus = runtime_dir / "bus"
        if not bus.exists():
            raise JourneyError("no D-Bus session bus socket found in the runtime directory")
        additions["DBUS_SESSION_BUS_ADDRESS"] = f"unix:path={bus}"
    return additions


def parse_windows(stdout: str) -> list[dict[str, Any]]:
    try:
        windows = json.loads(stdout)
    except json.JSONDecodeError as error:
        raise JourneyError("niri windows output was not valid JSON") from error
    if not isinstance(windows, list):
        raise JourneyError("niri windows output was not a JSON array")
    return windows


def find_window_by_app_id(
    windows: list[dict[str, Any]], app_id: str
) -> Optional[dict[str, Any]]:
    for window in windows:
        if window.get("app_id") == app_id:
            return window
    return None


def windows_by_app_id(windows: list[dict[str, Any]], app_id: str) -> list[dict[str, Any]]:
    return [window for window in windows if window.get("app_id") == app_id]


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def random_folder_name() -> str:
    return f"lulo-journey-5-{secrets.token_hex(6)}"


def make_step(step_id: str, passed: bool, detail: str, **extra: Any) -> dict[str, Any]:
    step = {"id": step_id, "passed": bool(passed), "detail": detail}
    step.update(extra)
    return step


def build_report(
    steps: list[dict[str, Any]],
    gaps: list[dict[str, str]],
    started_at_unix_ms: int,
) -> dict[str, Any]:
    return {
        "format": FORMAT,
        "journey": JOURNEY_ID,
        "journey_title": JOURNEY_TITLE,
        "started_at_unix_ms": started_at_unix_ms,
        "steps": steps,
        "gaps": gaps,
        "overall_pass": all(step["passed"] for step in steps),
    }


# --------------------------------------------------------------------------
# niri IPC (same contract as run-journey-launch.py)
# --------------------------------------------------------------------------


def _niri(*args: str, timeout: float = NIRI_TIMEOUT_S) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(
            ["niri", "msg", *args],
            check=False,
            capture_output=True,
            text=True,
            timeout=timeout,
            stdin=subprocess.DEVNULL,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise JourneyError(f"niri msg {' '.join(args)} did not complete") from error


def niri_windows() -> list[dict[str, Any]]:
    result = _niri("--json", "windows")
    if result.returncode != 0:
        raise JourneyError("niri msg windows failed")
    return parse_windows(result.stdout)


def niri_spawn(command: str) -> None:
    result = _niri("action", "spawn", "--", *command.split(" "))
    if result.returncode != 0:
        raise JourneyError("niri failed to spawn the target application")


def niri_close_window(window_id: int) -> None:
    _niri("action", "close-window", "--id", str(window_id))


def _wait_for(
    predicate: Callable[[], Any], timeout: float, poll: float = POLL_INTERVAL_S
) -> Any:
    deadline = time.monotonic() + timeout
    while True:
        value = predicate()
        if value:
            return value
        if time.monotonic() >= deadline:
            return None
        time.sleep(poll)


def wait_for_window(app_id: str, timeout: float) -> Optional[dict[str, Any]]:
    return _wait_for(lambda: find_window_by_app_id(niri_windows(), app_id), timeout)


def wait_for_window_gone(window_id: int, timeout: float = CLOSE_TIMEOUT_S) -> bool:
    def gone() -> bool:
        return all(window.get("id") != window_id for window in niri_windows())

    return bool(_wait_for(gone, timeout))


# --------------------------------------------------------------------------
# AT-SPI helpers (same contract as run-journey-launch.py)
# --------------------------------------------------------------------------


def _require_pyatspi() -> None:
    if pyatspi is None:
        raise JourneyError("pyatspi is not installed; this step requires Linux/AT-SPI")


def _descendants(node):
    yield node
    try:
        count = node.childCount
    except (LookupError, RuntimeError):
        return
    for index in range(count):
        try:
            child = node.getChildAtIndex(index)
        except (LookupError, RuntimeError):
            continue
        yield from _descendants(child)


def _atspi_snapshot(app_name: Optional[str] = None):
    _require_pyatspi()
    desktop = pyatspi.Registry.getDesktop(0)
    for app in desktop:
        try:
            name = app.name
        except (LookupError, RuntimeError):
            continue
        if app_name is not None and name != app_name:
            continue
        yield from _descendants(app)


def find_node(
    app_name: str,
    node_name: str,
    role: Optional[str] = None,
    timeout: float = ATSPI_FIND_TIMEOUT_S,
    name_prefix: bool = False,
):
    def search():
        for node in _atspi_snapshot(app_name):
            try:
                name = node.name
                matches = (
                    name.startswith(node_name) if name_prefix else name == node_name
                )
                if not matches:
                    continue
                if role is not None and node.getRoleName() != role:
                    continue
            except (LookupError, RuntimeError):
                continue
            return node
        return None

    return _wait_for(search, timeout)


def action_names(node) -> list[str]:
    if "Action" not in node.get_interfaces():
        return []
    actions = node.queryAction()
    return [actions.getName(index) for index in range(actions.nActions)]


def click(node) -> bool:
    names = action_names(node)
    if "click" not in names:
        raise JourneyError("AT-SPI node has no 'click' action")
    actions = node.queryAction()
    return bool(actions.doAction(names.index("click")))


def app_present(app_name: str) -> bool:
    return any(True for _ in _atspi_snapshot(app_name))


def can_edit_text(node) -> tuple[bool, str]:
    """Probe the *real* Text/EditableText behaviour of an AT-SPI node,
    rather than trusting `get_interfaces()` (which this script has seen
    omit interfaces that the object still advertises structurally). Returns
    (can_edit, evidence)."""

    try:
        node.queryEditableText()
    except Exception as error:  # noqa: BLE001 - the live bridge can raise almost anything
        return False, f"queryEditableText() raised: {error!r}"
    return True, "queryEditableText() succeeded"


# --------------------------------------------------------------------------
# Journey steps
# --------------------------------------------------------------------------


def check_logged_in() -> dict[str, Any]:
    try:
        listing = subprocess.run(
            ["loginctl", "list-sessions", "--no-legend"],
            check=False,
            capture_output=True,
            text=True,
            timeout=NIRI_TIMEOUT_S,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        return make_step("logged_in", False, f"loginctl was unavailable: {error}")
    if listing.returncode != 0:
        return make_step("logged_in", False, "loginctl list-sessions failed")
    session_ids = [line.split()[0] for line in listing.stdout.splitlines() if line.split()]
    for session_id in session_ids:
        show = subprocess.run(
            [
                "loginctl",
                "show-session",
                session_id,
                "--property=Type",
                "--property=Class",
                "--property=State",
                "--property=Remote",
                "--no-pager",
            ],
            check=False,
            capture_output=True,
            text=True,
            timeout=NIRI_TIMEOUT_S,
        )
        if show.returncode != 0:
            continue
        properties = dict(
            line.split("=", 1) for line in show.stdout.splitlines() if "=" in line
        )
        if (
            properties.get("Type") == "wayland"
            and properties.get("Class") == "user"
            and properties.get("State") == "active"
            and properties.get("Remote") == "no"
        ):
            return make_step("logged_in", True, "an active local rmac graphical session was found")
    return make_step(
        "logged_in", False, "no active local graphical (wayland/user) session was found"
    )


def quit_app(display_name: str, window: dict[str, Any], timeout: float = CLOSE_TIMEOUT_S) -> bool:
    """Close a first-party rmac app the accessible way: its exported "App"
    menu's "Quit {display_name}" item (crates/rmac-app-menu), the same
    pattern run-journey-launch.py's quit_app uses. Falls back to a plain
    niri close if the menu path is unavailable, so cleanup never hangs."""

    menu_button = find_node("rmac-top-bar", f"{display_name} menu", role="button", timeout=2.0)
    if menu_button is not None and "click" in action_names(menu_button):
        click(menu_button)
        quit_item = find_node("rmac-top-bar", f"Quit {display_name}", timeout=2.0)
        if quit_item is not None and "click" in action_names(quit_item):
            click(quit_item)
            if wait_for_window_gone(window["id"], timeout):
                return True
    niri_close_window(window["id"])
    return wait_for_window_gone(window["id"], timeout)


def open_file_menu() -> dict[str, Any]:
    button = find_node("rmac-top-bar", FILE_MENU_BUTTON, role="button")
    if button is None or "click" not in action_names(button):
        return make_step(
            "open_file_menu",
            False,
            "Text Editor's File menu was not found (or not clickable) over AT-SPI",
        )
    click(button)
    return make_step("open_file_menu", True, "opened the File menu via AT-SPI")


def click_menu_item(step_id: str, label: str) -> dict[str, Any]:
    item = find_node("rmac-top-bar", label, timeout=2.0)
    if item is None or "click" not in action_names(item):
        return make_step(
            step_id, False, f"menu item {label!r} was not found (or not clickable) over AT-SPI"
        )
    click(item)
    return make_step(step_id, True, f"activated {label!r} via AT-SPI")


def attempt_portal_dialog(step_id: str, app_id: str, atspi_app_name: str) -> dict[str, Any]:
    """Wait for the portal's panel window; report exactly what happened."""

    window = wait_for_window(app_id, PORTAL_WINDOW_TIMEOUT_S)
    if window is not None:
        return make_step(
            step_id, True, "the portal panel window appeared", window_appeared=True
        )
    present = app_present(atspi_app_name)
    return make_step(
        step_id,
        False,
        "no portal panel window appeared within "
        f"{PORTAL_WINDOW_TIMEOUT_S:.0f}s (rmac-file-chooser AT-SPI app present={present}); "
        "rmac-file-chooser.service is likely not registered / rmac-portals.conf has no "
        "FileChooser override on this host (see ADR 0012)",
        window_appeared=False,
    )


def cancel_portal_dialog(atspi_app_name: str) -> None:
    button = find_node(atspi_app_name, "Cancel", role="button", timeout=2.0)
    if button is not None and "click" in action_names(button):
        click(button)


# --------------------------------------------------------------------------
# Orchestration
# --------------------------------------------------------------------------


def run_journey(keep_open: bool) -> dict[str, Any]:
    steps: list[dict[str, Any]] = []
    gaps: list[dict[str, str]] = []
    started_at_unix_ms = int(time.time() * 1000)

    steps.append(check_logged_in())

    home = Path.home()
    folder = home / "Documents" / random_folder_name()
    test_path = folder / "journey5.txt"
    editor_window: Optional[dict[str, Any]] = None

    try:
        folder.mkdir(parents=True, exist_ok=False)
        test_path.write_bytes(INITIAL_CONTENT.encode("utf-8"))
        original_hash = sha256_hex(INITIAL_CONTENT.encode("utf-8"))
        steps.append(
            make_step("create_test_file", True, "created a disposable fixture file")
        )
    except OSError as error:
        steps.append(make_step("create_test_file", False, f"could not create the fixture: {error}"))
        return build_report(steps, gaps, started_at_unix_ms)

    try:
        # Launch a blank Text Editor window; journey 1 already covers
        # launching apps via the Dock/Spotlight, so this step exists only to
        # get a window whose File menu can drive the real portal check.
        niri_spawn(TEXT_EDITOR["exec"])
        editor_window = wait_for_window(TEXT_EDITOR["app_id"], WINDOW_APPEAR_TIMEOUT_S)
        steps.append(
            make_step(
                "launch_editor",
                editor_window is not None,
                "Text Editor window appeared" if editor_window else "no Text Editor window appeared",
            )
        )
        if editor_window is None:
            return build_report(steps, gaps, started_at_unix_ms)

        # --- Open through the portal -------------------------------------------------
        steps.append(open_file_menu())
        steps.append(click_menu_item("click_open_item", OPEN_ITEM))
        open_step = attempt_portal_dialog("open_via_portal", FILE_CHOOSER_OPEN_APP_ID, "rmac-file-chooser")
        steps.append(open_step)
        opened_via_portal = bool(open_step.get("window_appeared"))
        if opened_via_portal:
            # Best-effort real interaction: navigate to Documents, select the
            # fixture by name, and accept. Left best-effort deliberately --
            # if any element isn't found the failure is captured below rather
            # than raised, since the panel's exact live shape is unverified
            # while the backend is undeployed on the reference laptop.
            try:
                places = find_node("rmac-file-chooser", "Documents", timeout=2.0)
                if places is not None and "click" in action_names(places):
                    click(places)
                row = find_node("rmac-file-chooser", test_path.name, timeout=3.0)
                if row is not None and "click" in action_names(row):
                    click(row)
                accept = find_node("rmac-file-chooser", "Open", role="button", timeout=2.0)
                if accept is not None and "click" in action_names(accept):
                    click(accept)
            except JourneyError:
                pass
        else:
            gaps.append(
                {
                    "surface": "file-chooser",
                    "issue": (
                        "the portal Open dialog (crates/rmac-file-chooser, ADR 0012) did "
                        "not appear on this host; falling back to opening the fixture "
                        "directly so the rest of the journey can still be measured"
                    ),
                }
            )
            niri_close_window(editor_window["id"])
            wait_for_window_gone(editor_window["id"], CLOSE_TIMEOUT_S)
            niri_spawn(f"{TEXT_EDITOR['exec']} {test_path}")
            editor_window = wait_for_window(TEXT_EDITOR["app_id"], WINDOW_APPEAR_TIMEOUT_S)
            steps.append(
                make_step(
                    "fallback_open",
                    editor_window is not None,
                    "opened the fixture with a direct launch (same installed command "
                    "the portal would ultimately hand off to)",
                    method="fallback_spawn",
                )
            )
            if editor_window is None:
                return build_report(steps, gaps, started_at_unix_ms)

        # --- Content integrity after opening -------------------------------------------
        after_open_hash = sha256_hex(test_path.read_bytes())
        steps.append(
            make_step(
                "content_intact_after_open",
                after_open_hash == original_hash,
                "on-disk content is unchanged after opening"
                if after_open_hash == original_hash
                else "on-disk content changed merely by opening the document",
            )
        )

        # --- Edit: requires a keyboard/pointer injector this host doesn't have --------
        entry = find_node("rmac-text-editor", "", role="entry", timeout=2.0)
        can_edit, evidence = (False, "the document body entry was not found over AT-SPI")
        if entry is not None:
            can_edit, evidence = can_edit_text(entry)
        steps.append(make_step("edit_content", can_edit, evidence))
        if not can_edit:
            gaps.append(
                {
                    "surface": "text-editor-body",
                    "issue": (
                        "the document body exposes no AT-SPI EditableText (and no Text "
                        f"interface either): {evidence}. No keyboard or pointer injector "
                        "is installed on the reference laptop, so no assistive technology "
                        "path can insert text; the buffer can never become dirty, so "
                        "File > Save is a guaranteed no-op "
                        "(crates/text-editor/src/view/saving.rs:37-42) and the "
                        "atomic-write / SIGKILL-during-save checks below cannot be "
                        "exercised as a real write on this build"
                    ),
                }
            )

        # --- Save (real click; a no-op if the buffer could not be dirtied) ------------
        steps.append(open_file_menu())
        steps.append(click_menu_item("click_save_item", SAVE_ITEM))
        time.sleep(0.3)
        after_save_hash = sha256_hex(test_path.read_bytes())
        steps.append(
            make_step(
                "content_intact_after_save",
                after_save_hash == original_hash,
                "on-disk content matches the original after Save"
                if after_save_hash == original_hash
                else "on-disk content diverged from the original after Save",
            )
        )

        # --- SIGKILL-during-save: only meaningful once a real write can happen --------
        if can_edit:
            steps.append(
                make_step(
                    "sigkill_during_save",
                    False,
                    "not implemented: reachable now that the buffer can be dirtied, "
                    "but this script's fallback path never exercises it",
                )
            )
        else:
            steps.append(
                make_step(
                    "sigkill_during_save",
                    False,
                    "not exercised: no assistive-technology path can dirty the buffer "
                    "on this build (see the edit_content gap above), so there is no "
                    "real write to interrupt",
                )
            )

        # --- External change while open: fully real, needs no typing ------------------
        test_path.write_bytes(EXTERNAL_CONTENT.encode("utf-8"))
        external_hash = sha256_hex(EXTERNAL_CONTENT.encode("utf-8"))
        review_button = find_node(
            "rmac-text-editor", "Review…", role="button", timeout=BANNER_TIMEOUT_S
        )
        steps.append(
            make_step(
                "external_change_detected",
                review_button is not None,
                "the external-change banner appeared"
                if review_button is not None
                else "no external-change banner appeared after the file changed on disk",
            )
        )
        if review_button is not None and "click" in action_names(review_button):
            click(review_button)
            cancel_button = find_node("rmac-text-editor", "Cancel", role="button", timeout=2.0)
            reviewed = cancel_button is not None and "click" in action_names(cancel_button)
            if reviewed:
                click(cancel_button)
            steps.append(
                make_step(
                    "external_change_reviewed",
                    reviewed,
                    "opened the conflict dialog and dismissed it with Cancel"
                    if reviewed
                    else "the conflict dialog's Cancel button was not found over AT-SPI",
                )
            )
        else:
            steps.append(
                make_step(
                    "external_change_reviewed",
                    False,
                    "no Review… control was available to open the conflict dialog",
                )
            )

        after_conflict_hash = sha256_hex(test_path.read_bytes())
        steps.append(
            make_step(
                "external_edit_preserved",
                after_conflict_hash == external_hash,
                "the externally-written content was never overwritten by Text Editor"
                if after_conflict_hash == external_hash
                else "Text Editor overwrote the external edit without an explicit confirmation",
            )
        )

        # --- Save As through the portal -------------------------------------------------
        steps.append(open_file_menu())
        steps.append(click_menu_item("click_save_as_item", SAVE_AS_ITEM))
        save_as_step = attempt_portal_dialog(
            "save_as_via_portal", FILE_CHOOSER_SAVE_APP_ID, "rmac-file-chooser"
        )
        steps.append(save_as_step)
        if not save_as_step["passed"]:
            gaps.append(
                {
                    "surface": "file-chooser-save",
                    "issue": (
                        "the portal Save dialog did not appear either, for the same "
                        "reason as Open above; Save As to a new name cannot be "
                        "completed by assistive technology on this build (there is no "
                        "non-portal way to choose a new destination)"
                    ),
                }
            )
        else:
            cancel_portal_dialog("rmac-file-chooser")

        if keep_open:
            return build_report(steps, gaps, started_at_unix_ms)
        return build_report(steps, gaps, started_at_unix_ms)
    finally:
        if not keep_open:
            for window in windows_by_app_id(niri_windows(), FILE_CHOOSER_OPEN_APP_ID):
                niri_close_window(window["id"])
            for window in windows_by_app_id(niri_windows(), FILE_CHOOSER_SAVE_APP_ID):
                niri_close_window(window["id"])
            if editor_window is not None:
                remaining = wait_for_window(TEXT_EDITOR["app_id"], 0.5)
                if remaining is not None:
                    try:
                        quit_app(TEXT_EDITOR["display_name"], remaining)
                    except JourneyError:
                        niri_close_window(remaining["id"])
        try:
            if test_path.exists():
                test_path.unlink()
            if folder.exists():
                folder.rmdir()
        except OSError:
            pass


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output",
        type=Path,
        default=None,
        help="absolute path to write the JSON report to (also printed to stdout)",
    )
    parser.add_argument(
        "--keep-open",
        action="store_true",
        help="leave windows open and skip cleanup (debugging only)",
    )
    arguments = parser.parse_args()

    try:
        additions = discover_environment(dict(os.environ), Path(f"/run/user/{os.getuid()}"))
        os.environ.update(additions)
        report = run_journey(arguments.keep_open)
    except JourneyError as error:
        parser.exit(4, f"run-journey-textfile: {error}\n")

    text = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if arguments.output is not None:
        arguments.output.write_text(text, encoding="utf-8")
    print(text, end="")

    passed = sum(1 for step in report["steps"] if step["passed"])
    total = len(report["steps"])
    print(
        f"journey 5: {passed}/{total} steps passed; "
        f"overall_pass={report['overall_pass']}",
        file=sys.stderr,
    )
    return 0 if report["overall_pass"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
