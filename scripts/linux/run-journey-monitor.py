#!/usr/bin/env python3
"""Acceptance test for product journey 6 (todo.md "Product journeys"):

    Inspect resource use and safely stop a process, with confirmation.

Runs against the live rmac session on the reference laptop (niri + AT-SPI),
the same way scripts/linux/run-journey-launch.py exercises journey 1: no
keyboard or pointer injector is installed there (no wtype, no ydotool), so
System Monitor (crates/activity-monitor, binary `rmac-system-monitor`) is
driven only through AT-SPI actions and niri IPC.

This script starts a single, harmless, disposable process it owns (`sleep
600`, identified throughout by the exact PID this script spawned -- never by
a fuzzy name match, so it is never possible to act on any other process),
then tries to find and stop it through System Monitor exactly as a real user
would: locate the row (search field or list), choose Quit or Force Quit, see
a confirmation, cancel once (the process must survive), confirm (the process
must end).

On the reference laptop as of this writing, that path is completely blocked,
and this script proves it precisely rather than assuming it or giving up
silently:

  * System Monitor's whole process list has **zero** AT-SPI semantic
    representation. A live dump of `rmac-system-monitor`'s AT-SPI tree at
    its default 960x640 size contains exactly 14 nodes: 1 application, 1
    frame, 11 chrome buttons, and 1 (unlabelled) search entry -- no table, no
    row, no cell, for any process, ever. `crates/activity-monitor/src/
    accessibility.rs` defines exactly the right projection for this
    (`project_process_table`, `ProcessTableAccessibilitySnapshot`,
    `project_process_action_dialog` -- accessibility.rs:149,241) but grep
    confirms zero call sites for any of it outside its own unit tests: the
    live table (`crates/activity-monitor/src/process_table.rs`) renders each
    row as a plain `div()` with no AccessKit wiring
    (process_table.rs:363-389), so none of it reaches AT-SPI. No process can
    be found or selected by assistive technology on this build;
  * the search field, like every `InputState`-backed entry this script (and
    run-journey-textfile.py) has probed, exposes neither the AT-SPI Text nor
    EditableText interface (`queryText()`/`queryEditableText()` both raise),
    so a filter query cannot be typed either;
  * because no row can ever be selected, the toolbar's Quit/Force Quit
    controls and the exported "Process" menu's "Quit Process..."/"Force Quit
    Process..." items (crates/rmac-app-menu/src/lib.rs's MONITOR_MENUS) stay
    disabled for any process this script starts, and this script proves that
    too rather than assuming it.

Since selection is a hard prerequisite for everything the rest of the
journey needs (the confirmation dialog, Quit, Force Quit), there is no
separate real path left to fall back to the way run-journey-launch.py falls
back to a direct spawn for Dock/Spotlight -- there is nothing downstream of
"select a process" that a different accessible mechanism could still reach.
This script therefore fails honestly (todo.md: "an honest limitation beats
simulated system behaviour") rather than fabricate a synthetic pass, and
always cleans up its own marker process directly so no stray `sleep`
survives the run regardless of how the journey went.

The report is privacy-safe: no screenshots, no window titles, no absolute
paths, no other process's identity. Every wait in this script is bounded;
it never hangs.
"""

from __future__ import annotations

import argparse
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
JOURNEY_ID = 6
JOURNEY_TITLE = "Inspect resource use and safely stop a process, with confirmation."

NIRI_TIMEOUT_S = 5.0
WINDOW_APPEAR_TIMEOUT_S = 5.0
ATSPI_FIND_TIMEOUT_S = 5.0
CONFIRM_TIMEOUT_S = 3.0
CLOSE_TIMEOUT_S = 5.0
PROCESS_SETTLE_TIMEOUT_S = 3.0
POLL_INTERVAL_S = 0.05

SYSTEM_MONITOR: dict[str, str] = {
    "display_name": "System Monitor",
    "app_id": "org.rmac.SystemMonitor",
    "exec": "/usr/bin/rmac-system-monitor",
}
ATSPI_APP_NAME = "rmac-system-monitor"

# crates/rmac-app-menu/src/lib.rs MONITOR_MENUS -- exact exported labels.
PROCESS_MENU_BUTTON = "Process menu"
QUIT_PROCESS_ITEM = "Quit Process…"
FORCE_QUIT_PROCESS_ITEM = "Force Quit Process…"


# --------------------------------------------------------------------------
# Pure helpers (unit-tested from scripts/test_journey_monitor.py on macOS)
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


