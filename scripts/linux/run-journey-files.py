#!/usr/bin/env python3
"""Acceptance test for product journey 2 (todo.md "Product journeys"):

    Find a file, preview it, copy, move, rename and trash it, and undo a
    destructive operation.

Runs against the live rmac session on the reference laptop (niri + AT-SPI +
GDM), the same session a real user would be sitting at. As with journey 1
(see run-journey-launch.py), there is no keyboard or pointer injector
installed there (no wtype, no ydotool), so every step drives Files
(crates/finder, binary rmac-files) only through:

  * AT-SPI actions (pyatspi) on elements that expose a real "click" action;
  * niri IPC (`niri msg --json windows`, `action spawn`, `action
    focus-window`, `action close-window`).

Everything runs inside a disposable folder this script creates under
~/Documents/lulo-journey-2-<random> and always removes at the end, even on
failure. Nothing outside that folder is ever touched; the safety assertions
below verify this on disk rather than assuming it.

Live findings baked into this script's design (found by actually running it
against the reference laptop before writing the final version -- see
docs/journey-suite.md for the summary):

  * Files' own in-window controls -- every toolbar button, the search/path
    entry, and the icon-size slider -- expose no AT-SPI accessible name at
    all (confirmed live and by `grep -rn "aria_label\\|\\.role(\\|
    on_a11y_action" crates/finder/src`, which returns nothing, versus
    shell/bins/rmac-dock/src/main.rs, which uses that exact API for its own
    tiles). Neither the search field nor any file/folder row can be found or
    identified over AT-SPI. This blocks "find a file" and any row-level
    selection entirely -- see the `find_file` step below.
  * The file list and sidebar render no accessible children whatsoever (a
    Files window's frame has exactly the toolbar buttons/entry/slider as its
    only AT-SPI children); there is no way to select an individual item over
    AT-SPI.
  * The shell top bar's per-app menu for Files IS fully named and clickable:
    "File menu" (New Folder, New Tab, Close Tab, Move to Trash, Get Info),
    "Edit menu" (Undo, Cut, Copy, Paste, Select All), "View menu" (view
    modes, sort, Show Hidden Files, Quick Look), "Go menu" (Back, Forward,
    Enclosing Folder, Home, Applications, Downloads, Trash). There is no
    "Rename" item anywhere, and no "Quit Files" item in "Files menu" either
    (unlike journey 1's Text Editor/Notes). Because there is also no
    row-level AT-SPI access, and no keyboard injector to send Return/F2, an
    individual item cannot be renamed over AT-SPI at all -- see the
    `rename` step below.
  * Selection-scoped commands dispatched from the top bar's per-app menu
    (Select All, Move to Trash, Undo, Quick Look) silently no-op -- no
    error, no dialog, nothing on disk -- unless the target Files window's
    AT-SPI frame is given focus first via the Component interface's
    grabFocus(). Non-selection commands (New Folder) work either way. This
    script always calls grab_focus() on a window immediately before any
    selection-scoped menu action; production Files arguably should not
    require this (a menu click with no visible effect and no error is its
    own small gap, noted in the report).
  * Because there is no way to select one specific item among several (no
    row access), each operation that needs a single, unambiguous target
    runs in its own single-item folder and uses Edit > Select All as the
    only available "select the file" mechanism, then closes that window.
  * View > Quick Look does not produce any visible new window or
    layer-shell surface on this build (checked against both `niri msg
    --json windows` and `niri msg --json layers`), even with a real prior
    selection and a focused frame -- see the `preview` step below.
  * Copy/Cut + Paste between two Files windows never transfers anything on
    Linux: crates/finder/src/pasteboard.rs's `#[cfg(not(target_os =
    "macos"))]` module (lines 62-70) stubs `write_file_urls`,
    `read_file_urls`, and `clear_file_urls` to complete no-ops, so Copy
    never puts file references anywhere a Paste could read them back from,
    regardless of window focus or timing. This is a real product gap, not
    a flaky-automation one -- see the `copy`/`move` steps below. (Two
    simultaneously open rmac-files windows were also observed to register
    only one `org.rmac.Files.Menu` D-Bus name between them, which can add
    its own unreliability to which window a top-bar menu click reaches;
    this script therefore never keeps two Files windows open at once,
    closing the source before opening the destination.)

The report is privacy-safe: absolute paths are never included, only paths
relative to the disposable test root. Every wait in this script is bounded;
it never hangs.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any, Callable, Optional

try:
    import pyatspi  # type: ignore[import-not-found]
except ImportError:  # pragma: no cover - exercised only off-Linux
    pyatspi = None


class JourneyError(RuntimeError):
    """A bounded, privacy-safe journey-files failure."""


FORMAT = 1
JOURNEY_ID = 2
JOURNEY_TITLE = (
    "Find a file, preview it, copy, move, rename and trash it, and undo a "
    "destructive operation."
)

NIRI_TIMEOUT_S = 5.0
WINDOW_APPEAR_TIMEOUT_S = 5.0
WINDOW_GONE_TIMEOUT_S = 5.0
FOCUS_TIMEOUT_S = 3.0
ATSPI_FIND_TIMEOUT_S = 5.0
MENU_SETTLE_S = 0.4
ACTION_SETTLE_S = 0.8
FOCUS_SETTLE_S = 0.4
POLL_INTERVAL_S = 0.05

FILES_APP_ID = "org.rmac.Files"
FILES_EXEC = "/usr/bin/rmac-files"
# The reference laptop deploys fresh builds here; /usr/bin holds whatever the
# last installed package shipped, which can be days older than the code under
# test (the 2026-09-24 run exercised a 2026-09-20 package that predates Quick
# Look's panel).
DEV_FILES_EXEC = Path("rmac-dev-bin") / "rmac-files"
FILES_ATSPI_APP_NAME = "rmac-files"
TOPBAR_ATSPI_APP_NAME = "rmac-top-bar"

SAMPLE_TEXT = "Journey 2 acceptance test fixture. Safe to delete.\n"


# --------------------------------------------------------------------------
# Pure helpers (unit-tested from scripts/test_journey_files.py on macOS)
# --------------------------------------------------------------------------


def discover_environment(
    environ: dict[str, str], runtime_dir: Path
) -> dict[str, str]:
    """Return the environment additions needed to reach the session over
    SSH, without mutating ``environ``. Never guesses beyond what is on disk."""

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


def find_window_by_title(
    windows: list[dict[str, Any]], app_id: str, title_marker: str
) -> Optional[dict[str, Any]]:
    """Disambiguate between several windows sharing ``app_id`` (several
    Files windows can be open at once) by a substring of the title."""

    for window in windows:
        if window.get("app_id") == app_id and title_marker in (window.get("title") or ""):
            return window
    return None


def make_step(step_id: str, passed: bool, detail: str, **extra: Any) -> dict[str, Any]:
    step = {"id": step_id, "passed": bool(passed), "detail": detail}
    step.update(extra)
    return step


def build_report(
    steps: list[dict[str, Any]],
    gaps: list[dict[str, str]],
    started_at_unix_ms: int,
    test_root_name: str,
) -> dict[str, Any]:
    return {
        "format": FORMAT,
        "journey": JOURNEY_ID,
        "journey_title": JOURNEY_TITLE,
        "started_at_unix_ms": started_at_unix_ms,
        "test_root": test_root_name,
        "steps": steps,
        "gaps": gaps,
        "overall_pass": all(step["passed"] for step in steps),
    }


def relative_trashinfo_path_matches(trashinfo_text: str, marker: str) -> bool:
    """True if a .trashinfo file's ``Path=`` line contains ``marker``
    (the disposable test root's name), so trash bookkeeping never confuses
    this run's items with anything another user or run put in Trash."""

    for line in trashinfo_text.splitlines():
        if line.startswith("Path=") and marker in line:
            return True
    return False


# --------------------------------------------------------------------------
# niri IPC
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


def niri_spawn(*command: str) -> None:
    result = _niri("action", "spawn", "--", *command)
    if result.returncode != 0:
        raise JourneyError(f"niri failed to spawn {command!r}")


def niri_focus_window(window_id: int) -> None:
    result = _niri("action", "focus-window", "--id", str(window_id))
    if result.returncode != 0:
        raise JourneyError(f"niri failed to focus window {window_id}")


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


def wait_for_window(
    app_id: str, title_marker: str, timeout: float = WINDOW_APPEAR_TIMEOUT_S
):
    return _wait_for(
        lambda: find_window_by_title(niri_windows(), app_id, title_marker), timeout
    )


def wait_for_window_gone(window_id: int, timeout: float = WINDOW_GONE_TIMEOUT_S) -> bool:
    def gone() -> bool:
        return all(window.get("id") != window_id for window in niri_windows())

    return bool(_wait_for(gone, timeout))


# --------------------------------------------------------------------------
# AT-SPI helpers (same pattern as run-journey-launch.py)
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
        if child is not None:
            yield from _descendants(child)


def _atspi_app(app_name: str):
    _require_pyatspi()
    desktop = pyatspi.Registry.getDesktop(0)
    for app in desktop:
        try:
            if app.name == app_name:
                return app
        except (LookupError, RuntimeError):
            continue
    return None


def find_node(
    app_name: str,
    node_name: str,
    role: Optional[str] = None,
    timeout: float = ATSPI_FIND_TIMEOUT_S,
):
    def search():
        app = _atspi_app(app_name)
        if app is None:
            return None
        for node in _descendants(app):
            try:
                if node.name != node_name:
                    continue
                if role is not None and node.getRoleName() != role:
                    continue
            except (LookupError, RuntimeError):
                continue
            return node
        return None

    return _wait_for(search, timeout)


def list_names(app_name: str, role: str, timeout: float = ATSPI_FIND_TIMEOUT_S) -> list[str]:
    """All accessible names under ``app_name`` with the given role -- used to
    list a menu's real, live item set rather than assuming one."""

    def search():
        app = _atspi_app(app_name)
        if app is None:
            return None
        names = [
            node.name
            for node in _descendants(app)
            if _safe_role(node) == role
        ]
        return names or None

    result = _wait_for(search, timeout)
    return result or []


def _safe_role(node) -> Optional[str]:
    try:
        return node.getRoleName()
    except (LookupError, RuntimeError):
        return None


def action_names(node) -> list[str]:
    try:
        if "Action" not in node.get_interfaces():
            return []
    except (LookupError, RuntimeError):
        return []
    actions = node.queryAction()
    return [actions.getName(index) for index in range(actions.nActions)]


def click(node) -> bool:
    names = action_names(node)
    if "click" not in names:
        raise JourneyError("AT-SPI node has no 'click' action")
    actions = node.queryAction()
    return bool(actions.doAction(names.index("click")))


def grab_focus(node) -> bool:
    try:
        if "Component" not in node.get_interfaces():
            return False
        return bool(node.queryComponent().grabFocus())
    except (LookupError, RuntimeError):
        return False


def click_topbar_menu_item(menu_name: str, item_name: str, item_role: str = "menu item") -> bool:
    """Open a top-bar per-app menu and click one of its items, closing the
    menu again either way. Returns whether the item was found and clicked."""

    menu_button = find_node(TOPBAR_ATSPI_APP_NAME, menu_name, role="button", timeout=3.0)
    if menu_button is None or "click" not in action_names(menu_button):
        return False
    click(menu_button)
    time.sleep(MENU_SETTLE_S)
    item = find_node(TOPBAR_ATSPI_APP_NAME, item_name, role=item_role, timeout=2.0)
    found = item is not None and "click" in action_names(item)
    if found:
        click(item)
    else:
        # Close the menu we opened either way, so we leave no popup behind.
        click(menu_button)
    time.sleep(MENU_SETTLE_S)
    return found


MENU_ACTION_ATTEMPTS = 3


def menu_action_with_retry(
    window_id: int, title_marker: str, menu_name: str, item_name: str
) -> bool:
    """Re-focus the window and retry a top-bar menu click a bounded number
    of times. The reference session showed the shell top bar's per-app menu
    bridge for Files is not always reliably routed to the intended window
    (see docs/journey-suite.md and the D-Bus finding in this script's
    module docstring: multiple simultaneously open rmac-files processes
    each try to own the single well-known name org.rmac.Files.Menu). This
    retries the same real action rather than ever fabricating success."""

    for attempt in range(MENU_ACTION_ATTEMPTS):
        if not focus_files_window(window_id, title_marker):
            continue
        if click_topbar_menu_item(menu_name, item_name):
            return True
        time.sleep(MENU_SETTLE_S)
    return False


def set_gsettings_accessibility(enabled: bool) -> None:
    for schema, key in (
        ("org.gnome.desktop.interface", "toolkit-accessibility"),
        ("org.gnome.desktop.a11y.applications", "screen-reader-enabled"),
    ):
        subprocess.run(
            ["gsettings", "set", schema, key, "true" if enabled else "false"],
            check=False,
            capture_output=True,
            timeout=NIRI_TIMEOUT_S,
        )


def get_gsettings_accessibility() -> dict[str, str]:
    values: dict[str, str] = {}
    for schema, key in (
        ("org.gnome.desktop.interface", "toolkit-accessibility"),
        ("org.gnome.desktop.a11y.applications", "screen-reader-enabled"),
    ):
        result = subprocess.run(
            ["gsettings", "get", schema, key],
            check=False,
            capture_output=True,
            text=True,
            timeout=NIRI_TIMEOUT_S,
        )
        values[f"{schema}::{key}"] = result.stdout.strip()
    return values


def restore_gsettings_accessibility(saved: dict[str, str]) -> None:
    for combined, value in saved.items():
        schema, key = combined.split("::", 1)
        subprocess.run(
            ["gsettings", "set", schema, key, value],
            check=False,
            capture_output=True,
            timeout=NIRI_TIMEOUT_S,
        )


# --------------------------------------------------------------------------
# Journey steps
# --------------------------------------------------------------------------


def check_logged_in() -> dict[str, Any]:
    """"Log in": verify a real graphical rmac session with the shell top bar
    is already active (see run-journey-launch.py for the identical rationale
    -- this shared reference session is never logged out of by this test)."""

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
            break
    else:
        return make_step(
            "logged_in", False, "no active local graphical (wayland/user) session was found"
        )

    required_units = ("rmac-session.target", "rmac-top-bar.service")
    inactive = []
    for unit in required_units:
        result = subprocess.run(
            ["systemctl", "--user", "is-active", unit],
            check=False,
            capture_output=True,
            text=True,
            timeout=NIRI_TIMEOUT_S,
        )
        if result.stdout.strip() != "active":
            inactive.append(unit)
    if inactive:
        return make_step(
            "logged_in",
            False,
            f"required rmac units are not active: {', '.join(sorted(inactive))}",
        )
    return make_step("logged_in", True, "an active local rmac graphical session was found")


# Window ids this script itself opened. cleanup_windows() only ever closes
# ids in this set -- never every rmac-files window on the system -- because
# another agent or a real user may have their own Files window open at the
# same time (see the shared UI-driving lock this script is run under).
_OPENED_WINDOW_IDS: set[int] = set()


def choose_files_exec(explicit: Optional[str], home: Path) -> str:
    """The rmac-files binary to test: --files-exec when given, else the dev
    deployment under ~/rmac-dev-bin when it exists, else the packaged one."""

    if explicit:
        return explicit
    dev = home / DEV_FILES_EXEC
    if dev.is_file() and os.access(dev, os.X_OK):
        return str(dev)
    return FILES_EXEC


def open_files_window(path: Path, title_marker: str) -> tuple[Optional[int], dict[str, Any]]:
    niri_spawn(FILES_EXEC, "--path", str(path))
    window = wait_for_window(FILES_APP_ID, title_marker)
    if window is None:
        return None, make_step(
            "open_window",
            False,
            f"no Files window titled with {title_marker!r} appeared",
            folder=title_marker,
        )
    _OPENED_WINDOW_IDS.add(window["id"])
    return window["id"], make_step(
        "open_window", True, "Files window opened", folder=title_marker
    )


def focus_files_window(window_id: int, title_marker: str) -> bool:
    """Focus a specific Files window at the compositor level, find its
    AT-SPI frame by title, and grab AT-SPI focus on it -- required (see
    module docstring) before any selection-scoped top-bar menu action."""

    try:
        niri_focus_window(window_id)
    except JourneyError:
        return False
    time.sleep(FOCUS_SETTLE_S)
    frame = find_node(FILES_ATSPI_APP_NAME, f"{title_marker} — Files", role="frame", timeout=3.0)
    if frame is None:
        return False
    grabbed = grab_focus(frame)
    time.sleep(FOCUS_SETTLE_S)
    return grabbed


def close_files_window(window_id: int, title_marker: str) -> dict[str, Any]:
    used_accessible_close = False
    closed = False
    for _attempt in range(MENU_ACTION_ATTEMPTS):
        focus_files_window(window_id, title_marker)
        used_accessible_close = click_topbar_menu_item("File menu", "Close Tab")
        closed = wait_for_window_gone(window_id, timeout=2.0)
        if closed:
            break
    if not closed:
        # Fallback: never leave stray windows behind, but say so plainly --
        # this is a gap (no accessible way to close this window), not a
        # simulated success.
        niri_close_window(window_id)
        closed = wait_for_window_gone(window_id, timeout=3.0)
    _OPENED_WINDOW_IDS.discard(window_id)
    detail = (
        "closed via File > Close Tab"
        if used_accessible_close and closed
        else "closed via niri (File > Close Tab did not remove the window over AT-SPI)"
        if closed
        else "window could not be closed"
    )
    return make_step(
        "close_window",
        closed,
        detail,
        folder=title_marker,
        method="accessible_ui" if used_accessible_close and closed else "fallback",
    )


def run_journey(test_root: Path) -> tuple[list[dict[str, Any]], list[dict[str, str]]]:
    steps: list[dict[str, Any]] = []
    gaps: list[dict[str, str]] = []
    marker = test_root.name

    # --- fixtures -----------------------------------------------------
    find_dir = test_root / "find-me"
    copy_dir = test_root / "copy-me"
    move_dir = test_root / "move-me"
    trash_dir = test_root / "trash-me"
    # Copy and move each get their own destination folder so a name never
    # collides between the two (real folder navigation is unreachable over
    # AT-SPI -- see files_content gap -- so each step opens the exact
    # folder it needs directly via `rmac-files --path`, the same way a
    # Dock/Spotlight launch would ultimately open it).
    copy_dest_dir = test_root / "copy-dest"
    move_dest_dir = test_root / "move-dest"
    for directory in (
        find_dir,
        copy_dir,
        move_dir,
        trash_dir,
        copy_dest_dir,
        move_dest_dir,
    ):
        directory.mkdir(parents=True, exist_ok=True)
    (find_dir / "journey-note.txt").write_text(SAMPLE_TEXT, encoding="utf-8")
    (copy_dir / "journey-note.txt").write_text(SAMPLE_TEXT, encoding="utf-8")
    (move_dir / "journey-note.txt").write_text(SAMPLE_TEXT, encoding="utf-8")
    (trash_dir / "journey-note.txt").write_text(SAMPLE_TEXT, encoding="utf-8")

    # --- find a file ----------------------------------------------------
    find_window_id, open_step = open_files_window(find_dir, find_dir.name)
    steps.append(open_step)
    if find_window_id is None:
        gaps.append(
            {
                "surface": "files",
                "issue": "rmac-files did not open a window at all; the rest of "
                "journey 2 could not be attempted",
            }
        )
        return steps, gaps
    focus_files_window(find_window_id, find_dir.name)

    search_button = find_node(FILES_ATSPI_APP_NAME, "Search", role="button", timeout=2.0)
    search_entry_names = list_names(FILES_ATSPI_APP_NAME, "entry", timeout=1.0)
    # Files' own search control is unnamed (see module docstring), so it
    # cannot be identified by name over AT-SPI; this is recorded as a gap
    # below rather than attempted with a guessed element.
    found_via_search = False
    steps.append(
        make_step(
            "find_file",
            found_via_search,
            "Files' own toolbar (including its search field) exposes no "
            "AT-SPI accessible name on any control (search_button found="
            f"{search_button is not None}, entry names over AT-SPI="
            f"{search_entry_names!r}); a query cannot be identified or typed "
            "without a keyboard injector, so 'find a file' could not be "
            "driven through the accessible UI "
            "(crates/finder/src/view/chrome_presentation/toolbar.rs:22-56 "
            "never calls gpui's .aria_label()/.role(), unlike "
            "shell/bins/rmac-dock/src/main.rs, which does for its own "
            "tiles). Falling back to Edit > Select All against a "
            "single-item folder to obtain a definite target for the rest "
            "of the journey.",
        )
    )
    gaps.append(
        {
            "surface": "files_search",
            "issue": "Files' search field and every toolbar button are exposed "
            "over AT-SPI with an empty accessible name and (for the search "
            "field) no Text/EditableText interface at all; a file cannot be "
            "found by name over AT-SPI. See crates/finder/src/view/"
            "chrome_presentation/toolbar.rs (capsule_button) -- no call in "
            "crates/finder/src ever sets an aria label, role, or a11y "
            "action, unlike shell/bins/rmac-dock/src/main.rs.",
        }
    )
    gaps.append(
        {
            "surface": "files_content",
            "issue": "A Files window's AT-SPI frame exposes only its toolbar "
            "buttons/entry/slider as children; no file or folder row, no "
            "sidebar item, is exposed at all, so no individual item can be "
            "selected over AT-SPI (only whole-folder Edit > Select All is "
            "reachable).",
        }
    )

    selected = menu_action_with_retry(
        find_window_id, find_dir.name, "Edit menu", "Select All"
    )
    steps.append(
        make_step(
            "select_target",
            selected,
            "Edit > Select All was clicked with the window focused (stands "
            "in for 'find' -- see find_file above)"
            if selected
            else "could not reach Edit > Select All",
        )
    )

    # --- preview (Quick Look) -------------------------------------------
    preview_ok = False
    preview_detail = "Edit > Select All did not succeed; Quick Look was not attempted"
    if selected:
        windows_before = niri_windows()
        layers_before = _niri("--json", "layers")
        clicked_quick_look = menu_action_with_retry(
            find_window_id, find_dir.name, "View menu", "Quick Look"
        )
        time.sleep(ACTION_SETTLE_S)
        windows_after = niri_windows()
        layers_after = _niri("--json", "layers")
        new_window = len(windows_after) > len(windows_before)
        new_layer = layers_after.stdout != layers_before.stdout
        preview_ok = clicked_quick_look and (new_window or new_layer)
        preview_detail = (
            "View > Quick Look was clicked but produced no new window and no "
            "new layer-shell surface (checked against `niri msg --json "
            "windows` and `niri msg --json layers`); see "
            "crates/finder/src/view/quick_look_controller/controller.rs:33 "
            "(rmac_quick_look::open) -- Quick Look appears to be a no-op on "
            "this build"
            if clicked_quick_look and not (new_window or new_layer)
            else "View > Quick Look menu item was not found"
            if not clicked_quick_look
            else "a new window or layer-shell surface appeared after Quick Look"
        )
        if preview_ok:
            # Close it the same way it was opened (Space/Quick Look toggles).
            click_topbar_menu_item("View menu", "Quick Look")
            time.sleep(MENU_SETTLE_S)
    steps.append(make_step("preview", preview_ok, preview_detail))
    if not preview_ok:
        gaps.append(
            {
                "surface": "files_quick_look",
                "issue": "View > Quick Look does not open any visible surface "
                "on the reference session even with a real selection and a "
                "focused window (crates/finder/src/view/"
                "quick_look_controller/controller.rs:33).",
            }
        )

    steps.append(close_files_window(find_window_id, find_dir.name))

    # --- copy -------------------------------------------------------------
    # Only one rmac-files window is ever kept open at a time here: the
    # reference session showed the shell top bar's Files menu bridge is
    # backed by a single well-known D-Bus name (org.rmac.Files.Menu; see
    # module docstring), so with two Files windows open simultaneously a
    # menu click is not reliably routed to the intended (focused) one. This
    # is itself a recorded gap; the source window is fully closed before
    # the destination window is opened, for a deterministic copy/paste.
    copy_window_id, open_copy_step = open_files_window(copy_dir, copy_dir.name)
    steps.append(open_copy_step)
    copy_selected = copy_window_id is not None and menu_action_with_retry(
        copy_window_id, copy_dir.name, "Edit menu", "Select All"
    )
    copy_clicked = copy_selected and menu_action_with_retry(
        copy_window_id, copy_dir.name, "Edit menu", "Copy"
    )
    if copy_window_id is not None:
        steps.append(close_files_window(copy_window_id, copy_dir.name))

    copy_dest_window_id, open_copy_dest_step = open_files_window(
        copy_dest_dir, copy_dest_dir.name
    )
    steps.append(open_copy_dest_step)
    paste_clicked = copy_dest_window_id is not None and menu_action_with_retry(
        copy_dest_window_id, copy_dest_dir.name, "Edit menu", "Paste"
    )
    time.sleep(ACTION_SETTLE_S)
    source_kept = (copy_dir / "journey-note.txt").exists()
    pasted = (copy_dest_dir / "journey-note.txt").exists()
    copy_ok = copy_clicked and paste_clicked and source_kept and pasted
    copy_detail = (
        "Edit > Copy then Edit > Paste (in a second, separately opened "
        "Files window on the destination folder) produced "
        "copy-dest/journey-note.txt while the original stayed in place"
        if copy_ok
        else f"copy_selected={copy_selected} copy_clicked={copy_clicked} "
        f"paste_clicked={paste_clicked} source_kept={source_kept} "
        f"pasted={pasted}; both menu clicks succeeded but nothing was "
        "pasted -- crates/finder/src/pasteboard.rs's #[cfg(not(target_os "
        "= \"macos\"))] implementation (lines 62-70) is a complete no-op "
        "stub on Linux (write_file_urls/read_file_urls/clear_file_urls all "
        "do nothing), so Copy never puts anything on any clipboard at all "
        "on this platform"
    )
    steps.append(make_step("copy", copy_ok, copy_detail))
    if copy_dest_window_id is not None:
        steps.append(close_files_window(copy_dest_window_id, copy_dest_dir.name))
    if not copy_ok:
        gaps.append(
            {
                "surface": "files_clipboard",
                "issue": "crates/finder/src/pasteboard.rs has no Linux "
                "implementation: the #[cfg(not(target_os = \"macos\"))] "
                "module (lines 62-70) stubs write_file_urls, "
                "read_file_urls, and clear_file_urls to no-ops, so Copy/"
                "Cut/Paste between Files windows (or to/from any other "
                "app) is completely non-functional on Linux -- this is "
                "the actual product, not just an automation gap.",
            }
        )

    # --- move (cut + paste) ------------------------------------------------
    move_window_id, open_move_step = open_files_window(move_dir, move_dir.name)
    steps.append(open_move_step)
    move_selected = move_window_id is not None and menu_action_with_retry(
        move_window_id, move_dir.name, "Edit menu", "Select All"
    )
    cut_clicked = move_selected and menu_action_with_retry(
        move_window_id, move_dir.name, "Edit menu", "Cut"
    )
    if move_window_id is not None:
        steps.append(close_files_window(move_window_id, move_dir.name))

    move_dest_window_id, open_move_dest_step = open_files_window(
        move_dest_dir, move_dest_dir.name
    )
    steps.append(open_move_dest_step)
    paste_clicked = move_dest_window_id is not None and menu_action_with_retry(
        move_dest_window_id, move_dest_dir.name, "Edit menu", "Paste"
    )
    time.sleep(ACTION_SETTLE_S)
    source_gone = not (move_dir / "journey-note.txt").exists()
    moved_ok = (move_dest_dir / "journey-note.txt").exists()
    move_ok = cut_clicked and paste_clicked and source_gone and moved_ok
    move_detail = (
        "Edit > Cut then Edit > Paste (in a second, separately opened Files "
        "window on the destination folder) moved journey-note.txt out of "
        "its source folder and into move-dest/"
        if move_ok
        else f"move_selected={move_selected} cut_clicked={cut_clicked} "
        f"paste_clicked={paste_clicked} source_gone={source_gone} "
        f"moved_ok={moved_ok}; same root cause as copy -- "
        "crates/finder/src/pasteboard.rs has no Linux clipboard "
        "implementation (see files_clipboard gap)"
    )
    steps.append(make_step("move", move_ok, move_detail))
    if move_window_id is not None:
        steps.append(close_files_window(move_window_id, move_dir.name))
    if move_dest_window_id is not None:
        steps.append(close_files_window(move_dest_window_id, move_dest_dir.name))

    # --- rename (inline only; expected to be unreachable) ------------------
    # find_dir's window was already closed above; reopen it to check the
    # menus live rather than assuming last run's result still holds.
    rename_window_id, open_rename_step = open_files_window(find_dir, find_dir.name)
    steps.append(
        make_step(
            "open_rename_window",
            rename_window_id is not None,
            "reopened the find-me folder to check for a Rename action"
            if rename_window_id is not None
            else open_rename_step["detail"],
        )
    )
    rename_item_present = False
    if rename_window_id is not None:
        focus_files_window(rename_window_id, find_dir.name)
        for menu_name in ("File menu", "Edit menu"):
            menu_button = find_node(TOPBAR_ATSPI_APP_NAME, menu_name, role="button", timeout=2.0)
            if menu_button is None:
                continue
            click(menu_button)
            time.sleep(MENU_SETTLE_S)
            items = list_names(TOPBAR_ATSPI_APP_NAME, "menu item", timeout=1.0)
            if any("rename" in item.lower() for item in items):
                rename_item_present = True
            click(menu_button)
            time.sleep(MENU_SETTLE_S)
    steps.append(
        make_step(
            "rename",
            False,
            "no 'Rename' item exists in Files' File or Edit top-bar menus "
            "(live-checked this run), and renaming is otherwise only "
            "reachable by selecting a row inline and pressing Return "
            "(crates/finder/src/view/rename_controller.rs), which needs "
            "row-level AT-SPI selection (unavailable, see files_content gap) "
            "and a keyboard injector (not installed on this reference "
            "session) to send Return. File > Get Info was also checked and "
            "exposes only a Close button over AT-SPI, no editable name "
            "field."
            if not rename_item_present
            else "a 'Rename' menu item now exists; this script has not been "
            "updated to use it",
        )
    )
    if not rename_item_present:
        gaps.append(
            {
                "surface": "files_rename",
                "issue": "There is no accessible action to rename a file: no "
                "'Rename' menu item exists anywhere in Files' top-bar menus, "
                "renaming is otherwise inline-only "
                "(crates/finder/src/view/rename_controller.rs) requiring a "
                "Return keypress with no keyboard injector available, and "
                "File > Get Info's dialog exposes no editable content over "
                "AT-SPI (only a Close button).",
            }
        )
    if rename_window_id is not None:
        steps.append(close_files_window(rename_window_id, find_dir.name))

    # --- trash + undo --------------------------------------------------
    trash_before = snapshot_trash(marker)
    trash_window_id, open_trash_step = open_files_window(trash_dir, trash_dir.name)
    steps.append(open_trash_step)
    trash_ok = False
    trash_detail = "Files window for the trash fixture did not open"
    if trash_window_id is not None:
        trash_selected = menu_action_with_retry(
            trash_window_id, trash_dir.name, "Edit menu", "Select All"
        )
        trashed_clicked = trash_selected and menu_action_with_retry(
            trash_window_id, trash_dir.name, "File menu", "Move to Trash"
        )
        time.sleep(ACTION_SETTLE_S)
        gone_from_source = not (trash_dir / "journey-note.txt").exists()
        trash_after = snapshot_trash(marker)
        new_trash_entries = trash_after - trash_before
        trash_ok = trashed_clicked and gone_from_source and bool(new_trash_entries)
        trash_detail = (
            f"File > Move to Trash removed journey-note.txt from its folder "
            f"and it now appears in Trash ({len(new_trash_entries)} new "
            "entry/entries tied to this run)"
            if trash_ok
            else f"trashed_clicked={trashed_clicked} "
            f"gone_from_source={gone_from_source} "
            f"new_trash_entries={len(new_trash_entries)}"
        )
    steps.append(make_step("trash", trash_ok, trash_detail))

    # --- undo ------------------------------------------------------------
    undo_ok = False
    undo_detail = "trash did not succeed; undo was not attempted"
    if trash_ok and trash_window_id is not None:
        undo_clicked = menu_action_with_retry(
            trash_window_id, trash_dir.name, "Edit menu", "Undo"
        )
        time.sleep(ACTION_SETTLE_S)
        restored = (trash_dir / "journey-note.txt").exists()
        trash_after_undo = snapshot_trash(marker)
        residue = trash_after_undo - trash_before
        undo_ok = undo_clicked and restored and not residue
        undo_detail = (
            "Edit > Undo restored journey-note.txt to its original folder "
            "and removed the matching Trash entry"
            if undo_ok
            else f"undo_clicked={undo_clicked} restored={restored} "
            f"residue={len(residue)}"
        )
        if residue:
            cleanup_trash(residue)
    steps.append(make_step("undo", undo_ok, undo_detail))
    if trash_window_id is not None:
        steps.append(close_files_window(trash_window_id, trash_dir.name))

    # --- safety: nothing outside the test folder changed -------------------
    trash_final = snapshot_trash(marker)
    residue_final = trash_final - trash_before
    if residue_final:
        cleanup_trash(residue_final)
    steps.append(
        make_step(
            "safety_no_trash_residue",
            not residue_final,
            "no Trash entries tied to this run remain outside the restored "
            "state"
            if not residue_final
            else f"{len(residue_final)} leftover Trash entr(y/ies) were "
            "found and removed during cleanup",
        )
    )

    return steps, gaps


def snapshot_trash(marker: str) -> set[str]:
    """Names of this run's .trashinfo files (matched by the disposable test
    root's name in their recorded original Path=), never anyone else's."""

    info_dir = Path.home() / ".local" / "share" / "Trash" / "info"
    if not info_dir.is_dir():
        return set()
    matches: set[str] = set()
    for entry in info_dir.iterdir():
        if entry.suffix != ".trashinfo":
            continue
        try:
            text = entry.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        if relative_trashinfo_path_matches(text, marker):
            matches.add(entry.name)
    return matches


def cleanup_trash(entries: set[str]) -> None:
    info_dir = Path.home() / ".local" / "share" / "Trash" / "info"
    files_dir = Path.home() / ".local" / "share" / "Trash" / "files"
    for entry in entries:
        info_path = info_dir / entry
        stem = entry[: -len(".trashinfo")] if entry.endswith(".trashinfo") else entry
        file_path = files_dir / stem
        for path in (info_path, file_path):
            try:
                if path.is_dir() and not path.is_symlink():
                    shutil.rmtree(path, ignore_errors=True)
                elif path.exists() or path.is_symlink():
                    path.unlink()
            except OSError:
                pass


def cleanup_windows() -> None:
    """Close only the Files windows this script itself opened and did not
    already close (belt-and-braces for the failure path). This never closes
    a Files window this script did not open itself -- another agent or a
    real user may have one open at the same time (see the shared
    UI-driving lock this script is run under in docs/journey-suite.md)."""

    remaining = set(_OPENED_WINDOW_IDS)
    if not remaining:
        return
    current_ids = {window.get("id") for window in niri_windows()}
    for window_id in remaining:
        if window_id in current_ids:
            niri_close_window(window_id)
    _wait_for(
        lambda: not (remaining & {w.get("id") for w in niri_windows()}),
        WINDOW_GONE_TIMEOUT_S,
    )
    _OPENED_WINDOW_IDS.difference_update(remaining)


def main() -> int:
    global FILES_EXEC
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output",
        type=Path,
        default=None,
        help="absolute path to write the JSON report to (also printed to stdout)",
    )
    parser.add_argument(
        "--files-exec",
        default=None,
        help="rmac-files binary to test (default: ~/rmac-dev-bin/rmac-files when "
        f"present, else {FILES_EXEC})",
    )
    arguments = parser.parse_args()
    FILES_EXEC = choose_files_exec(arguments.files_exec, Path.home())
    print(f"journey 2: testing {FILES_EXEC}", file=sys.stderr)

    try:
        additions = discover_environment(dict(os.environ), Path(f"/run/user/{os.getuid()}"))
        os.environ.update(additions)
    except JourneyError as error:
        parser.exit(4, f"run-journey-files: {error}\n")

    started_at_unix_ms = int(time.time() * 1000)
    login_step = check_logged_in()
    steps: list[dict[str, Any]] = [login_step]
    gaps: list[dict[str, str]] = []

    if not login_step["passed"]:
        report = build_report(steps, gaps, started_at_unix_ms, "")
        text = json.dumps(report, indent=2, sort_keys=True) + "\n"
        if arguments.output is not None:
            arguments.output.write_text(text, encoding="utf-8")
        print(text, end="")
        print("journey 2: session not ready; nothing else was attempted", file=sys.stderr)
        return 1

    documents_dir = Path.home() / "Documents"
    documents_dir.mkdir(parents=True, exist_ok=True)
    before_listing = sorted(p.name for p in documents_dir.iterdir())

    test_root = Path(tempfile.mkdtemp(dir=documents_dir, prefix="lulo-journey-2-"))
    saved_accessibility = get_gsettings_accessibility()
    set_gsettings_accessibility(True)

    try:
        journey_steps, gaps = run_journey(test_root)
        steps.extend(journey_steps)
    finally:
        cleanup_windows()
        restore_gsettings_accessibility(saved_accessibility)
        shutil.rmtree(test_root, ignore_errors=True)
        after_listing = sorted(p.name for p in documents_dir.iterdir())
        steps.append(
            make_step(
                "safety_documents_unchanged",
                before_listing == after_listing,
                "~/Documents has exactly the entries it had before this run"
                if before_listing == after_listing
                else "~/Documents' top-level listing changed during this run "
                f"(before={len(before_listing)} entries, "
                f"after={len(after_listing)} entries)",
            )
        )

    report = build_report(steps, gaps, started_at_unix_ms, test_root.name)
    text = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if arguments.output is not None:
        arguments.output.write_text(text, encoding="utf-8")
    print(text, end="")

    passed = sum(1 for step in report["steps"] if step["passed"])
    total = len(report["steps"])
    print(
        f"journey 2: {passed}/{total} steps passed; "
        f"overall_pass={report['overall_pass']}",
        file=sys.stderr,
    )
    return 0 if report["overall_pass"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
