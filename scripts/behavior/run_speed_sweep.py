#!/usr/bin/env python3
"""The owner's "is Lulo as fast as the Mac" sweep: cold launch and panel-open
latency for every app and the system overlays, against the macOS 26 targets
(about 300 ms to an app's first frame, about 100 ms for a panel).

    python3 scripts/behavior/run_speed_sweep.py --niri PATH --bin-dir DIR
        [--bin-dir DIR ...] --json-output PATH [--markdown-output PATH]
        [--profile iterate|release] [--keep]

Run once against a "before" `--bin-dir` and once against an "after" one
(same profile, same laptop, back to back) and diff the two JSON reports --
this script does not itself know which binaries are "before" or "after".

Reuses scripts/behavior/run_frame_timing.py's isolation: a private
dbus-run-session, a temporary HOME/XDG_RUNTIME_DIR, headless Sway hosting a
nested niri with the shipped packaging/rmac-session/shell.kdl (so panels
spawn exactly as the real session spawns them), and RMAC_FRAME_TRACE
(shell/compat/gpui_linux/src/linux/wayland/frame_trace.rs) read back for
every traced process. See that script's docstring for the GPU-path caveat
(the outer compositing layer is forced to software; the traced apps
themselves use the laptop's real Vulkan driver).

Measurements per app, all wall-clock from the moment this script spawns
the process (the median of --repeat launches is reported):

  - backend_init_ms: RMAC_FRAME_TRACE's file appears, i.e. GPUI's Wayland
    backend initialised. Time before this is the app's own `main` work
    before it hands control to GPUI (anything synchronous there delays the
    first frame one-for-one).
  - first_frame_ms: the first `present` row in the app's frame trace -- the
    first swapchain image the window ever showed. The trace file is polled
    every 5 ms (a stat, not a process spawn), so this is the real first
    frame to within that resolution.
  - window_listed_ms: the window appears in `niri msg windows` (polled only
    after the first present, so the polling never competes with startup).
  - icons_painted_ms: the last `present` before the app's trace records no
    new `present` for SETTLE_S, i.e. first_frame_ms plus the trace-clock gap
    between the first and that last present (frame_callback rows, which
    GPUI records even when it draws nothing, are ignored). Icon decodes
    and asynchronously loaded content land with `cx.notify()`, which
    repaints -- so presents keep coming while content is still arriving
    and stop once it all has. This is a proxy, not a per-icon signal: a
    window that keeps animating for other reasons (a spinner, a cursor
    blink) would never look quiescent, which is why apps with those are
    noted rather than scored on this metric.

Before the first app, one throwaway Calculator launch warms the private
HOME's Mesa shader cache: the real session's Dock and menu bar have always
done that long before the owner opens an app, and without it whichever app
happens to be measured first pays a one-off pipeline compile (~0.6 s on
the reference laptop).

One measurement per panel (Spotlight, Control Centre, Notification Centre,
Launchpad, Mission Control, App Switcher): open_ms, wall-clock from the
triggering input (a shortcut-endpoint dispatch, a real niri keybind, or the
App Switcher's own `next` client command) to the first `present` that
process records after it. Every panel runs as its resident service, the way
the session runs it (Launchpad is `rmac-app-drawer --service`; the App
Switcher is `rmac-app-switcher --service` with two app windows open so it
has something to show).

Settings pane switching: Settings is launched, Tab moves keyboard focus to
the sidebar, then each Down arrow selects the next pane. The latency is the
Settings process's own trace: from the key's `input` row to the next
`present` row (one clock, so no cross-process skew), and every switch is
confirmed by the window title changing in `niri msg windows`.
"""

from __future__ import annotations

import argparse
import fcntl
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any, Optional

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
sys.path.insert(0, str(HERE))

import run_lulo  # noqa: E402
import statistics  # noqa: E402
import wlinput  # noqa: E402
from run_frame_timing import read_trace  # noqa: E402

OUTPUT_W, OUTPUT_H = run_lulo.OUTPUT_W, run_lulo.OUTPUT_H
APP_FIRST_FRAME_TARGET_MS = 300.0
PANEL_OPEN_TARGET_MS = 100.0
# How long a trace must stop growing before "every icon that was going to
# repaint has repainted" is called -- long enough that one scheduler hiccup
# doesn't end the window early, short enough not to dominate the budget.
SETTLE_S = 0.25
QUIESCENCE_TIMEOUT_S = 4.0

