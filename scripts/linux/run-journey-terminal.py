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

The terminal's own *content* surface has real accessibility gaps left
against todo.md's "no pointer-only controls" gate, but far fewer than before:
`crates/terminal/src/controller/renderer/accessibility.rs` now wires
`project_visible_terminal` into the running binary, publishing the grid as a
live, named `Role::Terminal` node ("Terminal") with a synthetic
`Role::TextRun` child carrying its visible text over AT-SPI's Text
interface, and the tab strip (`crates/terminal/src/controller/renderer/
chrome.rs:65-183`) now gives each tab a real `Role::Tab` with its title as
its accessible name, plus named "Close tab {title}" and "New Tab" buttons.
What is verified live, and what still isn't:

  * The grid's text is now readable over AT-SPI (`run_command`'s check reads
    it back), and Select All's effect is independently verifiable via the
    grid's AT-SPI Text selection span, rather than only a dispatched click.
  * There is still no AT-SPI EditableText or custom "insert text" action on
    the grid, so nothing can be typed into it without a keyboard injector
    (`crates/terminal/src/keyboard.rs`, `controller/input.rs`,
    `controller/ime_bridge.rs` still wire no `InsertText`/`EditableText`/
    `accesskit::Action`). This is the same pinned upstream `accesskit_unix`/
    `accesskit_atspi_common` gap documented for Spotlight and other fields
    (`docs/known-limitations.md:42-49`), not Terminal-specific.
  * No scroll action, Value, or Table interface exists on the grid, so
    scrollback still cannot be driven or read over AT-SPI.
  * Tabs can now be identified, switched, and closed by name/role ("page
    tab"), rather than only counted as unnamed clickable buttons.

Because typing is impossible, this script cannot literally "run a command"
by typing `seq 1 500` and pressing Return -- there is no keyboard injector on
the reference laptop, and no Wayland clipboard CLI installed there
(`wl-copy`/`wl-paste`/`xclip`/`xsel`/`wtype`/`ydotool`/`dotool` were all
confirmed absent live) that could be used to preload the clipboard from
outside and paste it in. The `run_command` step below fails for exactly this
reason and is the headline finding of this script; see the module docstring
principle in todo.md ("An honest limitation beats simulated system
behaviour"). The rest of the journey that *is* safely and honestly
drivable -- launch (with an adaptive window-appearance wait and the load
average at launch, since heavy concurrent CPU load has been observed to slow
a normally-instant launch), focus, the Edit menu's Select All (now
independently verified)/Copy/Paste actions, and tab open/close/switch (now
identified by name/role) -- is still exercised and measured.

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
# A generous, adaptive ceiling for the *first* window-appearance poll: under
# heavy concurrent CPU load (a cargo build elsewhere on the shared reference
# laptop, load average observed as high as ~7) a launch that is instant when
# idle has been seen to take several seconds longer than
# WINDOW_APPEAR_TIMEOUT_S. Waiting up to this long, and reporting the actual
# elapsed time plus the load average observed at launch, turns a load-related
# timeout into an honest, explained pass rather than a false "no window
# appeared" product failure.
ADAPTIVE_WINDOW_APPEAR_TIMEOUT_S = 15.0
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


def desktop_entry_dirs(env: dict[str, str], home: str) -> list[Path]:
    """Application directories in XDG lookup order (user entries first)."""
    data_home = env.get("XDG_DATA_HOME") or os.path.join(home, ".local/share")
    data_dirs = env.get("XDG_DATA_DIRS") or "/usr/local/share:/usr/share"
    return [Path(data_home) / "applications"] + [
        Path(entry) / "applications" for entry in data_dirs.split(":") if entry
    ]


def exec_from_desktop_entry(text: str) -> str | None:
    """The program path of a desktop entry's Exec= line, without field codes."""
    in_entry = False
    for line in text.splitlines():
        stripped = line.strip()
        if stripped.startswith("["):
            in_entry = stripped == "[Desktop Entry]"
            continue
        if in_entry and stripped.startswith("Exec="):
            words = [w for w in stripped[len("Exec="):].split() if not w.startswith("%")]
            return words[0] if words else None
    return None


def resolve_app_exec(app: dict[str, str], env: dict[str, str], home: str) -> str:
    """Launch what the Dock launches: the first desktop entry for the app id wins,
    so a stale /usr/bin copy never stands in for the user's current build."""
    for directory in desktop_entry_dirs(env, home):
        entry = directory / f"{app['app_id']}.desktop"
        try:
            program = exec_from_desktop_entry(entry.read_text(encoding="utf-8"))
        except OSError:
            continue
        if program:
            return program
    return app["exec"]


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


def get_load_average() -> Optional[tuple[float, float, float]]:
    """The 1/5/15-minute load average this host reports right now, or
    ``None`` where unavailable (e.g. non-Linux) -- included in the
    window-appearance report so a slow launch under heavy build load is
    distinguishable from a real regression."""

    try:
        return os.getloadavg()
    except OSError:
        return None


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


def has_text_interface(node) -> bool:
    try:
        node.queryText()
        return True
    except (LookupError, RuntimeError, NotImplementedError):
        return False


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


def find_terminal_grid_node(
    app_name: str = APP["atspi_app_name"], timeout: float = ATSPI_FIND_TIMEOUT_S
):
    """The terminal grid's own AT-SPI node -- `Role::Terminal`, named
    "Terminal" (crates/terminal/src/controller/renderer/interactions.rs:27-
    31), publishing the live grid's text via a synthetic `Role::TextRun`
    child (crates/terminal/src/controller/renderer/accessibility.rs)."""

    return find_node(app_name, "Terminal", role="terminal", timeout=timeout)


def selection_span(node) -> int:
    """Total selected character count over `node`'s AT-SPI Text interface,
    or 0 if it has none or nothing is selected."""

    try:
        text_iface = node.queryText()
        count = text_iface.getNSelections()
    except (LookupError, RuntimeError, NotImplementedError):
        return 0
    total = 0
    for index in range(count):
        try:
            start, end = text_iface.getSelection(index)
        except (LookupError, RuntimeError):
            continue
        total += max(0, end - start)
    return total


def count_tabs(app_name: str = APP["atspi_app_name"]) -> int:
    """Count AT-SPI "page tab" nodes -- each terminal tab now renders as a
    real, named `Role::Tab` (crates/terminal/src/controller/renderer/
    chrome.rs:65-158), distinct from its own "Close tab {title}" button and
    the shared "New Tab" button, both also named but a different role."""

    count = 0
    for node in _atspi_snapshot(app_name):
        try:
            if node.getRoleName() == "page tab":
                count += 1
        except (LookupError, RuntimeError):
            continue
    return count


def tab_names(app_name: str = APP["atspi_app_name"]) -> list[str]:
    names = []
    for node in _atspi_snapshot(app_name):
        try:
            if node.getRoleName() != "page tab":
                continue
            names.append(node.name)
        except (LookupError, RuntimeError):
            continue
    return names


def is_selected(node) -> bool:
    try:
        return bool(node.getState().contains(pyatspi.STATE_SELECTED))
    except (LookupError, RuntimeError, AttributeError):
        return False


def set_gsettings_accessibility(enabled: bool) -> None:
    # Only toolkit-accessibility is needed: since accesskit_unix 0.22
    # (ADR 0013, commit 8de9528a) every rmac window registers with AT-SPI
    # as soon as the bus reports IsEnabled, which this key drives. The
    # separate screen-reader-enabled key also flips GNOME's Orca autostart
    # condition, which starts Orca talking on a real session -- leave it
    # alone.
    for schema, key in (("org.gnome.desktop.interface", "toolkit-accessibility"),):
        subprocess.run(
            ["gsettings", "set", schema, key, "true" if enabled else "false"],
            check=False,
            capture_output=True,
            timeout=NIRI_TIMEOUT_S,
        )


def get_gsettings_accessibility() -> dict[str, str]:
    values: dict[str, str] = {}
    for schema, key in (("org.gnome.desktop.interface", "toolkit-accessibility"),):
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
        niri_spawn(resolve_app_exec(APP, dict(os.environ), str(Path.home())))
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

        load_average = get_load_average()
        window, elapsed_ms = wait_for_window(
            APP["app_id"], timeout=ADAPTIVE_WINDOW_APPEAR_TIMEOUT_S
        )
        performance["launch"] = evaluate_budget(elapsed_ms, budget_ms)
        performance["launch"]["load_average"] = load_average
        steps.append(
            make_step(
                "window_appeared",
                window is not None,
                (
                    f"window appeared with app_id={APP['app_id']!r} in "
                    f"{elapsed_ms:.0f} ms (load average at launch: "
                    f"{load_average}, adaptive wait up to "
                    f"{ADAPTIVE_WINDOW_APPEAR_TIMEOUT_S:.0f} s)"
                    if window
                    else "no window with the expected app_id appeared within "
                    f"{ADAPTIVE_WINDOW_APPEAR_TIMEOUT_S:.0f} s (load average "
                    f"at launch: {load_average})"
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
        # The terminal grid is no longer accessibility-dead: it is a live,
        # named `Role::Terminal` node exposing the visible grid's text over
        # AT-SPI's Text interface (crates/terminal/src/controller/renderer/
        # accessibility.rs, project_visible_terminal). It is still
        # read-only: there is no EditableText/insert-text action, so a
        # command still cannot be typed without a keyboard injector.
        grid_node = find_terminal_grid_node(timeout=3.0)
        missing_tools = missing_injector_tools()
        if grid_node is not None and has_text_interface(grid_node):
            grid_text_length = len(grid_node.queryText().getText(0, -1))
            steps.append(
                make_step(
                    "run_command",
                    False,
                    "the terminal grid now exposes its visible content over "
                    f"AT-SPI Text ({grid_text_length} characters read back), "
                    "but there is still no EditableText/insert-text action on "
                    "it, and no keyboard injector or clipboard CLI is "
                    f"installed on this host to enter a command (missing: "
                    f"{missing_tools}); a command still cannot be run",
                )
            )
        else:
            steps.append(
                make_step(
                    "run_command",
                    False,
                    "no AT-SPI 'terminal' node with a Text interface was "
                    "found for the grid (grid_node_found="
                    f"{grid_node is not None}), and no clipboard CLI is "
                    f"installed to preload input externally (missing: "
                    f"{missing_tools}); a command cannot be run without a "
                    "keyboard injector",
                )
            )
        gaps.append(
            {
                "surface": "terminal-grid-input",
                "issue": "the terminal grid (Role::Terminal, crates/terminal/"
                "src/controller/renderer/accessibility.rs) now exposes its "
                "visible text read-only over AT-SPI, but has no "
                "EditableText or custom insert-text action, so it still "
                "cannot be typed into without a keyboard injector -- the "
                "same pinned upstream accesskit_unix EditableText gap "
                "documented for Spotlight and other fields "
                "(docs/known-limitations.md:42-49), not a Terminal-specific "
                "regression.",
            }
        )

        # -- scroll: no accessible scroll surface on the grid. ---------------
        steps.append(
            make_step(
                "scroll",
                False,
                "the terminal grid's AT-SPI node exposes readable Text now, "
                "but no scroll action, Value, or Table interface exists for "
                "it (grid_node_found="
                f"{grid_node is not None}); scrollback still cannot be "
                "driven over AT-SPI",
            )
        )

        # -- select / copy / paste ------------------------------------------
        # Select All is now independently verifiable: the grid's Text
        # interface reports a real selection span. Copy/Paste dispatch is
        # still real but their clipboard effect remains unverifiable (no
        # Wayland clipboard CLI on this host).
        baseline_selection = selection_span(grid_node) if grid_node is not None else 0
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
            if step_id == "select_all" and clicked and grid_node is not None:
                selected_after = selection_span(grid_node)
                verified = selected_after > baseline_selection
                steps.append(
                    make_step(
                        step_id,
                        verified,
                        f"clicked {menu_label} > {item_label}; the grid's AT-SPI "
                        f"Text selection span grew from {baseline_selection} to "
                        f"{selected_after} character(s)"
                        if verified
                        else f"clicked {menu_label} > {item_label}, but the "
                        f"grid's AT-SPI Text selection span did not grow "
                        f"({baseline_selection} -> {selected_after})",
                    )
                )
                continue
            steps.append(
                make_step(
                    step_id,
                    clicked,
                    (
                        f"clicked {menu_label} > {item_label} via AT-SPI; no "
                        "Wayland clipboard CLI is installed on this host, so "
                        "the clipboard effect cannot be independently "
                        "verified"
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

        # -- tabs: each tab is now a real, named Role::Tab. ------------------
        baseline_names = set(tab_names())
        new_tab_button = find_node(APP["atspi_app_name"], "New Tab", role="button", timeout=3.0)
        new_tab_clicked = new_tab_button is not None and click(new_tab_button)
        after_new_tab_names = _wait_for(
            lambda: (set(tab_names()) - baseline_names) or None, TAB_SETTLE_TIMEOUT_S
        )
        created = next(iter(after_new_tab_names), None) if after_new_tab_names else None
        steps.append(
            make_step(
                "new_tab",
                new_tab_clicked and created is not None,
                f"clicked the named 'New Tab' button; a new tab "
                f"({len(tab_names())} total) appeared"
                if new_tab_clicked and created is not None
                else f"new_tab_button_found={new_tab_button is not None} "
                f"clicked={new_tab_clicked}; no new tab appeared",
            )
        )

        if created is not None:
            tab_node = find_node(APP["atspi_app_name"], created, role="page tab", timeout=2.0)
            was_selected_before = tab_node is not None and is_selected(tab_node)
            # The new tab is selected on creation; switch to the previous
            # one to have something real to verify, then back.
            other_names = [name for name in tab_names() if name != created]
            switched = False
            if other_names:
                other_tab = find_node(
                    APP["atspi_app_name"], other_names[0], role="page tab", timeout=2.0
                )
                switched = other_tab is not None and click(other_tab)
            after_switch_selected = tab_node is not None and is_selected(tab_node)
            steps.append(
                make_step(
                    "switch_tab",
                    switched and was_selected_before and not after_switch_selected,
                    f"the new tab was selected on creation "
                    f"(was_selected_before={was_selected_before}) and clicking "
                    f"another tab moved AT-SPI's STATE_SELECTED off it "
                    f"(after_switch_selected={after_switch_selected})"
                    if switched
                    else "could not find another tab to switch to over AT-SPI",
                )
            )

            close_button = find_node(
                APP["atspi_app_name"], f"Close tab {created}", role="button", timeout=2.0
            )
            closed = close_button is not None and click(close_button)
            after_close_names = _wait_for(
                lambda: created not in tab_names() or None, TAB_SETTLE_TIMEOUT_S
            )
            steps.append(
                make_step(
                    "close_tab",
                    bool(closed and after_close_names),
                    f"clicked the named 'Close tab {created}' button and it "
                    "left the tab strip"
                    if closed and after_close_names
                    else f"close_button_found={close_button is not None} "
                    f"clicked={closed}; the tab was still present afterward",
                )
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
