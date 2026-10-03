#!/usr/bin/env python3
"""Outside-click dismissal checks for every top-bar menu/popover, in a nested niri.

    python3 scripts/behavior/run_menu_dismiss.py --niri PATH --bin-dir DIR [--keep]

--bin-dir must hold this branch's `top-bar`, `dock`, `wallpaper`,
`rmac-quick-settings` and `rmac-shortcut-dispatch` (the root workspace's and
shell's `cargo build --profile iterate` output share one CARGO_TARGET_DIR on
the laptop, so one directory has all of them).

The bar's existing transparent surface spans the display. While a dropdown
is open, its input region accepts clicks across that surface and the root
handler closes the menu. The other popovers use
`rmac_ui::open_outside_click_catcher_around`.

Scenarios, each starting from a clean (all-closed) state:

  - a click on the Dock (never keyboard-interactive) closes an open app menu;
  - a click on the wallpaper below the bar's own band closes an open status
    menu;
  - a click on the wallpaper *inside* the bar's own band, away from the
    dropdown, closes an open app menu (the input-region widening fix,
    distinct from the separate catcher surface above);
  - a click on another app's real window (a dummy `foot` window) closes an
    open app menu;
  - clicking a different top-bar title while one menu is open switches to
    the other instead of just closing (macOS behaviour, `TopBar::open_menu`'s
    existing `stop_propagation`);
  - Escape closes an open menu;
  - opening Control Center alongside an open app menu, then clicking the
    wallpaper, closes both (checked with `grim` + a pixel-difference crop
    over Control Center's corner, since it is a layer-shell popover with no
    niri "window" entry and no accessible control labels in this build to
    search for by name).

Isolation is run_lulo.py's: a private dbus-run-session, a temporary HOME and
XDG_RUNTIME_DIR, wayland-0/wayland-1 held so no socket can collide with the
live session, and no input is sent anywhere but this run's own Sway.
"""

from __future__ import annotations

import argparse
import fcntl
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from PIL import Image, ImageChops

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
sys.path.insert(0, str(HERE))

import run_lulo  # noqa: E402
import wlinput  # noqa: E402

LAVAPIPE = "/usr/share/vulkan/icd.d/lvp_icd.json"
OUTPUT_W, OUTPUT_H = 1440, 900
# A stable point below all top-bar dropdowns, also used to distinguish the
# two outside-click locations exercised by this scenario.
MENU_SURFACE_HEIGHT = 680
# Top-right corner big enough to contain Control Center's popover regardless
# of its exact margins (`crates/rmac-quick-settings/src/surface.rs`).
CONTROL_CENTER_BOX = (OUTPUT_W - 420, 0, OUTPUT_W, 420)


