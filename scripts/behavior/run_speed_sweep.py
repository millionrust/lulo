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

Two measurements per app, both wall-clock from the moment this script spawns
the process:

  - first_frame_ms: the app's window appears in `niri msg windows`. niri
    only lists a window once its client has committed a buffer, so this is
    close to -- but not provably exactly -- the first present; treat it as
    the "window is now showing something" latency the owner actually sees.
  - icons_painted_ms: the app's RMAC_FRAME_TRACE stops growing (no new
    `present` line) for SETTLE_S. Icon decodes land asynchronously and each
    one calls `cx.notify()` on completion (see rmac_ui::svg_icon,
    shell/bins/rmac-wallpaper's warm_desktop_icons for the pattern this
    generalizes), which repaints -- so the trace keeps growing while any
    icon is still decoding and goes quiet once they all have. This is a
    proxy, not a per-icon signal: a window that keeps animating for other
    reasons (a spinner, a cursor blink) would never look quiescent, which
    is why apps with those are noted rather than scored on this metric.

One measurement per panel (Spotlight, Control Centre, Notification Centre,
Launchpad, Mission Control, App Switcher): open_ms, wall-clock from the
triggering input (a shortcut-endpoint dispatch, or a real niri keybind for
the two that are resident services) to that process's next `present`.

Settings pane switching is NOT measured here: it needs simulated clicks on
the sidebar's measured row coordinates, which this pass did not build: see
docs/perf/speed-sweep-2026-10-05.md.
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

# (binary, shortcut id for rmac-shortcut-dispatch) -- the panel registers
# its own endpoint socket; the dispatcher's call is the "open" action a
# real shortcut or menu click would send.
DISPATCH_PANELS: dict[str, tuple[str, str]] = {
    "spotlight": ("rmac-launcher", "launcher"),
    "control-centre": ("rmac-quick-settings", "quick-settings"),
    "notification-centre": ("rmac-notification-center-panel", "notification-center"),
    "launchpad": ("rmac-app-drawer", "app-drawer"),
}


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


