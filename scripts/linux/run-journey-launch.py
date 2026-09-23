#!/usr/bin/env python3
"""Acceptance test for product journey 1 (todo.md "Product journeys"):

    Log in, launch an app from the Dock or Spotlight, switch apps, and
    close it.

Runs against the live rmac session on the reference laptop (niri + AT-SPI +
GDM), the same session a real user would be sitting at. There is no keyboard
or pointer injector installed there (no wtype, no ydotool), so every step
drives the UI only through:

  * AT-SPI actions (pyatspi) on elements that expose a real "click"/text
    action -- see shell/scripts/assert_accessibility.py and
    shell/scripts/assert_dock_accessibility.py for the established pattern;
  * niri IPC (`niri msg --json windows`, `focused-window`,
    `action focus-window --id N`, `action spawn`).

"Log in" is checked as "a real graphical rmac session is already active"
(logind session identity + required rmac units), not a full logout/login
cycle -- forcing a real logout on this shared reference machine would tear
down whatever else is running there (see AGENTS.md and the shared agent
brief: never disrupt the live session). The destructive full GDM
login/logout journey already exists at scripts/linux/run-session-journey.py
against a disposable account; this script complements it rather than
duplicating it.

Launching "from the Dock or Spotlight" is attempted through real AT-SPI
actions first. As of this writing neither surface is actually drivable that
way (see the "dock_launch" and "spotlight_launch" steps in the JSON report):
Dock icons expose only the Accessible/Component AT-SPI interfaces (no
"click" action), and the Spotlight search field exposes neither Text nor
EditableText, so a query cannot be typed without a keyboard injector, and
there is no D-Bus query/activate entry point to fall back to either. Both
are real product accessibility gaps (see todo.md's "no pointer-only
controls" accessibility gate) and are reported as failed steps rather than
silently skipped. To still produce real timing/focus/close evidence for the
rest of the journey, the script then launches the target application the
same way the Dock/Spotlight would ultimately do it (its installed
`Exec=`), clearly labeled in the report as a fallback that did not go
through the accessible UI.

The report is privacy-safe: no screenshots, no home-directory paths, no
window titles, no user names. Every wait in this script is bounded; it never
hangs.
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
    """A bounded, privacy-safe journey-launch failure."""


FORMAT = 1
JOURNEY_ID = 1
JOURNEY_TITLE = (
    "Log in, launch an app from the Dock or Spotlight, switch apps, and close it."
)

NIRI_TIMEOUT_S = 5.0
WINDOW_APPEAR_TIMEOUT_S = 5.0
FOCUS_TIMEOUT_S = 3.0
ATSPI_FIND_TIMEOUT_S = 5.0
CLOSE_TIMEOUT_S = 5.0
POLL_INTERVAL_S = 0.05

# todo.md "Performance budgets": warm launch-to-interactive, p95 <= 500 ms for
# simple apps, <= 900 ms for Files/Terminal. Both journey apps below are
# "simple".
SIMPLE_APP_BUDGET_MS = 500.0

FIRST_APP: dict[str, str] = {
    "display_name": "Text Editor",
    "app_id": "org.rmac.TextEditor",
    "exec": "/usr/bin/rmac-text-editor",
}
SECOND_APP: dict[str, str] = {
    "display_name": "Notes",
    "app_id": "org.rmac.Notes",
    "exec": "/usr/bin/rmac-notes",
}


# --------------------------------------------------------------------------
# Pure helpers (unit-tested from scripts/test_journey_launch.py on macOS)
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


def parse_focused_window(stdout: str) -> Optional[dict[str, Any]]:
    try:
        window = json.loads(stdout)
    except json.JSONDecodeError as error:
        raise JourneyError("niri focused-window output was not valid JSON") from error
    return window if isinstance(window, dict) else None


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


def niri_focused_window() -> Optional[dict[str, Any]]:
    result = _niri("--json", "focused-window")
    if result.returncode != 0:
        raise JourneyError("niri msg focused-window failed")
    return parse_focused_window(result.stdout)


def niri_spawn(command: str) -> None:
    result = _niri("action", "spawn", "--", command)
    if result.returncode != 0:
        raise JourneyError("niri failed to spawn the target application")


def niri_focus_window(window_id: int) -> None:
    result = _niri("action", "focus-window", "--id", str(window_id))
    if result.returncode != 0:
        raise JourneyError(f"niri failed to focus window {window_id}")


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


def wait_for_focus(app_id: str, timeout: float = FOCUS_TIMEOUT_S) -> bool:
    def focused() -> bool:
        window = niri_focused_window()
        return bool(window and window.get("app_id") == app_id)

    return bool(_wait_for(focused, timeout))


def pid_alive(pid: int) -> bool:
    return Path(f"/proc/{pid}").exists()


# --------------------------------------------------------------------------
# AT-SPI helpers
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
):
    def search():
        for node in _atspi_snapshot(app_name):
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
    """"Log in": verify a real graphical rmac session is already active,
    rather than performing a full logout/login (see module docstring)."""

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

    required_units = (
        "rmac-session.target",
        "rmac-top-bar.service",
        "rmac-dock.service",
        "rmac-launcher.service",
    )
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


def attempt_dock_launch(app: dict[str, str]) -> dict[str, Any]:
    button = find_node("rmac-dock", app["display_name"], role="button")
    if button is None:
        return make_step(
            "dock_launch",
            False,
            f"no Dock button named {app['display_name']!r} was found over AT-SPI",
        )
    names = action_names(button)
    if "click" not in names:
        interfaces = sorted(button.get_interfaces())
        return make_step(
            "dock_launch",
            False,
            "Dock icon has no AT-SPI 'click' action (interfaces: "
            f"{interfaces}); it cannot be activated without a pointer device",
        )
    click(button)
    return make_step("dock_launch", True, "clicked the Dock icon via its AT-SPI click action")


def attempt_spotlight_launch(app: dict[str, str]) -> dict[str, Any]:
    button = find_node("rmac-top-bar", "Spotlight", role="button")
    if button is None or "click" not in action_names(button):
        return make_step(
            "spotlight_launch",
            False,
            "the top bar's Spotlight button was not found or has no AT-SPI click action",
        )
    click(button)
    entry = find_node("rmac-launcher", "", role="entry", timeout=2.0)
    try:
        if entry is None:
            return make_step(
                "spotlight_launch",
                False,
                "Spotlight opened but its search field was not found over AT-SPI",
            )
        interfaces = set(entry.get_interfaces())
        if "EditableText" not in interfaces and "Text" not in interfaces:
            return make_step(
                "spotlight_launch",
                False,
                "Spotlight's search field exposes only "
                f"{sorted(interfaces)} over AT-SPI, with no Text or "
                "EditableText interface, so a query cannot be typed "
                "without a keyboard injector (none is installed)",
            )
        if "EditableText" in interfaces:
            entry.queryEditableText().setTextContents(app["display_name"])
        result_row = find_node("rmac-launcher", app["display_name"], role="button", timeout=2.0)
        if result_row is None or "click" not in action_names(result_row):
            return make_step(
                "spotlight_launch",
                False,
                "typed the Spotlight query but found no actionable result row",
            )
        click(result_row)
        return make_step(
            "spotlight_launch", True, "typed a query and activated a Spotlight result"
        )
    finally:
        # Toggling the shortcut again closes the overlay (confirmed live);
        # leave the session as we found it either way.
        subprocess.run(
            ["/usr/libexec/rmac/rmac-shortcut-dispatch", "launcher"],
            check=False,
            capture_output=True,
            timeout=NIRI_TIMEOUT_S,
        )


def launch_app(app: dict[str, str]) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    """Try the real UI paths, then fall back to a direct spawn so the rest
    of the journey can still be exercised and measured. Returns the UI-path
    steps plus a launch-result step describing which method actually ran the
    application."""

    steps = [attempt_dock_launch(app), attempt_spotlight_launch(app)]
    if any(step["passed"] for step in steps):
        launch_step = make_step(
            "app_launched", True, "launched via an accessible Dock/Spotlight action",
            method="accessible_ui",
        )
        return steps, launch_step

    try:
        niri_spawn(app["exec"])
    except JourneyError as error:
        return steps, make_step("app_launched", False, str(error), method="none")
    return steps, make_step(
        "app_launched",
        True,
        "neither the Dock nor Spotlight could be driven over AT-SPI today "
        "(see dock_launch/spotlight_launch above); launched the same "
        "installed command a Dock/Spotlight activation would run, to still "
        "measure the rest of the journey",
        method="fallback_spawn",
    )


def quit_app(display_name: str, window: dict[str, Any]) -> dict[str, Any]:
    if not niri_focus_window_safe(window["id"]):
        return make_step(
            "close",
            False,
            f"could not focus the {display_name} window before quitting it",
            app_id=window.get("app_id"),
        )
    menu_button = find_node("rmac-top-bar", f"{display_name} menu", role="button")
    if menu_button is None or "click" not in action_names(menu_button):
        return make_step(
            "close",
            False,
            f"the {display_name} app menu was not found over AT-SPI",
            app_id=window.get("app_id"),
        )
    click(menu_button)
    quit_item = find_node("rmac-top-bar", f"Quit {display_name}", timeout=2.0)
    if quit_item is None or "click" not in action_names(quit_item):
        return make_step(
            "close",
            False,
            f"no actionable Quit {display_name} menu item was found over AT-SPI",
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


def niri_focus_window_safe(window_id: int) -> bool:
    try:
        niri_focus_window(window_id)
    except JourneyError:
        return False
    return wait_for_focus_id(window_id)


def wait_for_focus_id(window_id: int, timeout: float = FOCUS_TIMEOUT_S) -> bool:
    def focused() -> bool:
        window = niri_focused_window()
        return bool(window and window.get("id") == window_id)

    return bool(_wait_for(focused, timeout))


# --------------------------------------------------------------------------
# Orchestration
# --------------------------------------------------------------------------


def run_journey(budget_ms: float, keep_open: bool) -> dict[str, Any]:
    steps: list[dict[str, Any]] = []
    gaps: list[dict[str, str]] = []
    performance: dict[str, Any] = {}
    started_at_unix_ms = int(time.time() * 1000)

    steps.append(check_logged_in())
    gaps.append(
        {
            "surface": "login",
            "issue": (
                "this step checks that a graphical rmac session is already "
                "active rather than performing a real logout/login, to "
                "avoid disturbing the shared reference session; the full "
                "destructive GDM login/logout journey is "
                "scripts/linux/run-session-journey.py"
            ),
        }
    )

    saved_accessibility = get_gsettings_accessibility()
    set_gsettings_accessibility(True)
    first_window: Optional[dict[str, Any]] = None
    second_window: Optional[dict[str, Any]] = None
    try:
        ui_steps, launch_step = launch_app(FIRST_APP)
        steps.extend(ui_steps)
        steps.append(launch_step)
        if not any(step["passed"] for step in ui_steps):
            gaps.append(
                {
                    "surface": "dock",
                    "issue": "Dock icons expose only Accessible/Component over "
                    "AT-SPI (no 'click' action); they cannot be activated by "
                    "assistive technology or by this test today",
                }
            )
            gaps.append(
                {
                    "surface": "spotlight",
                    "issue": "the Spotlight search field exposes neither Text nor "
                    "EditableText over AT-SPI, so a query cannot be typed "
                    "without a keyboard injector, and there is no D-Bus "
                    "query/activate entry point to fall back to",
                }
            )

        if not launch_step["passed"]:
            return build_report(steps, gaps, performance, started_at_unix_ms)

        first_window, elapsed_ms = wait_for_window(FIRST_APP["app_id"])
        performance["first_app"] = evaluate_budget(elapsed_ms, budget_ms)
        steps.append(
            make_step(
                "window_appeared",
                first_window is not None,
                (
                    f"window appeared with app_id={FIRST_APP['app_id']!r} in "
                    f"{elapsed_ms:.0f} ms"
                    if first_window
                    else "no window with the expected app_id appeared"
                ),
            )
        )
        if first_window is None:
            return build_report(steps, gaps, performance, started_at_unix_ms)

        steps.append(
            make_step(
                "window_focused",
                bool(first_window.get("is_focused")),
                "the launched window is focused"
                if first_window.get("is_focused")
                else "the launched window is not focused",
            )
        )

        niri_spawn(SECOND_APP["exec"])
        second_window, elapsed_ms = wait_for_window(SECOND_APP["app_id"])
        performance["second_app"] = evaluate_budget(elapsed_ms, budget_ms)
        steps.append(
            make_step(
                "second_app_launched",
                second_window is not None,
                (
                    f"second window appeared with app_id={SECOND_APP['app_id']!r} "
                    f"in {elapsed_ms:.0f} ms"
                    if second_window
                    else "no second window appeared"
                ),
            )
        )
        if second_window is None:
            return build_report(steps, gaps, performance, started_at_unix_ms)

        switched_to_first = niri_focus_window_safe(first_window["id"])
        steps.append(
            make_step(
                "switch_to_first",
                switched_to_first,
                "focus moved back to the first app"
                if switched_to_first
                else "focus did not move to the first app",
            )
        )
        switched_to_second = niri_focus_window_safe(second_window["id"])
        steps.append(
            make_step(
                "switch_to_second",
                switched_to_second,
                "focus moved to the second app"
                if switched_to_second
                else "focus did not move to the second app",
            )
        )

        if keep_open:
            return build_report(steps, gaps, performance, started_at_unix_ms)

        # Close the second (focused) app first, then the first.
        steps.append(quit_app(SECOND_APP["display_name"], second_window))
        second_window = None
        steps.append(quit_app(FIRST_APP["display_name"], first_window))
        first_window = None

        return build_report(steps, gaps, performance, started_at_unix_ms)
    finally:
        if not keep_open:
            for window, app in (
                (first_window, FIRST_APP),
                (second_window, SECOND_APP),
            ):
                if window is None:
                    continue
                try:
                    quit_app(app["display_name"], window)
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
        help="leave launched windows open for manual inspection (debugging only)",
    )
    arguments = parser.parse_args()

    try:
        additions = discover_environment(dict(os.environ), Path(f"/run/user/{os.getuid()}"))
        os.environ.update(additions)
        report = run_journey(arguments.budget_ms, arguments.keep_open)
    except JourneyError as error:
        parser.exit(4, f"run-journey-launch: {error}\n")

    text = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if arguments.output is not None:
        arguments.output.write_text(text, encoding="utf-8")
    print(text, end="")

    passed = sum(1 for step in report["steps"] if step["passed"])
    total = len(report["steps"])
    print(
        f"journey 1: {passed}/{total} steps passed; "
        f"overall_pass={report['overall_pass']}",
        file=sys.stderr,
    )
    return 0 if report["overall_pass"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