class Run:
    def __init__(self, args: argparse.Namespace, work: Path) -> None:
        self.args = args
        self.env = dict(os.environ)
        run_lulo.refuse_live_session(self.env)
        self.runtime = Path(self.env["XDG_RUNTIME_DIR"])
        self.out = work / "logs"
        self.out.mkdir(exist_ok=True)
        self.work = work
        self.children: list[subprocess.Popen] = []
        self.results: list[tuple[str, bool, str]] = []

    def check(self, name: str, ok, detail: str = "") -> None:
        self.results.append((name, bool(ok), detail))
        print(
            f"{'PASS' if ok else 'FAIL'} {name} {detail if not ok else ''}".rstrip(),
            flush=True,
        )
        if not ok and hasattr(self, "keys"):
            identifier = f"failure-{len(self.results):03d}"
            try:
                self.capture(identifier)
                (self.work / f"{identifier}-layers.json").write_text(
                    json.dumps(self.niri("layers"), indent=2), encoding="utf-8"
                )
            except Exception as error:  # noqa: BLE001
                print(f"debug capture failed: {error}", flush=True)

    def spawn(self, argv: list[str], name: str, extra: dict[str, str] | None = None) -> subprocess.Popen:
        env = {**self.env, **(extra or {})}
        process = subprocess.Popen(argv, env=env, stdout=open(self.out / f"{name}.log", "a"),
                                   stderr=subprocess.STDOUT, close_fds=True)
        self.children.append(process)
        return process

    @staticmethod
    def wait_for(predicate, timeout: float = 30, step: float = 0.2):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                value = predicate()
            except Exception:  # noqa: BLE001
                value = None
            if value:
                return value
            time.sleep(step)
        return None

    def retry_until(self, action, condition, attempts: int = 4, step: float = 2.0) -> bool:
        for _ in range(attempts):
            action()
            if self.wait_for(condition, step):
                return True
        return bool(condition())

    # -- compositor ----------------------------------------------------------

    def start(self) -> None:
        self.locks = []
        for taken in ("wayland-0.lock", "wayland-1.lock"):
            handle = open(self.runtime / taken, "w")
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.locks.append(handle)
        sway_conf = self.out / "sway.conf"
        sway_conf.write_text("xwayland disable\ndefault_border none\n"
                             f"output HEADLESS-1 mode {OUTPUT_W}x{OUTPUT_H} position 0 0\n")
        self.spawn(["sway", "--unsupported-gpu", "--config", str(sway_conf)], "sway",
                   {"WLR_BACKENDS": "headless", "WLR_HEADLESS_OUTPUTS": "1",
                    "WLR_LIBINPUT_NO_DEVICES": "1", "WLR_RENDERER": "pixman"})
        self.sway_display = self.wait_for(lambda: next(
            (p.name for p in self.runtime.glob("wayland-*") if not p.name.endswith(".lock")), None))
        if not self.sway_display:
            raise SystemExit("sway did not start")

        shell = (REPO / "packaging/rmac-session/shell.kdl").read_text(encoding="utf-8")
        bins = Path(self.args.bin_dir)
        config = self.out / "niri.kdl"
        config.write_text(
            shell.replace("/usr/libexec/rmac/rmac-mission-control", str(bins / "mission-control"))
                 .replace("/usr/libexec/rmac/rmac-dock", str(bins / "dock"))
        )
        validate = subprocess.run([self.args.niri, "validate", "-c", str(config)], env=self.env,
                                  capture_output=True, text=True)
        self.check("niri validate accepts shell.kdl", validate.returncode == 0,
                   "" if validate.returncode == 0 else validate.stderr[-300:])

        before = {p.name for p in self.runtime.glob("wayland-*")}
        self.spawn([self.args.niri, "-c", str(config)], "niri",
                   {"WAYLAND_DISPLAY": self.sway_display, "LIBGL_ALWAYS_SOFTWARE": "1"})
        socket = self.wait_for(lambda: next(iter(self.runtime.glob("niri.*.sock")), None))
        display = self.wait_for(lambda: next(
            (p.name for p in self.runtime.glob("wayland-*")
             if not p.name.endswith(".lock") and p.name not in before), None))
        if not (socket and display):
            raise SystemExit("niri did not start")
        self.env.update({"WAYLAND_DISPLAY": display, "NIRI_SOCKET": str(socket)})

        self.dock = self.spawn([str(bins / "dock")], "dock", {"VK_ICD_FILENAMES": LAVAPIPE})
        self.top_bar = self.spawn([str(bins / "top-bar")], "top-bar", {"VK_ICD_FILENAMES": LAVAPIPE})
        self.wallpaper = self.spawn([str(bins / "wallpaper")], "wallpaper", {"VK_ICD_FILENAMES": LAVAPIPE})
        self.quick_settings = self.spawn(
            [str(bins / "rmac-quick-settings")], "quick-settings", {"VK_ICD_FILENAMES": LAVAPIPE}
        )
        self.launcher = self.spawn([str(bins / "rmac-launcher")], "launcher",
                                   {"VK_ICD_FILENAMES": LAVAPIPE})
        self.app_drawer = self.spawn([str(bins / "rmac-app-drawer"), "--service"], "app-drawer",
                                     {"VK_ICD_FILENAMES": LAVAPIPE})
        self.notification_center = self.spawn(
            [str(bins / "rmac-notification-center-panel")], "notification-center",
            {"VK_ICD_FILENAMES": LAVAPIPE},
        )
        self.dispatch_bin = str(bins / "rmac-shortcut-dispatch")
        subprocess.run(["busctl", "--user", "set-property", "org.a11y.Bus", "/org/a11y/bus",
                        "org.a11y.Status", "IsEnabled", "b", "true"],
                       env=self.env, capture_output=True, timeout=10, check=False)
        time.sleep(6)
        self.keys = wlinput.Wayland({**self.env, "WAYLAND_DISPLAY": self.sway_display,
                                     "RMAC_BEHAVIOR_NESTED": "1"})

    def niri(self, *request: str):
        result = subprocess.run([self.args.niri, "msg", "-j", *request], env=self.env,
                                capture_output=True, text=True, timeout=10)
        return json.loads(result.stdout) if result.stdout.strip() else None

    def dispatch(self, shortcut: str):
        return subprocess.run([self.dispatch_bin, shortcut], env=self.env, capture_output=True,
                              text=True, timeout=10, check=False)

    def has_layer(self, namespace: str) -> bool:
        def namespaces(value):
            if isinstance(value, dict):
                if isinstance(value.get("namespace"), str):
                    yield value["namespace"]
                for child in value.values():
                    yield from namespaces(child)
            elif isinstance(value, list):
                for child in value:
                    yield from namespaces(child)

        return namespace in set(namespaces(self.niri("layers")))

    def popover_gone(self, namespace: str) -> bool:
        return not self.has_layer(namespace) and not self.has_layer(f"{namespace}-click-catcher")

    # -- AT-SPI ----------------------------------------------------------

    def find_node(self, roles: tuple[str, ...], matches):
        import pyatspi

        desktop = pyatspi.Registry.getDesktop(0)
        stack = [desktop.getChildAtIndex(i) for i in range(desktop.childCount)]
        while stack:
            node = stack.pop()
            try:
                if node is None:
                    continue
                if node.getRoleName() in roles and matches(node.name or ""):
                    return node
                stack.extend(node.getChildAtIndex(i) for i in range(node.childCount))
            except Exception:  # noqa: BLE001
                continue
        return None

    def find_button(self, label: str):
        return self.find_node(("push button", "button"), lambda name: name == label)

    def find_menu_item(self, prefix: str):
        return self.find_node(("menu item",), lambda name: name.startswith(prefix))

    def find_menu(self, label_prefix: str):
        return self.find_node(("menu",), lambda name: name.startswith(label_prefix))

    def extents(self, node) -> tuple[int, int, int, int] | None:
        import pyatspi

        try:
            box = node.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
            return box.x, box.y, box.width, box.height
        except Exception:  # noqa: BLE001
            return None

    # -- real input ------------------------------------------------------

    def click_node(self, node) -> bool:
        """A real pointer click at `node`'s centre — this is what actually
        grants an on-demand layer-shell surface real keyboard focus, the way
        clicking the menu does for a mouse user."""

        box = self.extents(node)
        if not box:
            return False
        x, y, w, h = box
        return self.click_at(x + w / 2, y + h / 2)

    def click_at(self, x: float, y: float, button: str = "left") -> bool:
        self.keys.move(x, y, OUTPUT_W, OUTPUT_H)
        time.sleep(0.3)
        self.keys.button(True, button)
        time.sleep(0.03)
        self.keys.button(False, button)
        time.sleep(0.2)
        return True

    def click_button(self, label: str) -> bool:
        button = self.find_button(label)
        return button is not None and self.click_node(button)

    def click_menu_item(self, prefix: str) -> bool:
        item = self.find_menu_item(prefix)
        return item is not None and self.click_node(item)

    def open_system_menu(self) -> bool:
        """A real click on the logo, opening the Lulo menu the same way a
        mouse user would."""

        logo = self.wait_for(lambda: self.find_button("menu"), 10, 0.3)
        if logo is None:
            return False
        return self.click_button("menu")

    def close_everything(self) -> None:
        self.keys.key("escape")
        self.keys.key("escape")
        time.sleep(0.2)
        for shortcut, namespace in (("quick-settings", "rmac-quick-settings"),
                                    ("launcher", "rmac-launcher"),
                                    ("app-drawer", "rmac-app-drawer"),
                                    ("notification-center", "rmac-notification-center")):
            if self.has_layer(namespace):
                self.dispatch(shortcut)
            self.wait_for(lambda namespace=namespace: self.popover_gone(namespace), 5)

    def capture(self, name: str) -> Image.Image:
        path = self.work / f"{name}.png"
        result = subprocess.run(["grim", str(path)], env=self.env,
                                capture_output=True, text=True, timeout=15)
        if result.returncode:
            raise RuntimeError(f"grim {name}: {result.stderr[-300:]}")
        return Image.open(path).convert("RGB")

    @staticmethod
    def changed_pixels(before: Image.Image, after: Image.Image, box: tuple[int, int, int, int]) -> int:
        difference = ImageChops.difference(before.crop(box), after.crop(box)).convert("L")
        return sum(difference.histogram()[21:])

    # -- scenarios ---------------------------------------------------------

    def dock_click_closes_app_menu(self) -> None:
        self.close_everything()
        opened = self.retry_until(self.open_system_menu, lambda: self.find_menu_item("About"))
        self.check("Dock click: the Lulo menu opens first", opened)
        if not opened:
            return
        # The Dock's own shelf is never keyboard-interactive, so a blur
        # callback cannot be relied on here. This is deliberately one click
        # right after the first menu opens, with no retry or warm-up.
        # Aim at a Dock tile, not the shelf's transparent bottom border.
        self.click_at(OUTPUT_W / 2, OUTPUT_H - 48)
        closed = self.wait_for(lambda: self.find_menu_item("About") is None, 10)
        self.check("Dock click: closes the open Lulo menu", closed)

    def wallpaper_click_below_band_closes_status_menu(self) -> None:
        self.close_everything()
        opened = self.retry_until(
            lambda: self.click_at(OUTPUT_W - 60, 14),  # the Wi-Fi/Sound status cluster, right of the clock
            lambda: self.find_node(("menu", "list"), lambda name: True) is not None,
        )
        # Fall back to the System menu if no status item could be clicked
        # (status availability depends on hardware present in the sandbox);
        # the input-region fix is the same code path for both.
        if not opened:
            opened = self.retry_until(self.open_system_menu, lambda: self.find_menu_item("About"))
            marker = lambda: self.find_menu_item("About") is not None  # noqa: E731
        else:
            marker = lambda: self.find_node(("menu", "list"), lambda name: True) is not None  # noqa: E731
        self.check("Wallpaper click below the bar: a menu opens first", opened)
        if not opened:
            return
        closed = self.retry_until(
            lambda: self.click_at(200, MENU_SURFACE_HEIGHT + 70),
            lambda: not marker(),
        )
        self.check("Wallpaper click below the bar's own band: closes the open menu", closed)

    def wallpaper_click_inside_band_closes_app_menu(self) -> None:
        """The bar's own surface spans the whole `MENU_SURFACE_HEIGHT` band;
        before the fix its *input region* only covered the bar strip plus
        the open dropdown, so a click on the wallpaper peeking through the
        rest of that band fell through to whatever was behind it instead of
        reaching the bar's own click-to-close handler."""

        self.close_everything()
        opened = self.retry_until(self.open_system_menu, lambda: self.find_menu_item("About"))
        self.check("Wallpaper click inside the bar's band: the Lulo menu opens first", opened)
        if not opened:
            return
        # Far right, well clear of the Lulo menu's own dropdown (anchored at
        # the left under the logo) but still above MENU_SURFACE_HEIGHT.
        closed = self.retry_until(
            lambda: self.click_at(OUTPUT_W - 40, MENU_SURFACE_HEIGHT - 40),
            lambda: self.find_menu_item("About") is None,
        )
        self.check("Wallpaper click inside the bar's band: closes the open menu", closed)

    def other_window_click_closes_app_menu(self) -> None:
        foot = shutil.which("foot", path=self.env.get("PATH"))
        if foot is None:
            self.check("Other window click: closes the open menu", False, "foot is required")
            return
        dummy = self.spawn([foot, "--app-id=org.rmac.MenuDismissProbe", "sleep", "60"], "dummy-window")
        focused = self.wait_for(
            lambda: next(
                (item for item in self.niri("windows") or []
                 if item.get("app_id") == "org.rmac.MenuDismissProbe" and item.get("is_focused")),
                None,
            ),
            10,
            0.2,
        )
        try:
            if focused is None:
                self.check("Other window click: closes the open menu", False,
                           "the dummy window never became focused")
                return
            self.close_everything()
            opened = self.retry_until(self.open_system_menu, lambda: self.find_menu_item("About"))
            self.check("Other window click: the Lulo menu opens first", opened)
            if not opened:
                return
            # The lone tiled window fills the working area below the bar.
            closed = self.retry_until(
                lambda: self.click_at(OUTPUT_W / 2, OUTPUT_H / 2),
                lambda: self.find_menu_item("About") is None,
            )
            self.check("Other window click: closes the open menu", closed)
        finally:
            dummy.terminate()
            try:
                dummy.wait(timeout=5)
            except subprocess.TimeoutExpired:
                dummy.kill()
                dummy.wait(timeout=3)
            self.wait_for(lambda: not any(
                item.get("app_id") == "org.rmac.MenuDismissProbe"
                for item in self.niri("windows") or []
            ), 10)

    def escape_closes_app_menu(self) -> None:
        self.close_everything()
        opened = self.retry_until(self.open_system_menu, lambda: self.find_menu_item("About"))
        self.check("Escape: the Lulo menu opens first", opened)
        if not opened:
            return
        self.keys.key("escape")
        closed = self.wait_for(lambda: self.find_menu_item("About") is None, 10)
        self.check("Escape: closes the open Lulo menu", closed)

    def clicking_another_title_switches_menus(self) -> None:
        """macOS: clicking a different top-bar title while one menu is open
        switches straight to the other instead of just closing the first."""

        self.close_everything()
        opened = self.retry_until(self.open_system_menu, lambda: self.find_menu_item("About"))
        self.check("Title switch: the Lulo menu opens first", opened)
        if not opened:
            return
        # Index 1: the active app's own title, right of the logo
        # (`TopBar::keyboard_titles`'s "{app} menu" label). With nothing
        # focused this is the desktop's ("Finder"-equivalent) title; the
        # logo itself is named exactly "menu", so excluding that finds it
        # without guessing a pixel offset.
        title = self.wait_for(
            lambda: self.find_node(
                ("push button", "button"), lambda name: name != "menu" and name.endswith(" menu")
            ),
            10,
            0.3,
        )
        self.check("Title switch: the active app's own title is present", title is not None)
        if title is None:
            return
        title_name = title.name
        clicked = self.retry_until(
            lambda: self.click_node(
                self.find_node(("push button", "button"), lambda name: name == title_name)
            ),
            lambda: self.find_menu_item("About") is None
            and self.find_node(("menu",), lambda name: name == title_name) is not None,
        )
        self.check(
            "Title switch: clicking the next title switches straight to its own menu "
            "instead of just closing the first",
            clicked,
        )

    def clicking_same_title_keeps_menu(self) -> None:
        self.close_everything()
        opened = self.retry_until(self.open_system_menu, lambda: self.find_menu_item("About"))
        self.check("Same title: the Lulo menu opens first", opened)
        if opened:
            self.click_button("menu")
            self.check("Same title: a second click keeps the menu open",
                       self.find_menu_item("About") is not None)

    def status_menu_dismissal(self, label: str) -> None:
        captures = {"Wi-Fi": "wifi", "Bluetooth": "bluetooth", "Sound": "sound"}
        for method in ("outside click", "Escape"):
            self.close_everything()
            title = self.wait_for(lambda: self.find_node(
                ("push button", "button"), lambda name: name.startswith(label)), 3)
            if title is not None:
                opened = self.retry_until(lambda: self.click_node(title),
                                          lambda: self.find_menu(label) is not None)
            else:
                # Sound and Bluetooth extras are hidden by default, just as
                # on the Mac. Reopen the same top-bar binary with its built-in
                # capture setting to exercise their actual dropdown paths.
                self.top_bar.terminate()
                self.top_bar.wait(timeout=5)
                self.top_bar = self.spawn([str(Path(self.args.bin_dir) / "top-bar")],
                                          "top-bar-captured", {
                                              "VK_ICD_FILENAMES": LAVAPIPE,
                                              "RMAC_CAPTURE_STATUS_MENU": captures[label],
                                          })
                opened = self.wait_for(lambda: self.find_menu(label) is not None, 10)
            self.check(f"{label}: opens for {method}", opened)
            if not opened:
                continue
            if method == "Escape":
                menu = self.find_menu(label)
                if menu is not None:
                    box = self.extents(menu)
                    if box:
                        self.click_at(box[0] + 2, box[1] + 2)
                self.keys.key("escape")
            else:
                self.click_at(200, MENU_SURFACE_HEIGHT + 70)
            self.check(f"{label}: closes on {method}",
                       self.wait_for(lambda: self.find_menu(label) is None, 10))

    def layer_popover_dismissal(self, shortcut: str, namespace: str) -> None:
        for method in ("outside click", "inside-band click", "Escape"):
            self.close_everything()
            self.dispatch(shortcut)
            opened = self.wait_for(
                lambda: self.has_layer(namespace)
                and self.has_layer(f"{namespace}-click-catcher"), 10
            )
            if not opened and self.popover_gone(namespace):
                # Activation may race the prior window's final removal in a
                # rapid scripted sequence. Retry only from a closed state so
                # a late first open cannot be toggled shut.
                self.dispatch(shortcut)
                opened = self.wait_for(
                    lambda: self.has_layer(namespace)
                    and self.has_layer(f"{namespace}-click-catcher"), 10
                )
            self.check(f"{shortcut}: opens for {method}", opened)
            if not opened:
                continue
            # Both layer surfaces can be listed before the catcher's first
            # input-region commit. Let that frame present before input.
            time.sleep(0.5)
            if method == "Escape":
                self.keys.key("escape")
            else:
                self.click_at(200, 300 if method == "inside-band click" else MENU_SURFACE_HEIGHT + 70)
            closed = self.wait_for(lambda: self.popover_gone(namespace), 10)
            self.check(f"{shortcut}: closes on {method}", closed)
            if not closed:
                self.dispatch(shortcut)
                self.wait_for(lambda: self.popover_gone(namespace), 5)

    def clock_popover_dismissal(self) -> None:
        namespace = "rmac-notification-center"
        for method in ("outside click", "inside-band click", "Escape"):
            self.close_everything()
            clock = self.wait_for(lambda: self.find_node(
                ("push button", "button"),
                lambda name: name.startswith("Date and time:")
            ), 5)
            opened = clock is not None and self.click_node(clock) and self.wait_for(
                lambda: self.has_layer(namespace)
                and self.has_layer(f"{namespace}-click-catcher"), 10
            )
            if not opened and self.popover_gone(namespace):
                clock = self.find_node(
                    ("push button", "button"),
                    lambda name: name.startswith("Date and time:")
                )
                opened = clock is not None and self.click_node(clock) and self.wait_for(
                    lambda: self.has_layer(namespace)
                    and self.has_layer(f"{namespace}-click-catcher"), 10
                )
            self.check(f"clock/date popover: opens for {method}", opened)
            if not opened:
                continue
            time.sleep(0.5)
            if method == "Escape":
                self.keys.key("escape")
            else:
                self.click_at(200, 300 if method == "inside-band click" else MENU_SURFACE_HEIGHT + 70)
            closed = self.wait_for(lambda: self.popover_gone(namespace), 10)
            self.check(f"clock/date popover: closes on {method}", closed)
            if not closed:
                self.dispatch("notification-center")
                self.wait_for(lambda: self.popover_gone(namespace), 5)

    def dock_context_menu_dismissal(self) -> None:
        tile = self.wait_for(lambda: self.find_node(
            ("push button", "button"),
            lambda name: name.startswith("Files") and not name.endswith(" menu")), 5)
        self.check("Dock context menu: Files tile available", tile is not None)
        if tile is None:
            return
        for method in ("outside click", "Escape"):
            self.close_everything()
            self.check(f"Dock context menu: keyboard surface absent before {method}",
                       not self.has_layer("rmac-dock-menu-keyboard"))
            tile = self.wait_for(lambda: self.find_node(
                ("push button", "button"),
                lambda name: name.startswith("Files") and not name.endswith(" menu")
            ), 5)
            box = self.extents(tile) if tile is not None else None
            if box is None:
                self.check(f"Dock context menu: tile has bounds for {method}", False)
                continue
            x, y, w, h = box
            # GPUI's AT-SPI Y extent can shift when niri's work area changes
            # after the dummy window closes. The private output is fixed at
            # 900 px and the Dock shelf stays against its bottom edge.
            self.click_at(x + w / 2, OUTPUT_H - 48, "right")
            opened = self.wait_for(lambda: self.find_menu("Files") is not None, 5)
            self.check(f"Dock context menu: opens for {method}", opened, f"tile={box}")
            if not opened:
                continue
            self.check(f"Dock context menu: keyboard surface opens for {method}",
                       self.wait_for(lambda: self.has_layer("rmac-dock-menu-keyboard"), 5))
            if method == "Escape":
                self.keys.key("escape")
            else:
                self.click_at(200, MENU_SURFACE_HEIGHT + 70)
            closed = self.wait_for(lambda: self.find_menu("Files") is None, 10)
            self.check(f"Dock context menu: closes on {method}", closed)
            self.check(f"Dock context menu: keyboard surface closes on {method}",
                       self.wait_for(lambda: not self.has_layer("rmac-dock-menu-keyboard"), 5))
            if not closed:
                self.dock.terminate()
                self.dock.wait(timeout=5)
                self.dock = self.spawn([str(Path(self.args.bin_dir) / "dock")], "dock-restarted",
                                       {"VK_ICD_FILENAMES": LAVAPIPE})
                self.wait_for(lambda: self.find_menu("Files") is None, 5)

        foot = shutil.which("foot", path=self.env.get("PATH"))
        if foot is None:
            self.check("Dock focus: probe app available", False, "foot is required")
            return
        probe_id = "org.rmac.DockFocusProbe"
        probe = self.spawn([foot, f"--app-id={probe_id}", "sleep", "60"], "dock-focus-probe")
        focused = lambda: any(
            item.get("app_id") == probe_id and item.get("is_focused")
            for item in self.niri("windows") or []
        )
        try:
            frontmost = self.wait_for(focused, 10)
            self.check("Dock focus: probe app is frontmost", frontmost)
            if not frontmost:
                return
            tile = self.find_node(("push button", "button"),
                                  lambda name: name.startswith("Files") and not name.endswith(" menu"))
            box = self.extents(tile) if tile is not None else None
            if box is None:
                self.check("Dock focus: Files tile has bounds", False)
                return
            x, _, w, _ = box
            # A new toplevel can consume the nested virtual pointer's first
            # press while pointer focus transfers to the Dock layer.
            opened = self.retry_until(
                lambda: self.click_at(x + w / 2, OUTPUT_H - 48, "right"),
                lambda: self.find_menu("Files") is not None
                and self.has_layer("rmac-dock-menu-keyboard"),
                attempts=2,
                step=2.0,
            )
            self.check("Dock focus: context menu takes Escape", opened)
            if not opened:
                return
            self.keys.key("escape")
            closed = self.wait_for(lambda: self.find_menu("Files") is None, 5)
            self.check("Dock focus: Escape closes the menu", closed)
            self.check("Dock focus: Escape returns to the frontmost app",
                       self.wait_for(focused, 5))
        finally:
            probe.terminate()
            try:
                probe.wait(timeout=5)
            except subprocess.TimeoutExpired:
                probe.kill()
                probe.wait(timeout=3)

    def control_center_and_app_menu_close_on_wallpaper_click(self) -> None:
        self.close_everything()
        # Nothing has touched Control Center before this scenario, so this
        # is a true "closed" baseline — unlike opening-then-closing it just
        # to capture one, which leaves `quick-settings`'s own dismiss
        # animation (if any) or a stray frame in the shot instead.
        baseline = self.capture("cc-closed")

        opened_menu = self.retry_until(self.open_system_menu, lambda: self.find_menu_item("About"))
        self.check("Control Center: the Lulo menu opens first", opened_menu)

        opened_pixels = 0
        opened = baseline
        namespace = "rmac-quick-settings"
        layer_open = False
        for _ in range(4):
            if self.popover_gone(namespace):
                self.dispatch("quick-settings")
            layer_open = self.wait_for(
                lambda: self.has_layer(namespace)
                and self.has_layer(f"{namespace}-click-catcher"), 3
            )
            if layer_open:
                time.sleep(0.5)
                layer_open = self.has_layer(namespace)
            if layer_open:
                break
        for _ in range(4 if layer_open else 0):
            time.sleep(0.5)
            opened = self.capture("cc-open")
            opened_pixels = self.changed_pixels(baseline, opened, CONTROL_CENTER_BOX)
            if opened_pixels > 500:
                break
        self.check("Control Center: opening it changes its corner of the screen",
                   opened_pixels > 500, f"changed={opened_pixels}")

        menu_closed = self.retry_until(
            lambda: self.click_at(200, MENU_SURFACE_HEIGHT + 70),
            lambda: self.find_menu_item("About") is None,
        )
        time.sleep(0.6)
        after = self.capture("cc-dismissed")
        self.check("Control Center: wallpaper click also closes the Lulo menu", menu_closed)
        after_pixels = self.changed_pixels(baseline, after, CONTROL_CENTER_BOX)
        self.check(
            "Control Center: wallpaper click closes Control Center too",
            after_pixels < opened_pixels / 2,
            f"opened_changed={opened_pixels}, after_changed={after_pixels}",
        )

    def run(self) -> int:
        self.start()
        if self.args.only:
            if self.args.only == "topbar":
                self.dock_click_closes_app_menu()
                self.clicking_same_title_keeps_menu()
            elif self.args.only == "status":
                for label in ("Wi-Fi", "Bluetooth", "Sound"):
                    self.status_menu_dismissal(label)
            elif self.args.only == "dock":
                self.dock_context_menu_dismissal()
            elif self.args.only == "notification-center":
                self.clock_popover_dismissal()
            elif self.args.only == "combined":
                self.control_center_and_app_menu_close_on_wallpaper_click()
            else:
                namespaces = {
                    "quick-settings": "rmac-quick-settings",
                    "launcher": "rmac-launcher",
                    "app-drawer": "rmac-app-drawer",
                    "notification-center": "rmac-notification-center",
                }
                self.layer_popover_dismissal(self.args.only, namespaces[self.args.only])
            return self.finish()
        self.dock_click_closes_app_menu()
        self.wallpaper_click_inside_band_closes_app_menu()
        self.wallpaper_click_below_band_closes_status_menu()
        self.escape_closes_app_menu()
        self.other_window_click_closes_app_menu()
        self.clicking_another_title_switches_menus()
        self.clicking_same_title_keeps_menu()
        for label in ("Wi-Fi", "Bluetooth", "Sound"):
            self.status_menu_dismissal(label)
        self.dock_context_menu_dismissal()
        for shortcut, namespace in (("quick-settings", "rmac-quick-settings"),
                                    ("launcher", "rmac-launcher"),
                                    ("app-drawer", "rmac-app-drawer")):
            self.layer_popover_dismissal(shortcut, namespace)
        self.clock_popover_dismissal()
        self.control_center_and_app_menu_close_on_wallpaper_click()
        # Log Out ends this run's own nested niri for real; nothing after
        # this point runs.
        self.close_everything()
        return self.finish()

    def finish(self) -> int:
        try:
            self.keys.close()
        except Exception:  # noqa: BLE001
            pass
        for process in reversed(self.children):
            if process.poll() is None:
                process.terminate()
        for process in reversed(self.children):
            try:
                process.wait(5)
            except subprocess.TimeoutExpired:
                process.kill()
        failed = [result for result in self.results if not result[1]]
        print(f"\n{len(self.results) - len(failed)}/{len(self.results)} checks passed", flush=True)
        return 1 if failed else 0