# (binary names to try, in --bin-dir order; niri app_id). The binary list
# mirrors run_lulo.APP_BINARIES, this behaviour suite's existing "every
# app" list; "desktop" is excluded -- it is a layer-shell background, not a
# toplevel window, and already has its own paint-latency harness
# (run_desktop_first_paint.py).
APPS: dict[str, tuple[list[str], str]] = {
    "files": (["rmac-files"], "org.rmac.Files"),
    "text-editor": (["rmac-text-editor"], "org.rmac.TextEditor"),
    "settings": (["rmac-system-settings"], "org.rmac.SystemSettings"),
    "calculator": (["rmac-calculator"], "org.rmac.Calculator"),
    "calendar": (["rmac-calendar"], "org.rmac.Calendar"),
    "mail": (["rmac-mail"], "org.rmac.Mail"),
    "clock": (["rmac-clock"], "org.rmac.Clock"),
    "weather": (["rmac-weather"], "org.rmac.Weather"),
    "preview": (["rmac-preview"], "org.rmac.Preview"),
    "notes": (["rmac-notes"], "org.rmac.Notes"),
    "system-monitor": (["rmac-system-monitor"], "org.rmac.SystemMonitor"),
    "terminal": (["rmac-terminal"], "org.rmac.Terminal"),
}
# Apps whose window keeps animating on its own right after launch (a
# spinner, a blinking caret) and so never looks "quiescent" -- their
# icons_painted_ms is recorded as n/a rather than a misleading timeout.
NO_QUIESCENCE = {"terminal"}
# Shell surfaces the session starts at login (layer-shell, so never listed
# in `niri msg windows`): cold start to first present and to settled, the
# same way as the apps. app_id None skips the window listing.
SHELL_SURFACES: dict[str, list[str]] = {
    "desktop": ["rmac-wallpaper", "wallpaper"],
    "dock": ["rmac-dock", "dock"],
}

# (argv, shortcut id for rmac-shortcut-dispatch) -- the panel registers
# its own endpoint socket; the dispatcher's call is the "open" action a
# real shortcut or menu click would send. argv mirrors the packaged
# systemd user units' ExecStart.
DISPATCH_PANELS: dict[str, tuple[list[str], str]] = {
    "spotlight": (["rmac-launcher"], "launcher"),
    "control-centre": (["rmac-quick-settings"], "quick-settings"),
    "notification-centre": (["rmac-notification-center-panel"], "notification-center"),
    "launchpad": (["rmac-app-drawer", "--service"], "app-drawer"),
}
SETTINGS_APP_ID = "org.rmac.SystemSettings"
SETTINGS_PANE_SWITCHES = 6
POLL_S = 0.005


class StepFailed(RuntimeError):
    pass


# --------------------------------------------------------------------------
# Trace-derived timings -- pure, unit-testable without a live session.
# --------------------------------------------------------------------------


def first_present_micros(events: list[tuple[str, int]]) -> Optional[int]:
    for event, micros in events:
        if event == "present":
            return micros
    return None


def present_count(events: list[tuple[str, int]]) -> int:
    return sum(1 for event, _ in events if event == "present")


def input_to_next_present_ms(events: list[tuple[str, int]], after_index: int) -> Optional[float]:
    """Latency from the first `input` row at or after `after_index` to the
    first `present` that follows it, in one process's own trace clock."""

    for index in range(after_index, len(events)):
        event, micros = events[index]
        if event != "input":
            continue
        for later_event, later in events[index + 1:]:
            if later_event == "present":
                return (later - micros) / 1000.0
        return None
    return None


def median(values: list[Optional[float]]) -> Optional[float]:
    present = [value for value in values if value is not None]
    return statistics.median(present) if present else None


def wait_for_present(trace_path: Path, baseline: int, deadline: float) -> Optional[float]:
    """`time.monotonic()` once `trace_path` has more than `baseline`
    `present` rows, polled every POLL_S (a stat plus a read only when the
    file grew), or None at `deadline`."""

    last_size = -1
    while time.monotonic() < deadline:
        try:
            size = trace_path.stat().st_size
        except OSError:
            size = -1
        if size != last_size and size > 0:
            now = time.monotonic()
            last_size = size
            if present_count(read_trace(trace_path)) > baseline:
                return now
        time.sleep(POLL_S)
    return None


