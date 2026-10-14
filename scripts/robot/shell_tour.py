"""The robot's tour of the Lulo shell (docs/behavior-suite.md, "Live robot").

The Lulo menu, the status menus, Control Centre, Spotlight, Notification
Centre, the Apps grid, the Dock's right-click menus and the desktop's
right-click menu. Every surface is opened, screenshotted, checked and
dismissed with Esc; nothing inside them is confirmed, except opening About
This Lulo OS and System Settings from the Lulo menu, which are then closed.

Status items and Dock icons are found on the screenshot (`find_clusters`),
since their positions depend on the hardware and the Dock's contents.
"""

from __future__ import annotations

import time
from pathlib import Path
from typing import Optional

import live
import vision

SCALE = 1.25
TOP_BAR_H = 30  # logical px, enough to cover rmac-top-bar's 24-30 pt bar
DOCK_BAND = 90  # logical px from the bottom edge


def find_clusters(image, band: tuple[int, int, int, int], min_gap: int = 6, threshold: int = 40,
                  min_width: int = 6, glyph: Optional[str] = None) -> list[tuple[int, int]]:
    """Horizontal runs of columns in `band` (image pixels) that hold
    something other than the band's background: the status icons in the
    top bar, or the icons in the Dock. Returns (left, right) spans.

    The top bar is translucent over the wallpaper, whose bright streaks are
    not icons: `glyph="light"` or `"dark"` counts only near-neutral pixels
    of the bar's label colour (Dark or Light appearance) instead."""

    rgb = image.crop(band).convert("RGB")
    crop = rgb.convert("L")
    width, height = crop.size
    hist = crop.histogram()
    background = max(range(256), key=lambda v: hist[v])

    def ink(x: int, y: int) -> bool:
        if glyph is None:
            return abs(crop.getpixel((x, y)) - background) > threshold
        r, g, b = rgb.getpixel((x, y))
        neutral = max(r, g, b) - min(r, g, b) < 40
        return neutral and (min(r, g, b) > 200 if glyph == "light" else max(r, g, b) < 60)

    busy = []
    for x in range(width):
        busy.append(sum(1 for y in range(0, height, 2) if ink(x, y)) >= 2)
    spans: list[tuple[int, int]] = []
    start: Optional[int] = None
    gap = 0
    for x, on in enumerate(busy + [False] * (min_gap + 1)):
        if on:
            if start is None:
                start = x
            gap = 0
            end = x
        elif start is not None:
            gap += 1
            if gap > min_gap:
                if end - start + 1 >= min_width:
                    spans.append((band[0] + start, band[0] + end + 1))
                start = None
    return spans


def layer_names(robot) -> set[str]:
    return {layer.get("namespace", "") for layer in robot.niri.layers()}


def open_and_dismiss(robot, label: str, opener, settle: float = 0.6, dismiss: str = "escape") -> None:
    """Open a surface with `opener`, screenshot and check it, dismiss it,
    then check nothing was left behind."""

    robot.check_session()
    layers_before = layer_names(robot)
    before_path, before = robot.shot(f"shell-before-{label}")
    started = time.monotonic()
    opener()
    time.sleep(settle)
    path, image = robot.shot(f"shell-{label}")
    new_layers = layer_names(robot) - layers_before
    box = vision.changed_box(before, image)
    robot.step("shell", "open", surface=label, layers=sorted(new_layers), box=box,
               seconds=round(time.monotonic() - started, 2))
    if box is None:
        robot.finding("high", "shell", "did-not-open", f"{label} did not open (screen unchanged)", "", path)
    else:
        for problem in vision.low_contrast(image, box) + vision.edge_ink(image, box):
            robot.finding("low", "shell", problem["kind"], f"{label}: {problem['kind']}", problem["detail"], path)
    robot.key(dismiss, 0.6)
    if layer_names(robot) - layers_before:
        robot.key("escape", 0.6)
    after_path, after = robot.shot(f"shell-after-{label}")
    clock = (int(1250 * SCALE), 0, after.size[0], int(TOP_BAR_H * SCALE))
    for problem in vision.residue(before, after, [clock], min_fraction=0.003):
        robot.finding("medium", "shell", "residue", f"{label} left pixels behind after Esc", problem["detail"], after_path)
    still = layer_names(robot) - layers_before
    if still:
        robot.finding("medium", "shell", "would-not-close", f"{label} still open after Esc", f"layers {sorted(still)}", after_path)


