#!/usr/bin/env python3
"""Acceptance test for product journey 3 (todo.md "Product journeys"):

    Open Terminal, run a command, scroll, select, copy and paste, and manage
    tabs.

Runs against the live rmac session on the reference laptop (niri + AT-SPI +
GDM), the same session a real user would be sitting at. There is no keyboard
or pointer injector installed there (no wtype, no ydotool) and no Wayland
clipboard CLI either (no wl-copy/wl-paste/xclip/xsel -- confirmed absent on
the reference laptop), so every step drives the UI only through:

  * AT-SPI actions (pyatspi) on elements that expose a real "click" action --
    see scripts/linux/run-journey-launch.py for the established pattern this
    script follows (find_node/click/_atspi_snapshot);
  * niri IPC (`niri msg --json windows`, `focused-window`, `action spawn`).

Terminal (crates/terminal, package `rmac-terminal`) has real, well-formed
AT-SPI surfaces for its *menus* -- the global top bar (AT-SPI application
"rmac-top-bar") renders a "Terminal menu" (About/Hide/Quit), a "Shell menu"
(New Tab/Close Tab/Next Tab/Previous Tab), an "Edit menu" (Copy/Paste/Select
All) and a "View menu" (Find/Clear/zoom), each a real AT-SPI button with
accessible name "{label} menu" and each item a real AT-SPI menuitem with the
item's exact label as its name (crates/rmac-app-menu/src/lib.rs:103-131
TERMINAL_MENUS; rendering in shell/bins/rmac-menubar/src/main.rs:1566-1569,
1727-1728). Clicking these really dispatches the terminal's own GPUI actions
end-to-end over D-Bus (crates/rmac-ui/src/runtime.rs:172-195
install_app_menu; crates/terminal/src/controller/renderer/interactions.rs:70-
107 wires Copy/Paste/SelectAll/NewTab/CloseTab/NextTab/PrevTab handlers).
This was observed live once (a dump of the reference laptop's AT-SPI tree
found button nodes named exactly "Terminal menu", "Shell menu", "Edit menu"
and "View menu" under the "rmac-top-bar" application), but a dedicated
40+ second follow-up poll from a fresh launch found only the generic
"Terminal menu" (About/Hide/Quit) -- the Shell/Edit/View category menus did
not reappear, while stale category-less menu entries for two other,
no-longer-running apps were still present. This script cannot explain the
inconsistency from available evidence (possibly a focus/timing condition
this script does not track, or simply reference-laptop flakiness under the
heavy, sustained CPU contention observed throughout -- load average ~7,
consistent with a concurrent cargo build per AGENTS.md); it is reported here
rather than papered over. `select_all`/`copy`/`paste`/`new_tab`/`close_tab`
below still attempt to find and click these menus with a generous retry
budget and report precisely whether they were present on whatever the
reference laptop was actually running at run time.

But the terminal's own *content* surface has real, confirmed accessibility
gaps against todo.md's "no pointer-only controls" gate, both live-verified
against a real `rmac-terminal` window on the reference laptop (its AT-SPI
tree exposed exactly 4 button nodes -- a profile-picker button plus 3
unnamed tab-strip buttons -- and *no* text/entry node at all for the grid):

  * The terminal grid publishes no accessible text, caret or selection
    whatsoever. `crates/terminal/src/accessibility.rs` fully implements and
    unit-tests `TerminalAccessibilitySnapshot`/`project_visible_terminal`,
    but that module is compiled only into `crates/terminal/src/lib.rs:1-3`
    (an orphaned library target), never into the running `rmac-terminal`
    binary: `crates/terminal/src/main.rs`'s own module list never declares
    `mod accessibility;`, and nothing else in the workspace depends on the
    `rmac-terminal` library (`grep -rn "rmac_terminal::"` across the repo:
    zero hits). Independently, `crates/terminal/src/controller/renderer*.rs`
    and `chrome.rs` never call `.role(...)`/`.aria_label(...)` anywhere
    (zero matches), unlike Dock tiles and top-bar menu items which do. This
    is *why* `run_command`, `scroll` and the read-back half of `select`/
    `copy` below cannot be driven or verified over AT-SPI: there is no node
    to find.
  * There is consequently no AT-SPI EditableText, no custom accesskit
    "insert text" action, and no way to set the terminal's input focus
    content at all without a keyboard injector (`crates/terminal/src/
    keyboard.rs`, `controller/input.rs`, `controller/ime_bridge.rs` contain
    no `InsertText`/`EditableText`/`accesskit::Action` wiring). This compounds
    the already-documented, shared upstream gap: the pinned `accesskit_unix`/
    `accesskit_atspi_common` AT-SPI bridge (accesskit_unix 0.21.0, per
    `shell/compat/gpui_linux/Cargo.toml:57`) does not implement
    `org.a11y.atspi.EditableText` at all (see `docs/known-limitations.md:42-
    49` and `docs/journey-suite.md`'s Spotlight write-up) -- so even a fixed
    terminal could not be typed into without a keyboard injector, but today
    the terminal additionally has no accessible surface to type into or read
    from in the first place.
  * The tab strip (`crates/terminal/src/controller/renderer/chrome.rs:65-
    183`) uses only `.id(...)` for its tab rows, close ("x") buttons and the
    "+" new-tab button -- never `.role()`/`.aria_label()`. Live evidence
    shows GPUI/accesskit still publishes these as unnamed AT-SPI "button"
    nodes with a working "click" action (unlike Notes' note rows, see
    scripts/linux/run-journey-notes.py, which are pruned from the tree
    entirely), so this script can still drive and count tabs structurally
    (see `count_tab_buttons` below), but cannot identify a specific tab by
    name, nor read back which tab is active.

Because typing is impossible, this script cannot literally "run a command"
by typing `seq 1 500` and pressing Return -- there is no keyboard injector on
the reference laptop, no accessible text-entry surface on the terminal grid,
and no Wayland clipboard CLI installed there (`wl-copy`/`wl-paste`/`xclip`/
`xsel`/`wtype`/`ydotool`/`dotool` were all confirmed absent live) that could
be used to preload the clipboard from outside and paste it in. The
`run_command` step below fails for exactly this reason and is the headline
finding of this script; see the module docstring principle in todo.md ("An
honest limitation beats simulated system behaviour"). The rest of the
journey that *is* safely and honestly drivable -- launch, window timing,
focus, the Edit menu's Select All/Copy/Paste actions (dispatch verified,
functional effect unverifiable), and tab open/close/switch (verified by
counting unnamed clickable "button" nodes before and after, since each tab
contributes exactly one tab-row and one close button) -- is still exercised
and measured.

The report is privacy-safe: no screenshots, no home-directory paths, no
window titles (the terminal's own window title on this laptop is a shell
prompt string containing the account name, e.g. "user@host: ~ — Terminal" --
never read or logged by this script), no user names. Every wait is bounded;
this script never hangs.
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
    """A bounded, privacy-safe journey-terminal failure."""


FORMAT = 1
JOURNEY_ID = 3
JOURNEY_TITLE = (
    "Open Terminal, run a command, scroll, select, copy and paste, and manage tabs."
)

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
TAB_SETTLE_TIMEOUT_S = 3.0
POLL_INTERVAL_S = 0.05

# todo.md "Performance budgets": warm launch-to-interactive, p95 <= 900 ms for
# Files/Terminal (Terminal is explicitly named in that budget, unlike the
# generic 500 ms "simple apps" budget used by run-journey-launch.py).
TERMINAL_BUDGET_MS = 900.0

APP: dict[str, str] = {
    "display_name": "Terminal",
    "app_id": "org.rmac.Terminal",
    "exec": "/usr/bin/rmac-terminal",
    "atspi_app_name": "rmac-terminal",
}

# Clipboard/keyboard injector tools confirmed absent on the reference
# laptop (checked live: `command -v` for each returned nothing). Kept as a
# named constant so the report can cite exactly what was checked.
MISSING_INJECTOR_TOOLS = (
    "wl-copy",
    "wl-paste",
    "xclip",
    "xsel",
    "wtype",
    "ydotool",
    "dotool",
)


# --------------------------------------------------------------------------
# Pure helpers (unit-tested from scripts/test_journey_terminal.py on macOS)
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


def expected_tab_button_delta(before: int, after: int) -> int:
    """Each tab contributes exactly one tab-row button and one close
    button, so opening/closing one tab must change the unnamed clickable
    button count by exactly 2."""

    return after - before


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


def any_text_capable_node(app_name: str) -> bool:
    """True if any node in the app exposes AT-SPI Text or EditableText --
    i.e. any surface a query/command could plausibly be typed into or read
    back from."""

    for node in _atspi_snapshot(app_name):
        try:
            interfaces = set(node.get_interfaces())
        except (LookupError, RuntimeError):
            continue
        if "EditableText" in interfaces or "Text" in interfaces:
            return True
    return False


def count_tab_buttons(app_name: str = APP["atspi_app_name"]) -> int:
    """Count unnamed, clickable AT-SPI "button" nodes -- the tab strip's
    per-tab row/close buttons and the "+" new-tab button all match this
    (crates/terminal/src/controller/renderer/chrome.rs:65-183 gives them no
    accessible name), while the profile-picker button does not (it carries
    a real name, e.g. "rmac Dark  ▼")."""

    count = 0
    for node in _atspi_snapshot(app_name):
        try:
            if node.getRoleName() != "button":
                continue
            if node.name != "":
                continue
            if "click" not in action_names(node):
                continue
        except (LookupError, RuntimeError):
            continue
        count += 1
    return count


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


def missing_injector_tools() -> list[str]:
    """Which clipboard/keyboard-injector CLIs are absent on this host. Used
    only to make the run_command gap's report precise and current; never to
    decide whether to attempt typing (there is no path that would work
    either way -- see module docstring)."""

    missing = []
    for tool in MISSING_INJECTOR_TOOLS:
        result = subprocess.run(
            ["sh", "-c", f"command -v {tool}"],
            check=False,
            capture_output=True,
            timeout=NIRI_TIMEOUT_S,
        )
        if result.returncode != 0:
            missing.append(tool)
    return missing


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


def launch_terminal() -> tuple[dict[str, Any], dict[str, Any]]:
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


def open_top_bar_menu(display_name_menu: str, timeout: float = ATSPI_FIND_TIMEOUT_S):
    """Find and click a top-bar category menu button (e.g. "Edit menu",
    "Shell menu") and return it, or None if it could not be found/clicked."""

    button = find_node("rmac-top-bar", display_name_menu, role="button", timeout=timeout)
    if button is None or "click" not in action_names(button):
        return None
    click(button)
    return button


def click_menu_item(label: str, timeout: float = 2.0) -> bool:
    item = find_node("rmac-top-bar", label, timeout=timeout)
    if item is None or "click" not in action_names(item):
        return False
    return click(item)


def quit_terminal(window: dict[str, Any]) -> dict[str, Any]:
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
        dock_step, launch_step = launch_terminal()
        steps.append(dock_step)
        steps.append(launch_step)
        if not dock_step["passed"]:
            gaps.append(
                {
                    "surface": "dock",
                    "issue": "the Terminal Dock icon could not be activated over "
                    "AT-SPI (no button named 'Terminal' found, or it has no "
                    "'click' action); see scripts/linux/run-journey-launch.py's "
                    "own dock_launch step for the same class of gap",
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

        # -- run_command: the headline, expected-to-fail gap. --------------
        has_text_surface = any_text_capable_node(APP["atspi_app_name"])
        missing_tools = missing_injector_tools()
        if has_text_surface:
            # Unexpected on today's build, but keep the script honest if a
            # future build wires this up: still cannot type without an
            # injector, so this remains a failure, with a narrower detail.
            steps.append(
                make_step(
                    "run_command",
                    False,
                    "a text-capable AT-SPI node now exists on the terminal, but "
                    "there is still no keyboard injector or clipboard CLI on "
                    f"this host to enter a command (missing: {missing_tools})",
                )
            )
        else:
            steps.append(
                make_step(
                    "run_command",
                    False,
                    "the terminal grid exposes no AT-SPI Text/EditableText node "
                    "at all (crates/terminal/src/main.rs never compiles "
                    "crates/terminal/src/accessibility.rs's "
                    "TerminalAccessibilitySnapshot into the running binary; "
                    "crates/terminal/src/controller/renderer/chrome.rs and "
                    "renderer.rs never call .role()/.aria_label()), and no "
                    f"clipboard CLI is installed to preload input externally "
                    f"(missing: {missing_tools}); a command cannot be run "
                    "without a keyboard injector",
                )
            )
        gaps.append(
            {
                "surface": "terminal-grid",
                "issue": "no AT-SPI text/caret/selection exposure and no "
                "EditableText/insert-text action exist for the terminal's "
                "content area; TerminalAccessibilitySnapshot/"
                "project_visible_terminal (crates/terminal/src/"
                "accessibility.rs) are fully implemented and unit-tested but "
                "live only in an orphaned lib.rs target that the running "
                "rmac-terminal binary never compiles or calls "
                "(crates/terminal/src/main.rs's module list omits `mod "
                "accessibility;`; `grep -rn \"rmac_terminal::\"` across the "
                "repo returns zero hits). Typing is a core accessibility "
                "need (todo.md); this blocks it entirely on this build, "
                "independent of the separately-documented upstream "
                "accesskit_unix EditableText gap "
                "(docs/known-limitations.md:42-49).",
            }
        )

        # -- scroll: same root cause, no accessible grid surface. -----------
        steps.append(
            make_step(
                "scroll",
                False,
                "no AT-SPI node for the terminal viewport exposes a scroll "
                "action, Value, or Table interface (confirmed: the terminal's "
                "AT-SPI tree exposes only button nodes -- a profile picker "
                "plus tab-strip controls -- no scrollable content node at "
                "all); scrollback cannot be driven or read over AT-SPI",
            )
        )

        # -- select / copy / paste: dispatch is real, effect unverifiable. --
        for step_id, menu_label, item_label in (
            ("select_all", "Edit menu", "Select All"),
            ("copy", "Edit menu", "Copy"),
            ("paste", "Edit menu", "Paste"),
        ):
            menu_button = open_top_bar_menu(menu_label)
            if menu_button is None:
                steps.append(
                    make_step(
                        step_id,
                        False,
                        f"the top bar's {menu_label!r} button was not found over AT-SPI",
                    )
                )
                continue
            clicked = click_menu_item(item_label)
            steps.append(
                make_step(
                    step_id,
                    clicked,
                    (
                        f"clicked {menu_label} > {item_label} via AT-SPI; the "
                        "terminal exposes no accessible text/selection state "
                        "or clipboard-reading tool on this host, so the "
                        "functional effect cannot be independently verified"
                        if clicked
                        else f"the {item_label!r} menu item was not found or "
                        "not actionable over AT-SPI"
                    ),
                )
            )
        gaps.append(
            {
                "surface": "clipboard",
                "issue": "no Wayland clipboard CLI (wl-copy/wl-paste/xclip/xsel) "
                "is installed on the reference laptop, so this script cannot "
                "independently verify Copy/Paste content even where the menu "
                "actions themselves dispatch successfully",
            }
        )

        # -- tabs: verified by counting unnamed clickable buttons. ----------
        baseline_tabs = count_tab_buttons()
        new_tab_menu = open_top_bar_menu("Shell menu")
        if new_tab_menu is None:
            steps.append(make_step("new_tab", False, "the Shell menu was not found over AT-SPI"))
        else:
            clicked = click_menu_item("New Tab")
            after_new_tab = _wait_for(
                lambda: count_tab_buttons() if count_tab_buttons() != baseline_tabs else None,
                TAB_SETTLE_TIMEOUT_S,
            )
            after_new_tab = after_new_tab if after_new_tab is not None else count_tab_buttons()
            delta = expected_tab_button_delta(baseline_tabs, after_new_tab)
            steps.append(
                make_step(
                    "new_tab",
                    clicked and delta == 2,
                    f"Shell > New Tab clicked={clicked}; tab-button count "
                    f"{baseline_tabs} -> {after_new_tab} (delta {delta}, "
                    "expected 2: one new tab row plus one close button)",
                )
            )

            if clicked and delta == 2:
                switch_menu = open_top_bar_menu("Shell menu")
                switched = switch_menu is not None and click_menu_item("Previous Tab")
                after_switch = count_tab_buttons()
                steps.append(
                    make_step(
                        "switch_tab",
                        switched and after_switch == after_new_tab,
                        f"Shell > Previous Tab clicked={switched}; tab-button "
                        f"count unchanged at {after_switch} (active-tab identity "
                        "itself is not exposed over AT-SPI, so only the tab "
                        "count's stability can be verified)",
                    )
                )

                close_menu = open_top_bar_menu("Shell menu")
                closed = close_menu is not None and click_menu_item("Close Tab")
                after_close = _wait_for(
                    lambda: count_tab_buttons() if count_tab_buttons() != after_new_tab else None,
                    TAB_SETTLE_TIMEOUT_S,
                )
                after_close = after_close if after_close is not None else count_tab_buttons()
                close_delta = expected_tab_button_delta(after_new_tab, after_close)
                steps.append(
                    make_step(
                        "close_tab",
                        closed and close_delta == -2,
                        f"Shell > Close Tab clicked={closed}; tab-button count "
                        f"{after_new_tab} -> {after_close} (delta {close_delta}, "
                        "expected -2)",
                    )
                )
        gaps.append(
            {
                "surface": "terminal-tabs",
                "issue": "tab rows, their close buttons, and the new-tab '+' "
                "button carry no AT-SPI accessible name (crates/terminal/src/"
                "controller/renderer/chrome.rs:65-183 use only .id(), never "
                ".role()/.aria_label()), so a specific tab cannot be targeted "
                "by name and active-tab identity cannot be read back over "
                "AT-SPI; this script verifies tab lifecycle only by counting "
                "unnamed clickable button nodes before and after each Shell "
                "menu action",
            }
        )

        if keep_open:
            return build_report(steps, gaps, performance, started_at_unix_ms)

        steps.append(quit_terminal(window))
        window = None
        return build_report(steps, gaps, performance, started_at_unix_ms)
    finally:
        if not keep_open and window is not None:
            try:
                quit_terminal(window)
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
        default=TERMINAL_BUDGET_MS,
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
        parser.exit(4, f"run-journey-terminal: {error}\n")

    text = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if arguments.output is not None:
        arguments.output.write_text(text, encoding="utf-8")
    print(text, end="")

    passed = sum(1 for step in report["steps"] if step["passed"])
    total = len(report["steps"])
    print(
        f"journey 3: {passed}/{total} steps passed; "
        f"overall_pass={report['overall_pass']}",
        file=sys.stderr,
    )
    return 0 if report["overall_pass"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