def quiescence_wait(trace_path: Path, deadline: float) -> Optional[float]:
    """Wall-clock `time.monotonic()` once `trace_path` has at least one
    `present` row and has recorded no new `present` for SETTLE_S, or None
    if that never happens before `deadline`. Other rows (GPUI records
    `frame_callback` even for frames it does not draw) do not count."""

    last_size = -1
    presents = 0
    last_change = time.monotonic()
    while time.monotonic() < deadline:
        try:
            size = trace_path.stat().st_size
        except OSError:
            size = 0
        now = time.monotonic()
        if size != last_size:
            last_size = size
            count = present_count(read_trace(trace_path))
            if count != presents:
                presents = count
                last_change = now
        if presents and (now - last_change) >= SETTLE_S:
            return now
        time.sleep(0.02)
    return None


def settled_span_ms(events: list[tuple[str, int]]) -> Optional[float]:
    """Trace-clock milliseconds from the first `present` to the last one."""

    presents = [micros for event, micros in events if event == "present"]
    if not presents:
        return None
    return (presents[-1] - presents[0]) / 1000.0


# --------------------------------------------------------------------------
# Outer process: isolated environment, then re-exec inside it.
# --------------------------------------------------------------------------


def outer(args: argparse.Namespace, argv: list[str]) -> int:
    for tool in ("sway", "niri", "dbus-run-session"):
        if shutil.which(tool) is None:
            raise SystemExit(f"{tool} is required")
    journey_lock = open("/tmp/lulo-journey.lock", "w")
    fcntl.flock(journey_lock, fcntl.LOCK_EX)
    work = Path(tempfile.mkdtemp(prefix="lulo-speed-sweep-"))
    try:
        env = run_lulo.isolated_environment(work)
        run_lulo.refuse_live_session(env)
        for key in (
            "WLR_BACKENDS", "WLR_HEADLESS_OUTPUTS", "WLR_LIBINPUT_NO_DEVICES",
            "WLR_RENDERER", "LIBGL_ALWAYS_SOFTWARE", "VK_ICD_FILENAMES",
        ):
            env.pop(key, None)
        services = work / "dbus-services"
        services.mkdir()
        config = work / "session.conf"
        config.write_text(
            "<!DOCTYPE busconfig PUBLIC \"-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN\"\n"
            " \"http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd\">\n"
            "<busconfig><type>session</type>"
            f"<listen>unix:dir={work}</listen><auth>EXTERNAL</auth>"
            f"<servicedir>{services}</servicedir>"
            "<policy context=\"default\"><allow send_destination=\"*\" eavesdrop=\"true\"/>"
            "<allow eavesdrop=\"true\"/><allow own=\"*\"/></policy></busconfig>\n"
        )
        (work / "logs").mkdir(exist_ok=True)
        command = [
            "dbus-run-session", f"--config-file={config}", "--", sys.executable,
            str(Path(__file__).resolve()), "--inner", str(work), *argv,
        ]
        with open(work / "logs" / "session.log", "w") as log:
            status = subprocess.call(command, env=env, close_fds=True, stderr=log)
        if status != 0:
            print((work / "logs" / "session.log").read_text(errors="replace")[-2000:], file=sys.stderr)
        return status
    finally:
        if run_lulo.reap(work / "runtime"):
            time.sleep(1.0)
            run_lulo.reap(work / "runtime")
        if args.keep:
            print(f"kept {work}", file=sys.stderr)
        else:
            run_lulo.remove_tree(work)
        journey_lock.close()


# --------------------------------------------------------------------------
# Inner process: headless Sway hosting a nested niri, then the sweep.
# --------------------------------------------------------------------------


