#!/usr/bin/env python3
"""Acceptance test for product journey 6 (todo.md "Product journeys"):

    Inspect resource use and safely stop a process, with confirmation.

Runs against the live rmac session on the reference laptop (niri + AT-SPI),
driving `rmac-system-monitor` (crates/activity-monitor, binary name
rmac-system-monitor, AT-SPI application name confirmed live as
"rmac-system-monitor"). Like run-journey-launch.py and
run-journey-textfile.py, every step is a real AT-SPI action or an honestly
reported gap -- this script never injects a keystroke or a synthetic pointer
event (none is installed on the reference laptop: no wtype, no ydotool, and
this script does not use pyatspi's `generateMouseEvent` either, since niri
implements no virtual-pointer protocol for it to reach).

Live introspection (dumping the real AT-SPI tree of a running
rmac-system-monitor, see the commit that added this script for the session
transcript) found a safety-relevant gap that changes what this script can
respect­fully do:

  * The process table renders no per-row AT-SPI structure at all -- no
    "table row"/"list item" node exists for any process, so nothing (a
    screen reader, a switch device, or this script) can browse, identify, or
    select one *specific* process over AT-SPI. This is despite
    `crates/activity-monitor/src/accessibility.rs` already defining a
    complete, unit-tested pure model for exactly this
    (`project_process_table`, `AccessibleProcessRow { actions: ["Inspect",
    "Quit", "Force Quit"], .. }`, `project_process_action_dialog`) -- a
    repository-wide search shows it is referenced only by its own
    `#[cfg(test)]` module and is never called from `view.rs` or
    `process_table.rs`. It is complete, correct, dead code.
  * The search field (`crates/activity-monitor/src/view.rs:59`, the same
    `SearchField`/`InputState` widget Text Editor's document buffer uses)
    exposes AT-SPI's Accessible and Component interfaces only -- no Text, no
    EditableText (confirmed live) -- so it cannot be used to filter to a
    specific process by typing either, even with a keyboard.
  * Quit/Force Quit are *meant* to exist over AT-SPI as plain buttons named
    "Quit"/"Force Quit" mirroring the `QuitProcess`/`ForceQuitProcess`
    keyboard actions (`crates/activity-monitor/src/main.rs:16-24,42-51`),
    and as the top bar's global "Process" menu once System Monitor is
    focused (`crates/rmac-app-menu/src/lib.rs:236-244`: "Quit Process…" /
    "Force Quit Process…", both real `menu item` nodes) -- one live run did
    find both toolbar buttons present with a `click` action; a later run
    against the same desktop-entry-resolved binary found the window's whole
    AT-SPI tree collapsed to its 3 (unlabelled) title-bar buttons only, with
    no toolbar, tabs, search field, or Quit/Force Quit at all (see
    `check_quit_controls_exist`'s reported detail for what a given run
    actually found -- this script never assumes either shape). Whichever
    shape is present, both Quit routes act on whatever `selected_pid` a
    **mouse** click (left or right) on a row
    (`crates/activity-monitor/src/process_table.rs:343-365`, both
    `MouseButton::Left` and `MouseButton::Right`) or **keyboard** table navigation
    (`crates/activity-monitor/src/view.rs:165-176`) last set -- there is no
    AT-SPI action that sets it. Since this laptop's session is shared with
    other automated agents and possibly a person, this script cannot even
    safely assume "nothing is selected": another actor could have a row
    highlighted right now. Blindly invoking Quit/Force Quit could therefore
    signal a process this script does not own, which the brief for this
    journey explicitly forbids ("Never touch any other process").

Given that, this script:

  1. Starts one harmless, disposable process it owns (`sleep 600
     <unique-marker>`), with a marker recorded only in the process's own
     argv, never printed into the report.
  2. Launches System Monitor and verifies, structurally and non-
     destructively (reading node names/roles/interfaces only -- no clicks
     that could act on an unknown selection), whether a specific process can
     be identified over AT-SPI at all: by a per-row node carrying the label
     `crates/activity-monitor/src/accessibility.rs` already defines
     (`"{name} (PID {pid})"`), or by typing into the search field.
  3. If (and only if) a row for its own disposable process can be safely
     identified this way, it proceeds with the full journey: select it,
     invoke Quit through the top bar's Process menu, confirm the dialog
     appears, Cancel once (assert the process survives), invoke Quit again,
     confirm (assert the process exits). This is the forward-compatible
     path: if a future build wires `accessibility.rs`'s model into the live
     table (closing the gap above), this script starts passing the real
     journey without modification.
  4. Otherwise -- the current reality -- it stops short of touching Quit/
     Force Quit at all, reports the two gaps above precisely, and still
     verifies what it safely can: the disposable process launches and stays
     alive, System Monitor launches, and both the AT-SPI buttons and the top
     bar's Process menu items exist (a structural check, not an invocation).

Cleanup always terminates the disposable process directly (`os.kill`, never
through the UI it just finished testing) and closes System Monitor through
its own top-bar Quit menu item.

The report is privacy-safe: no screenshots, no process command lines, no
usernames; only the PID this script itself created.
"""

