#!/usr/bin/env python3
"""Sweep every System Settings pane and reachable subpage for visible error
and warning text, over live AT-SPI on the reference laptop.

    exec 9>/tmp/lulo-journey.lock; flock -w 900 9 && \
    python3 scripts/linux/sweep-settings-errors.py --binary PATH [--output report.json]

The owner reported "a lot of warnings and errors in Settings". This script
answers precisely, not anecdotally: it drives ONE real `rmac-system-settings`
against the live session's real Wayland compositor, real session D-Bus bus
and real system D-Bus bus -- so NetworkManager, BlueZ, UPower,
power-profiles-daemon, systemd-timedated/-localed, PipeWire and every other
real backend answer exactly as they do for the owner -- while giving the
process temporary `XDG_CONFIG_HOME`/`XDG_STATE_HOME`/`XDG_DATA_HOME`/
`XDG_CACHE_HOME` directories so this run never reads or writes the owner's
real Settings state (window position, last-selected pane, cached Wi-Fi
passwords, wallpaper choice, etc.). `XDG_RUNTIME_DIR`, `WAYLAND_DISPLAY` and
both D-Bus bus addresses are left exactly as the live session set them.

Like `scripts/atspi_assert_support.py`'s own checks, this script never
injects a keystroke or a synthetic pointer event: every step is a real
AT-SPI `Action.doAction("click")` on a node the app itself published, or a
`--pane <id>` launch that hands its arguments to the one running instance
instead of opening a second window (SET-57, `controller.rs::run()`,
`rmac_ui::boot_unified_single_window_app_with_assets`). That hand-off is
also how this script reaches every one of the 25 top-level panes in
`crates/system-settings/src/navigation.rs`'s `PANE_ROUTES`: a fresh
`--pane <id>` launch always lands on that pane's own root
(`NavigationState::navigate_to_pane` -> `select_category` ->
`select_position_keeping_focus`, which unconditionally clears any pushed
subpage), so it doubles as a cheap, reliable way back to a pane's root
between subpage visits without needing to find and click a "Back" control.

Reachable subpages (`SubPage` in navigation.rs) are visited too:

  * "about" and "software-update" have their own `--pane` routes
    (`subpage_route`), so this script uses those directly.
  * General's "Storage" row, Accessibility's four section rows (Screen
    Reader, Display, Motion, Pointer Control), and every row Focus,
    Notifications, Network and Privacy & Security currently list (Focus
    modes, notification-history apps, network services/interfaces, and the
    Camera/Microphone privacy categories) are entered by an AT-SPI click.

Those rows are found by ONE safety-motivated structural rule, not by name:
`crates/rmac-ui/src/controls.rs`'s `ListRow` (which `nav_row`/`icon_nav_row`/
`large_nav_row`/`page_row` all build on) defaults to `Role::ListItem`, while
every control that mutates live state -- a toggle switch, a checkbox, a
"Refresh"/"Turn Off Focus"/"Forget This Network…" button, a combo box -- is a
different AT-SPI role (`Role::Switch`, `Role::Button`, `Role::CheckBox`,
`Role::ComboBox`, ...). This script therefore only ever clicks nodes whose
AT-SPI role is exactly "list item", and only those outside the sidebar (the
sidebar's own rows, named after `PANE_ROUTES`'s 25 display names, are
excluded so this script never re-drives navigation it already reached via
`--pane`). It never clicks a "push button", so it never presses
Connect/Forget/Empty/Refresh/Turn Off/any other mutating control, and never
opens a picker or a destructive confirmation. This intentionally means
`FocusSchedule` (only reachable through Focus's "Edit Schedule…" *button*,
and only once a schedule exists) is not visited; that is a documented
limitation, not an oversight.

On each pane/subpage, every visible text this script can reach --
`node.name`, `node.description`, and (for anything with a Text interface)
`node.queryText()`'s content, for every descendant -- is checked against
`ERROR_PATTERNS`, plus every node whose AT-SPI role name contains "alert" or
"notification"/"banner" (`crates/rmac-ui/src/feedback.rs`'s `Toast` and the
error variant of `EmptyState` both set `Role::Alert` with the title and
message folded into one accessible name) is recorded outright. Each landing
is swept twice (`sweep_immediate_and_settled`): once ~0.3s after navigating,
before most background watchers have had a chance to run, and once after
`wait_settle` finds the pane's visible text has stopped changing. A finding
tagged "immediate" with no "settled" counterpart at the same (pane, role,
text) self-healed before the pane settled -- the race shape the owning task
calls out ("live updates starting before the watchers") -- while one tagged
"settled" is a persistent problem. Findings are deduplicated by (pane, role,
text, when): the same lingering backend error can legitimately show up on
every single pane, because `Settings::global_settings_error()` is a single
window-wide banner checked regardless of which pane is open.

Output is a JSON array of `{"pane": ..., "role": ..., "text": ..., "reasons":
[...], "when": "immediate"|"settled"}` objects, plus a short human report on
stdout. `scripts/test_sweep_settings_errors.py` unit-tests the pure
classifier and the row-safety rule with no live session.

Cleanup always sends SIGTERM (then SIGKILL after a timeout) to the one
`rmac-system-settings` process this script started, by PID -- never a
pattern match -- so no second instance is ever left running.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any, Iterable, Optional

try:
    import pyatspi  # type: ignore[import-not-found]
except ImportError:  # pragma: no cover - exercised only off-Linux
    pyatspi = None

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
try:
    import atspi_assert_support as support  # noqa: E402
except ImportError:  # pragma: no cover - exercised only off-Linux
    support = None  # type: ignore[assignment]


class SweepError(RuntimeError):
    pass


FORMAT = 1

# Kept in sync with crates/system-settings/src/navigation.rs's PANE_ROUTES;
# scripts/test_sweep_settings_errors.py checks that sync against the Rust
# source directly so this list can't silently drift.
PANE_ROUTES: list[tuple[str, str]] = [
    ("wifi", "Wi-Fi"),
    ("bluetooth", "Bluetooth"),
    ("network", "Network"),
    ("vpn", "VPN"),
    ("battery", "Battery"),
    ("general", "General"),
    ("date-time", "Date & Time"),
    ("language-region", "Language & Region"),
    ("login-items", "Login Items"),
    ("sharing", "Sharing"),
    ("accessibility", "Accessibility"),
    ("appearance", "Appearance"),
    ("desktop-dock", "Desktop & Dock"),
    ("displays", "Displays"),
    ("menu-bar", "Menu Bar"),
    ("spotlight", "Spotlight"),
    ("wallpaper", "Wallpaper"),
    ("notifications", "Notifications"),
    ("sound", "Sound"),
    ("keyboard", "Keyboard"),
    ("mouse", "Mouse"),
    ("trackpad", "Trackpad"),
    ("focus", "Focus"),
    ("lock-screen", "Lock Screen"),
    ("privacy-security", "Privacy & Security"),
]
CATEGORY_NAMES = {name for _pane_id, name in PANE_ROUTES}

# Panes with a fixed, statically-known set of extra subpages worth entering
# by name, beyond the generic "every list-item row" walk below. "about" and
# "software-update" are `--pane` routes in their own right (subpage_route in
# navigation.rs); Storage and the four Accessibility pages are only reached
# by a click.
STATIC_SUBPAGES: dict[str, list[tuple[str, str]]] = {
    "general": [("about", "About"), ("software-update", "Software Update")],
}
STATIC_ROW_SUBPAGES: dict[str, list[str]] = {
    "general": ["Storage"],
    "accessibility": ["Screen Reader", "Display", "Motion", "Pointer Control"],
}
# Panes whose *every* current list-item row is worth entering (Focus modes,
# notification apps, network services, privacy categories: all dynamic,
# hardware/config-dependent, never destructive to open).
GENERIC_ROW_SUBPAGES = {"focus", "notifications", "network", "privacy-security"}

APP_NAMES = ("rmac-system-settings", "System Settings")

ERROR_PATTERNS = [
    "could not",
    "unavailable",
    "failed",
    "error",
    "not available",
    "unsupported",
    "unknown",
    "cannot",
]
ALERT_ROLE_MARKERS = ("alert", "notification", "banner")

SETTLE_TIMEOUT_S = 4.0
SETTLE_POLL_S = 0.3
APP_FIND_TIMEOUT_S = 15.0
RELAUNCH_WAIT_S = 10.0
CLOSE_TIMEOUT_S = 5.0


# --------------------------------------------------------------------------
# Pure logic: the text/role classifier and the row-safety rule. Unit-tested
# from scripts/test_sweep_settings_errors.py with no live session.
# --------------------------------------------------------------------------


def classify(text: str, role: str = "") -> list[str]:
    """Reasons `text`/`role` look like an error or warning banner, or []
    if this node looks like ordinary content. A node can match on either
    axis independently (a role-alert node with fine-sounding text still
    gets recorded, since a real screen reader announces it regardless)."""

    reasons = []
    lower = (text or "").lower()
    for pattern in ERROR_PATTERNS:
        if pattern in lower:
            reasons.append(f"text:{pattern}")
    role_lower = (role or "").lower()
    for marker in ALERT_ROLE_MARKERS:
        if marker in role_lower:
            reasons.append(f"role:{marker}")
    return reasons


def is_safe_subpage_row(role: str, name: str) -> bool:
    """Whether an AT-SPI node is safe to click when generically discovering
    subpages: exactly the "list item" rows `ListRow` defaults to, with a
    name, and not one of the sidebar's own 25 category rows (already
    visited through `--pane`, and clicking one would just re-navigate to a
    pane this script is already going to reach on its own)."""

    return bool(name) and role == "list item" and name not in CATEGORY_NAMES


def navigation_rs_pane_routes(navigation_rs_text: str) -> list[tuple[str, str]]:
    """Parses `PANE_ROUTES` straight out of navigation.rs's source text, so
    a test can catch this script's own copy drifting from it."""

    match = re.search(
        r"PANE_ROUTES[^=]*=\s*\[(.*?)\];", navigation_rs_text, re.DOTALL
    )
    if not match:
        raise SweepError("could not find PANE_ROUTES in navigation.rs")
    pairs = re.findall(r'\(\s*"([^"]+)"\s*,\s*"([^"]+)"\s*\)', match.group(1))
    if not pairs:
        raise SweepError("PANE_ROUTES parsed to no entries")
    return pairs


