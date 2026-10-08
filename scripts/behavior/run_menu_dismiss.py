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
  - Escape hands the keyboard back to the window the menu opened over, so
    it stays key and its shortcuts keep working (UIA-14);
  - holding ⌥ while Finder's Application menu is open swaps Empty Bin…
    for its hidden alternate, Empty Bin, in the same row, reverting the
    instant ⌥ is released (UIA-22);
  - Control Centre's Display and Sound titles open their detail views
    (Display: brightness and Dark Mode; Sound: the fake outputs), choosing
    an output switches the default device, and Esc returns to the grid;
  - Control Centre's Sound view with 1 (the reference laptop's real
    `pw-dump`), 0 and 2 outputs, opened by real clicks on the title, the
    empty space and the output button, with the slider, the Output list
    and Sound Settings… (`--audio real`: a private PipeWire instead);
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

import fake_audio  # noqa: E402
import fake_hardware  # noqa: E402
import real_audio  # noqa: E402
import run_lulo  # noqa: E402
import wlinput  # noqa: E402

LAVAPIPE = "/usr/share/vulkan/icd.d/lvp_icd.json"
# The top bar's leftmost item (the Lulo mark). Its accessible name is
# "Lulo menu" (shell/bins/rmac-menubar/src/main.rs `LULO_MENU_LABEL`), so
# Orca announces what it opens rather than a bare "menu".
LULO_MENU = "Lulo menu"
OUTPUT_W, OUTPUT_H = 1440, 900
# A stable point below all top-bar dropdowns, also used to distinguish the
# two outside-click locations exercised by this scenario.
MENU_SURFACE_HEIGHT = 680
# Top-right corner big enough to contain Control Center's popover regardless
# of its exact margins (`crates/rmac-quick-settings/src/surface.rs`).
CONTROL_CENTER_BOX = (OUTPUT_W - 420, 0, OUTPUT_W, 420)
# Control Centre's surface (crates/rmac-quick-settings/src/surface.rs): 316
# wide, 8 from the right edge, 6 below the 29 pt menu bar.
CONTROL_CENTRE_WIDTH, CONTROL_CENTRE_RIGHT, CONTROL_CENTRE_TOP = 316, 8, 35
# `--audio real`'s null sinks: the first is the one-output case.
REAL_SINKS = [("lulo_null_speakers", "Lulo Null Speakers"), ("lulo_null_hdmi", "Lulo Null HDMI")]


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

    def check(self, name: str, ok, detail: str = "") -> bool:
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
        return bool(ok)

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

        self.real_audio = None
        if self.args.audio == "real":
            # Before any Lulo process starts, so each one finds the private
            # sound server the way a fresh login does.
            self.real_audio = real_audio.RealAudio(self.env, self.work)
            self.real_audio.start()
            self.real_audio.set_sinks(REAL_SINKS[:1])

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

    def dump_nodes(self, root_name: str) -> list[tuple[str, str]]:
        """(role, name) of every node under the first node named
        `root_name`, for a failing check's log."""
        import pyatspi

        root = self.find_node(("panel", "group", "filler", "frame"), lambda name: name == root_name)
        found = []
        stack = [root] if root is not None else []
        while stack:
            node = stack.pop()
            try:
                found.append((node.getRoleName(), node.name or ""))
                stack.extend(node.getChildAtIndex(i) for i in range(node.childCount))
            except Exception:  # noqa: BLE001
                continue
        return found

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

        logo = self.wait_for(lambda: self.find_button(LULO_MENU), 10, 0.3)
        if logo is None:
            return False
        return self.click_button(LULO_MENU)

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

    def escape_returns_focus_to_window(self) -> None:
        """UIA-14: macOS keeps the app window key while a menu is open and
        after Esc closes it. The bar's layer surface takes the keyboard
        while a menu is open; Esc must give it back to the window."""

        foot = shutil.which("foot", path=self.env.get("PATH"))
        if foot is None:
            self.check("Escape focus: probe window available", False, "foot is required")
            return
        probe_id = "org.rmac.MenuFocusProbe"
        probe = self.spawn([foot, f"--app-id={probe_id}", "sleep", "60"], "menu-focus-probe")
        focused = lambda: any(
            item.get("app_id") == probe_id and item.get("is_focused")
            for item in self.niri("windows") or []
        )
        try:
            frontmost = self.wait_for(focused, 10)
            self.check("Escape focus: probe window is focused first", frontmost)
            if not frontmost:
                return
            self.close_everything()
            for label, opener, item in (
                ("Lulo menu", self.open_system_menu, lambda: self.find_menu_item("About")),
                ("Wi-Fi menu",
                 lambda: (lambda node: node is not None and self.click_node(node))(
                     self.find_node(("push button", "button"),
                                    lambda name: name.startswith("Wi-Fi"))),
                 lambda: self.find_menu("Wi-Fi")),
            ):
                opened = self.retry_until(opener, lambda: item() is not None)
                self.check(f"Escape focus: the {label} opens", opened)
                if not opened:
                    continue
                took = self.wait_for(lambda: not focused(), 5)
                self.check(f"Escape focus: the {label} holds the keyboard while open", took)
                self.keys.key("escape")
                closed = self.wait_for(lambda: item() is None, 5)
                self.check(f"Escape focus: Escape closes the {label}", closed)
                self.check(f"Escape focus: Escape returns the keyboard from the {label}",
                           self.wait_for(focused, 10))
        finally:
            probe.terminate()
            try:
                probe.wait(timeout=5)
            except subprocess.TimeoutExpired:
                probe.kill()
                probe.wait(timeout=3)
            self.wait_for(lambda: not any(
                item.get("app_id") == probe_id for item in self.niri("windows") or []
            ), 10)

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
        # logo itself is named LULO_MENU, so excluding that finds it
        # without guessing a pixel offset.
        title = self.wait_for(
            lambda: self.find_node(
                ("push button", "button"), lambda name: name != LULO_MENU and name.endswith(" menu")
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
            self.click_button(LULO_MENU)
            self.check("Same title: a second click keeps the menu open",
                       self.find_menu_item("About") is not None)

    def option_alternate_swaps_in_open_app_menu(self) -> None:
        """UIA-22: Finder's Application menu shows "Empty Bin…"; AppKit
        replaces it in place with its ⌥ alternate, "Empty Bin", for as
        long as Option is held, reverting the instant it's released. No
        Files window is open in this harness, so the top bar falls back to
        Files' own static menu definition (`static_fallback_menus`), which
        already carries the pair — `crates/rmac-app-menu/src/lib.rs`'s
        `FILES_MENUS`. Exercises `TopBar::app_menu_option`
        (`on_modifiers_changed`) and `menu_model::displayed_items`.
        """

        def exact(label: str):
            return self.find_node(("menu item",), lambda name: name == label)

        self.close_everything()
        # Index 1, specifically: the active app's own bold title
        # (`format!("{} menu", bar.active_app)`, `main.rs`), not any of the
        # exported File/Edit/View/… titles beside it — those also end with
        # " menu", so a generic suffix match (as
        # `clicking_another_title_switches_menus` uses, where it genuinely
        # doesn't matter which title it finds) can land on the wrong one.
        # With nothing focused, the desktop default is Files
        # (`static_fallback_menus`, `app_display_name(FILES)` == "Files"),
        # whose Application menu — not File/Edit/etc. — carries Empty
        # Bin…/Empty Bin.
        title = self.wait_for(lambda: self.find_button("Files menu"), 10, 0.3)
        self.check("⌥ alternate: the Files application title is present", title is not None)
        if title is None:
            return
        opened = self.retry_until(
            lambda: self.click_node(self.find_button("Files menu")),
            lambda: exact("Empty Bin…") is not None,
        )
        self.check("⌥ alternate: the Application menu opens with Empty Bin…", opened)
        if not opened:
            return
        # `.hold("alt-f5")`, not a bare modifier: F5 is tapped and released
        # immediately (harmless — nothing in an open app menu binds it) so
        # the modifier state change rides along with a real key event, the
        # same proven path `.hold("cmd-tab")` already uses elsewhere.
        self.keys.hold("alt-f5")
        try:
            swapped = self.wait_for(
                lambda: exact("Empty Bin") is not None and exact("Empty Bin…") is None,
                5,
                0.2,
            )
            self.check(
                "⌥ alternate: holding ⌥ swaps Empty Bin… to Empty Bin in the same slot",
                swapped,
            )
        finally:
            self.keys.release("alt-f5")
        reverted = self.wait_for(
            lambda: exact("Empty Bin…") is not None and exact("Empty Bin") is None,
            5,
            0.2,
        )
        self.check("⌥ alternate: releasing ⌥ reverts the row to Empty Bin…", reverted)
        self.close_everything()

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
            # input-region commit, so a dismiss input sent right away can
            # still land before the catcher accepts it. This is worst on
            # the very first popover of each kind opened in the whole run
            # (this one, for "quick-settings"): a cold shader/JIT cost that
            # "inside-band click" and "Escape" right after it, and every
            # later shortcut's own first open, do not pay again. GitHub's
            # shared CI runners are also markedly slower at this
            # software-rendered first frame ("ZINK: failed to choose
            # pdev" in this nested session's own logs) than the reference
            # laptop, where three quick retries were enough but CI still
            # saw this fail outright. Escape and an outside click only
            # ever dismiss, never toggle back open, so retrying either is
            # safe; give it generous headroom instead of guessing a
            # single delay.
            time.sleep(0.5)

            def dismiss_input() -> None:
                if method == "Escape":
                    self.keys.key("escape")
                else:
                    self.click_at(200, 300 if method == "inside-band click" else MENU_SURFACE_HEIGHT + 70)

            closed = self.retry_until(dismiss_input, lambda: self.popover_gone(namespace), attempts=6, step=3.0)
            self.check(f"{shortcut}: closes on {method}", closed)
            if not closed:
                self.dispatch(shortcut)
                self.wait_for(lambda: self.popover_gone(namespace), 5)

    def launcher_expanded_row_press(self) -> None:
        """SPOT catcher audit: Spotlight's layer surface opens at its
        expanded size from the start (`view.rs`'s `set_compact`), and
        typing a query widens the window's own input region to match,
        while the outside-click catcher opened around the *compact* 88 pt
        bar only. Spotlight's own window maps after the catcher and so
        far has always claimed a real press inside its current input
        region first regardless of the catcher's hole, but this presses
        and holds a result row well below that 88 pt anyway, as a
        regression guard: releasing without moving confirms the row was
        really hit, not just left alone by accident."""

        import pyatspi

        namespace = "rmac-launcher"
        # GPUI reports a layer surface's AT-SPI extents from the surface's
        # own corner, not the desktop's (the same quirk `on_screen()`
        # works around for Control Centre): Spotlight's surface
        # (`rmac_launcher::surface`) is 672 pt wide, centred, with its top
        # `top_margin` below the output's own top edge.
        launcher_left = (OUTPUT_W - 672) / 2
        launcher_top = round((OUTPUT_H - 576) / 2 - 16)

        def on_screen(box):
            x, y, w, h = box
            return (x + launcher_left, y + launcher_top, w, h)

        self.close_everything()
        self.dispatch("launcher")
        opened = self.wait_for(
            lambda: self.has_layer(namespace) and self.has_layer(f"{namespace}-click-catcher"), 10
        )
        self.check("Spotlight expanded: opens", opened)
        if not opened:
            return
        self.keys.type_text("e")
        roles = ("push button", "button", "menu item", "list item")

        def rows():
            desktop = pyatspi.Registry.getDesktop(0)
            found = []
            stack = [desktop.getChildAtIndex(i) for i in range(desktop.childCount)]
            while stack:
                node = stack.pop()
                try:
                    if node is None:
                        continue
                    if node.getRoleName() in roles and (node.name or "").strip():
                        box = self.extents(node)
                        if box is not None:
                            x, y, w, h = on_screen(box)
                            # A result row (measured ~56 pt tall), not the
                            # top bar's items (22 pt, y < 30) or the Dock's
                            # tiles and separators (64 pt, against the
                            # bottom edge).
                            if 45 <= h <= 60 and 60 <= y <= OUTPUT_H - 60:
                                found.append((node, (x, y, w, h)))
                    stack.extend(node.getChildAtIndex(i) for i in range(node.childCount))
                except Exception:  # noqa: BLE001
                    continue
            return found

        candidates = self.wait_for(lambda: rows() or None, 5) or []
        self.check("Spotlight expanded: typing \"e\" shows at least one result row",
                   bool(candidates), f"{len(candidates)} candidates")
        if not candidates:
            self.close_everything()
            return
        # Three quarters down the expanded surface's own span
        # (`launcher_top` to `launcher_top + 608`): clear of the compact
        # bar's 88 pt hole, and clear too of a *vertically centred* 88 pt
        # hole (`centered_bounds`, the output's own centre +/- 44 pt) --
        # another plausible but wrong position for it -- so this cannot
        # coincidentally land inside a stale hole by sheer luck, while
        # still certain to be within the visible list rather than a row
        # the accessibility tree lays out below what the (clipped,
        # `max_h`) results card actually shows.
        target = launcher_top + 608 * 0.75
        ordered = sorted(candidates, key=lambda item: abs(item[1][1] - target))
        row, (x, y, w, h) = ordered[0]
        print(f"Spotlight result row on screen: name={row.name!r} box=({x}, {y}, {w}, {h})", flush=True)
        self.capture("launcher-expanded-before-press")
        cx, cy = x + w / 2, y + h / 2
        self.keys.move(cx, cy, OUTPUT_W, OUTPUT_H)
        time.sleep(0.3)
        self.keys.button(True, "left")
        time.sleep(0.3)
        still_open = self.has_layer(namespace)
        self.check(
            "Spotlight expanded: pressing a result row does not dismiss Spotlight",
            still_open, f"row={row.name!r} at ({cx:.0f}, {cy:.0f})",
        )
        self.keys.button(False, "left")
        time.sleep(0.5)
        # The release completes an ordinary click, which activates the
        # row (launching whatever it names) and closes Spotlight on its
        # own -- a legitimate dismiss, not the catcher's.
        self.close_everything()

    def fake_hardware_shows_real_data(self) -> None:
        """With fake_hardware.py's NetworkManager/BlueZ/UPower mocks
        running (docs/behavior-suite.md), the top bar and Control Centre
        should show that laptop's actual state instead of "unavailable":
        a battery indicator, three Wi-Fi networks with Casa Lulo
        connected, and a paired Bluetooth headset. Every step also checks
        the status-bar/quick-settings processes are still alive, since
        this is the realistic hardware state the real Control Centre
        crash (2026-10-03) needed and no empty-hardware nested run ever
        exercised."""

        self.close_everything()
        battery = self.wait_for(lambda: self.find_node(
            ("push button", "button", "label"), lambda name: name.startswith("Battery 80%")), 10)
        self.check("Top bar: battery indicator shows the fake 80%, discharging battery",
                   battery is not None)

        self.dispatch("quick-settings")
        opened = self.wait_for(lambda: self.has_layer("rmac-quick-settings"), 10)
        self.check("Control Centre: opens with fake hardware running", opened)
        if not opened:
            return

        def open_detail(label: str) -> bool:
            button = self.wait_for(lambda: self.find_node(
                ("push button", "button"), lambda name: name == f"{label} details"), 10)
            if button is None:
                return False
            try:
                button.queryAction().doAction(0)
            except Exception:  # noqa: BLE001
                return False
            return True

        wifi_opened = open_detail("Wi-Fi")
        self.check("Control Centre: Wi-Fi details opens without crashing",
                   wifi_opened and self.quick_settings.poll() is None)
        if wifi_opened:
            network = self.wait_for(lambda: self.find_node(
                ("push button", "button", "menu item", "list item", "label"),
                lambda name: "Casa Lulo" in name), 10)
            self.check("Control Centre: Wi-Fi list shows the fake connected network",
                       network is not None)
            self.keys.key("escape")
            self.wait_for(lambda: not self.has_layer("rmac-quick-settings")
                          or self.find_node(("push button", "button"),
                                            lambda name: name == "Wi-Fi details") is not None, 5)

        if not self.has_layer("rmac-quick-settings"):
            self.dispatch("quick-settings")
            self.wait_for(lambda: self.has_layer("rmac-quick-settings"), 10)
        bluetooth_opened = open_detail("Bluetooth")
        self.check("Control Centre: Bluetooth details opens without crashing",
                   bluetooth_opened and self.quick_settings.poll() is None)
        if bluetooth_opened:
            device = self.wait_for(lambda: self.find_node(
                ("push button", "button", "menu item", "list item", "label"),
                lambda name: "Lulo Headphones" in name), 10)
            self.check("Control Centre: Bluetooth list shows the fake paired device",
                       device is not None)
            self.keys.key("escape")

        wifi_toggle = self.wait_for(lambda: self.find_node(
            ("push button", "button", "switch", "toggle button"), lambda name: name == "Wi-Fi"), 5)
        toggled = False
        if wifi_toggle is not None:
            try:
                wifi_toggle.queryAction().doAction(0)
                time.sleep(0.3)
                wifi_toggle.queryAction().doAction(0)
                toggled = self.quick_settings.poll() is None
            except Exception:  # noqa: BLE001
                toggled = False
        self.check("Control Centre: the Wi-Fi toggle switches without crashing", toggled)
        self.close_everything()

    def control_centre_detail_panels(self) -> None:
        """Display and Sound expand into detail views inside Control
        Centre, as on macOS 26: each module's title opens its view, the
        view is announced as a named group with its controls, choosing an
        output in Sound switches the default device (fake_audio.py's
        PipeWire stand-ins), and Esc returns to the grid."""

        namespace = "rmac-quick-settings"
        items = ("push button", "button", "menu item", "list item", "label")
        switches = ("toggle button", "switch", "check box", "push button", "button")
        state_path = Path(self.env.get(fake_audio.STATE_VARIABLE, ""))

        def open_panel() -> bool:
            if not self.has_layer(namespace):
                self.dispatch("quick-settings")
            return bool(self.wait_for(lambda: self.has_layer(namespace), 10))

        def press(label: str) -> bool:
            node = self.wait_for(lambda: self.find_node(("push button", "button"),
                                                        lambda name: name == label), 10)
            if node is None:
                return False
            try:
                node.queryAction().doAction(0)
            except Exception:  # noqa: BLE001
                return False
            return True

        def showing(roles, label: str) -> bool:
            return self.find_node(roles, lambda name: name == label) is not None

        def back_to_grid(settings_label: str, title: str) -> None:
            self.keys.key("escape")
            back = self.wait_for(lambda: not showing(items, settings_label)
                                 and showing(("push button", "button"), f"{title} details"), 5)
            self.check(f"Control Centre {title}: Esc returns to the grid",
                       back and self.has_layer(namespace))

        self.close_everything()
        self.check("Control Centre detail: opens", open_panel())
        if os.environ.get("LULO_FAKE_SYS_ROOT"):
            opened = press("Display details")
            listed = opened and self.wait_for(lambda: showing(items, "Display Settings\u2026"), 10)
            self.check("Control Centre Display: the title opens the Display view", listed)
            if listed:
                panel = self.find_node(("panel", "group", "filler"), lambda name: name == "Display")
                self.check("Control Centre Display: the view is a named group", panel is not None)
                self.check("Control Centre Display: the brightness slider is in the view",
                           self.find_node(("slider",), lambda name: name == "Display") is not None)
                self.check("Control Centre Display: Dark Mode is a switch",
                           showing(switches, "Dark Mode"))
                self.check("Control Centre Display: no Night Shift or True Tone without a backend",
                           not showing(switches, "Night Shift") and not showing(switches, "True Tone"))
                self.capture("control-centre-display-detail")
                back_to_grid("Display Settings\u2026", "Display")
        else:
            print("SKIP Control Centre Display view: no fake backlight in this session", flush=True)

        if not open_panel():
            return
        opened = press("Sound details")
        listed = opened and self.wait_for(lambda: showing(items, "Sound Settings\u2026"), 10)
        self.check("Control Centre Sound: the title opens the Sound view", listed)
        if not listed:
            self.close_everything()
            return
        speakers = self.wait_for(lambda: self.find_node(
            items, lambda name: name == fake_audio.SPEAKERS["description"]), 10)
        hdmi = self.find_node(items, lambda name: name == fake_audio.HDMI["description"])
        self.check("Control Centre Sound: the Output list shows both fake devices",
                   speakers is not None and hdmi is not None)
        selected = False
        try:
            import pyatspi
            selected = speakers is not None and speakers.getState().contains(pyatspi.STATE_SELECTED)
        except Exception:  # noqa: BLE001
            selected = False
        self.check("Control Centre Sound: the default output is marked selected", selected)
        self.capture("control-centre-sound-detail")
        switched = False
        if hdmi is not None:
            try:
                hdmi.queryAction().doAction(0)
                switched = bool(self.wait_for(
                    lambda: json.loads(state_path.read_text())["default"] == fake_audio.HDMI["name"],
                    10))
            except Exception:  # noqa: BLE001
                switched = False
        self.check("Control Centre Sound: choosing an output switches the default device", switched)
        if switched:
            def hdmi_selected() -> bool:
                import pyatspi
                node = self.find_node(items, lambda name: name == fake_audio.HDMI["description"])
                return node is not None and node.getState().contains(pyatspi.STATE_SELECTED)
            self.check("Control Centre Sound: the list follows the new default",
                       self.wait_for(hdmi_selected, 10))
        back_to_grid("Sound Settings\u2026", "Sound")
        self.close_everything()

    def control_centre_sound_outputs(self) -> None:
        """Control Centre's Sound view with 1, 0 and 2 outputs, opened with
        real pointer clicks, as on macOS 26: the title, the module's empty
        space and the output button each open it; it always shows the
        volume slider, an "Output" heading with every output listed and the
        current one selected (even when it is the only one), or "No Output
        Device" when there is none, then "Sound Settings…"; clicking the
        slider changes the volume. The 1-output graph is the reference
        laptop's real `pw-dump` (fake_audio.py "laptop")."""

        import pyatspi

        namespace = "rmac-quick-settings"
        items = ("push button", "button", "menu item", "list item", "label")
        state_path = Path(self.env.get(fake_audio.STATE_VARIABLE, ""))
        settings = "Sound Settings…"

        def showing(roles, label: str):
            return self.find_node(roles, lambda name: name == label)

        def open_panel() -> bool:
            if not self.has_layer(namespace):
                self.dispatch("quick-settings")
            return bool(self.wait_for(lambda: self.has_layer(namespace)
                                      and showing(("push button", "button"), "Sound details"), 10))

        def on_screen(box):
            """GPUI reports a layer surface's AT-SPI extents from the
            surface's own corner; Control Centre hangs CONTROL_CENTRE_TOP
            below the screen top, CONTROL_CENTRE_RIGHT in from its right
            edge."""
            if box is None:
                return None
            x, y, w, h = box
            return (x + OUTPUT_W - CONTROL_CENTRE_RIGHT - CONTROL_CENTRE_WIDTH,
                    y + CONTROL_CENTRE_TOP, w, h)

        def module_box():
            node = showing(("push button", "button"), "Sound details")
            return on_screen(self.extents(node)) if node is not None else None

        def in_grid() -> bool:
            return showing(items, settings) is None and showing(
                ("push button", "button"), "Sound details") is not None

        def back() -> None:
            self.keys.key("escape")
            self.wait_for(in_grid, 5)

        def state() -> dict:
            return json.loads(state_path.read_text())

        audio = getattr(self, "real_audio", None)

        def apply(graph_name: str) -> None:
            if audio is None:
                fake_audio.set_graph(state_path, graph_name)
            else:
                audio.set_sinks({"laptop": REAL_SINKS[:1], "none": [], "pair": REAL_SINKS}[graph_name])

        def current_output() -> str | None:
            if audio is None:
                current = state()["default"]
                return next((sink["description"] for sink in fake_audio.sinks(state())
                             if sink["name"] == current), None)
            current = audio.default_sink()
            return next((description for name, description in REAL_SINKS if name == current), None)

        def volume() -> float | None:
            if audio is None:
                current = state()["default"]
                return state()["volume"].get(current) if current else None
            return audio.volume()

        if audio is None:
            cases = (
                ("laptop", "1 output (real laptop graph)", [fake_audio.LAPTOP_SINK["description"]]),
                ("none", "0 outputs", []),
                ("pair", "2 outputs", [fake_audio.SPEAKERS["description"],
                                       fake_audio.HDMI["description"]]),
            )
        else:
            cases = (
                ("laptop", "1 output (real PipeWire)", [REAL_SINKS[0][1]]),
                ("none", "0 outputs (real PipeWire)", []),
                ("pair", "2 outputs (real PipeWire)", [description for _, description in REAL_SINKS]),
            )
        for graph_name, label, outputs in cases:
            self.close_everything()
            apply(graph_name)
            time.sleep(1.0)
            if not self.check(f"Control Centre Sound, {label}: Control Centre opens", open_panel()):
                continue
            # The module reflects the graph before anything is clicked.
            expect_enabled = bool(outputs)
            self.wait_for(lambda: (showing(("slider",), "Sound") is not None
                                   and showing(("slider",), "Sound").getState().contains(
                                       pyatspi.STATE_FOCUSABLE)), 5)
            time.sleep(0.5)
            grid = f"control-centre-sound-grid-{graph_name}{'-pipewire' if audio else ''}"
            self.capture(grid)
            self.keep_capture(grid)
            box = module_box()
            print(f"Sound module extents ({graph_name}): {box}", flush=True)
            if box is None:
                self.check(f"Control Centre Sound, {label}: the module has screen extents", False)
                continue
            x, y, w, h = box
            outputs_button = showing(("push button", "button"), "Sound Outputs")
            accessory = on_screen(self.extents(outputs_button)) if outputs_button is not None else None
            targets = (
                ("the title", (x + 30, y + 23)),
                ("the empty space", (x + w * 0.55, y + 16)),
                ("the output button", (accessory[0] + accessory[2] / 2, accessory[1] + accessory[3] / 2)
                 if accessory else (x + 265, y + 42)),
            )
            for where, (cx, cy) in targets:
                if not in_grid():
                    self.close_everything()
                    open_panel()
                self.click_at(cx, cy)
                opened = self.wait_for(lambda: showing(items, settings) is not None, 5)
                self.check(f"Control Centre Sound, {label}: clicking {where} opens the Sound view",
                           opened, f"at ({cx:.0f}, {cy:.0f})")
                if opened and where != "the output button":
                    back()
            if showing(items, settings) is None:
                # Carry on with the view's own checks after a failed click.
                self.close_everything()
                open_panel()
                time.sleep(0.5)
                self.click_at(x + 30, y + 23)
                self.wait_for(lambda: showing(items, settings) is not None, 5)
            if showing(items, settings) is None:
                continue
            time.sleep(0.5)
            slider = showing(("slider",), "Sound")
            self.check(f"Control Centre Sound, {label}: the view has the volume slider",
                       slider is not None)
            self.check(f"Control Centre Sound, {label}: the slider is "
                       f"{'enabled' if expect_enabled else 'disabled without an output'}",
                       slider is not None and slider.getState().contains(pyatspi.STATE_FOCUSABLE)
                       == expect_enabled)
            if outputs:
                heading = showing(("heading", "label", "static"), "Output")
                self.check(f"Control Centre Sound, {label}: the \"Output\" heading is shown",
                           heading is not None)
                rows = [showing(items, name) for name in outputs]
                self.check(f"Control Centre Sound, {label}: every output is listed",
                           all(row is not None for row in rows), str(outputs))
                current_name = current_output()
                selected = [row.name for row in rows
                            if row is not None and row.getState().contains(pyatspi.STATE_SELECTED)]
                self.check(f"Control Centre Sound, {label}: the current output is selected",
                           current_name is not None and selected == [current_name],
                           f"selected={selected}, current={current_name}")
            else:
                empty = showing(("label", "static", "heading"), "No Output Device")
                if empty is None:
                    print("DEBUG nodes:", self.dump_nodes("Sound"), flush=True)
                self.check(f"Control Centre Sound, {label}: \"No Output Device\" is shown",
                           empty is not None)
                self.check(f"Control Centre Sound, {label}: no Output heading without an output",
                           showing(("heading", "label", "static"), "Output") is None)
            self.check(f"Control Centre Sound, {label}: \"Sound Settings…\" is shown",
                       showing(items, settings) is not None)
            capture = f"control-centre-sound-{graph_name}{'-pipewire' if audio else ''}"
            self.capture(capture)
            self.keep_capture(capture)
            if expect_enabled and slider is not None:
                sx, sy, sw, sh = on_screen(self.extents(slider)) or (0, 0, 0, 0)
                # The hit box has 8 pt of padding on either end of the track.
                self.click_at(sx + 8 + (sw - 16) * 0.75, sy + sh / 2)
                changed = self.wait_for(lambda: abs((volume() or 0.0) - 0.75) <= 0.03, 5)
                self.check(f"Control Centre Sound, {label}: clicking the slider sets the volume",
                           changed, f"volume={volume()}")
            back()
            self.check(f"Control Centre Sound, {label}: Esc returns to the grid", in_grid())
        self.close_everything()
        apply("pair")

    def set_appearance(self, scheme: str) -> None:
        """Switch every nested Lulo surface to `scheme` ("light"/"dark")
        through the theme store they all watch."""

        theme = Path(self.env["XDG_CONFIG_HOME"]) / "rmac" / "theme.json"
        theme.parent.mkdir(parents=True, exist_ok=True)
        theme.write_text(json.dumps({"version": 1, "preferences": {"color_scheme": scheme}}),
                         encoding="utf-8")
        time.sleep(2.0)

    def keep_capture(self, name: str) -> None:
        if self.args.capture_dir:
            target = Path(self.args.capture_dir)
            target.mkdir(parents=True, exist_ok=True)
            shutil.copy(self.work / f"{name}.png", target / f"{name}.png")

    @staticmethod
    def text_contrast(image: Image.Image, box: tuple[int, int, int, int], dark_text: bool) -> float:
        """WCAG contrast between a menu row's text and its background: the
        row's median luminance is the background, its 2nd (dark text) or
        98th (light text) percentile is the text."""

        def linear(value: int) -> float:
            channel = value / 255.0
            return channel / 12.92 if channel <= 0.03928 else ((channel + 0.055) / 1.055) ** 2.4

        x, y, w, h = box
        pixels = image.crop((x, y, x + w, y + h)).getdata()
        values = sorted(0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
                        for r, g, b in pixels)
        if not values:
            return 0.0
        background = values[len(values) // 2]
        text = values[len(values) * 2 // 100] if dark_text else values[len(values) * 98 // 100]
        light, dark = max(background, text), min(background, text)
        return (light + 0.05) / (dark + 0.05)

    def desktop_menu_appearance(self) -> None:
        """The desktop's context menu in Light and Dark (DESK-13): enabled
        items must read as enabled, at least WCAG AA (4.5:1) text contrast,
        and Control Centre, its Display and Sound views and Apps are
        captured in both appearances for side-by-side review."""

        for scheme in ("light", "dark"):
            self.close_everything()
            self.set_appearance(scheme)
            baseline = self.capture(f"desktop-{scheme}")
            box = None
            shot = None
            name = f"desktop-menu-{scheme}"
            # The menu is painted by the wallpaper surface; wait until its
            # row really shows on screen, not just in the AT-SPI tree.
            for _ in range(3):
                self.click_at(OUTPUT_W / 2 - 200, 420, "right")
                row = self.wait_for(lambda: self.find_node(
                    ("menu item",), lambda name: name.startswith("Change Wallpaper")), 10)
                box = self.extents(row) if row is not None else None
                if box is None:
                    continue
                x, y, w, h = box
                crop = (x, y, x + w, y + h)

                def painted():
                    nonlocal shot
                    shot = self.capture(name)
                    # The label's glyphs change at least a tenth of the row,
                    # even where the Light menu matches a pale wallpaper.
                    return self.changed_pixels(baseline, shot, crop) > (w * h) // 10
                if self.wait_for(painted, 5, 0.5):
                    break
                box = None
                self.keys.key("escape")
                time.sleep(0.5)
            if shot is not None:
                self.keep_capture(name)
            self.check(f"desktop menu ({scheme}): opens with its items", box is not None)
            if box is not None and shot is not None:
                ratio = self.text_contrast(shot, box, dark_text=scheme == "light")
                print(f"desktop menu ({scheme}): enabled item contrast {ratio:.2f}:1", flush=True)
                self.check(f"desktop menu ({scheme}): enabled items have >= 4.5:1 contrast",
                           ratio >= 4.5, f"{ratio:.2f}:1")
            self.keys.key("escape")
            time.sleep(0.4)

            self.dispatch("quick-settings")
            if self.wait_for(lambda: self.has_layer("rmac-quick-settings"), 10):
                time.sleep(1.0)
                self.capture(f"control-centre-{scheme}")
                self.keep_capture(f"control-centre-{scheme}")
                for title, settings in (("Display", "Display Settings\u2026"),
                                        ("Sound", "Sound Settings\u2026")):
                    node = self.find_node(("push button", "button"),
                                          lambda name, title=title: name == f"{title} details")
                    if node is None:
                        continue
                    try:
                        node.queryAction().doAction(0)
                    except Exception:  # noqa: BLE001
                        continue
                    if self.wait_for(lambda settings=settings: self.find_node(
                            ("push button", "button"), lambda name: name == settings), 5):
                        time.sleep(0.6)
                        name = f"control-centre-{title.lower()}-{scheme}"
                        self.capture(name)
                        self.keep_capture(name)
                    self.keys.key("escape")
                    time.sleep(0.5)
            self.close_everything()

            self.dispatch("app-drawer")
            if self.wait_for(lambda: self.has_layer("rmac-app-drawer"), 15):
                time.sleep(1.5)
                self.capture(f"apps-{scheme}")
                self.keep_capture(f"apps-{scheme}")
            self.close_everything()
        self.set_appearance("dark")

    def control_centre_detail_escape(self) -> None:
        """Esc inside a Control Centre list (Sound's outputs) backs out to
        the grid, and a second Esc closes Control Centre, as on the Mac.
        The list replaces the control that opened it, so the panel itself
        must hold keyboard focus or Esc is lost."""

        namespace = "rmac-quick-settings"
        self.close_everything()
        self.dispatch("quick-settings")
        opened = self.wait_for(lambda: self.has_layer(namespace), 10)
        def sound_outputs():
            return self.find_node(("push button", "button"), lambda name: name == "Sound Outputs")

        outputs = sound_outputs() if opened and self.wait_for(
            lambda: sound_outputs() is not None, 10) else None
        listed = False
        if outputs is not None:
            try:
                outputs.queryAction().doAction(0)
                listed = self.wait_for(
                    lambda: self.find_node(("push button", "button", "menu item", "list item", "label"),
                                           lambda name: name == "Sound Settings\u2026") is not None, 10)
            except Exception:  # noqa: BLE001
                listed = False
        self.check("Control Centre list: Sound Outputs opens the output list", listed)
        if not listed:
            self.dispatch("quick-settings")
            self.wait_for(lambda: self.popover_gone(namespace), 5)
            return
        time.sleep(0.3)
        self.keys.key("escape")
        back = self.wait_for(
            lambda: self.find_node(("push button", "button", "menu item", "list item", "label"),
                                   lambda name: name == "Sound Settings\u2026") is None, 5)
        self.check("Control Centre list: Esc returns to the grid", back and self.has_layer(namespace))
        self.keys.key("escape")
        closed = self.wait_for(lambda: self.popover_gone(namespace), 5)
        self.check("Control Centre list: a second Esc closes Control Centre", closed)
        if not closed:
            self.dispatch("quick-settings")
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
            # Layer listing can precede the catcher's first input-region
            # commit, especially while Notification Center paints its panel.
            time.sleep(1.0)
            if method == "Escape":
                self.keys.key("escape")
            else:
                self.click_at(200, 300 if method == "inside-band click" else MENU_SURFACE_HEIGHT + 70)
            closed = self.wait_for(lambda: self.popover_gone(namespace), 10)
            self.check(f"clock/date popover: closes on {method}", closed)
            if not closed:
                self.dispatch("notification-center")
                self.wait_for(lambda: self.popover_gone(namespace), 5)

    def notification_center_shrunk_wallpaper_click(self) -> None:
        """Notification Center opens its layer surface at the tallest
        height (720 pt, `rmac_notifications_linux::center_surface`) and
        shrinks it to the column's natural height once it has laid out its
        cards. With only one notification that is far less than 720 pt, so
        the outside-click catcher's hole must shrink with it (NC-13):
        before the fix it stayed at 720 pt, and a real press in the
        wallpaper now showing through the gap between the shrunk panel and
        the hole's still-tall bottom edge fell through both and did
        nothing, instead of dismissing the panel the way any other
        wallpaper click does (MENU-15)."""

        namespace = "rmac-notification-center"
        self.close_everything()
        subprocess.run(
            ["notify-send", "Lulo", "catcher audit probe"], env=self.env, check=False, timeout=5,
        )
        clock = self.wait_for(lambda: self.find_node(
            ("push button", "button"), lambda name: name.startswith("Date and time:")), 5)
        opened = clock is not None and self.click_node(clock) and self.wait_for(
            lambda: self.has_layer(namespace) and self.has_layer(f"{namespace}-click-catcher"), 10
        )
        self.check("Notification Center shrink: opens with a notification showing", opened)
        if not opened:
            return
        # The surface opens at 720 pt and shrinks to its content on the
        # first render; give that resize (and the catcher's hole update)
        # time to land before probing the gap it leaves behind.
        time.sleep(1.0)
        # Well inside the panel's 1020-1440 pt horizontal span, far below
        # any realistic one-card height and well short of the 720 pt the
        # stale hole used to sit at.
        probe_x, probe_y = OUTPUT_W - 190, 500
        closed = self.retry_until(
            lambda: self.click_at(probe_x, probe_y),
            lambda: self.popover_gone(namespace),
            attempts=3,
            step=1.0,
        )
        self.check(
            "Notification Center shrink: a wallpaper click below the shrunk panel still dismisses it",
            closed, f"at ({probe_x}, {probe_y})",
        )
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
            elif self.args.only == "control-centre-list":
                self.control_centre_detail_escape()
            elif self.args.only == "control-centre-detail":
                self.control_centre_detail_panels()
            elif self.args.only == "control-centre-sound":
                self.control_centre_sound_outputs()
            elif self.args.only == "appearance":
                self.desktop_menu_appearance()
            elif self.args.only == "fake-hardware":
                self.fake_hardware_shows_real_data()
            elif self.args.only == "focus-return":
                self.escape_returns_focus_to_window()
            elif self.args.only == "option-alternate":
                self.option_alternate_swaps_in_open_app_menu()
            elif self.args.only == "notification-center-shrink":
                self.notification_center_shrunk_wallpaper_click()
            elif self.args.only == "launcher-expanded":
                self.launcher_expanded_row_press()
            else:
                namespaces = {
                    "quick-settings": "rmac-quick-settings",
                    "launcher": "rmac-launcher",
                    "app-drawer": "rmac-app-drawer",
                    "notification-center": "rmac-notification-center",
                }
                self.layer_popover_dismissal(self.args.only, namespaces[self.args.only])
            return self.finish()
        # Before `dock_click_closes_app_menu` below: it deliberately clicks
        # a Dock tile to close a menu, which (correctly — clicking a Dock
        # icon launches that app, as on the Mac) starts whatever app sits
        # at the shelf's horizontal centre, so every later scenario here
        # finds that app focused rather than the empty desktop Files' own
        # static fallback menus (and so Empty Bin…) need.
        self.option_alternate_swaps_in_open_app_menu()
        self.dock_click_closes_app_menu()
        self.wallpaper_click_inside_band_closes_app_menu()
        self.wallpaper_click_below_band_closes_status_menu()
        self.escape_closes_app_menu()
        self.escape_returns_focus_to_window()
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
        self.control_centre_detail_escape()
        self.control_centre_detail_panels()
        self.control_centre_sound_outputs()
        self.desktop_menu_appearance()
        self.fake_hardware_shows_real_data()
        self.clock_popover_dismissal()
        self.notification_center_shrunk_wallpaper_click()
        self.launcher_expanded_row_press()
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
        if getattr(self, "real_audio", None) is not None:
            self.real_audio.stop()
        failed = [result for result in self.results if not result[1]]
        print(f"\n{len(self.results) - len(failed)}/{len(self.results)} checks passed", flush=True)
        return 1 if failed else 0


def outer(args: argparse.Namespace, argv: list[str]) -> int:
    for tool in ("sway", "grim", "dbus-run-session", "busctl", "foot"):
        if subprocess.run(["which", tool], capture_output=True).returncode != 0:
            raise SystemExit(f"{tool} is required")
    work = Path(tempfile.mkdtemp(prefix="lulo-menu-dismiss-"))
    hardware = fake_hardware.start(work) if getattr(args, "fake_hardware", True) else None
    try:
        env = run_lulo.isolated_environment(work)
        run_lulo.refuse_live_session(env)
        run_lulo.install_shortcut_dispatcher(env, Path(args.bin_dir))
        if hardware is not None:
            env.update(hardware.env)
        else:
            # Without python3-dbusmock the backlight is still faked, so
            # Control Centre's Display module and view can be checked.
            sys_root = work / "fake-sys"
            fake_hardware.fake_sysfs(sys_root)
            env["LULO_FAKE_SYS_ROOT"] = str(sys_root)
        if args.audio == "fake":
            env.update(fake_audio.install(work))
        elif not real_audio.available():
            raise SystemExit("--audio real needs pipewire, wireplumber, pw-cli, pw-dump and wpctl")
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
        if result:
            for name in ("top-bar", "notification-center", "session"):
                log = work / "logs" / f"{name}.log"
                if log.exists():
                    print(f"{name} log tail:\n{log.read_text(errors='replace')[-2000:]}")
        return result
    finally:
        if run_lulo.reap(work / "runtime"):
            time.sleep(1.0)
            run_lulo.reap(work / "runtime")
        if hardware is not None:
            hardware.stop()
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
    parser.add_argument("--capture-dir", type=Path,
                        help="keep the Light/Dark desktop menu, Control Centre and Apps captures here")
    parser.add_argument(
        "--no-fake-hardware", dest="fake_hardware", action="store_false", default=True,
        help="skip the private NetworkManager/BlueZ/UPower mocks (docs/behavior-suite.md)",
    )
    parser.add_argument("--only", choices=("topbar", "status", "dock", "quick-settings",
                                           "launcher", "app-drawer", "notification-center",
                                           "combined", "control-centre-list",
                                           "control-centre-detail", "control-centre-sound",
                                           "appearance", "fake-hardware",
                                           "focus-return", "option-alternate",
                                           "notification-center-shrink", "launcher-expanded"))
    parser.add_argument(
        "--audio", choices=("fake", "real"), default="fake",
        help="fake: fake_audio.py's recorded graphs (default); real: a private PipeWire and "
             "WirePlumber with null sinks (scripts/behavior/real_audio.py)",
    )
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