def lulo_menu(robot) -> None:
    def walk():
        robot.key("ctrl-f2", 0.4)
        robot.key("down", 0.4)
        for _ in range(4):
            robot.key("down", 0.15)
        robot.key("right", 0.5)  # Recent Items
        robot.shot("shell-lulo-recent-items")
        robot.key("left", 0.2)
        for _ in range(6):
            robot.key("down", 0.12)

    open_and_dismiss(robot, "lulo-menu", walk, dismiss="escape")
    robot.key("escape", 0.3)
    # About This Lulo OS: the menu's first row.
    known = {w["id"] for w in robot.niri.windows()}
    robot.key("ctrl-f2", 0.4)
    robot.key("down", 0.4)
    robot.key("down", 0.2)
    robot.key("return", 0.2)
    window = None
    end = time.monotonic() + 5
    while window is None and time.monotonic() < end:
        window = next((w for w in robot.niri.windows() if w["id"] not in known), None)
        time.sleep(0.05)
    if window is None:
        path, _ = robot.shot("shell-about-missing")
        robot.finding("high", "shell", "no-window", "About This Lulo OS opened no window", "", path)
        robot.key("escape", 0.3)
        return
    time.sleep(0.8)
    robot.window_shot("shell", "about-this-lulo", window)
    robot.close_window("shell", window, "About This Lulo OS")


