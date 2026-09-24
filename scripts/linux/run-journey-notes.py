#!/usr/bin/env python3
"""Acceptance test for product journey 4 (todo.md "Product journeys"):

    Create, search and edit a note, and recover it after a crash.

Runs against the live rmac session on the reference laptop (niri + AT-SPI +
GDM) -- the same session, and the same real Notes library
($XDG_DATA_HOME/rmac/notes, crates/rmac-notes-storage/src/startup.rs:67-92),
that a real user relies on. There is no keyboard or pointer injector
installed there (no wtype, no ydotool), so every step must drive the UI only
through AT-SPI actions (pyatspi) and niri IPC -- see
scripts/linux/run-journey-launch.py for the established pattern this script
follows (find_node/click/_atspi_snapshot).

This script deliberately stops short of creating, editing, trashing or
permanently deleting any note, including a throwaway test note, and explains
why below. This is a safety decision required by the brief's "never read or
modify other notes" constraint, not an oversight: two independent,
live-confirmed accessibility gaps make it impossible to satisfy that
constraint on this build.

1. Every text-entry surface in Notes (the search field, and the new-note
   title/tags/body fields) exposes neither AT-SPI Text nor EditableText --
   live-confirmed: a fresh `rmac-notes` window's AT-SPI tree lists its four
   `entry`-roled nodes with `interfaces=['Accessible', 'Component']` only, no
   Text, no EditableText, not even a readable value. Source-side,
   `crates/rmac-ui/src/controls.rs` and the vendored `gpui_component::Input`
   set only `.role(Role::TextInput/MultilineTextInput)` -- no
   `aria_label`/`aria_value`/`aria_placeholder`, and (unlike Spotlight's
   search field, `crates/launcher-app/src/view/render.rs:351-360`) Notes
   wires no `on_a11y_action(AccessibleAction::SetValue/ReplaceSelectedText,
   ...)` handler at all (`grep -rn "on_a11y_action|AccessibleAction"
   crates/notes/src/*.rs crates/rmac-editor/src/lib.rs`: zero matches). A
   title, tag, search query or body cannot be read or typed via pure AT-SPI
   on this build -- there is no accessible-name-bearing, uniquely
   identifiable string this script could give a test note even if it could
   otherwise select one.

2. The entire note list and folder sidebar -- including "Recently Deleted"
   (`crates/notes/src/note_navigation.rs:95-104`, a plain
   `div().id("trash-notes")...on_click(...)` with no `.role()`/
   `.aria_label()`, per `crates/notes/src/presentation.rs:49-90`'s
   `folder_row`) and every note row (`crates/notes/src/note_navigation.rs:
   271-274`, same pattern) -- is completely absent from the AT-SPI tree, not
   merely unnamed. Live-confirmed twice, ~2 seconds apart, against a freshly
   launched `rmac-notes`: its AT-SPI tree contains exactly one frame with 20
   flat children (16 `button` nodes -- 12 clickable and unnamed, 2 disabled/
   inert, and 2 named "Edit"/"Preview" view-toggle buttons -- and 4 `entry`
   nodes for search/title/tags/body), and *no* additional
   container, list, or row nodes of any kind. (For comparison, Terminal's
   equally unnamed, equally `.id()`-only tab-strip buttons -- see
   scripts/linux/run-journey-terminal.py -- *do* still appear as generic
   AT-SPI "button" nodes; Notes' sidebar rows do not appear at all. This
   script cannot state the exact mechanism from source alone -- it may be
   pruned entirely, or the sidebar may be collapsed by a responsive layout
   at the window's default size -- and flags this uncertainty rather than
   asserting a root cause it did not verify live beyond the two dumps
   above.) Without any accessible node for a note or folder row, this script
   cannot select an existing note, open "Recently Deleted", or verify that a
   newly created note was not left behind: a wrong guess among the flat,
   nameless toolbar buttons (`crates/notes/src/toolbar.rs`'s `sort`,
   `import-note`, `import-bundle`, `compose`, `export-notes`, `checklist`,
   `add-image`, `move-note`, `pin`, `trash`, `delete-permanently`, all
   `Button::new(id, "")`) risks acting on whatever note the app already had
   open -- which could be real user data -- with no way to verify or undo it
   afterward.

Given (1) and (2) together, any note this script created would (a) be
untitled/unidentifiable, and (b) be permanently un-removable by this script
afterward (no AT-SPI path exists to select it in a list it cannot see), which
would leave clutter in the reference user's real Notes library forever --
exactly the outcome the brief's cleanup requirement exists to prevent. The
only accessible-name-bearing, safely reversible actions available today are
the global top-bar per-app menu items (`crates/rmac-app-menu/src/
lib.rs:133-156` NOTES_MENUS: File > New Note/New Folder/Export Notes..., Edit
> Find..., Format > Checklist, View > Sort by ...) and the in-editor
Edit/Preview toggle -- and even File > New Note is excluded here because it
is a note-creating action this script could not clean up afterward. So this
script verifies session liveness, launch, window timing/focus, the
AT-SPI-confirmed absence of the note-list/sidebar surface, and the presence
(read-only: opened and inspected, but no destructive item is invoked) of the
top-bar's Notes-specific menus, then quits the app cleanly. Live testing
found the File/Edit/Format category menus present on one run and absent on
another from an otherwise-identical fresh launch (the same inconsistency
observed for Terminal's Shell/Edit/View menus, see
scripts/linux/run-journey-terminal.py) -- this script cannot explain it from
available evidence and reports precisely whichever state it finds each run
rather than assuming success. This is the same
honest-limitation approach todo.md asks for ("An honest limitation beats
simulated system behaviour"), applied one gap earlier than
scripts/linux/run-journey-terminal.py's equivalent stop, because here even
the identify-and-clean-up precondition cannot be met.

The report is privacy-safe: no screenshots, no home-directory paths, no note
titles or bodies (impossible to read anyway -- see above), no user names.
Every wait is bounded; this script never hangs.
"""

