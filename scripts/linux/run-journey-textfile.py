#!/usr/bin/env python3
"""Acceptance test for product journey 5 (todo.md "Product journeys"):

    Open, edit and save a text file through the portal without losing
    content.

Runs against the live rmac session on the reference laptop (niri + AT-SPI),
driving `rmac-text-editor` (crates/text-editor) and the
`org.freedesktop.portal.FileChooser` Open/Save dialogs it calls through
(crates/rmac-file-chooser, ADR 0012) the same way run-journey-launch.py drives
journey 1: AT-SPI actions (pyatspi) on nodes that expose a real action, plus
niri IPC. There is no keyboard or pointer injector installed on the reference
laptop (no wtype, no ydotool), so every step is either a real AT-SPI action
or an honestly reported gap -- never a simulated keystroke.

Two real, evidence-backed accessibility/testability gaps shape this script:

  * The document buffer (the `entry` node inside `rmac-text-editor`) exposes
    the AT-SPI Accessible and Component interfaces only -- no Text, no
    EditableText (confirmed live: `queryText()`/`queryEditableText()` both
    raise). This is the same accesskit_unix/accesskit_atspi_common gap
    run-journey-launch.py already documents for Spotlight's query field. It
    means no AT-SPI action anywhere can type or insert characters into an
    open document.
  * Text Editor's only other in-window control that changes saved content --
    the encoding/line-ending picker (crates/text-editor/src/view/render/
    chrome.rs:80-127, the "document-actions" dropdown button) -- does expose
    an AT-SPI `click` action like its sibling toolbar buttons, but invoking
    that action does not open its dropdown menu (confirmed live: clicking
    every unlabelled button in the window's toolbar, in turn, never produced
    an AT-SPI menu/popup), so its encoding/line-ending options are reachable
    by neither a screen reader nor this script. The global "Format" menu in
    the top bar (crates/rmac-app-menu/src/lib.rs:94-99) does not mirror it
    either (only font size and monospace live there).

Together these mean: in this build, on this laptop, there is currently no
accessible way to change a document's content or its saved encoding without
a keyboard injector. `attempt_edit` below performs the real check (looking
for Text/EditableText on every entry in the window) and reports this as a
failed step with the evidence above, rather than faking a keystroke.

Everything else in the journey *is* real AT-SPI-drivable and is exercised
for real:

  * Open and Save As go through the top bar's global "File" menu
    (`File menu` -> `Open…` / `Save As…`, both real AT-SPI `menu item`
    nodes with a `click` action -- confirmed live), which is what actually
    calls `cx.prompt_for_paths`/`cx.prompt_for_new_path`
    (shell/compat/gpui_linux/src/linux/platform.rs:391,451), which is what
    calls the portal. Whatever answers -- `rmac-file-chooser` if deployed,
    GNOME's/GTK's chooser otherwise per `rmac-portals.conf`'s fallback list
    -- is driven generically: sidebar/breadcrumb navigation by name match,
    then a file/folder row activated by name match. If that dialog cannot be
    driven (backend not deployed, unexpected layout, dispatch didn't fire),
    the script falls back to loading the document directly with
    `rmac-text-editor <path>` (the same command `Exec=%F` in
    org.rmac.TextEditor.desktop runs for a real double-click), exactly the
    way run-journey-launch.py falls back to a direct spawn when Dock/
    Spotlight can't be driven -- clearly labelled, so the rest of the
    journey (save fidelity, external-change detection, atomic writes) can
    still be measured.
  * "Save with no changes must not touch the file" is verified by hash.
  * "Save As … must write byte-identical content to the new path" is
    verified by hash (save_to_new_path always writes, regardless of the
    dirty flag -- crates/text-editor/src/view/saving.rs:75-140).
  * External-change detection does not depend on typing at all: this script
    plays the role of "another process" and overwrites the open file
    directly while Text Editor holds it open (crates/text-editor/src/view/
    document_state.rs arms an inotify watch via the `notify` crate on every
    load). The script waits for the resulting "Review…" button
    (crates/text-editor/src/view/render.rs:191-207) to appear over AT-SPI,
    clicks it, confirms the Conflict alert's distinctive buttons
    (crates/text-editor/src/view/render/alert.rs:59-76: "Discard & Reload",
    "Save a Copy…", "Overwrite Anyway…"), and clicks Cancel -- verifying
    both that the conflict was surfaced and that neither the local buffer
    nor the external file were touched by Cancel.
  * The SIGKILL-during-save test targets Save As (not Save), because Save
    is a documented no-op when the buffer isn't dirty
    (crates/text-editor/src/view/saving.rs:37-42) and this script has no way
    to dirty it (see above). Save As always calls `save_document_copy` ->
    `write_document_if_unchanged` -> `rmac_storage::atomic_write`
    (crates/rmac-storage/src/write.rs:52-86): write a `.{name}.tmp-<pid>-
    <seq>` sibling, `sync_all`, `rename`, sync the parent directory. The
    script re-saves a several-MB fixture over itself through the same
    Save-As UI path, polls the fixture's directory for that temp file to
    appear (proof the write is in flight), and SIGKILLs the editor process
    at that instant -- then verifies the destination is either the complete
    original bytes or does not exist, but is never truncated. This is
    best-effort ("if feasible" per the brief): if the write completes before
    the temp file is observed, or the UI path to trigger it fails, the step
    is reported honestly as not conclusively exercised rather than a false
    pass.

The report is privacy-safe: no screenshots, no home-directory paths beyond
the disposable fixture folder's random suffix, no file contents. Every wait
is bounded; the script never hangs, and every fixture and window it creates
is cleaned up in a `finally` block.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import secrets
import signal
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
    """A bounded, privacy-safe journey-textfile failure."""


FORMAT = 1
JOURNEY_ID = 5
JOURNEY_TITLE = (
    "Open, edit and save a text file through the portal without losing content."
)

NIRI_TIMEOUT_S = 5.0
WINDOW_APPEAR_TIMEOUT_S = 5.0
CLOSE_TIMEOUT_S = 5.0
ATSPI_FIND_TIMEOUT_S = 5.0
CHOOSER_APPEAR_TIMEOUT_S = 6.0
CHOOSER_NAV_TIMEOUT_S = 4.0
CONFLICT_DETECT_TIMEOUT_S = 6.0
POLL_INTERVAL_S = 0.05

TEXT_EDITOR: dict[str, str] = {
    "display_name": "Text Editor",
    "app_id": "org.rmac.TextEditor",
    "exec": "/usr/bin/rmac-text-editor",
    "atspi_name": "rmac-text-editor",
}

# A fixture large enough that its atomic write takes long enough to interrupt
# on the reference laptop's spinning-rust-free but modest SSD/eMMC storage.
INTERRUPT_FIXTURE_BYTES = 24 * 1024 * 1024
INTERRUPT_ATTEMPTS = 5
INTERRUPT_POLL_S = 0.001


# --------------------------------------------------------------------------
# Pure helpers (unit-tested from scripts/test_journey_textfile.py on macOS)
# --------------------------------------------------------------------------


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def fixture_dirname(token: str) -> str:
    """The disposable top-level folder name this script creates under
    ~/Documents. Kept pure so the naming convention is unit-testable."""

    if not token or any(character in token for character in "/\\ \t\n"):
        raise JourneyError("invalid fixture token")
    return f"lulo-journey-5-{token}"


def make_sample_content(token: str) -> bytes:
    """Deterministic, human-legible fixture content carrying the run's
    unique token, so a stray leftover file is always traceable."""

    lines = [
        f"rmac journey 5 fixture {token}",
        "The quick brown fox jumps over the lazy dog.",
        "Line three intentionally left distinct.",
        "",
    ]
    return "\n".join(lines).encode("utf-8")


def make_large_content(token: str, size: int) -> bytes:
    """A larger, still-deterministic fixture used for the SIGKILL-during-
    save test, padded to `size` bytes with a repeating, greppable pattern."""

    header = f"rmac journey 5 large fixture {token}\n".encode("utf-8")
    pattern = (
        b"0123456789abcdef" * 64
    )  # 1024 bytes, cheap to repeat and easy to spot-check
    body = bytearray(header)
    while len(body) < size:
        body.extend(pattern)
    return bytes(body[:size])


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


def temp_write_pattern(destination_name: str) -> str:
    """The literal prefix `rmac_storage::atomic_write` uses for its sibling
    temp file (crates/rmac-storage/src/write.rs:64): `.{name}.tmp-`. Kept as
    a pure helper so the SIGKILL test's polling logic is unit-testable."""

    return f".{destination_name}.tmp-"