def status_menus(robot) -> None:
    _, image = robot.shot("shell-top-bar")
    width = image.size[0]
    band = (width // 2, 2, width - 2, int(TOP_BAR_H * SCALE) - 2)
    glyph = "light" if "dark" in robot.scheme_start else "dark"
    spans = [s for s in find_clusters(image, band, min_gap=12, glyph=glyph) if s[1] - s[0] >= 10]
    robot.step("shell", "status items", spans=spans)
    if not spans:
        robot.finding("medium", "shell", "robot", "found no status items in the top bar")
        return
    # Rightmost is the clock (Notification Centre); the rest are status
    # items, Control Centre among them.
    for index, (left, right) in enumerate(reversed(spans[-8:])):
        x = (left + right) / 2 / SCALE
        y = TOP_BAR_H / 2
        label = "notification-centre" if index == 0 else f"status-{index}"
        open_and_dismiss(robot, label, lambda x=x, y=y: robot.input.click(x, y), settle=0.8)


def spotlight(robot) -> None:
    for query in ("calc", "2+2*3", "weather", "notes", "dark mode", "what time is it in tokyo", "open terminal"):
        def type_query(query=query):
            robot.key("cmd-space", 0.6)
            robot.key("type:" + query)
            time.sleep(0.8)
        open_and_dismiss(robot, f"spotlight-{query}", type_query, settle=0.3)


DOCK_ITEMS = 12  # pinned apps, recent apps and the Bin; extra presses wrap


def dock_menus(robot) -> None:
    """Every Dock item's menu, reached the keyboard way (⌃F3, Right to the
    item, Up opens its menu: `rmac_dock::keyboard`), since the Dock's
    translucent material makes its icons hard to find on a screenshot.
    The first item also gets a real right-click."""

    for index in range(DOCK_ITEMS):
        def open_menu(index=index):
            robot.key("ctrl-f3", 0.5)
            for _ in range(index):
                robot.key("right", 0.12)
            robot.key("up", 0.3)
        open_and_dismiss(robot, f"dock-menu-{index}", open_menu, settle=0.5)
        robot.key("escape", 0.3)  # leave the Dock's keyboard mode
    _, image = robot.shot("shell-dock")
    width, height = image.size
    x = width / 2 / SCALE
    y = (height - int(DOCK_BAND * SCALE) / 2) / SCALE
    open_and_dismiss(robot, "dock-right-click", lambda: robot.input.click(x, y, button="right"), settle=0.6)


def desktop_menu(robot) -> None:
    if robot.niri.windows():
        robot.step("shell", "desktop menu skipped: windows are open")
        return
    open_and_dismiss(robot, "desktop-menu", lambda: robot.input.click(700, 420, button="right"), settle=0.6)


def apps_grid(robot) -> None:
    def open_grid():
        robot.niri.spawn(["/usr/bin/rmac-app-drawer"])
        time.sleep(1.0)
        robot.key("type:cal", 0.2)
        time.sleep(0.5)
    open_and_dismiss(robot, "apps-grid", open_grid, settle=0.4)
    for window in robot.niri.windows():
        if window.get("app_id") == "org.rmac.AppDrawer":
            robot.close_window("shell", window, "Apps grid")


def tour(robot) -> None:
    robot.current_app = "shell"
    for part in (lulo_menu, status_menus, spotlight, control_centre_panels, apps_grid, dock_menus,
                 desktop_menu):
        if robot.over_time():
            robot.log("time budget used up in the shell tour")
            return
        part(robot)


# Control Centre's Display and Sound modules open their detail panels from
# the round button at the right end of each module, measured from the
# panel's own top-right corner (image pixels, 1.25x) on the reference
# laptop's build. The click only happens if that spot really is the light
# round button, never a slider (which would change brightness or volume).
# Control Centre's Display and Sound modules open their detail panels from
# the round button at the right end of each module. The robot scans the
# column those buttons share (this far in from the panel's right edge, in
# image pixels at 1.25x) for the button pattern, so a taller or reordered
# panel still works; the last two buttons found are Display, then Sound.
# It only ever clicks a spot that matches the button, never a slider
# (which would change brightness or volume).
CC_BUTTON_FROM_RIGHT = 59
QUICK_SETTINGS = ["/usr/libexec/rmac/rmac-shortcut-dispatch", "quick-settings"]


def is_module_button(luma, x: int, y: int) -> bool:
    """A white glyph on a light, translucent disc. A slider end or the
    sun/speaker icon has darker module material beside it."""

    try:
        centre, left, right = (luma.getpixel((x + dx, y)) for dx in (0, -10, 10))
    except IndexError:
        return False
    return centre >= 230 and 130 <= left <= 235 and 130 <= right <= 235


def module_buttons(image, box) -> list[tuple[int, int]]:
    """Centres of the module buttons in Control Centre's right column."""

    luma = image.convert("L")
    x = box[2] - CC_BUTTON_FROM_RIGHT
    centres: list[tuple[int, int]] = []
    run: list[int] = []
    for y in range(box[1], box[3]):
        if is_module_button(luma, x, y):
            run.append(y)
        elif run:
            centres.append((x, (run[0] + run[-1]) // 2))
            run = []
    return centres


def quick_settings_monitors() -> int:
    """`pw-dump --monitor` processes Control Centre is running."""

    parents = set(live.pids_of("rmac-quick-settings"))
    count = 0
    for pid in live.pids_of("pw-dump"):
        try:
            ppid = int(Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[1])
        except (OSError, ValueError, IndexError):
            continue
        count += ppid in parents
    return count


def control_centre_panels(robot) -> None:
    monitors_before = quick_settings_monitors()
    for index, name in enumerate(("display", "sound")):
        robot.check_session()
        _, before = robot.shot(f"shell-before-cc-{name}")
        robot.niri.spawn(QUICK_SETTINGS)
        time.sleep(1.0)
        _, opened = robot.shot(f"shell-cc-for-{name}")
        box = vision.changed_box(before, opened)
        if box is None:
            robot.finding("high", "shell", "did-not-open", "Control Centre did not open", "")
            return
        buttons = module_buttons(opened, box)
        if len(buttons) < 2:
            robot.step("shell", "cc panel button not found", panel=name, buttons=buttons)
            robot.key("escape", 0.5)
            continue
        x, y = buttons[-2 + index]
        volume, brightness = live.volume(robot.env), live.brightness()
        robot.input.click(x / SCALE, y / SCALE)
        time.sleep(0.8)
        path, panel = robot.shot(f"shell-cc-{name}-panel")
        changed = vision.changed_fraction(opened, panel, box)
        robot.step("shell", "cc panel", panel=name, changed=round(changed, 4), at=(x, y))
        if changed < 0.01:
            robot.finding("high", "shell", "no-effect", f"Control Centre {name} button opened nothing", "", path)
        for problem in vision.low_contrast(panel, box) + vision.edge_ink(panel, box):
            robot.finding("low", "shell", problem["kind"], f"Control Centre {name}: {problem['kind']}",
                          problem["detail"], path)
        robot.key("escape", 0.5)
        if any(n.startswith("rmac-quick-settings") for n in layer_names(robot)):
            robot.key("escape", 0.5)
        if live.volume(robot.env) != volume or live.brightness() != brightness:
            robot.finding("high", "shell", "state", f"opening Control Centre {name} changed volume or brightness",
                          f"{volume}/{brightness} -> {live.volume(robot.env)}/{live.brightness()}", path)
    time.sleep(1.0)
    monitors_after = quick_settings_monitors()
    robot.step("shell", "cc monitors", before=monitors_before, after=monitors_after)
    if monitors_after > monitors_before:
        robot.finding("high", "shell", "process-leak", "Control Centre leaves a pw-dump monitor running per open",
                      f"{monitors_before} -> {monitors_after} after two opens and closes")