class Run:
    def __init__(self, args: argparse.Namespace, work: Path) -> None:
        self.args = args
        self.work = work
        self.env = dict(os.environ)
        run_lulo.refuse_live_session(self.env)
        self.runtime = Path(self.env["XDG_RUNTIME_DIR"])
        self.logs = work / "logs"
        self.logs.mkdir(exist_ok=True)
        self.children: list[subprocess.Popen] = []
        self._trace_count = 0

    def bin(self, *candidates: str) -> Optional[Path]:
        for directory in self.args.bin_dir:
            for candidate in candidates:
                found = Path(directory) / candidate
                if found.is_file() and os.access(found, os.X_OK):
                    return found.resolve()
        return None

    def spawn(self, argv: list[str], label: str, extra: Optional[dict[str, str]] = None) -> subprocess.Popen:
        process = subprocess.Popen(
            argv, env={**self.env, **(extra or {})},
            stdout=open(self.logs / f"{label}.log", "w"),
            stderr=subprocess.STDOUT, close_fds=True,
        )
        self.children.append(process)
        return process

    def traced(self, argv: list[str], label: str, extra: Optional[dict[str, str]] = None) -> tuple[subprocess.Popen, Path]:
        self._trace_count += 1
        trace = self.logs / f"trace-{label}-{self._trace_count}.csv"
        trace.unlink(missing_ok=True)
        app_env = dict(item.split("=", 1) for item in self.args.app_env)
        process = self.spawn(
            argv, f"{label}-{self._trace_count}",
            {**app_env, **(extra or {}), "RMAC_FRAME_TRACE": str(trace)},
        )
        return process, trace

    @staticmethod
    def wait_for(predicate, timeout: float = 20.0, step: float = 0.02):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                value = predicate()
            except (OSError, RuntimeError, ValueError, KeyError):
                value = None
            if value:
                return value
            time.sleep(step)
        return None

    def stop(self, process: subprocess.Popen, grace: float = 5.0) -> None:
        if process.poll() is None:
            process.terminate()
        try:
            process.wait(grace)
        except subprocess.TimeoutExpired:
            process.kill()
        if process in self.children:
            self.children.remove(process)

    # -- compositors ---------------------------------------------------

    def start(self) -> None:
        self.locks = []
        for taken in ("wayland-0.lock", "wayland-1.lock"):
            handle = open(self.runtime / taken, "w")
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.locks.append(handle)
        sway_conf = self.logs / "sway.conf"
        sway_conf.write_text(
            "xwayland disable\ndefault_border none\ndefault_floating_border none\n"
            f"output HEADLESS-1 mode {self.args.output} position 0 0\n"
            "seat seat0 fallback true\nfocus_follows_mouse no\n"
        )
        self.spawn(
            ["sway", "--unsupported-gpu", "--config", str(sway_conf)], "sway",
            {"WLR_BACKENDS": "headless", "WLR_HEADLESS_OUTPUTS": "1",
             "WLR_LIBINPUT_NO_DEVICES": "1", "WLR_RENDERER": "pixman"},
        )
        self.sway_display = self.wait_for(lambda: next(
            (p.name for p in self.runtime.glob("wayland-*") if not p.name.endswith(".lock")), None))
        if not self.sway_display:
            raise StepFailed("sway did not start")

        shell = (REPO / "packaging/rmac-session/shell.kdl").read_text(encoding="utf-8")
        mission_control = self.bin("rmac-mission-control", "mission-control")
        if mission_control:
            shell = shell.replace("/usr/libexec/rmac/rmac-mission-control", str(mission_control))
        app_switcher = self.bin("rmac-app-switcher", "app-switcher")
        if app_switcher:
            shell = shell.replace("/usr/libexec/rmac/rmac-app-switcher", str(app_switcher))
        niri_config = self.logs / "niri.kdl"
        niri_config.write_text(shell)
        validate = subprocess.run(
            [self.args.niri, "validate", "-c", str(niri_config)], env=self.env,
            capture_output=True, text=True, timeout=15,
        )
        if validate.returncode != 0:
            raise StepFailed(f"shipped shell.kdl does not validate: {validate.stderr[-500:]}")

        before = {p.name for p in self.runtime.glob("wayland-*")}
        self.spawn(
            [self.args.niri, "-c", str(niri_config)], "niri",
            {"WAYLAND_DISPLAY": self.sway_display, "LIBGL_ALWAYS_SOFTWARE": "1"},
        )
        self.socket = self.wait_for(lambda: next(iter(self.runtime.glob("niri.*.sock")), None))
        self.display = self.wait_for(lambda: next(
            (p.name for p in self.runtime.glob("wayland-*")
             if not p.name.endswith(".lock") and p.name not in before), None))
        if not self.socket or not self.display:
            raise StepFailed("nested niri did not start under Sway")
        self.env.update({"WAYLAND_DISPLAY": self.display, "NIRI_SOCKET": str(self.socket)})
        self.input = wlinput.Wayland({**self.env, "WAYLAND_DISPLAY": self.sway_display})

    def niri_msg(self, *request: str) -> Any:
        result = subprocess.run(
            [self.args.niri, "msg", "-j", *request], env=self.env,
            capture_output=True, text=True, timeout=10,
        )
        return json.loads(result.stdout) if result.stdout.strip() else None

    def window_by_app_id(self, app_id: str) -> Optional[dict[str, Any]]:
        return next((w for w in self.niri_msg("windows") or [] if w.get("app_id") == app_id), None)

    # -- per-app cold launch ---------------------------------------------

    def launch_once(self, name: str, binary: Path, app_id: Optional[str]) -> dict[str, Any]:
        start = time.monotonic()
        process, trace = self.traced([str(binary)], name)
        init_at = None
        deadline = start + 20.0
        while time.monotonic() < deadline and process.poll() is None:
            if trace.exists():
                init_at = time.monotonic()
                break
            time.sleep(POLL_S)
        presented = wait_for_present(trace, 0, start + 20.0)
        window = self.wait_for(lambda: self.window_by_app_id(app_id), timeout=10.0) if app_id else None
        listed_at = time.monotonic() if window else None
        icons_painted_ms = None
        if name not in NO_QUIESCENCE and presented is not None:
            settled = quiescence_wait(trace, deadline=start + QUIESCENCE_TIMEOUT_S)
            span = settled_span_ms(read_trace(trace))
            if settled is not None and span is not None:
                icons_painted_ms = (presented - start) * 1000.0 + span
        self.stop(process)
        time.sleep(0.5)  # let the compositor unmap before the next launch

        def since(at: Optional[float]) -> Optional[float]:
            return None if at is None else (at - start) * 1000.0

        return {
            "backend_init_ms": since(init_at),
            "first_frame_ms": since(presented),
            "window_listed_ms": since(listed_at),
            "icons_painted_ms": icons_painted_ms,
        }

    def measure_app(self, name: str, binaries: list[str], app_id: Optional[str]) -> dict[str, Any]:
        binary = self.bin(*binaries)
        if binary is None:
            return {"error": f"none of {binaries} found under --bin-dir"}
        runs = [self.launch_once(name, binary, app_id) for _ in range(self.args.repeat)]
        first_frame_ms = median([run["first_frame_ms"] for run in runs])
        result: dict[str, Any] = {
            "runs": runs,
            "backend_init_ms": median([run["backend_init_ms"] for run in runs]),
            "first_frame_ms": first_frame_ms,
            "window_listed_ms": median([run["window_listed_ms"] for run in runs]),
            "first_frame_pass": first_frame_ms is not None and first_frame_ms <= APP_FIRST_FRAME_TARGET_MS,
        }
        if name in NO_QUIESCENCE:
            result["icons_painted_ms"] = None
            result["icons_painted_note"] = "animates continuously (not scored)"
        else:
            icons_painted_ms = median([run["icons_painted_ms"] for run in runs])
            result["icons_painted_ms"] = icons_painted_ms
            result["icons_painted_pass"] = (
                icons_painted_ms is not None and icons_painted_ms <= APP_FIRST_FRAME_TARGET_MS
            )
        if first_frame_ms is None:
            result["error"] = "no present recorded"
        return result

    # -- Settings pane switching -------------------------------------------

    def measure_settings_panes(self) -> dict[str, Any]:
        binary = self.bin("rmac-system-settings")
        if binary is None:
            return {"error": "rmac-system-settings not found under --bin-dir"}
        process, trace = self.traced([str(binary)], "settings-panes")
        try:
            window = self.wait_for(lambda: self.window_by_app_id(SETTINGS_APP_ID), timeout=20.0)
            if window is None:
                return {"error": "Settings window never appeared"}
            if quiescence_wait(trace, deadline=time.monotonic() + 8.0) is None:
                return {"error": "Settings never settled after launch"}
            self.input.key("tab")
            quiescence_wait(trace, deadline=time.monotonic() + 3.0)
            latencies: list[float] = []
            titles: list[str] = [window.get("title") or ""]
            for _ in range(SETTINGS_PANE_SWITCHES):
                before = len(read_trace(trace))
                previous = titles[-1]

                def switched(previous: str = previous) -> Optional[dict[str, Any]]:
                    current = self.window_by_app_id(SETTINGS_APP_ID)
                    if current and (current.get("title") or "") != previous:
                        return current
                    return None

                self.input.key("down")
                changed = self.wait_for(switched, timeout=3.0)
                quiescence_wait(trace, deadline=time.monotonic() + 3.0)
                if not changed:
                    return {"error": f"Down did not switch panes (title stayed {previous!r})",
                            "titles": titles}
                titles.append(changed.get("title") or "")
                latency = input_to_next_present_ms(read_trace(trace), before)
                if latency is not None:
                    latencies.append(latency)
            if not latencies:
                return {"error": "no input/present pair recorded", "titles": titles}
            switch_ms = statistics.median(latencies)
            return {
                "switch_ms": switch_ms,
                "switch_max_ms": max(latencies),
                "switches": latencies,
                "titles": titles,
                "switch_pass": switch_ms <= PANEL_OPEN_TARGET_MS,
            }
        finally:
            self.stop(process)

    # -- panels ------------------------------------------------------------

    def measure_dispatch_panel(self, argv: list[str], shortcut_id: str) -> dict[str, Any]:
        binary = self.bin(argv[0])
        if binary is None:
            return {"error": f"{argv[0]} not found under --bin-dir"}
        dispatcher = self.bin("rmac-shortcut-dispatch")
        if dispatcher is None:
            return {"error": "rmac-shortcut-dispatch not found under --bin-dir"}
        process, trace = self.traced([str(binary), *argv[1:]], shortcut_id)
        endpoint = self.runtime / "rmac" / f"shortcut-{shortcut_id}.sock"
        if not self.wait_for(endpoint.exists, timeout=20.0):
            self.stop(process)
            return {"error": f"{argv[0]} did not register its shortcut endpoint"}
        time.sleep(1.0)  # let the resident process's own startup settle first
        opens: list[Optional[float]] = []
        for _ in range(self.args.repeat):
            baseline = present_count(read_trace(trace))
            start = time.monotonic()
            result = subprocess.run(
                [str(dispatcher), shortcut_id], env=self.env, capture_output=True, text=True, timeout=10,
            )
            if result.returncode != 0:
                self.stop(process)
                return {"error": f"rmac-shortcut-dispatch {shortcut_id} failed: {result.stderr[:200]}"}
            presented = wait_for_present(trace, baseline, start + 5.0)
            opens.append(None if presented is None else (presented - start) * 1000.0)
            quiescence_wait(trace, deadline=time.monotonic() + 2.0)
            self.input.key("escape")
            quiescence_wait(trace, deadline=time.monotonic() + 2.0)
            time.sleep(0.3)
        self.stop(process)
        open_ms = median(opens)
        return {
            "open_ms": open_ms,
            "opens": opens,
            "open_pass": open_ms is not None and open_ms <= PANEL_OPEN_TARGET_MS,
            **({"error": "no present recorded after dispatch"} if open_ms is None else {}),
        }

    def measure_mission_control(self) -> dict[str, Any]:
        mission_control = self.bin("rmac-mission-control", "mission-control")
        if mission_control is None:
            return {"error": "rmac-mission-control not found under --bin-dir"}
        process, trace = self.traced([str(mission_control), "--service"], "mission-control")
        self.wait_for(lambda: trace.exists(), timeout=20.0)
        time.sleep(1.0)  # let the resident service's own startup settle first
        opens: list[Optional[float]] = []
        for _ in range(self.args.repeat):
            baseline = present_count(read_trace(trace))
            start = time.monotonic()
            self.input.key("ctrl-up")
            presented = wait_for_present(trace, baseline, start + 5.0)
            opens.append(None if presented is None else (presented - start) * 1000.0)
            quiescence_wait(trace, deadline=time.monotonic() + 2.0)
            self.input.key("escape")
            quiescence_wait(trace, deadline=time.monotonic() + 2.0)
            time.sleep(0.3)
        self.stop(process)
        open_ms = median(opens)
        return {
            "open_ms": open_ms,
            "opens": opens,
            "open_pass": open_ms is not None and open_ms <= PANEL_OPEN_TARGET_MS,
            **({"error": "no present recorded after Ctrl+Up"} if open_ms is None else {}),
        }

    def measure_app_switcher(self) -> dict[str, Any]:
        # The session runs `rmac-app-switcher --service`; Mod+Tab spawns the
        # short-lived `rmac-app-switcher next` client, which only sends one
        # datagram. Two app windows are opened first so the switcher has
        # something to show (it shows nothing with no apps).
        app_switcher = self.bin("rmac-app-switcher", "app-switcher")
        if app_switcher is None:
            return {"error": "rmac-app-switcher not found under --bin-dir"}
        helpers: list[subprocess.Popen] = []
        for name in ("calculator", "clock"):
            binaries, app_id = APPS[name]
            helper_binary = self.bin(*binaries)
            if helper_binary is not None:
                helpers.append(self.spawn([str(helper_binary)], f"switcher-helper-{name}"))
                self.wait_for(lambda app_id=app_id: self.window_by_app_id(app_id), timeout=20.0)
        process, trace = self.traced([str(app_switcher), "--service"], "app-switcher")
        socket = self.runtime / "rmac" / "app-switcher.sock"
        try:
            if not self.wait_for(socket.exists, timeout=20.0):
                return {"error": "rmac-app-switcher --service did not bind its socket"}
            time.sleep(1.0)
            opens: list[Optional[float]] = []
            for _ in range(self.args.repeat):
                baseline = present_count(read_trace(trace))
                start = time.monotonic()
                client = subprocess.run(
                    [str(app_switcher), "next"], env=self.env, capture_output=True, text=True, timeout=10,
                )
                if client.returncode != 0:
                    return {"error": f"rmac-app-switcher next failed: {client.stderr[:200]}"}
                presented = wait_for_present(trace, baseline, start + 5.0)
                opens.append(None if presented is None else (presented - start) * 1000.0)
                quiescence_wait(trace, deadline=time.monotonic() + 2.0)
                self.input.key("escape")
                quiescence_wait(trace, deadline=time.monotonic() + 2.0)
                time.sleep(0.3)
            open_ms = median(opens)
            return {
                "open_ms": open_ms,
                "opens": opens,
                "open_pass": open_ms is not None and open_ms <= PANEL_OPEN_TARGET_MS,
                **({"error": "no present recorded"} if open_ms is None else {}),
            }
        finally:
            self.stop(process)
            for helper in helpers:
                self.stop(helper)

    # -- lifecycle -------------------------------------------------------

    def finish(self) -> None:
        try:
            self.input.close()
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

    def run(self) -> dict[str, Any]:
        self.start()
        report: dict[str, Any] = {
            "schema_version": 1,
            "captured_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "profile": self.args.profile,
            "targets": {
                "app_first_frame_ms": APP_FIRST_FRAME_TARGET_MS,
                "panel_open_ms": PANEL_OPEN_TARGET_MS,
            },
            "repeat": self.args.repeat,
            "apps": {},
            "panels": {},
        }
        only = set(self.args.only or [])

        def wanted(name: str) -> bool:
            return not only or name in only

        # A desktop like a fresh account's: one folder and one document.
        desktop = Path(self.env["HOME"]) / "Desktop"
        (desktop / "Projects").mkdir(parents=True, exist_ok=True)
        (desktop / "Notes.txt").write_text("Speed sweep fixture.\n")
        warm = self.bin("rmac-calculator")
        if warm is not None:
            process = self.spawn([str(warm)], "shader-cache-warmup")
            self.wait_for(lambda: self.window_by_app_id(APPS["calculator"][1]), timeout=20.0)
            time.sleep(1.0)
            self.stop(process)
            time.sleep(0.5)
        for name, (binaries, app_id) in APPS.items():
            if not wanted(name):
                continue
            print(f"app: {name}...", flush=True)
            try:
                report["apps"][name] = self.measure_app(name, binaries, app_id)
            except StepFailed as error:
                report["apps"][name] = {"error": str(error)}
        for name, (argv, shortcut_id) in DISPATCH_PANELS.items():
            if not wanted(name):
                continue
            print(f"panel: {name}...", flush=True)
            try:
                report["panels"][name] = self.measure_dispatch_panel(argv, shortcut_id)
            except StepFailed as error:
                report["panels"][name] = {"error": str(error)}
        for name, binaries in SHELL_SURFACES.items():
            if not wanted(name):
                continue
            print(f"shell: {name}...", flush=True)
            report.setdefault("shell", {})[name] = self.measure_app(name, binaries, None)
        if wanted("mission-control"):
            print("panel: mission-control...", flush=True)
            report["panels"]["mission-control"] = self.measure_mission_control()
        if wanted("app-switcher"):
            print("panel: app-switcher...", flush=True)
            report["panels"]["app-switcher"] = self.measure_app_switcher()
        if wanted("settings-panes"):
            print("settings pane switching...", flush=True)
            report["settings_panes"] = self.measure_settings_panes()
        self.finish()
        return report