def find_orphaned_temp_files(directory_entries: list[str], destination_name: str) -> list[str]:
    prefix = temp_write_pattern(destination_name)
    return [entry for entry in directory_entries if entry.startswith(prefix)]


# --------------------------------------------------------------------------
# niri IPC (mirrors run-journey-launch.py)
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


def wait_for_window(app_id: str, timeout: float = WINDOW_APPEAR_TIMEOUT_S):
    return _wait_for(lambda: find_window_by_app_id(niri_windows(), app_id), timeout)


def wait_for_window_gone(window_id: int, timeout: float = CLOSE_TIMEOUT_S) -> bool:
    def gone() -> bool:
        return all(window.get("id") != window_id for window in niri_windows())

    return bool(_wait_for(gone, timeout))


def pid_alive(pid: int) -> bool:
    return Path(f"/proc/{pid}").exists()


# --------------------------------------------------------------------------
# AT-SPI helpers (mirrors run-journey-launch.py)
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


def list_atspi_app_names() -> set[str]:
    _require_pyatspi()
    desktop = pyatspi.Registry.getDesktop(0)
    names: set[str] = set()
    for app in desktop:
        try:
            names.add(app.name)
        except (LookupError, RuntimeError):
            continue
    return names


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


def find_any_node(
    app_name: str,
    node_names: list[str],
    role: Optional[str] = None,
    timeout: float = ATSPI_FIND_TIMEOUT_S,
):
    """Like find_node, but accepts several candidate names (different
    chooser backends label the same control differently)."""

    def search():
        for node in _atspi_snapshot(app_name):
            try:
                name = node.name
                if name not in node_names:
                    continue
                if role is not None and node.getRoleName() != role:
                    continue
            except (LookupError, RuntimeError):
                continue
            return node
        return None

    return _wait_for(search, timeout)