def quiescence_wait(trace_path: Path, deadline: float) -> Optional[float]:
    """Wall-clock `time.monotonic()` once `trace_path` has at least one
    `present` row and has not grown for SETTLE_S, or None if that never
    happens before `deadline`."""

    last_size = -1
    last_change = time.monotonic()
    seen_present = False
    while time.monotonic() < deadline:
        try:
            size = trace_path.stat().st_size
        except OSError:
            size = 0
        now = time.monotonic()
        if size != last_size:
            last_size = size
            last_change = now
            if not seen_present and size > 0:
                seen_present = "present" in trace_path.read_text(errors="replace")
        elif seen_present and (now - last_change) >= SETTLE_S:
            return now
        time.sleep(0.02)
    return None


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
        process = self.spawn(argv, f"{label}-{self._trace_count}", {**(extra or {}), "RMAC_FRAME_TRACE": str(trace)})
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
            f"output HEADLESS-1 mode {OUTPUT_W}x{OUTPUT_H} position 0 0\n"
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

    def measure_app(self, name: str, binaries: list[str], app_id: str) -> dict[str, Any]:
        binary = self.bin(*binaries)
        if binary is None:
            return {"error": f"none of {binaries} found under --bin-dir"}
        start = time.monotonic()
        process, trace = self.traced([str(binary)], name)
        window = self.wait_for(lambda: self.window_by_app_id(app_id), timeout=20.0)
        first_frame_ms = (time.monotonic() - start) * 1000.0 if window else None
        icons_painted_ms = None
        if name not in NO_QUIESCENCE:
            settled = quiescence_wait(trace, deadline=start + QUIESCENCE_TIMEOUT_S)
            if settled is not None:
                icons_painted_ms = (settled - start) * 1000.0
        self.stop(process)
        result: dict[str, Any] = {
            "first_frame_ms": first_frame_ms,
            "first_frame_pass": first_frame_ms is not None and first_frame_ms <= APP_FIRST_FRAME_TARGET_MS,
        }
        if name in NO_QUIESCENCE:
            result["icons_painted_ms"] = None
            result["icons_painted_note"] = "animates continuously (not scored)"
        else:
            result["icons_painted_ms"] = icons_painted_ms
            result["icons_painted_pass"] = (
                icons_painted_ms is not None and icons_painted_ms <= APP_FIRST_FRAME_TARGET_MS
            )
        if window is None:
            result["error"] = "window never appeared in `niri msg windows`"
        return result

    # -- panels ------------------------------------------------------------

    def measure_dispatch_panel(self, binary_name: str, shortcut_id: str) -> dict[str, Any]:
        binary = self.bin(binary_name)
        if binary is None:
            return {"error": f"{binary_name} not found under --bin-dir"}
        dispatcher = self.bin("rmac-shortcut-dispatch")
        if dispatcher is None:
            return {"error": "rmac-shortcut-dispatch not found under --bin-dir"}
        process, trace = self.traced([str(binary)], shortcut_id)
        endpoint = self.runtime / "rmac" / f"shortcut-{shortcut_id}.sock"
        if not self.wait_for(endpoint.exists, timeout=20.0):
            self.stop(process)
            return {"error": f"{binary_name} did not register its shortcut endpoint"}
        time.sleep(0.3)  # let the resident process's own startup settle first
        start = time.monotonic()
        result = subprocess.run(
            [str(dispatcher), shortcut_id], env=self.env, capture_output=True, text=True, timeout=10,
        )
        if result.returncode != 0:
            self.stop(process)
            return {"error": f"rmac-shortcut-dispatch {shortcut_id} failed: {result.stderr[:200]}"}
        settled = self.wait_for(
            lambda: len(read_trace(trace)) > 0 and _has_present_after(trace, start), timeout=5.0,
        )
        open_ms = (time.monotonic() - start) * 1000.0 if settled else None
        self.input.key("escape")
        time.sleep(0.3)
        self.stop(process)
        return {
            "open_ms": open_ms,
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
        start = time.monotonic()
        self.input.key("ctrl-up")
        settled = self.wait_for(lambda: _has_present_after(trace, start), timeout=5.0)
        open_ms = (time.monotonic() - start) * 1000.0 if settled else None
        self.input.key("escape")
        time.sleep(0.3)
        self.stop(process)
        return {
            "open_ms": open_ms,
            "open_pass": open_ms is not None and open_ms <= PANEL_OPEN_TARGET_MS,
            **({"error": "no present recorded after Ctrl+Up"} if open_ms is None else {}),
        }

    def measure_app_switcher(self) -> dict[str, Any]:
        # Unlike Mission Control, App Switcher is a fresh spawn per Mod+Tab
        # (packaging/rmac-session/shell.kdl), so its "open" latency is a
        # cold process start to first present, measured directly rather
        # than through a resident service's key bind.
        app_switcher = self.bin("rmac-app-switcher", "app-switcher")
        if app_switcher is None:
            return {"error": "rmac-app-switcher not found under --bin-dir"}
        start = time.monotonic()
        process, trace = self.traced([str(app_switcher), "next"], "app-switcher")
        settled = self.wait_for(lambda: _has_present_after(trace, start), timeout=10.0)
        open_ms = (time.monotonic() - start) * 1000.0 if settled else None
        self.stop(process)
        return {
            "open_ms": open_ms,
            "open_pass": open_ms is not None and open_ms <= PANEL_OPEN_TARGET_MS,
            **({"error": "no present recorded"} if open_ms is None else {}),
        }

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
            "apps": {},
            "panels": {},
            "not_measured": ["settings-pane-switching"],
        }
        for name, (binaries, app_id) in APPS.items():
            print(f"app: {name}...", flush=True)
            try:
                report["apps"][name] = self.measure_app(name, binaries, app_id)
            except StepFailed as error:
                report["apps"][name] = {"error": str(error)}
        for name, (binary, shortcut_id) in DISPATCH_PANELS.items():
            print(f"panel: {name}...", flush=True)
            try:
                report["panels"][name] = self.measure_dispatch_panel(binary, shortcut_id)
            except StepFailed as error:
                report["panels"][name] = {"error": str(error)}
        print("panel: mission-control...", flush=True)
        report["panels"]["mission-control"] = self.measure_mission_control()
        print("panel: app-switcher...", flush=True)
        report["panels"]["app-switcher"] = self.measure_app_switcher()
        self.finish()
        return report


def _has_present_after(trace: Path, start: float) -> bool:
    """Whether `trace` already has a `present` row, polled after `start`
    (wall clock) -- the trace's own timestamps are relative to that
    process's `init()`, not comparable across processes, so this only
    checks presence, and the caller's own wall clock supplies the latency."""

    return any(event == "present" for event, _ in read_trace(trace))


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
    lines.append("| App | First frame | Pass | Icons painted | Pass |")
    lines.append("|---|---:|---|---:|---|")
    for name, entry in report.get("apps", {}).items():
        if "error" in entry and entry.get("first_frame_ms") is None:
            lines.append(f"| {name} | -- | error: {entry['error']} | -- | -- |")
            continue
        icons_cell = entry.get("icons_painted_note", _fmt_ms(entry.get("icons_painted_ms")))
        icons_pass = "--" if "icons_painted_note" in entry else _fmt_pass(entry, "icons_painted_pass")
        lines.append(
            f"| {name} | {_fmt_ms(entry.get('first_frame_ms'))} | "
            f"{_fmt_pass(entry, 'first_frame_pass')} | {icons_cell} | {icons_pass} |"
        )
    lines.append("")
    lines.append("## Panels")
    lines.append("")
    lines.append("| Panel | Open | Pass |")
    lines.append("|---|---:|---|")
    for name, entry in report.get("panels", {}).items():
        lines.append(f"| {name} | {_fmt_ms(entry.get('open_ms'))} | {_fmt_pass(entry, 'open_pass')} |")
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