def unique_marker() -> str:
    return f"lulo-journey-6-{secrets.token_hex(6)}"


def count_nodes_by_role(nodes: list[str]) -> dict[str, int]:
    """Pure summary helper: how many AT-SPI nodes of each role a dump found.
    Used both to build the gap evidence in this script and directly by its
    unit tests."""

    counts: dict[str, int] = {}
    for role in nodes:
        counts[role] = counts.get(role, 0) + 1
    return counts


def has_only_chrome(counts: dict[str, int], selectable_roles: tuple[str, ...]) -> bool:
    """True when a role census contains none of the roles a real process
    list would need (table/row/cell/list item/…)."""

    return not any(role in counts for role in selectable_roles)


SELECTABLE_ROLES = ("table", "table row", "table cell", "list item", "tree item")


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
    result = _niri("action", "spawn", "--", command)
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


def pid_alive(pid: int) -> bool:
    return Path(f"/proc/{pid}").exists()


def cmdline_of(pid: int) -> str:
    try:
        return Path(f"/proc/{pid}/cmdline").read_bytes().replace(b"\0", b" ").decode(
            "utf-8", "replace"
        )
    except OSError:
        return ""


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


def role_census(app_name: str) -> list[str]:
    roles: list[str] = []
    for node in _atspi_snapshot(app_name):
        try:
            roles.append(node.getRoleName())
        except (LookupError, RuntimeError):
            roles.append("<err>")
    return roles


def can_edit_text(node) -> tuple[bool, str]:
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


def spawn_marker_process(marker: str) -> int:
    """Start a disposable `sleep 600` this script (and only this script)
    owns, with argv[0] set to a unique marker so its identity can be
    double-checked before any signal is ever sent to it."""

    process = subprocess.Popen(
        ["sh", "-c", f'exec -a {marker} sleep 600'],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )
    return process.pid


def is_our_marker_process(pid: int, marker: str) -> bool:
    """Refuse to act unless /proc still shows *our* marker for this PID --
    the one safety check standing between this script and "never touch any
    other process" if a PID were ever reused."""

    if not pid_alive(pid):
        return False
    return marker in cmdline_of(pid)


def open_process_menu() -> dict[str, Any]:
    button = find_node("rmac-top-bar", PROCESS_MENU_BUTTON, role="button", timeout=3.0)
    if button is None or "click" not in action_names(button):
        return make_step(
            "open_process_menu",
            False,
            "System Monitor's Process menu was not found (or not clickable) over AT-SPI",
        )
    click(button)
    return make_step("open_process_menu", True, "opened the Process menu via AT-SPI")


# --------------------------------------------------------------------------
# Orchestration
# --------------------------------------------------------------------------