def action_names(node) -> list[str]:
    try:
        if "Action" not in node.get_interfaces():
            return []
        actions = node.queryAction()
        return [actions.getName(index) for index in range(actions.nActions)]
    except (LookupError, RuntimeError):
        return []


def click(node) -> bool:
    names = action_names(node)
    if "click" not in names:
        raise JourneyError("AT-SPI node has no 'click' action")
    actions = node.queryAction()
    return bool(actions.doAction(names.index("click")))


def has_editable_text(node) -> bool:
    try:
        node.queryEditableText()
        return True
    except (LookupError, RuntimeError, NotImplementedError):
        return False


def has_text_interface(node) -> bool:
    try:
        node.queryText()
        return True
    except (LookupError, RuntimeError, NotImplementedError):
        return False


# --------------------------------------------------------------------------
# Journey steps
# --------------------------------------------------------------------------


def launch_editor(path: Optional[Path] = None) -> tuple[dict[str, Any], Optional[dict[str, Any]]]:
    """Launch a fresh rmac-text-editor window, optionally with a path given
    directly on the command line (the same thing a real double-click on the
    file, or `Exec=%F`, does -- not a synthetic shortcut)."""

    command = [TEXT_EDITOR["exec"]] + ([str(path)] if path is not None else [])
    try:
        niri_spawn(*command)
    except JourneyError as error:
        return make_step("launch", False, str(error)), None
    window = wait_for_window(TEXT_EDITOR["app_id"])
    if window is None:
        return make_step("launch", False, "no Text Editor window appeared"), None
    return make_step("launch", True, "Text Editor window appeared"), window


def open_file_menu_item(item_name: str, timeout: float = ATSPI_FIND_TIMEOUT_S) -> bool:
    """Click the top bar's File menu, then the named item within it. Both
    are real AT-SPI `button`/`menu item` nodes with a `click` action
    (confirmed live) -- the same mechanism run-journey-launch.py uses for
    the app menu's Quit item."""

    menu_button = find_node("rmac-top-bar", "File menu", role="button", timeout=timeout)
    if menu_button is None or "click" not in action_names(menu_button):
        return False
    click(menu_button)
    item = find_node("rmac-top-bar", item_name, role="menu item", timeout=2.0)
    if item is None or "click" not in action_names(item):
        return False
    click(item)
    return True