# --------------------------------------------------------------------------
# Markdown rendering
# --------------------------------------------------------------------------


def _fmt_ms(value: Optional[float]) -> str:
    return "n/a" if value is None else f"{value:.0f} ms"


def _fmt_pass(entry: dict[str, Any], key: str) -> str:
    if entry.get("error"):
        return f"error: {entry['error']}"
    value = entry.get(key)
    if value is None:
        return "n/a"
    return "PASS" if value else "FAIL"


def render_markdown(report: dict[str, Any]) -> str:
    lines = [f"# Speed sweep -- {report.get('captured_at', 'unknown')}", ""]
    lines.append(f"Profile: {report.get('profile', 'unknown')} binaries.")
    targets = report.get("targets", {})
    lines.append(
        f"Targets: app first frame and icons painted <= {targets.get('app_first_frame_ms')} ms; "
        f"panel open <= {targets.get('panel_open_ms')} ms."
    )
    lines.append("")
    lines.append("## Apps")
    lines.append("")
    lines.append("| App | Backend init | First frame | Pass | Window listed | Icons painted | Pass |")
    lines.append("|---|---:|---:|---|---:|---:|---|")
    for name, entry in report.get("apps", {}).items():
        if "error" in entry and entry.get("first_frame_ms") is None:
            lines.append(f"| {name} | -- | -- | error: {entry['error']} | -- | -- | -- |")
            continue
        icons_cell = entry.get("icons_painted_note", _fmt_ms(entry.get("icons_painted_ms")))
        icons_pass = "--" if "icons_painted_note" in entry else _fmt_pass(entry, "icons_painted_pass")
        lines.append(
            f"| {name} | {_fmt_ms(entry.get('backend_init_ms'))} | {_fmt_ms(entry.get('first_frame_ms'))} | "
            f"{_fmt_pass(entry, 'first_frame_pass')} | {_fmt_ms(entry.get('window_listed_ms'))} | "
            f"{icons_cell} | {icons_pass} |"
        )
    lines.append("")
    if report.get("shell"):
        lines.append("## Shell surfaces")
        lines.append("")
        lines.append("| Surface | First frame | Settled |")
        lines.append("|---|---:|---:|")
        for name, entry in report["shell"].items():
            lines.append(
                f"| {name} | {_fmt_ms(entry.get('first_frame_ms'))} | {_fmt_ms(entry.get('icons_painted_ms'))} |"
            )
        lines.append("")
    lines.append("## Panels")
    lines.append("")
    lines.append("| Panel | Open | Pass |")
    lines.append("|---|---:|---|")
    for name, entry in report.get("panels", {}).items():
        lines.append(f"| {name} | {_fmt_ms(entry.get('open_ms'))} | {_fmt_pass(entry, 'open_pass')} |")
    lines.append("")
    panes = report.get("settings_panes")
    if panes is not None:
        lines.append("## Settings pane switching")
        lines.append("")
        if panes.get("error"):
            lines.append(f"error: {panes['error']}")
        else:
            lines.append(
                f"Median {_fmt_ms(panes.get('switch_ms'))} (max {_fmt_ms(panes.get('switch_max_ms'))}) "
                f"over {len(panes.get('switches', []))} switches: {_fmt_pass(panes, 'switch_pass')}."
            )
        lines.append("")
    not_measured = report.get("not_measured", [])
    if not_measured:
        lines.append(f"Not measured: {', '.join(not_measured)}.")
        lines.append("")
    return "\n".join(lines)