from __future__ import annotations

import argparse
import json
import os
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
    """A bounded, privacy-safe journey-notes failure."""


FORMAT = 1
JOURNEY_ID = 4
JOURNEY_TITLE = "Create, search and edit a note, and recover it after a crash."

NIRI_TIMEOUT_S = 5.0
WINDOW_APPEAR_TIMEOUT_S = 5.0
FOCUS_TIMEOUT_S = 3.0
# The top bar's per-app menu can take longer than 5 s to switch to a
# just-focused app's menu spec under heavy CPU contention (observed live on
# the reference laptop while a concurrent cargo build was running, load
# average ~7); 8 s gives it enough slack without masking a real gap (an
# absent menu still fails after 8 s).
ATSPI_FIND_TIMEOUT_S = 8.0
CLOSE_TIMEOUT_S = 5.0
SETTLE_TIMEOUT_S = 2.0
POLL_INTERVAL_S = 0.05

# todo.md "Performance budgets": p95 <= 500 ms for simple apps. Notes is not
# named among the 900 ms Files/Terminal exceptions.
SIMPLE_APP_BUDGET_MS = 500.0

APP: dict[str, str] = {
    "display_name": "Notes",
    "app_id": "org.rmac.Notes",
    "exec": "/usr/bin/rmac-notes",
    "atspi_app_name": "rmac-notes",
}

# The top-bar-rendered category menus Notes registers
# (crates/rmac-app-menu/src/lib.rs:133-156 NOTES_MENUS) and the items each
# one is expected to expose. Verified read-only: opened, item presence
# checked, never invoked (see module docstring for why).
NOTES_MENUS: tuple[tuple[str, tuple[str, ...]], ...] = (
    ("File menu", ("New Note", "New Folder", "Export Notes…")),
    ("Edit menu", ("Find…",)),
    ("Format menu", ("Checklist",)),
)


# --------------------------------------------------------------------------
# Pure helpers (unit-tested from scripts/test_journey_notes.py on macOS)
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


def find_window_by_app_id(
    windows: list[dict[str, Any]], app_id: str
) -> Optional[dict[str, Any]]:
    for window in windows:
        if window.get("app_id") == app_id:
            return window
    return None


def evaluate_budget(elapsed_ms: float, budget_ms: float) -> dict[str, Any]:
    return {
        "elapsed_ms": round(elapsed_ms, 1),
        "budget_ms": budget_ms,
        "within_budget": elapsed_ms <= budget_ms,
    }


def make_step(step_id: str, passed: bool, detail: str, **extra: Any) -> dict[str, Any]:
    step = {"id": step_id, "passed": bool(passed), "detail": detail}
    step.update(extra)
    return step


def build_report(
    steps: list[dict[str, Any]],
    gaps: list[dict[str, str]],
    performance: dict[str, Any],
    started_at_unix_ms: int,
) -> dict[str, Any]:
    return {
        "format": FORMAT,
        "journey": JOURNEY_ID,
        "journey_title": JOURNEY_TITLE,
        "started_at_unix_ms": started_at_unix_ms,
        "steps": steps,
        "performance": performance,
        "gaps": gaps,
        "overall_pass": all(step["passed"] for step in steps),
    }