def run_journey(keep_open: bool) -> dict[str, Any]:
    steps: list[dict[str, Any]] = []
    gaps: list[dict[str, str]] = []
    started_at_unix_ms = int(time.time() * 1000)

    steps.append(check_logged_in())

    marker = unique_marker()
    marker_pid = spawn_marker_process(marker)
    time.sleep(0.2)
    spawned_ok = is_our_marker_process(marker_pid, marker)
    steps.append(
        make_step(
            "spawn_marker_process",
            spawned_ok,
            "started a disposable, uniquely-identified sleep process"
            if spawned_ok
            else "the marker process could not be confirmed alive after spawning",
        )
    )

    monitor_window: Optional[dict[str, Any]] = None
    try:
        if not spawned_ok:
            return build_report(steps, gaps, started_at_unix_ms)

        niri_spawn(SYSTEM_MONITOR["exec"])
        monitor_window = wait_for_window(SYSTEM_MONITOR["app_id"], WINDOW_APPEAR_TIMEOUT_S)
        steps.append(
            make_step(
                "launch_monitor",
                monitor_window is not None,
                "System Monitor window appeared"
                if monitor_window
                else "no System Monitor window appeared",
            )
        )
        if monitor_window is None:
            return build_report(steps, gaps, started_at_unix_ms)

        # --- Try the search field (real click; typing is expected to fail) -----------
        search_entry = find_node(ATSPI_APP_NAME, "", role="entry", timeout=2.0)
        can_type, type_evidence = (
            (False, "no search entry was found over AT-SPI")
            if search_entry is None
            else can_edit_text(search_entry)
        )
        steps.append(make_step("find_via_search", can_type, type_evidence))

        # --- Try the raw process list: does *any* row/cell/table node exist? ---------
        roles = role_census(ATSPI_APP_NAME)
        counts = count_nodes_by_role(roles)
        list_has_semantics = not has_only_chrome(counts, SELECTABLE_ROLES)
        steps.append(
            make_step(
                "find_via_list",
                list_has_semantics,
                "the process list exposes row/cell semantics over AT-SPI"
                if list_has_semantics
                else f"the process list has no selectable AT-SPI nodes at all (role census: {counts})",
                role_census=counts,
            )
        )

        if not can_type and not list_has_semantics:
            gaps.append(
                {
                    "surface": "system-monitor-process-list",
                    "issue": (
                        "System Monitor's process list has no AT-SPI semantic "
                        f"representation on this build (role census: {counts}; "
                        "accessibility.rs's row/dialog projections, "
                        "crates/activity-monitor/src/accessibility.rs:149,241, are never "
                        "called from the live table, crates/activity-monitor/src/"
                        "process_table.rs:363-389) and its search field cannot be typed "
                        f"into ({type_evidence}), so no process -- including this "
                        "script's own disposable one -- can be found or selected by "
                        "assistive technology"
                    ),
                }
            )

        steps.append(
            make_step(
                "select_process",
                False,
                "no accessible way exists to select a specific process row on this "
                "build; see find_via_search / find_via_list above",
            )
        )

        # --- Attempt the confirmation flow anyway, to observe real behaviour ----------
        menu_step = open_process_menu()
        steps.append(menu_step)
        confirm_dialog_appeared = False
        if menu_step["passed"]:
            item = find_node(ATSPI_APP_NAME, QUIT_PROCESS_ITEM, timeout=1.5)
            if item is not None and "click" in action_names(item):
                click(item)
                confirm_dialog_appeared = (
                    find_node(ATSPI_APP_NAME, "Cancel", role="button", timeout=CONFIRM_TIMEOUT_S)
                    is not None
                )
        steps.append(
            make_step(
                "quit_with_confirmation",
                False,
                "a confirmation dialog appeared even without a selected process "
                "(unexpected)"
                if confirm_dialog_appeared
                else "Quit Process… had no effect without a selected process "
                "(expected, given select_process above); journey 6 cannot be "
                "completed by assistive technology on this build",
            )
        )

        # However this went, our marker process must be untouched.
        steps.append(
            make_step(
                "marker_process_unaffected",
                is_our_marker_process(marker_pid, marker),
                "the disposable process is still running, exactly as a blocked "
                "quit attempt should leave it",
            )
        )

        return build_report(steps, gaps, started_at_unix_ms)
    finally:
        if is_our_marker_process(marker_pid, marker):
            try:
                os.kill(marker_pid, 15)
            except OSError:
                pass
            _wait_for(lambda: not pid_alive(marker_pid), PROCESS_SETTLE_TIMEOUT_S)
            if pid_alive(marker_pid):
                try:
                    os.kill(marker_pid, 9)
                except OSError:
                    pass
        if not keep_open and monitor_window is not None:
            remaining = wait_for_window(SYSTEM_MONITOR["app_id"], 0.5)
            if remaining is not None:
                menu_button = find_node(
                    "rmac-top-bar", f"{SYSTEM_MONITOR['display_name']} menu", role="button", timeout=2.0
                )
                closed = False
                if menu_button is not None and "click" in action_names(menu_button):
                    click(menu_button)
                    quit_item = find_node(
                        "rmac-top-bar", f"Quit {SYSTEM_MONITOR['display_name']}", timeout=2.0
                    )
                    if quit_item is not None and "click" in action_names(quit_item):
                        click(quit_item)
                        closed = wait_for_window_gone(remaining["id"], CLOSE_TIMEOUT_S)
                if not closed:
                    niri_close_window(remaining["id"])


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
        help="leave the System Monitor window open (debugging only); the marker "
        "process is still always cleaned up",
    )
    arguments = parser.parse_args()

    try:
        additions = discover_environment(dict(os.environ), Path(f"/run/user/{os.getuid()}"))
        os.environ.update(additions)
        report = run_journey(arguments.keep_open)
    except JourneyError as error:
        parser.exit(4, f"run-journey-monitor: {error}\n")

    text = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if arguments.output is not None:
        arguments.output.write_text(text, encoding="utf-8")
    print(text, end="")

    passed = sum(1 for step in report["steps"] if step["passed"])
    total = len(report["steps"])
    print(
        f"journey 6: {passed}/{total} steps passed; "
        f"overall_pass={report['overall_pass']}",
        file=sys.stderr,
    )
    return 0 if report["overall_pass"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