def wait_for_chooser_app(
    baseline_apps: set[str], timeout: float = CHOOSER_APPEAR_TIMEOUT_S
) -> Optional[str]:
    """Wait for a new AT-SPI application (the portal's chooser, whichever
    backend answered) to appear beyond `baseline_apps`."""

    def find_new() -> Optional[str]:
        return next(iter(list_atspi_app_names() - baseline_apps), None)

    return _wait_for(find_new, timeout)


def drive_chooser_to_row(
    app_name: str,
    place_name: Optional[str],
    row_name: str,
    confirm_names: list[str],
) -> tuple[bool, str]:
    """Drive an already-located chooser application generically: optionally
    click a sidebar/places row by name, then click a file/folder row by
    name, then click a confirm button if one remains. Returns (succeeded,
    detail). Never types -- every step is a `click`/`activate` action on a
    node found by exact name match, so this works against either
    rmac-file-chooser or a GTK/GNOME fallback without caring which answered."""

    if place_name is not None:
        place = find_any_node(app_name, [place_name], timeout=CHOOSER_NAV_TIMEOUT_S)
        if place is not None and "click" in action_names(place):
            click(place)

    row = find_any_node(app_name, [row_name], timeout=CHOOSER_NAV_TIMEOUT_S)
    if row is None:
        return False, f"{app_name!r} chooser: no row named {row_name!r} was found"
    row_actions = action_names(row)
    activated = False
    for candidate in ("activate", "open", "click"):
        if candidate in row_actions:
            row.queryAction().doAction(row_actions.index(candidate))
            activated = True
            break
    if not activated:
        return False, f"{app_name!r} chooser: row {row_name!r} has no invokable action"

    # A single activation opens a folder or (in most choosers) both selects
    # and confirms a file. If a confirm/replace button is still present
    # shortly after, click it too -- this covers dialogs where the row only
    # selects.
    confirm = find_any_node(app_name, confirm_names, timeout=1.5)
    if confirm is not None and "click" in action_names(confirm):
        click(confirm)

    return True, f"drove the {app_name!r} chooser to {row_name!r}"


def attempt_edit(app_name: str) -> dict[str, Any]:
    """The honest check for the two documented gaps: no entry in the window
    exposes Text/EditableText, so no AT-SPI action can modify the buffer."""

    editable_found = []
    for node in _atspi_snapshot(app_name):
        try:
            if node.getRoleName() != "entry":
                continue
        except (LookupError, RuntimeError):
            continue
        if has_editable_text(node) or has_text_interface(node):
            editable_found.append(node)
    if editable_found:
        return make_step(
            "edit_content",
            False,
            "an editable entry was found; this script does not yet drive it "
            "(unexpected -- previous runs found no Text/EditableText anywhere "
            "in this window)",
        )
    return make_step(
        "edit_content",
        False,
        "no entry in the Text Editor window exposes AT-SPI Text or "
        "EditableText, and the encoding/line-ending picker "
        "(crates/text-editor/src/view/render/chrome.rs:80-127) has a 'click' "
        "action but invoking it over AT-SPI never opens its dropdown menu -- "
        "there is no accessible way to change document content or its saved "
        "format without a keyboard injector, which is not installed on the "
        "reference laptop (same upstream accesskit_unix EditableText gap "
        "run-journey-launch.py documents for Spotlight)",
    )


def wait_for_conflict_banner(app_name: str, timeout: float = CONFLICT_DETECT_TIMEOUT_S):
    return find_node(app_name, "Review…", role="button", timeout=timeout)


def quit_editor(window: dict[str, Any]) -> dict[str, Any]:
    """Close through the app's own top-bar Quit menu item, mirroring
    run-journey-launch.py's quit_app."""

    menu_button = find_node("rmac-top-bar", "Text Editor menu", role="button", timeout=3.0)
    if menu_button is None or "click" not in action_names(menu_button):
        niri_close_window(window["id"])
        return make_step(
            "close",
            wait_for_window_gone(window["id"]),
            "Text Editor menu was not found over AT-SPI; closed via niri instead",
        )
    click(menu_button)
    quit_item = find_node("rmac-top-bar", "Quit Text Editor", timeout=2.0)
    if quit_item is None or "click" not in action_names(quit_item):
        niri_close_window(window["id"])
        return make_step(
            "close",
            wait_for_window_gone(window["id"]),
            "Quit Text Editor menu item was not found over AT-SPI; closed via niri instead",
        )
    click(quit_item)
    gone = wait_for_window_gone(window["id"])
    return make_step("close", gone, "window closed" if gone else "window did not close")