# --------------------------------------------------------------------------
# Environment and process management.
# --------------------------------------------------------------------------


def build_env(work: Path) -> dict[str, str]:
    """The live session's own environment (real WAYLAND_DISPLAY,
    XDG_RUNTIME_DIR, DBUS_SESSION_BUS_ADDRESS and the system bus), with only
    the config/state/data/cache directories replaced so this run never
    touches the owner's real Settings state."""

    env = dict(os.environ)
    config_home = work / "config"
    state_home = work / "state"
    data_home = work / "data"
    cache_home = work / "cache"
    for directory in (config_home, state_home, data_home, cache_home):
        directory.mkdir(parents=True, exist_ok=True)
    env["XDG_CONFIG_HOME"] = str(config_home)
    env["XDG_STATE_HOME"] = str(state_home)
    env["XDG_DATA_HOME"] = str(data_home)
    env["XDG_CACHE_HOME"] = str(cache_home)
    return env


def require_live_session(env: dict[str, str]) -> None:
    for var in ("WAYLAND_DISPLAY", "XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS"):
        if not env.get(var):
            raise SweepError(
                f"refusing to run: {var} is not set -- this must run inside "
                "the live graphical session, with the real session bus"
            )


def launch(binary: Path, env: dict[str, str], pane: Optional[str] = None) -> "subprocess.Popen[bytes]":
    args = [str(binary)]
    if pane:
        args += ["--pane", pane]
    # close_fds=True (the default on POSIX) keeps this script's own file
    # descriptors -- including fd 9, the caller's /tmp/lulo-journey.lock
    # flock -- out of the launched process, exactly as "launched with
    # 9>&-" asks: a long-lived rmac-system-settings must never end up
    # holding the journey lock open.
    return subprocess.Popen(
        args,
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        stdin=subprocess.DEVNULL,
        close_fds=True,
    )


