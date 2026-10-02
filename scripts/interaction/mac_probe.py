#!/usr/bin/env python3
"""Record interaction-probe facts on the owner's Mac.

    python3 scripts/interaction/mac_probe.py [SURFACE_ID...]
    python3 scripts/interaction/mac_probe.py --all

Writes tests/interaction/mac/<surface>.json: words, numbers and booleans
only, never a screenshot (docs/behavior-suite.md's rule for the whole suite
applies here too).

Safety: holds /tmp/mac-gui.lock for the whole run (scripts/parallel/run_mac.py's
mkdir lock protocol - never flock, which this macOS has no `flock(1)` for).
Every surface here is opened by clicking exactly the control the owner would
click (a menu-bar title, a menu extra) and is always closed again (Escape)
before the run ends. Hover probes only ever *move* the pointer over a
control; the suite never clicks a control inside a surface (a slider, a
checkbox, a toggle), and never confirms anything destructive.
"""

from __future__ import annotations

import argparse
import datetime
import json
import platform
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Optional

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / "behavior"))
sys.path.insert(0, str(HERE.parent / "parallel"))

import surfaces as sf  # noqa: E402
import mac_pointer  # noqa: E402
import mac_capture  # noqa: E402
from record_mac import Stop, as_string, osascript  # noqa: E402

OBSERVER = HERE / "mac_observe.js"
MAC_CLICK = HERE.parent / "behavior" / "mac_click.py"
LOCK = Path("/tmp/mac-gui.lock")
OUT_DIR = HERE.parent.parent / "tests" / "interaction" / "mac"


def lock_gui() -> None:
    # Match the coordinator's mkdir lock protocol exactly (scripts/parallel/run_mac.py).
    while True:
        try:
            LOCK.mkdir()
            return
        except FileExistsError:
            time.sleep(1)


def unlock_gui() -> None:
    try:
        LOCK.rmdir()
    except OSError:
        pass


def observe(process: str, mode: str, *args: str) -> str:
    result = subprocess.run(
        ["osascript", "-l", "JavaScript", str(OBSERVER), process, mode, *args],
        capture_output=True, text=True, timeout=20,
    )
    if result.returncode != 0:
        raise Stop(f"observer failed ({process} {mode} {args}): {result.stderr.strip()[:200]}")
    return result.stdout.strip()


def click(x: float, y: float) -> None:
    subprocess.run([sys.executable, str(MAC_CLICK), "left", str(x), str(y)], check=True, timeout=10)


def key_escape() -> None:
    osascript("tell application \"System Events\" to key code 53")


def click_menu_bar_item(process: str, title: str) -> None:
    """Open, switch to, or re-toggle a top-level menu-bar pulldown through
    the accessibility `click` action (System Events), not a synthetic
    Quartz click at a point. A raw CGEventPost click reliably opens and
    closes one menu, but does not reproduce the real switch-to-a-neighbour
    interaction (verified live: two Quartz clicks a few hundred ms apart
    just close the first menu without opening the second; the AX `click`
    action switches correctly, the same way record_mac.py already drives
    every other Mac menu)."""

    osascript(
        f'tell application "System Events" to tell process {as_string(process)} to '
        f"click menu bar item {as_string(title)} of menu bar 1"
    )


def menu_bar_background_point() -> tuple[float, float]:
    """A point on the menu bar's own background: never covered by a window
    (the menu bar always draws on top), and clicking it dismisses an open
    menu or popover the same as clicking the Desktop does, without this
    suite having to guess which part of the screen is empty wallpaper."""

    rect = mac_capture.api.CGDisplayBounds(mac_capture.api.CGMainDisplayID())
    return rect.size.width * 0.45, 10.0


def bar_item_bounds(process: str, matcher: str) -> tuple[float, float, float, float]:
    raw = observe(process, "bar_item_bounds", matcher)
    if raw == "none":
        raise Stop(f"no menu-bar item {matcher!r} found on {process}")
    x, y, w, h = (float(v) for v in raw.split())
    return x, y, w, h


def center(box: tuple[float, float, float, float]) -> tuple[float, float]:
    x, y, w, h = box
    return x + w / 2, y + h / 2


def menu_open(process: str, matcher: str) -> bool:
    raw = observe(process, "menu_open", matcher)
    if raw == "unknown":
        raise Stop(f"no menu-bar item {matcher!r} found on {process}")
    return raw == "true"


def window_count(process: str, title: str = "") -> int:
    return int(observe(process, "window_count", title))


def control_bounds(process: str, role: str, description: str) -> Optional[tuple[tuple[float, float, float, float], Optional[float]]]:
    raw = observe(process, "control_bounds", role, description)
    if raw == "none":
        return None
    parts = raw.split()
    x, y, w, h = (float(v) for v in parts[:4])
    value = float(parts[4]) if len(parts) > 4 and parts[4] else None
    return (x, y, w, h), value


def pad_region(box: tuple[float, float, float, float], margin: float) -> tuple[float, float, float, float]:
    x, y, w, h = box
    return (max(0.0, x - margin), max(0.0, y - margin), w + 2 * margin, h + 2 * margin)


def region_changed(before, after, threshold: int = 40) -> bool:
    from PIL import ImageChops

    if before.size != after.size:
        return True
    difference = ImageChops.difference(before, after).convert("L")
    changed_pixels = sum(difference.histogram()[25:])
    return changed_pixels > threshold


# --------------------------------------------------------------------------
# Per-kind probe runners
# --------------------------------------------------------------------------