def classify_note_surface(nodes: list[dict[str, str]]) -> dict[str, int]:
    """Pure classification of a (role, name, has_click) node list into
    counts, used to decide whether any list/row-shaped surface exists.
    Exposed separately so the classification logic (not the live pyatspi
    walk) is unit-testable."""

    # "application" and "frame" are the app's own root/window container
    # nodes, always present and structural -- never a candidate note/folder
    # row, so they must not count toward "other" (a real bug caught live:
    # without this exclusion, every snapshot -- including the flat,
    # list-free one -- reported a false-positive list surface, because the
    # snapshot always includes the app and frame nodes themselves).
    STRUCTURAL_ROLES = {"application", "frame"}

    counts = {
        "button_named": 0,
        "button_unnamed_clickable": 0,
        "button_unnamed_inert": 0,
        "entry": 0,
        "other": 0,
    }
    for node in nodes:
        role = node.get("role")
        name = node.get("name", "")
        if role in STRUCTURAL_ROLES:
            continue
        if role == "button" and name:
            counts["button_named"] += 1
        elif role == "button" and node.get("has_click"):
            counts["button_unnamed_clickable"] += 1
        elif role == "button":
            counts["button_unnamed_inert"] += 1
        elif role == "entry":
            counts["entry"] += 1
        else:
            counts["other"] += 1
    return counts


def has_list_shaped_surface(counts: dict[str, int]) -> bool:
    """True only if something other than the known flat toolbar/editor
    layout (buttons + entries) is present -- i.e. a candidate list/row
    container that today's build does not have."""

    return counts.get("other", 0) > 0


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


def niri_spawn(command: str) -> None:
    result = _niri("action", "spawn", "--", command)
    if result.returncode != 0:
        raise JourneyError("niri failed to spawn the target application")


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


def wait_for_window(app_id: str, timeout: float = WINDOW_APPEAR_TIMEOUT_S):
    started = time.monotonic()
    window = _wait_for(lambda: find_window_by_app_id(niri_windows(), app_id), timeout)
    elapsed_ms = (time.monotonic() - started) * 1000.0
    return window, elapsed_ms


def wait_for_window_gone(window_id: int, timeout: float = CLOSE_TIMEOUT_S) -> bool:
    def gone() -> bool:
        return all(window.get("id") != window_id for window in niri_windows())

    return bool(_wait_for(gone, timeout))


def pid_alive(pid: int) -> bool:
    return Path(f"/proc/{pid}").exists()


# --------------------------------------------------------------------------
# AT-SPI helpers (same idiom as scripts/linux/run-journey-launch.py)
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


def snapshot_note_surface(app_name: str = APP["atspi_app_name"]) -> list[dict[str, Any]]:
    """Structural (role/name/has_click) snapshot of every node in the Notes
    window -- deliberately never reads text content (there is none to
    read; see module docstring), only role/name/action shape."""

    nodes = []
    for node in _atspi_snapshot(app_name):
        try:
            role = node.getRoleName()
        except (LookupError, RuntimeError):
            role = "<err>"
        try:
            name = node.name
        except (LookupError, RuntimeError):
            name = ""
        has_click = "click" in action_names(node)
        nodes.append({"role": role, "name": name, "has_click": has_click})
    return nodes


def open_top_bar_menu(display_name_menu: str, timeout: float = ATSPI_FIND_TIMEOUT_S):
    button = find_node("rmac-top-bar", display_name_menu, role="button", timeout=timeout)
    if button is None or "click" not in action_names(button):
        return None
    click(button)
    return button


def menu_item_present(label: str, timeout: float = 2.0) -> bool:
    item = find_node("rmac-top-bar", label, timeout=timeout)
    return item is not None and "click" in action_names(item)


def close_top_bar_menu(menu_button) -> None:
    """Close a just-opened top-bar menu without invoking any item, by
    clicking its own toggle button again."""

    try:
        if menu_button is not None and "click" in action_names(menu_button):
            click(menu_button)
    except JourneyError:
        pass


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
    return make_step("logged_in", True, "an active local rmac graphical session was found")


def attempt_dock_launch() -> dict[str, Any]:
    button = find_node("rmac-dock", APP["display_name"], role="button")
    if button is None:
        return make_step(
            "dock_launch",
            False,
            f"no Dock button named {APP['display_name']!r} was found over AT-SPI",
        )
    if "click" not in action_names(button):
        return make_step(
            "dock_launch",
            False,
            "Dock icon has no AT-SPI 'click' action; it cannot be activated "
            "without a pointer device",
        )
    click(button)
    return make_step("dock_launch", True, "clicked the Dock icon via its AT-SPI click action")