def find_settings_app():
    for candidate in APP_NAMES:
        app = support.find_app(candidate)
        if app is not None:
            return app
    if pyatspi is None:
        return None
    desktop = pyatspi.Registry.getDesktop(0)
    for index in range(desktop.childCount):
        try:
            app = desktop.getChildAtIndex(index)
        except (LookupError, RuntimeError):
            continue
        if app is not None and "settings" in (app.name or "").lower():
            return app
    return None


def content_fingerprint(app) -> str:
    parts = []
    for node in support.descendants(app):
        for text in node_texts(node):
            parts.append(text)
    return "\x1e".join(parts)


def wait_settle(app, timeout: float = SETTLE_TIMEOUT_S) -> None:
    """Waits for the pane's visible text to stop changing between two
    polls, or gives up after `timeout` -- a pane still mid-async-fetch at
    that point is swept as-is (its loading/placeholder text is real
    output, not a script bug)."""

    deadline = time.monotonic() + timeout
    previous = None
    while time.monotonic() < deadline:
        current = content_fingerprint(app)
        if current == previous:
            return
        previous = current
        time.sleep(SETTLE_POLL_S)


# How long to wait before the "immediate" sweep below: long enough for the
# just-navigated view to paint at all, short enough to still catch a
# placeholder/error shown before a background watcher's first tick lands --
# exactly the race the owning task calls out ("live updates starting before
# the watchers"). `wait_settle`'s own poll then runs after it for the
# second, "settled" sweep, so a message seen only immediately (and gone by
# the time content stops changing) is reported as a transient/race finding,
# distinct from one that is still there once the pane has settled.
IMMEDIATE_SWEEP_DELAY_S = 0.3