def outer(args: argparse.Namespace, argv: list[str]) -> int:
    for tool in ("sway", "grim", "dbus-run-session", "busctl", "foot"):
        if subprocess.run(["which", tool], capture_output=True).returncode != 0:
            raise SystemExit(f"{tool} is required")
    work = Path(tempfile.mkdtemp(prefix="lulo-menu-dismiss-"))
    try:
        env = run_lulo.isolated_environment(work)
        run_lulo.refuse_live_session(env)
        for key in ("WLR_BACKENDS", "WLR_HEADLESS_OUTPUTS", "WLR_LIBINPUT_NO_DEVICES", "WLR_RENDERER",
                    "LIBGL_ALWAYS_SOFTWARE", "VK_ICD_FILENAMES"):
            env.pop(key, None)
        services = work / "dbus-services"
        services.mkdir()
        bus = Path("/usr/share/dbus-1/services/org.a11y.Bus.service")
        if bus.exists():
            (services / bus.name).write_text(bus.read_text())
        config = work / "session.conf"
        config.write_text(
            "<!DOCTYPE busconfig PUBLIC \"-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN\"\n"
            " \"http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd\">\n"
            f"<busconfig><type>session</type><listen>unix:dir={work}</listen><auth>EXTERNAL</auth>"
            f"<servicedir>{services}</servicedir>"
            "<policy context=\"default\"><allow send_destination=\"*\" eavesdrop=\"true\"/>"
            "<allow eavesdrop=\"true\"/><allow own=\"*\"/></policy></busconfig>\n"
        )
        (work / "logs").mkdir(exist_ok=True)
        command = ["dbus-run-session", f"--config-file={config}", "--", sys.executable,
                   str(Path(__file__).resolve()), "--inner", str(work), *argv]
        with open(work / "logs" / "session.log", "w") as log:
            result = subprocess.call(command, env=env, close_fds=True, stderr=log)
        return result
    finally:
        if run_lulo.reap(work / "runtime"):
            time.sleep(1.0)
            run_lulo.reap(work / "runtime")
        if args.keep:
            print(f"kept {work}", file=sys.stderr)
        else:
            run_lulo.remove_tree(work)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--niri", default="/usr/bin/niri")
    parser.add_argument("--bin-dir", help="directory with this branch's top-bar, dock, wallpaper, "
                                          "rmac-quick-settings and rmac-shortcut-dispatch")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--only", choices=("topbar", "status", "dock", "quick-settings",
                                           "launcher", "app-drawer", "notification-center",
                                           "combined"))
    parser.add_argument("--inner", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if not args.bin_dir:
        parser.error("--bin-dir is required")
    if args.inner:
        return Run(args, args.inner).run()
    argv = [a for a in sys.argv[1:] if a != "--keep"]
    return outer(args, argv)


if __name__ == "__main__":
    sys.exit(main())