def launch_notes() -> tuple[dict[str, Any], dict[str, Any]]:
    dock_step = attempt_dock_launch()
    if dock_step["passed"]:
        return dock_step, make_step(
            "app_launched", True, "launched via an accessible Dock action", method="accessible_ui"
        )
    try:
        niri_spawn(APP["exec"])
    except JourneyError as error:
        return dock_step, make_step("app_launched", False, str(error), method="none")
    return dock_step, make_step(
        "app_launched",
        True,
        "the Dock icon could not be driven over AT-SPI today (see dock_launch "
        "above); launched the same installed command a Dock activation would "
        "run, to still measure the rest of the journey",
        method="fallback_spawn",
    )


def quit_notes(window: dict[str, Any]) -> dict[str, Any]:
    menu_button = open_top_bar_menu(f"{APP['display_name']} menu")
    if menu_button is None:
        return make_step(
            "close",
            False,
            f"the {APP['display_name']} app menu was not found over AT-SPI",
            app_id=window.get("app_id"),
        )
    quit_item = find_node("rmac-top-bar", f"Quit {APP['display_name']}", timeout=2.0)
    if quit_item is None or "click" not in action_names(quit_item):
        return make_step(
            "close",
            False,
            f"no actionable Quit {APP['display_name']} menu item was found over AT-SPI",
            app_id=window.get("app_id"),
        )
    click(quit_item)
    window_gone = wait_for_window_gone(window["id"])
    pid = window.get("pid")
    process_gone = True
    if window_gone and isinstance(pid, int):
        process_gone = bool(_wait_for(lambda: not pid_alive(pid), CLOSE_TIMEOUT_S))
    passed = window_gone and process_gone
    detail = (
        "Quit removed the window and its process exited"
        if passed
        else f"window_gone={window_gone} process_gone={process_gone}"
    )
    return make_step("close", passed, detail, app_id=window.get("app_id"))


# --------------------------------------------------------------------------
# Orchestration
# --------------------------------------------------------------------------