# --------------------------------------------------------------------------
# AT-SPI text collection.
# --------------------------------------------------------------------------


def node_texts(node) -> list[str]:
    texts = []
    name = support.name(node)
    if name:
        texts.append(name)
    description = support.description(node)
    if description:
        texts.append(description)
    if support.has_text(node):
        try:
            text, _caret, _selections = support.text_of(node)
            if text:
                texts.append(text)
        except (LookupError, RuntimeError):
            pass
    return texts


def sweep(
    app,
    pane_label: str,
    findings: list[dict[str, Any]],
    seen: set[tuple[str, str, str, str]],
    when: str = "settled",
) -> None:
    for node in support.descendants(app):
        role = support.role(node)
        for text in node_texts(node):
            reasons = classify(text, role)
            if not reasons:
                continue
            key = (pane_label, role, text, when)
            if key in seen:
                continue
            seen.add(key)
            findings.append(
                {"pane": pane_label, "role": role, "text": text, "reasons": reasons, "when": when}
            )


def sweep_immediate_and_settled(
    app,
    pane_label: str,
    findings: list[dict[str, Any]],
    seen: set[tuple[str, str, str, str]],
) -> None:
    """Two passes per landing: a fast one that can still catch a
    placeholder/error a background watcher hasn't corrected yet, and one
    after `wait_settle` -- see IMMEDIATE_SWEEP_DELAY_S. A finding tagged
    "immediate" with no "settled" counterpart at the same (pane, role, text)
    self-healed before the pane settled, which is itself worth reporting: it
    is exactly the transient-race shape the owning task asks to hunt for."""

    time.sleep(IMMEDIATE_SWEEP_DELAY_S)
    sweep(app, pane_label, findings, seen, when="immediate")
    wait_settle(app)
    sweep(app, pane_label, findings, seen, when="settled")


def safe_subpage_rows(app) -> list[str]:
    """Every currently-listed list-item row name outside the sidebar, in
    document order, deduplicated. Called once per pane visit, before any
    click, so later clicks never see a list mutated by an earlier one."""

    names: list[str] = []
    seen: set[str] = set()
    for node in support.descendants(app):
        name = support.name(node)
        if not is_safe_subpage_row(support.role(node), name):
            continue
        if name in seen:
            continue
        seen.add(name)
        names.append(name)
    return names


# --------------------------------------------------------------------------
# The sweep itself.
# --------------------------------------------------------------------------