from __future__ import annotations

import argparse
import json
import os
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
    """A bounded, privacy-safe journey-monitor failure."""


FORMAT = 1
JOURNEY_ID = 6
JOURNEY_TITLE = "Inspect resource use and safely stop a process, with confirmation."

NIRI_TIMEOUT_S = 5.0
WINDOW_APPEAR_TIMEOUT_S = 5.0
CLOSE_TIMEOUT_S = 5.0
ATSPI_FIND_TIMEOUT_S = 5.0
ROW_SEARCH_TIMEOUT_S = 4.0
CONFIRM_TIMEOUT_S = 3.0
TERMINATE_TIMEOUT_S = 5.0
POLL_INTERVAL_S = 0.05

SYSTEM_MONITOR: dict[str, str] = {
    "display_name": "System Monitor",
    "app_id": "org.rmac.SystemMonitor",
    # Launched by desktop id, never a hard-coded binary path: on the
    # reference laptop /usr/bin/rmac-* are stale packaged binaries, while the
    # user's actual desktop entries
    # (~/.local/share/applications/org.rmac.*.desktop, XDG lookup order puts
    # this ahead of /usr/share/applications) point at the current dev build
    # via ~/.local/libexec/rmac -> ~/rmac-dev-bin. `gtk-launch` resolves and
    # runs a desktop entry's Exec= exactly the way a real launch (Dock,
    # Spotlight, a file manager) would.
    "desktop_id": "org.rmac.SystemMonitor",
    "atspi_name": "rmac-system-monitor",
}

DISPOSABLE_COMMAND_NAME = "sleep"
DISPOSABLE_DURATION_S = "600"


# --------------------------------------------------------------------------
# Pure helpers (unit-tested from scripts/test_journey_monitor.py on macOS)
# --------------------------------------------------------------------------


def disposable_marker(token: str) -> str:
    if not token or any(character.isspace() for character in token):
        raise JourneyError("invalid disposable-process token")
    return f"lulo-journey-6-{token}"


def build_disposable_command(token: str) -> list[str]:
    """The exact argv this script spawns for its own harmless process --
    kept pure so the marker convention is unit-testable without spawning
    anything.

    GNU `sleep` treats every argument as a duration and sums them
    (`sleep 600 marker` fails outright with "invalid time interval");
    the marker is instead carried as argv[0] via the shell's `exec -a`,
    which replaces the shell with `sleep` in the same PID and leaves
    `sleep 600` as the only real duration argument."""

    return [
        "bash",
        "-c",
        f"exec -a {disposable_marker(token)} {DISPOSABLE_COMMAND_NAME} {DISPOSABLE_DURATION_S}",
    ]


def expected_row_label(name: str, pid: int) -> str:
    """Mirrors `crates/activity-monitor/src/accessibility.rs`'s
    `AccessibleProcessRow.label` format (`"{name} (PID {pid})"`) exactly, so
    this script recognizes a row the moment the live view starts projecting
    that pure model."""

    return f"{name} (PID {pid})"


def expected_dialog_title(force: bool) -> str:
    """Mirrors the exact strings rendered by
    `crates/activity-monitor/src/view/render/overlays.rs`'s
    `render_confirm`."""

    return "Force Quit Process" if force else "Are you sure you want to quit this process?"


def expected_confirm_button_label(force: bool) -> str:
    return "Force Quit" if force else "Quit"


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
# AT-SPI helpers (mirrors run-journey-launch.py / run-journey-textfile.py)
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
                matches = name.startswith(node_name) if name_prefix else name == node_name
                if not matches:
                    continue
                if role is not None and node.getRoleName() != role:
                    continue
            except (LookupError, RuntimeError):
                continue
            return node
        return None

    return _wait_for(search, timeout)


def is_selected(node) -> bool:
    """True if the AT-SPI node's states include SELECTED -- reflects
    `.aria_selected(..)` on the process row (crates/activity-monitor/src/
    process_table.rs:466-468)."""

    try:
        return bool(node.getState().contains(pyatspi.STATE_SELECTED))
    except (LookupError, RuntimeError, AttributeError):
        return False


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