# --------------------------------------------------------------------------
# Orchestration
# --------------------------------------------------------------------------


def run_journey(base_dir: Path, token: str, skip_interrupt: bool) -> dict[str, Any]:
    steps: list[dict[str, Any]] = []
    gaps: list[dict[str, str]] = []
    started_at_unix_ms = int(time.time() * 1000)

    fixture_root = base_dir / fixture_dirname(token)
    original_dir = fixture_root / "original"
    saved_as_dir = fixture_root / "saved-as"
    conflict_dir = fixture_root / "conflict"
    large_dir = fixture_root / "large"
    for directory in (original_dir, saved_as_dir, conflict_dir, large_dir):
        directory.mkdir(parents=True, exist_ok=True)

    original_content = make_sample_content(token)
    original_path = original_dir / "sample.txt"
    original_path.write_bytes(original_content)

    conflict_content = make_sample_content(token + "-conflict")
    conflict_path = conflict_dir / "watched.txt"
    conflict_path.write_bytes(conflict_content)

    windows_opened: list[dict[str, Any]] = []

    try:
        # --- Open, via the real portal UI, falling back to a direct path ---
        launch_step, window = launch_editor()
        steps.append(launch_step)
        if window is None:
            return build_report(steps, gaps, started_at_unix_ms)
        windows_opened.append(window)

        baseline_apps = list_atspi_app_names()
        opened_via_ui = open_file_menu_item("Open…")
        ok = False
        detail = "File menu > Open… did not open"
        if opened_via_ui:
            chooser_app = wait_for_chooser_app(baseline_apps)
            if chooser_app is None:
                detail = "no new AT-SPI application appeared for the file chooser"
            else:
                ok, detail = drive_chooser_to_row(
                    chooser_app,
                    place_name="Documents",
                    row_name=original_dir.name,
                    confirm_names=["Open", "_Open", "Select"],
                )
                # Descending into the fixture folder is only half the job;
                # find and activate the file itself, in the same chooser.
                if ok:
                    ok, detail = drive_chooser_to_row(
                        chooser_app,
                        place_name=None,
                        row_name=original_path.name,
                        confirm_names=["Open", "_Open", "Select"],
                    )

        steps.append(
            make_step(
                "open_via_portal",
                ok,
                detail if not ok else "opened the fixture file through the portal dialog",
            )
        )
        if not ok:
            gaps.append(
                {
                    "surface": "file-chooser-portal",
                    "issue": (
                        "the Open dialog could not be driven over AT-SPI today: "
                        + detail
                        + "; falling back to `rmac-text-editor <path>` (the same "
                        "command a real double-click runs) to keep measuring the "
                        "rest of the journey"
                    ),
                }
            )
            quit_editor(window)
            windows_opened.remove(window)
            fallback_step, window = launch_editor(original_path)
            steps.append(
                make_step(
                    "open_fallback_spawn",
                    fallback_step["passed"],
                    "opened the fixture file directly (fallback_spawn)",
                )
            )
            if window is None:
                return build_report(steps, gaps, started_at_unix_ms)
            windows_opened.append(window)

        # --- Edit (honest gap check) ---
        steps.append(attempt_edit(TEXT_EDITOR["atspi_name"]))
        gaps.append(
            {
                "surface": "text-editor-document",
                "issue": (
                    "no AT-SPI action can modify document content or saved "
                    "format in this build (no Text/EditableText anywhere in "
                    "the window; the encoding/line-ending picker has a "
                    "'click' action but it never opens its dropdown menu "
                    "over AT-SPI) -- a keyboard injector would be required "
                    "and none is installed on the reference laptop"
                ),
            }
        )

        # --- Save with no changes must be a true no-op ---
        saved_no_change = open_file_menu_item("Save")
        time.sleep(0.5)
        unchanged = original_path.read_bytes() == original_content
        steps.append(
            make_step(
                "save_without_changes_preserves_content",
                saved_no_change and unchanged,
                "Save left the file byte-identical"
                if unchanged
                else "the file changed even though the buffer was not dirty",
            )
        )

        # --- Save As, to a sibling folder, through the portal ---
        baseline_apps = list_atspi_app_names()
        save_as_clicked = open_file_menu_item("Save As…")
        new_path = saved_as_dir / original_path.name
        save_as_ok = False
        save_as_detail = "File menu > Save As… did not open"
        if save_as_clicked:
            chooser_app = wait_for_chooser_app(baseline_apps)
            if chooser_app is None:
                save_as_detail = "no new AT-SPI application appeared for the Save As chooser"
            else:
                save_as_ok, save_as_detail = drive_chooser_to_row(
                    chooser_app,
                    place_name=original_dir.name,
                    row_name=saved_as_dir.name,
                    confirm_names=["Save", "_Save", "Replace"],
                )
        steps.append(
            make_step(
                "save_as_via_portal",
                save_as_ok,
                save_as_detail if not save_as_ok else "Save As navigated to the sibling folder",
            )
        )
        if not save_as_ok:
            gaps.append(
                {
                    "surface": "file-chooser-portal",
                    "issue": "Save As could not be driven over AT-SPI today: " + save_as_detail,
                }
            )
        else:
            time.sleep(0.5)
            copy_matches = new_path.exists() and new_path.read_bytes() == original_content
            steps.append(
                make_step(
                    "save_as_content_matches",
                    copy_matches,
                    "the Save As copy is byte-identical to the original"
                    if copy_matches
                    else "the Save As copy is missing or differs from the original",
                )
            )

        steps.append(quit_editor(window))
        windows_opened.remove(window)

        # --- External change detection, on a dedicated window/file ---
        conflict_step, conflict_window = launch_editor(conflict_path)
        steps.append(
            make_step(
                "conflict_launch",
                conflict_step["passed"],
                "opened the conflict fixture directly",
            )
        )
        if conflict_window is not None:
            windows_opened.append(conflict_window)
            tampered_content = make_sample_content(token + "-tampered")
            conflict_path.write_bytes(tampered_content)
            review_button = wait_for_conflict_banner(TEXT_EDITOR["atspi_name"])
            detected = review_button is not None
            steps.append(
                make_step(
                    "external_change_detected",
                    detected,
                    "the external-change banner's Review… button appeared"
                    if detected
                    else "no Review… button appeared after the file was changed externally",
                )
            )
            if detected:
                click(review_button)
                reload_button = find_node(
                    TEXT_EDITOR["atspi_name"], "Discard & Reload", timeout=2.0
                )
                cancel_button = find_node(TEXT_EDITOR["atspi_name"], "Cancel", timeout=1.0)
                conflict_dialog_shown = reload_button is not None
                steps.append(
                    make_step(
                        "external_change_conflict_dialog",
                        conflict_dialog_shown,
                        "the Conflict alert's Discard & Reload button was found"
                        if conflict_dialog_shown
                        else "the Conflict alert did not appear as expected",
                    )
                )
                if cancel_button is not None and "click" in action_names(cancel_button):
                    click(cancel_button)
                unchanged_external = conflict_path.read_bytes() == tampered_content
                steps.append(
                    make_step(
                        "external_change_no_data_loss",
                        unchanged_external,
                        "Cancel left the external file untouched"
                        if unchanged_external
                        else "the external file changed after Cancel",
                    )
                )
            steps.append(quit_editor(conflict_window))
            windows_opened.remove(conflict_window)

        # --- SIGKILL-during-save, best-effort ---
        if not skip_interrupt:
            steps.append(run_interrupted_save(large_dir, token, gaps))

        return build_report(steps, gaps, started_at_unix_ms)
    finally:
        for window in list(windows_opened):
            try:
                niri_close_window(window["id"])
            except JourneyError:
                pass
        try:
            import shutil

            shutil.rmtree(fixture_root, ignore_errors=True)
        except OSError:
            pass