def run_sweep(binary: Path, work: Path) -> list[dict[str, Any]]:
    env = build_env(work)
    require_live_session(env)
    findings: list[dict[str, Any]] = []
    seen: set[tuple[str, str, str, str]] = set()

    first_id, _first_name = PANE_ROUTES[0]
    owner = launch(binary, env, pane=first_id)
    try:
        app = support.wait_for(find_settings_app, "rmac-system-settings on the AT-SPI bus", APP_FIND_TIMEOUT_S)

        for pane_id, pane_name in PANE_ROUTES:
            print(f"sweep-settings-errors: visiting {pane_name} ({pane_id})", file=sys.stderr)
            if pane_id != first_id:
                relauncher = launch(binary, env, pane=pane_id)
                relauncher.wait(timeout=RELAUNCH_WAIT_S)
            app = support.wait_for(find_settings_app, f"the {pane_name} pane's window", APP_FIND_TIMEOUT_S)
            sweep_immediate_and_settled(app, pane_name, findings, seen)

            for row_name in STATIC_ROW_SUBPAGES.get(pane_id, []):
                print(f"sweep-settings-errors:   entering {pane_name} > {row_name}", file=sys.stderr)
                if not enter_named_row(app, row_name):
                    print(f"sweep-settings-errors:     row not found, skipping", file=sys.stderr)
                    continue
                app = support.wait_for(find_settings_app, f"{pane_name} > {row_name}", APP_FIND_TIMEOUT_S)
                sweep_immediate_and_settled(app, f"{pane_name} > {row_name}", findings, seen)
                app = reset_to_pane(binary, env, pane_id, pane_name)

            for sub_id, sub_label in STATIC_SUBPAGES.get(pane_id, []):
                print(f"sweep-settings-errors:   entering {pane_name} > {sub_label} (--pane {sub_id})", file=sys.stderr)
                relauncher = launch(binary, env, pane=sub_id)
                relauncher.wait(timeout=RELAUNCH_WAIT_S)
                app = support.wait_for(find_settings_app, f"{pane_name} > {sub_label}", APP_FIND_TIMEOUT_S)
                sweep_immediate_and_settled(app, f"{pane_name} > {sub_label}", findings, seen)

            if pane_id in GENERIC_ROW_SUBPAGES:
                app = reset_to_pane(binary, env, pane_id, pane_name)
                rows = safe_subpage_rows(app)
                print(f"sweep-settings-errors:   {pane_name} generic rows: {rows}", file=sys.stderr)
                for row_name in rows:
                    if not enter_named_row(app, row_name):
                        continue
                    app = support.wait_for(
                        find_settings_app, f"{pane_name} > {row_name}", APP_FIND_TIMEOUT_S
                    )
                    sweep_immediate_and_settled(app, f"{pane_name} > {row_name}", findings, seen)
                    app = reset_to_pane(binary, env, pane_id, pane_name)
    finally:
        close(owner)

    return findings


def enter_named_row(app, row_name: str) -> bool:
    for node in support.nodes_with(app, "list item", row_name):
        if "click" in support.actions(node):
            support.click(node)
            return True
    return False


def reset_to_pane(binary: Path, env: dict[str, str], pane_id: str, pane_name: str):
    relauncher = launch(binary, env, pane=pane_id)
    relauncher.wait(timeout=RELAUNCH_WAIT_S)
    app = support.wait_for(find_settings_app, f"back to {pane_name}", APP_FIND_TIMEOUT_S)
    wait_settle(app)
    return app


def close(owner: "subprocess.Popen[bytes]") -> None:
    if owner.poll() is not None:
        return
    try:
        owner.send_signal(signal.SIGTERM)
        owner.wait(timeout=CLOSE_TIMEOUT_S)
    except subprocess.TimeoutExpired:
        owner.kill()
        try:
            owner.wait(timeout=CLOSE_TIMEOUT_S)
        except subprocess.TimeoutExpired:
            pass
    except ProcessLookupError:
        pass


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--binary", type=Path, required=True, help="path to a freshly built rmac-system-settings")
    parser.add_argument("--output", type=Path, default=None, help="absolute path to write the JSON report to")
    arguments = parser.parse_args()

    if pyatspi is None or support is None:
        parser.exit(3, "sweep-settings-errors: pyatspi is required (this must run on the reference laptop)\n")
    if not arguments.binary.is_file():
        parser.exit(2, f"sweep-settings-errors: {arguments.binary} is not a file\n")

    with tempfile.TemporaryDirectory(prefix="lulo-settings-sweep-") as work_dir:
        try:
            findings = run_sweep(arguments.binary, Path(work_dir))
        except SweepError as error:
            parser.exit(4, f"sweep-settings-errors: {error}\n")

    text = json.dumps(findings, indent=2, sort_keys=True) + "\n"
    if arguments.output is not None:
        arguments.output.write_text(text, encoding="utf-8")
    print(text, end="")

    panes = sorted({finding["pane"] for finding in findings})
    print(
        f"sweep-settings-errors: {len(findings)} finding(s) across {len(panes)} pane(s)/subpage(s)",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