# --------------------------------------------------------------------------
# Journey steps
# --------------------------------------------------------------------------


def launch_monitor() -> tuple[dict[str, Any], Optional[dict[str, Any]]]:
    try:
        niri_spawn("gtk-launch", SYSTEM_MONITOR["desktop_id"])
    except JourneyError as error:
        return make_step("launch", False, str(error)), None
    window = wait_for_window(SYSTEM_MONITOR["app_id"])
    if window is None:
        return make_step("launch", False, "no System Monitor window appeared"), None
    return make_step("launch", True, "System Monitor window appeared"), window


def check_search_field_editable() -> dict[str, Any]:
    for node in _atspi_snapshot(SYSTEM_MONITOR["atspi_name"]):
        try:
            if node.getRoleName() != "entry":
                continue
        except (LookupError, RuntimeError):
            continue
        if has_editable_text(node):
            return make_step(
                "search_field_editable",
                True,
                "the search field exposes AT-SPI EditableText",
            )
    return make_step(
        "search_field_editable",
        False,
        "the search field (crates/activity-monitor/src/view.rs:59) exposes no "
        "AT-SPI EditableText; a process cannot be located by typing without a "
        "keyboard injector",
    )


def check_quit_controls_exist() -> dict[str, Any]:
    # rmac-top-bar's AT-SPI tree carries one frame per workspace/output, most
    # of them empty placeholders on a laptop this many agents have been
    # exercising concurrently; a full traversal to find "Process menu" can
    # take noticeably longer than the in-window lookups below, so it gets a
    # more generous timeout rather than being (mis)reported as absent.
    #
    # The toolbar's Quit control is now a real, named AT-SPI button --
    # "Quit Process" -- via an outer accessible wrapper
    # (crates/activity-monitor/src/view/render/chrome.rs:44-63
    # accessible_icon_button); there is no equivalent in-window "Force
    # Quit" icon (Force Quit is reachable only via the top bar's Process
    # menu, or the confirmation dialog's own escalation button), so this
    # checks the menu item instead of a nonexistent toolbar button.
    quit_button = find_node(
        SYSTEM_MONITOR["atspi_name"], "Quit Process", role="button", timeout=2.0
    )
    menu_button = find_node("rmac-top-bar", "Process menu", role="button", timeout=8.0)
    force_quit_item = None
    if menu_button is not None and "click" in action_names(menu_button):
        click(menu_button)
        force_quit_item = find_node(
            "rmac-top-bar", "Force Quit Process…", role="menu item", timeout=2.0
        )
        # Close the menu again without invoking anything.
        click(menu_button)
    found = bool(quit_button and menu_button and force_quit_item)
    missing = [
        label
        for label, node in (
            ("in-window 'Quit Process' button", quit_button),
            ("top bar Process menu", menu_button),
            ("'Force Quit Process…' menu item", force_quit_item),
        )
        if node is None
    ]
    return make_step(
        "quit_controls_exist",
        found,
        "the 'Quit Process' button, the top bar's Process menu, and its "
        "'Force Quit Process…' item all exist over AT-SPI"
        if found
        else f"not found over AT-SPI: {', '.join(missing)}",
    )


def find_disposable_row(pid: int, timeout: float = ROW_SEARCH_TIMEOUT_S):
    """Looks for a "table row" whose accessible name starts with the exact
    label `accessibility.rs::AccessibleProcessRow.label` defines
    (`process_table.rs::render_tr` appends ", {cpu}% CPU, {mem}" after it,
    so this matches by prefix rather than exact equality). Returns the
    node, or None -- never guesses by process name alone, since more than
    one process can share a name and this script must only ever touch its
    own."""

    label = expected_row_label(DISPOSABLE_COMMAND_NAME, pid)
    return find_node(
        SYSTEM_MONITOR["atspi_name"],
        label,
        role="table row",
        timeout=timeout,
        name_prefix=True,
    )


def row_name_matches_pid(name: str, pid: int) -> bool:
    """True if a process row's accessible name carries exactly this PID --
    the safety check this script runs immediately before every Quit/Force
    Quit invocation, so it never signals a process it did not start."""

    return expected_row_label(DISPOSABLE_COMMAND_NAME, pid) in name


def request_process_action(force: bool) -> bool:
    menu_button = find_node("rmac-top-bar", "Process menu", role="button", timeout=2.0)
    if menu_button is None or "click" not in action_names(menu_button):
        return False
    click(menu_button)
    item_name = "Force Quit Process…" if force else "Quit Process…"
    item = find_node("rmac-top-bar", item_name, role="menu item", timeout=2.0)
    if item is None or "click" not in action_names(item):
        return False
    click(item)
    return True