def run_journey(budget_ms: float, keep_open: bool) -> dict[str, Any]:
    steps: list[dict[str, Any]] = []
    gaps: list[dict[str, str]] = []
    performance: dict[str, Any] = {}
    started_at_unix_ms = int(time.time() * 1000)

    steps.append(check_logged_in())

    saved_accessibility = get_gsettings_accessibility()
    set_gsettings_accessibility(True)
    window: Optional[dict[str, Any]] = None
    try:
        dock_step, launch_step = launch_notes()
        steps.append(dock_step)
        steps.append(launch_step)
        if not dock_step["passed"]:
            gaps.append(
                {
                    "surface": "dock",
                    "issue": "the Notes Dock icon could not be activated over "
                    "AT-SPI (no button named 'Notes' found, or it has no "
                    "'click' action)",
                }
            )
        if not launch_step["passed"]:
            return build_report(steps, gaps, performance, started_at_unix_ms)

        window, elapsed_ms = wait_for_window(APP["app_id"])
        performance["launch"] = evaluate_budget(elapsed_ms, budget_ms)
        steps.append(
            make_step(
                "window_appeared",
                window is not None,
                (
                    f"window appeared with app_id={APP['app_id']!r} in {elapsed_ms:.0f} ms"
                    if window
                    else "no window with the expected app_id appeared"
                ),
            )
        )
        if window is None:
            return build_report(steps, gaps, performance, started_at_unix_ms)

        steps.append(
            make_step(
                "window_focused",
                bool(window.get("is_focused")),
                "the launched window is focused"
                if window.get("is_focused")
                else "the launched window is not focused",
            )
        )

        # Let the window settle before snapshotting; confirmed live that a
        # 2 s settle does not change the tree shape versus an immediate
        # snapshot, but keep the wait for reproducibility.
        time.sleep(SETTLE_TIMEOUT_S)
        nodes = snapshot_note_surface()
        counts = classify_note_surface(nodes)
        list_present = has_list_shaped_surface(counts)
        steps.append(
            make_step(
                "note_list_reachable",
                list_present,
                (
                    "found a non-toolbar/editor AT-SPI node that could be a "
                    "note or folder row"
                    if list_present
                    else "the Notes AT-SPI tree contains only flat toolbar "
                    f"buttons ({counts['button_named']} named, "
                    f"{counts['button_unnamed_clickable']} unnamed clickable) "
                    f"and {counts['entry']} text-entry nodes; no note row, "
                    "folder row, or 'Recently Deleted' control is reachable "
                    "over AT-SPI"
                ),
            )
        )
        gaps.append(
            {
                "surface": "notes-sidebar",
                "issue": "the note list and folder sidebar (including "
                "'Recently Deleted') are absent from the AT-SPI tree, not "
                "merely unnamed (crates/notes/src/note_navigation.rs:271-274 "
                "note rows and crates/notes/src/presentation.rs:49-90 "
                "folder_row are plain .id()-only divs with no .role()/"
                ".aria_label()); a specific note cannot be selected, opened, "
                "trashed, or permanently deleted over AT-SPI on this build",
            }
        )
        gaps.append(
            {
                "surface": "notes-text-entry",
                "issue": "the search field and the title/tags/body fields "
                "expose neither AT-SPI Text nor EditableText (confirmed live: "
                "their interfaces are ['Accessible', 'Component'] only), and "
                "Notes wires no on_a11y_action SetValue/ReplaceSelectedText "
                "handler either (unlike Spotlight's search field); a note's "
                "title, tags, body, or a search query cannot be read or typed "
                "over AT-SPI on this build",
            }
        )

        steps.append(
            make_step(
                "note_lifecycle_blocked",
                False,
                "create/search/edit/recover/delete were not attempted: with "
                "no accessible note-list/sidebar surface (note_list_reachable "
                "above) and no way to type or read a title, this script "
                "cannot create a uniquely identifiable test note, verify it "
                "is the note it later acts on, or clean it up afterward -- "
                "attempting any of File > New Note / trash / permanent "
                "delete blind would risk leaving unremovable clutter in, or "
                "acting on, the real reference-laptop Notes library, which "
                "violates the 'never read or modify other notes' safety "
                "requirement; see module docstring for the full reasoning",
            )
        )

        # Read-only structural check of the Notes-specific top-bar menus:
        # open each, verify its expected items are present, close it again
        # without invoking anything.
        for menu_label, expected_items in NOTES_MENUS:
            menu_button = open_top_bar_menu(menu_label)
            if menu_button is None:
                steps.append(
                    make_step(
                        f"menu_{menu_label.split()[0].lower()}",
                        False,
                        f"the top bar's {menu_label!r} button was not found over AT-SPI",
                    )
                )
                continue
            missing = [item for item in expected_items if not menu_item_present(item)]
            close_top_bar_menu(menu_button)
            steps.append(
                make_step(
                    f"menu_{menu_label.split()[0].lower()}",
                    not missing,
                    (
                        f"{menu_label} exposes all expected items over AT-SPI "
                        f"({', '.join(expected_items)}); none were invoked"
                        if not missing
                        else f"{menu_label} is missing expected item(s): {missing}"
                    ),
                )
            )

        if keep_open:
            return build_report(steps, gaps, performance, started_at_unix_ms)

        steps.append(quit_notes(window))
        window = None
        return build_report(steps, gaps, performance, started_at_unix_ms)
    finally:
        if not keep_open and window is not None:
            try:
                quit_notes(window)
            except JourneyError:
                pass
        restore_gsettings_accessibility(saved_accessibility)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output",
        type=Path,
        default=None,
        help="absolute path to write the JSON report to (also printed to stdout)",
    )
    parser.add_argument(
        "--budget-ms",
        type=float,
        default=SIMPLE_APP_BUDGET_MS,
        help="warm launch-to-window budget in milliseconds, for reporting only",
    )
    parser.add_argument(
        "--keep-open",
        action="store_true",
        help="leave the launched window open for manual inspection (debugging only)",
    )
    arguments = parser.parse_args()

    try:
        additions = discover_environment(dict(os.environ), Path(f"/run/user/{os.getuid()}"))
        os.environ.update(additions)
        report = run_journey(arguments.budget_ms, arguments.keep_open)
    except JourneyError as error:
        parser.exit(4, f"run-journey-notes: {error}\n")

    text = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if arguments.output is not None:
        arguments.output.write_text(text, encoding="utf-8")
    print(text, end="")

    passed = sum(1 for step in report["steps"] if step["passed"])
    total = len(report["steps"])
    print(
        f"journey 4: {passed}/{total} steps passed; "
        f"overall_pass={report['overall_pass']}",
        file=sys.stderr,
    )
    return 0 if report["overall_pass"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
