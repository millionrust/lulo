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
from typing import Optional

import live
import vision

SCALE = 1.25
TOP_BAR_H = 30  # logical px, enough to cover rmac-top-bar's 24-30 pt bar
DOCK_BAND = 90  # logical px from the bottom edge


def find_clusters(image, band: tuple[int, int, int, int], min_gap: int = 6, threshold: int = 40,
                  min_width: int = 6) -> list[tuple[int, int]]:
    """Horizontal runs of columns in `band` (image pixels) that hold
    something other than the band's background: the status icons in the
    top bar, or the icons in the Dock. Returns (left, right) spans."""

    crop = image.crop(band).convert("L")
    width, height = crop.size
    hist = crop.histogram()
    background = max(range(256), key=lambda v: hist[v])
    busy = []
    for x in range(width):
        column = [crop.getpixel((x, y)) for y in range(0, height, 2)]
        busy.append(sum(1 for v in column if abs(v - background) > threshold) >= 2)
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
    spans = find_clusters(image, band, min_gap=12)
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


def dock_menus(robot) -> None:
    _, image = robot.shot("shell-dock")
    width, height = image.size
    band = (0, height - int(DOCK_BAND * SCALE), width, height - 4)
    spans = [s for s in find_clusters(image, band, min_gap=4) if 20 * SCALE <= s[1] - s[0] <= 90 * SCALE]
    robot.step("shell", "dock icons", spans=spans)
    y = (height - int(DOCK_BAND * SCALE) / 2) / SCALE
    for index, (left, right) in enumerate(spans[:16]):
        x = (left + right) / 2 / SCALE
        open_and_dismiss(robot, f"dock-menu-{index}", lambda x=x: robot.input.click(x, y, button="right"), settle=0.6)


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
    for part in (lulo_menu, status_menus, spotlight, notification_and_cc_placeholder, apps_grid, dock_menus,
                 desktop_menu):
        if robot.over_time():
            robot.log("time budget used up in the shell tour")
            return
        part(robot)


def notification_and_cc_placeholder(robot) -> None:
    """Notification Centre and Control Centre open as status items above;
    this hook is where their inner panels (Display, Sound) are walked."""

    return None