def quit_monitor(window: dict[str, Any]) -> dict[str, Any]:
    menu_button = find_node("rmac-top-bar", "System Monitor menu", role="button", timeout=3.0)
    if menu_button is None or "click" not in action_names(menu_button):
        niri_close_window(window["id"])
        return make_step(
            "close",
            wait_for_window_gone(window["id"]),
            "System Monitor menu was not found over AT-SPI; closed via niri instead",
        )
    click(menu_button)
    quit_item = find_node("rmac-top-bar", "Quit System Monitor", timeout=2.0)
    if quit_item is None or "click" not in action_names(quit_item):
        niri_close_window(window["id"])
        return make_step(
            "close",
            wait_for_window_gone(window["id"]),
            "Quit System Monitor menu item was not found over AT-SPI; closed via niri instead",
        )
    click(quit_item)
    gone = wait_for_window_gone(window["id"])
    return make_step("close", gone, "window closed" if gone else "window did not close")


# --------------------------------------------------------------------------
# Orchestration
# --------------------------------------------------------------------------


def run_journey(token: str) -> dict[str, Any]:
    steps: list[dict[str, Any]] = []
    gaps: list[dict[str, str]] = []
    started_at_unix_ms = int(time.time() * 1000)

    disposable: Optional[subprocess.Popen[bytes]] = None
    window: Optional[dict[str, Any]] = None

    try:
        command = build_disposable_command(token)
        disposable = subprocess.Popen(
            command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        alive = pid_alive(disposable.pid)
        steps.append(
            make_step(
                "disposable_process_started",
                alive,
                "the disposable sleep process is running"
                if alive
                else "the disposable sleep process did not start",
            )
        )
        if not alive:
            return build_report(steps, gaps, started_at_unix_ms)

        launch_step, window = launch_monitor()
        steps.append(launch_step)
        if window is None:
            return build_report(steps, gaps, started_at_unix_ms)

        steps.append(check_search_field_editable())
        gaps.append(
            {
                "surface": "system-monitor-search",
                "issue": (
                    "the search field exposes no AT-SPI EditableText, so a "
                    "process cannot be located by typing without a keyboard "
                    "injector (same accesskit_unix gap documented for "
                    "Spotlight and Text Editor's document buffer)"
                ),
            }
        )

        steps.append(check_quit_controls_exist())

        row = find_disposable_row(disposable.pid)
        row_found = row is not None
        steps.append(
            make_step(
                "process_row_identifiable",
                row_found,
                "found a row for the disposable process over AT-SPI"
                if row_found
                else (
                    "the process table exposes no per-row AT-SPI structure at all "
                    "(crates/activity-monitor/src/process_table.rs) -- "
                    "crates/activity-monitor/src/accessibility.rs already defines "
                    "the row/label/action model this needs "
                    "(project_process_table, AccessibleProcessRow) but it is "
                    "never called outside its own tests, so no assistive "
                    "technology or this script can select one specific process"
                ),
            )
        )

        if not row_found:
            gaps.append(
                {
                    "surface": "system-monitor-process-table",
                    "issue": (
                        "Quit/Force Quit act on whatever a mouse click (left or right) or "
                        "keyboard table-navigation last selected "
                        "(crates/activity-monitor/src/process_table.rs:352-382, "
                        "crates/activity-monitor/src/view.rs:165-176); with no "
                        "row exposed over AT-SPI and no way to type into the "
                        "search field, this script cannot safely select its own "
                        "disposable process, so it does not invoke Quit or Force "
                        "Quit at all -- doing so blind, on a laptop whose session "
                        "is shared with other agents, could act on an unrelated "
                        "process. The confirmation/cancel/confirm-and-terminate "
                        "steps of this journey are blocked by this gap, not "
                        "attempted, and reported failed rather than faked."
                    ),
                }
            )
            steps.append(
                make_step(
                    "quit_confirmation_cancel_then_confirm",
                    False,
                    "not attempted: no safe, AT-SPI-verified way to select the "
                    "disposable process (see process_row_identifiable and "
                    "system-monitor-process-table gap above)",
                )
            )
            return build_report(steps, gaps, started_at_unix_ms)

        # Forward-compatible real path: a future build that wires
        # accessibility.rs's model into the live table will reach here and
        # this script will exercise the actual destructive journey.
        selected = "click" in action_names(row) and click(row)
        steps.append(
            make_step(
                "select_disposable_process",
                selected,
                "selected the disposable process row via its AT-SPI Click "
                "action"
                if selected
                else "found the row but could not select it",
            )
        )
        if not selected:
            return build_report(steps, gaps, started_at_unix_ms)

        # Re-find the row and confirm both that it now reports itself
        # selected (aria_selected -> AT-SPI STATE_SELECTED) and that its
        # name still carries this exact PID, before this script ever
        # invokes Quit -- "Never touch any other process" per the journey
        # brief.
        reselected_row = find_disposable_row(disposable.pid, timeout=CONFIRM_TIMEOUT_S)
        selection_confirmed = reselected_row is not None and is_selected(reselected_row)
        pid_confirmed = reselected_row is not None and row_name_matches_pid(
            reselected_row.name, disposable.pid
        )
        steps.append(
            make_step(
                "selection_verified",
                selection_confirmed and pid_confirmed,
                "the selected row reports STATE_SELECTED and its name still "
                "carries this run's own PID"
                if selection_confirmed and pid_confirmed
                else f"selection_confirmed={selection_confirmed} "
                f"pid_confirmed={pid_confirmed}; refusing to proceed to Quit "
                "without both",
            )
        )
        if not (selection_confirmed and pid_confirmed):
            return build_report(steps, gaps, started_at_unix_ms)

        requested = request_process_action(force=False)
        confirm_button = find_node(
            SYSTEM_MONITOR["atspi_name"],
            expected_confirm_button_label(force=False),
            timeout=CONFIRM_TIMEOUT_S,
        )
        cancel_button = find_node(SYSTEM_MONITOR["atspi_name"], "Cancel", timeout=1.0)
        confirmation_shown = requested and confirm_button is not None and cancel_button is not None
        steps.append(
            make_step(
                "confirmation_appears",
                confirmation_shown,
                "the Quit confirmation dialog appeared"
                if confirmation_shown
                else "no confirmation dialog appeared after requesting Quit",
            )
        )
        if not confirmation_shown:
            return build_report(steps, gaps, started_at_unix_ms)

        click(cancel_button)
        survived_cancel = pid_alive(disposable.pid)
        steps.append(
            make_step(
                "cancel_preserves_process",
                survived_cancel,
                "the process survived Cancel"
                if survived_cancel
                else "the process was gone after Cancel",
            )
        )

        request_process_action(force=False)
        confirm_button = find_node(
            SYSTEM_MONITOR["atspi_name"],
            expected_confirm_button_label(force=False),
            timeout=CONFIRM_TIMEOUT_S,
        )
        # Last check before the destructive action: re-read the still-
        # selected row's name one more time and refuse to click Quit if it
        # no longer names this run's own PID.
        final_row = find_disposable_row(disposable.pid, timeout=1.0)
        pid_still_matches = final_row is not None and row_name_matches_pid(
            final_row.name, disposable.pid
        )
        if confirm_button is not None and "click" in action_names(confirm_button) and pid_still_matches:
            click(confirm_button)
        elif not pid_still_matches:
            steps.append(
                make_step(
                    "confirm_terminates_process",
                    False,
                    "refused to click Quit: the selected row no longer named "
                    "this run's own PID immediately before confirming",
                )
            )
            return build_report(steps, gaps, started_at_unix_ms)
        terminated = _wait_for(lambda: not pid_alive(disposable.pid), TERMINATE_TIMEOUT_S)
        steps.append(
            make_step(
                "confirm_terminates_process",
                bool(terminated),
                "the process exited after confirming Quit"
                if terminated
                else "the process was still running after confirming Quit",
            )
        )

        return build_report(steps, gaps, started_at_unix_ms)
    finally:
        if window is not None:
            try:
                quit_monitor(window)
            except JourneyError:
                pass
        if disposable is not None and pid_alive(disposable.pid):
            try:
                os.kill(disposable.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            try:
                disposable.wait(timeout=3.0)
            except subprocess.TimeoutExpired:
                pass


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output",
        type=Path,
        default=None,
        help="absolute path to write the JSON report to (also printed to stdout)",
    )
    arguments = parser.parse_args()

    import secrets

    token = secrets.token_hex(4)
    try:
        report = run_journey(token)
    except JourneyError as error:
        parser.exit(4, f"run-journey-monitor: {error}\n")

    text = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if arguments.output is not None:
        arguments.output.write_text(text, encoding="utf-8")
    print(text, end="")

    passed = sum(1 for step in report["steps"] if step["passed"])
    total = len(report["steps"])
    print(
        f"journey 6: {passed}/{total} steps passed; overall_pass={report['overall_pass']}",
        file=sys.stderr,
    )
    return 0 if report["overall_pass"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