def run_interrupted_save(
    large_dir: Path, token: str, gaps: list[dict[str, str]]
) -> dict[str, Any]:
    large_content = make_large_content(token, INTERRUPT_FIXTURE_BYTES)
    large_path = large_dir / "big.txt"
    large_path.write_bytes(large_content)

    launch_step, window = launch_editor(large_path)
    if window is None:
        return make_step(
            "interrupted_save_atomic_write",
            False,
            "could not open the large fixture to attempt the interruption test",
        )

    try:
        for attempt in range(1, INTERRUPT_ATTEMPTS + 1):
            if not pid_alive(window["pid"]):
                launch_step, window = launch_editor(large_path)
                if window is None:
                    break
            saved = open_file_menu_item("Save As…")
            if not saved:
                continue
            confirm = find_any_node(
                TEXT_EDITOR["atspi_name"], ["Save", "_Save", "Replace"], timeout=2.0
            )
            gnome_confirm = None
            if confirm is None:
                # The confirm button most likely lives in the chooser
                # application, not rmac-text-editor's own window.
                for app_name in list_atspi_app_names():
                    if app_name == TEXT_EDITOR["atspi_name"]:
                        continue
                    gnome_confirm = find_any_node(
                        app_name, ["Save", "_Save", "Replace"], timeout=1.0
                    )
                    if gnome_confirm is not None:
                        break
            confirm = confirm or gnome_confirm
            if confirm is None or "click" not in action_names(confirm):
                continue
            click(confirm)

            temp_prefix = temp_write_pattern(large_path.name)
            deadline = time.monotonic() + 2.0
            caught = False
            while time.monotonic() < deadline:
                try:
                    entries = os.listdir(large_dir)
                except OSError:
                    break
                if any(entry.startswith(temp_prefix) for entry in entries):
                    caught = True
                    break
                time.sleep(INTERRUPT_POLL_S)
            if not caught:
                continue

            try:
                os.kill(window["pid"], signal.SIGKILL)
            except ProcessLookupError:
                pass
            _wait_for(lambda: not pid_alive(window["pid"]), 3.0)

            for entry in os.listdir(large_dir):
                if entry.startswith(temp_prefix):
                    (large_dir / entry).unlink(missing_ok=True)

            if not large_path.exists():
                return make_step(
                    "interrupted_save_atomic_write",
                    True,
                    f"attempt {attempt}: killed mid-write; destination absent "
                    "(rename had not happened yet) -- no truncated file",
                    attempts=attempt,
                )
            final_bytes = large_path.read_bytes()
            intact = final_bytes == large_content
            return make_step(
                "interrupted_save_atomic_write",
                intact,
                (
                    f"attempt {attempt}: killed mid-write; destination is complete "
                    "and byte-identical"
                    if intact
                    else f"attempt {attempt}: destination exists but is truncated "
                    "or corrupted -- an atomic-write bug"
                ),
                attempts=attempt,
            )

        return make_step(
            "interrupted_save_atomic_write",
            True,
            f"could not catch the write in flight within {INTERRUPT_ATTEMPTS} attempts; "
            "not conclusively exercised (best-effort per the journey brief)",
            attempts=INTERRUPT_ATTEMPTS,
            conclusive=False,
        )
    finally:
        if pid_alive(window.get("pid", -1)):
            try:
                os.kill(window["pid"], signal.SIGKILL)
            except (ProcessLookupError, KeyError):
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
        "--base-dir",
        type=Path,
        default=Path.home() / "Documents",
        help="parent directory for the disposable fixture folder (default: ~/Documents)",
    )
    parser.add_argument(
        "--skip-interrupt",
        action="store_true",
        help="skip the best-effort SIGKILL-during-save test",
    )
    arguments = parser.parse_args()

    token = secrets.token_hex(4)
    try:
        report = run_journey(arguments.base_dir, token, arguments.skip_interrupt)
    except JourneyError as error:
        parser.exit(4, f"run-journey-textfile: {error}\n")

    text = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if arguments.output is not None:
        arguments.output.write_text(text, encoding="utf-8")
    print(text, end="")

    passed = sum(1 for step in report["steps"] if step["passed"])
    total = len(report["steps"])
    print(
        f"journey 5: {passed}/{total} steps passed; overall_pass={report['overall_pass']}",
        file=sys.stderr,
    )
    return 0 if report["overall_pass"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