def run_menu_surface(item: dict[str, Any]) -> dict[str, Any]:
    mac = item["mac"]
    process = mac["process"]
    title_matcher = f"title:{mac['title']}"
    neighbor_matcher = f"title:{mac['neighbor_title']}"
    out: dict[str, Any] = {}

    if process == "Finder":
        osascript('tell application "Finder" to activate')
        time.sleep(0.5)

    def open_menu() -> None:
        click_menu_bar_item(process, mac["title"])
        time.sleep(0.5)
        if not menu_open(process, title_matcher):
            raise Stop(f"{mac['title']!r} menu did not open on {process}")

    def close_safety_net() -> None:
        for _ in range(2):
            if not menu_open(process, title_matcher) and not menu_open(process, neighbor_matcher):
                return
            key_escape()
            time.sleep(0.3)

    try:
        open_menu()
        click(*menu_bar_background_point())
        time.sleep(0.4)
        out["outside_click"] = {"closed": not menu_open(process, title_matcher)}
        close_safety_net()

        open_menu()
        key_escape()
        time.sleep(0.4)
        out["escape"] = {"closed": not menu_open(process, title_matcher)}
        close_safety_net()

        open_menu()
        click_menu_bar_item(process, mac["title"])
        time.sleep(0.4)
        out["reopen_same_title"] = {"closed": not menu_open(process, title_matcher)}
        close_safety_net()

        open_menu()
        click_menu_bar_item(process, mac["neighbor_title"])
        time.sleep(0.4)
        switched = menu_open(process, neighbor_matcher) and not menu_open(process, title_matcher)
        out["switch_neighbor"] = {"switched": switched}
        close_safety_net()
    finally:
        close_safety_net()
    return out


def run_popover_surface(item: dict[str, Any]) -> dict[str, Any]:
    mac = item["mac"]
    process = mac["process"]
    desc_matcher = f"desc:{mac['extra_description']}"
    title = mac["extra_description"]
    out: dict[str, Any] = {}

    def open_popover() -> None:
        click(*center(bar_item_bounds(process, desc_matcher)))
        time.sleep(1.3)  # let the open animation finish before any rest/hover capture
        if window_count(process, title) < 1:
            raise Stop(f"{title!r} did not open a window on {process}")

    def is_open() -> bool:
        return window_count(process, title) >= 1

    def close_safety_net() -> None:
        for _ in range(2):
            if not is_open():
                return
            key_escape()
            time.sleep(0.4)

    try:
        open_popover()
        click(*menu_bar_background_point())
        time.sleep(0.4)
        out["outside_click"] = {"closed": not is_open()}
        close_safety_net()

        open_popover()
        key_escape()
        time.sleep(0.4)
        out["escape"] = {"closed": not is_open()}
        close_safety_net()

        for control in item.get("hover_controls", []):
            label = control["label"]
            open_popover()
            found = control_bounds(process, "AXSlider", control["mac_description"])
            if found is None:
                out[f"hover:{label}"] = {"changed": None}
                close_safety_net()
                continue
            (sx, sy, sw, sh), _value = found
            region = pad_region((sx, sy, sw, sh), margin=14)
            rest = mac_capture.grab(region)
            mac_pointer.move(sx + sw / 2, sy + sh / 2)
            time.sleep(0.5)
            hovered = mac_capture.grab(region)
            out[f"hover:{label}"] = {"changed": region_changed(rest, hovered)}
            close_safety_net()
    finally:
        close_safety_net()
    return out


RUNNERS = {
    "menu": run_menu_surface,
    "popover": run_popover_surface,
}


def record(item: dict[str, Any]) -> dict[str, Any]:
    runner = RUNNERS.get(item["kind"])
    result: dict[str, Any] = {
        "format": 1,
        "surface": item["id"],
        "platform": f"macOS {platform.mac_ver()[0]}",
        "recorded": datetime.date.today().isoformat(),
        "probes": {},
    }
    if runner is None:
        result["error"] = f"no Mac driver for surface kind {item['kind']!r}"
        return result
    try:
        result["probes"] = runner(item)
    except Stop as error:
        result["error"] = str(error)
    return result


def main(argv: Optional[list[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("surfaces", nargs="*", help="surface ids (default: none; see --all)")
    parser.add_argument("--all", action="store_true", help="every surface with a Mac driver")
    parser.add_argument("--dry-run", action="store_true", help="print results instead of writing them")
    args = parser.parse_args(argv)
    if sys.platform != "darwin":
        parser.error("mac_probe.py runs on the owner's Mac")
    if not args.all and not args.surfaces:
        parser.error("name surfaces or pass --all")
    wanted = [item for item in sf.SURFACES if item["status"] == "automated" and item["kind"] in RUNNERS and "mac" in item]
    if not args.all:
        chosen = set(args.surfaces)
        missing = chosen - {item["id"] for item in wanted}
        if missing:
            parser.error(f"unknown or undriven surface id(s): {sorted(missing)}")
        wanted = [item for item in wanted if item["id"] in chosen]
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    lock_gui()
    failures = 0
    try:
        for item in wanted:
            result = record(item)
            if "error" in result:
                failures += 1
                print(f"ERROR {item['id']}: {result['error']}")
            else:
                print(f"ok    {item['id']}: {json.dumps(result['probes'])}")
            if args.dry_run:
                print(json.dumps(result, indent=2))
            else:
                (OUT_DIR / f"{item['id']}.json").write_text(json.dumps(result, indent=2) + "\n")
            time.sleep(0.5)
    finally:
        unlock_gui()
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