# --------------------------------------------------------------------------
# main
# --------------------------------------------------------------------------


def parse_args() -> tuple[argparse.Namespace, list[str]]:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--niri", default="/usr/bin/niri")
    parser.add_argument("--bin-dir", action="append", default=[], required=False)
    parser.add_argument("--profile", default="unknown")
    parser.add_argument("--json-output", type=Path, required=True)
    parser.add_argument("--markdown-output", type=Path, default=None)
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--output", default="1920x1080",
                        help="nested output size; the default is the reference laptop's 1080p panel")
    parser.add_argument("--repeat", type=int, default=3,
                        help="launches/opens per item; the median is reported")
    parser.add_argument("--app-env", action="append", default=[], metavar="KEY=VALUE",
                        help="extra environment for every measured process (experiments)")
    parser.add_argument("--only", action="append", default=[],
                        help="measure only these items (app names, panel names, settings-panes)")
    parser.add_argument("--inner", type=Path, help=argparse.SUPPRESS)
    args, unknown = parser.parse_known_args()
    return args, unknown


def main() -> int:
    args, _unknown = parse_args()
    if args.inner:
        run = Run(args, args.inner)
        report = run.run()
        args.json_output.parent.mkdir(parents=True, exist_ok=True)
        args.json_output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
        if args.markdown_output is not None:
            args.markdown_output.parent.mkdir(parents=True, exist_ok=True)
            args.markdown_output.write_text(render_markdown(report))
        return 0
    if not args.bin_dir:
        raise SystemExit("--bin-dir is required (at least once)")
    argv = [a for a in sys.argv[1:]]
    return outer(args, argv)


if __name__ == "__main__":
    sys.exit(main())
