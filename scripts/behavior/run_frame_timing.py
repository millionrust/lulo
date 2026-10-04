#!/usr/bin/env python3
"""Measure input-to-present latency and frame-budget compliance on Lulo.

    python3 scripts/behavior/run_frame_timing.py --niri PATH --bin-dir DIR
        [--bin-dir DIR ...] --json-output PATH [--markdown-output PATH]
        [--profile iterate|release] [--keep]

docs/beta-checklist.md has two performance rows marked "Not yet run":
"Input to visible response p95 <= 50 ms" and the 60/120 Hz animation frame
budget (">=99% / >=95% within budget"). This is the harness that fills
them in, by driving four real interactions inside a private *nested niri*
session (never the owner's live session -- see AGENT-BRIEF.md) and reading
the per-frame trace shell/compat/gpui_linux/src/linux/wayland/frame_trace.rs
writes when RMAC_FRAME_TRACE is set:

  - typing in Text Editor;
  - scrolling Files' list;
  - opening and closing Control Centre (rmac-quick-settings, via its
    shortcut endpoint -- the same mechanism
    scripts/interaction/lulo_probe.py's ShellSession.dispatch uses);
  - Mission Control (niri's own Ctrl+Up bind in the shipped shell.kdl,
    reaching the resident `rmac-mission-control --service`).

GPU path: the traced processes above (Text Editor, Files, Quick Settings,
the Mission Control service) are launched with no VK_ICD_FILENAMES
override, so they pick up the laptop's real Vulkan driver, the same one
GPUI's WgpuRenderer uses live. Only the *nested niri's own* compositing
(onto the outer headless Sway's virtual output) and the outer Sway itself
are forced to software (LIBGL_ALWAYS_SOFTWARE / WLR_RENDERER=pixman) -- the
same arrangement scripts/behavior/run_niri_minimize.py uses, since that
layer is just hosting the session, not what this script measures. One
consequence worth keeping in mind when reading the numbers: the vblank
pacing a traced window's `frame_callback` rides on is the outer headless
Sway's synthetic output clock, not the laptop panel's real refresh rate, so
treat the frame-budget share as "GPU+CPU per-frame cost against the 16.7 /
8.3 ms budgets", not a live-hardware vsync measurement.

Isolation is the same skeleton as run_niri_minimize.py and lulo_probe.py's
"shell" harness: a private dbus-run-session, a temporary HOME and
XDG_RUNTIME_DIR, wayland-0/1 held so the inner niri is never handed the
live session's socket name, and all input injected into this run's Sway
through wlinput.py (niri is just one of its clients). No AT-SPI is used
here (this harness needs only process control, niri's IPC and the frame
trace files), so, unlike lulo_probe.py, pyatspi is never imported.
"""

from __future__ import annotations

import argparse
import bisect
import csv
import fcntl
import json
import math
import os
import shutil
import signal
import statistics
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

OUTPUT_W, OUTPUT_H = run_lulo.OUTPUT_W, run_lulo.OUTPUT_H
BUDGET_60HZ_MS = 1000.0 / 60.0
BUDGET_120HZ_MS = 1000.0 / 120.0
INPUT_TO_PRESENT_BUDGET_MS = 50.0


class StepFailed(RuntimeError):
    pass


# --------------------------------------------------------------------------
# Trace parsing -- pure, unit-tested from scripts/test_run_frame_timing.py
# without a live session.
# --------------------------------------------------------------------------


def read_trace(path: Path) -> list[tuple[str, int]]:
    """Parse one frame_trace.rs CSV file: (event, micros-since-init) rows."""

    if not path.exists():
        return []
    rows: list[tuple[str, int]] = []
    with path.open(newline="") as handle:
        reader = csv.reader(handle)
        next(reader, None)  # header: "event,micros"
        for row in reader:
            if len(row) != 2:
                continue
            event, micros_text = row
            try:
                rows.append((event, int(micros_text)))
            except ValueError:
                continue
    return rows


def input_to_present_latencies_ms(events: list[tuple[str, int]]) -> list[float]:
    """For each `input` timestamp, the latency to the first `present` that
    follows it (an input with no later present, e.g. right at the end of a
    trace, contributes nothing)."""

    presents = sorted(micros for event, micros in events if event == "present")
    out: list[float] = []
    for event, micros in events:
        if event != "input":
            continue
        index = bisect.bisect_left(presents, micros)
        if index < len(presents):
            out.append((presents[index] - micros) / 1000.0)
    return out


def frame_durations_ms(events: list[tuple[str, int]]) -> list[float]:
    """Pair each `draw_start` with the `present` that immediately follows
    it; a `draw_skip` (the renderer didn't present) discards that pending
    start rather than pairing it with a later, unrelated present."""

    out: list[float] = []
    pending_start: Optional[int] = None
    for event, micros in events:
        if event == "draw_start":
            pending_start = micros
        elif event == "present" and pending_start is not None:
            out.append((micros - pending_start) / 1000.0)
            pending_start = None
        elif event == "draw_skip":
            pending_start = None
    return out


def percentile_nearest_rank(values: list[float], pct: float) -> Optional[float]:
    if not values:
        return None
    ordered = sorted(values)
    rank = max(1, math.ceil(pct * len(ordered)))
    return ordered[rank - 1]


def share_within_budget(values: list[float], budget_ms: float) -> Optional[float]:
    if not values:
        return None
    return sum(1 for v in values if v <= budget_ms) / len(values)


def summarize_scenario(events: list[tuple[str, int]]) -> dict[str, Any]:
    latencies = input_to_present_latencies_ms(events)
    durations = frame_durations_ms(events)
    return {
        "frame_count": len(durations),
        "input_count": len(latencies),
        "input_to_present_p95_ms": percentile_nearest_rank(latencies, 0.95),
        "within_60hz_budget_share": share_within_budget(durations, BUDGET_60HZ_MS),
        "within_120hz_budget_share": share_within_budget(durations, BUDGET_120HZ_MS),
        "worst_frame_ms": max(durations) if durations else None,
    }


def merge_events(all_events: list[list[tuple[str, int]]]) -> dict[str, Any]:
    """Combine every scenario's own latencies/durations (not the raw
    per-process timestamps, which share no common clock across processes)
    into one overall figure."""

    latencies: list[float] = []
    durations: list[float] = []
    for events in all_events:
        latencies.extend(input_to_present_latencies_ms(events))
        durations.extend(frame_durations_ms(events))
    return {
        "frame_count": len(durations),
        "input_count": len(latencies),
        "input_to_present_p95_ms": percentile_nearest_rank(latencies, 0.95),
        "within_60hz_budget_share": share_within_budget(durations, BUDGET_60HZ_MS),
        "within_120hz_budget_share": share_within_budget(durations, BUDGET_120HZ_MS),
        "worst_frame_ms": max(durations) if durations else None,
    }


# --------------------------------------------------------------------------
# Outer process: build the isolated environment, then re-run inside it.
# --------------------------------------------------------------------------


def outer(args: argparse.Namespace, argv: list[str]) -> int:
    for tool in ("sway", "niri", "dbus-run-session"):
        if shutil.which(tool) is None:
            raise SystemExit(f"{tool} is required")
    journey_lock = open("/tmp/lulo-journey.lock", "w")
    fcntl.flock(journey_lock, fcntl.LOCK_EX)
    work = Path(tempfile.mkdtemp(prefix="lulo-frame-timing-"))
    try:
        env = run_lulo.isolated_environment(work)
        run_lulo.refuse_live_session(env)
        for key in ("WLR_BACKENDS", "WLR_HEADLESS_OUTPUTS", "WLR_LIBINPUT_NO_DEVICES",
                    "WLR_RENDERER", "LIBGL_ALWAYS_SOFTWARE", "VK_ICD_FILENAMES"):
            env.pop(key, None)  # set per process below: niri differs from the traced apps
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
        command = ["dbus-run-session", f"--config-file={config}", "--", sys.executable,
                   str(Path(__file__).resolve()), "--inner", str(work), *argv]
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
# Inner process: headless Sway hosting a nested niri, then the four scenarios.
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
        self.traces: dict[str, Path] = {}

    def bin(self, *candidates: str) -> Path:
        for directory in self.args.bin_dir:
            for candidate in candidates:
                found = Path(directory) / candidate
                if found.is_file() and os.access(found, os.X_OK):
                    return found.resolve()
        raise StepFailed(f"none of {candidates} found under {self.args.bin_dir}")

    def spawn(self, argv: list[str], label: str, extra: Optional[dict[str, str]] = None) -> subprocess.Popen:
        process = subprocess.Popen(argv, env={**self.env, **(extra or {})},
                                   stdout=open(self.logs / f"{label}.log", "w"),
                                   stderr=subprocess.STDOUT, close_fds=True)
        self.children.append(process)
        return process

    def traced(self, argv: list[str], label: str, extra: Optional[dict[str, str]] = None) -> subprocess.Popen:
        """Spawn a process whose RMAC_FRAME_TRACE output this run will read."""

        trace = self.logs / f"trace-{label}.csv"
        trace.unlink(missing_ok=True)
        self.traces[label] = trace
        return self.spawn(argv, label, {**(extra or {}), "RMAC_FRAME_TRACE": str(trace)})

    @staticmethod
    def wait_for(predicate, timeout: float = 20.0, step: float = 0.2):
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
        self.spawn(["sway", "--unsupported-gpu", "--config", str(sway_conf)], "sway",
                  {"WLR_BACKENDS": "headless", "WLR_HEADLESS_OUTPUTS": "1",
                   "WLR_LIBINPUT_NO_DEVICES": "1", "WLR_RENDERER": "pixman"})
        self.sway_display = self.wait_for(lambda: next(
            (p.name for p in self.runtime.glob("wayland-*") if not p.name.endswith(".lock")), None))
        if not self.sway_display:
            raise StepFailed("sway did not start")

        shell = (REPO / "packaging/rmac-session/shell.kdl").read_text(encoding="utf-8")
        mission_control = self.bin("rmac-mission-control")
        shell = shell.replace("/usr/libexec/rmac/rmac-mission-control", str(mission_control))
        niri_config = self.logs / "niri.kdl"
        niri_config.write_text(shell)
        validate = subprocess.run([self.args.niri, "validate", "-c", str(niri_config)], env=self.env,
                                  capture_output=True, text=True, timeout=15)
        if validate.returncode != 0:
            raise StepFailed(f"shipped shell.kdl does not validate: {validate.stderr[-500:]}")

        before = {p.name for p in self.runtime.glob("wayland-*")}
        self.spawn([self.args.niri, "-c", str(niri_config)], "niri",
                  {"WAYLAND_DISPLAY": self.sway_display, "LIBGL_ALWAYS_SOFTWARE": "1"})
        self.socket = self.wait_for(lambda: next(iter(self.runtime.glob("niri.*.sock")), None))
        self.display = self.wait_for(lambda: next(
            (p.name for p in self.runtime.glob("wayland-*")
             if not p.name.endswith(".lock") and p.name not in before), None))
        if not self.socket or not self.display:
            raise StepFailed("nested niri did not start under Sway")
        self.env.update({"WAYLAND_DISPLAY": self.display, "NIRI_SOCKET": str(self.socket)})
        # Input goes to the *parent* Sway: niri is just one of its clients,
        # exactly as run_niri_minimize.py and lulo_probe.py's ShellSession do.
        self.input = wlinput.Wayland({**self.env, "WAYLAND_DISPLAY": self.sway_display})
        # Resident Mission Control service: niri's own Ctrl+Up bind only
        # sends it a one-shot "show" message (docs/decisions/0014); it is
        # this long-lived process, not that transient CLI call, that renders
        # the overlay and is worth tracing.
        self.mission_control = self.traced([str(mission_control), "--service"], "mission-control")

    def niri_msg(self, *request: str) -> Any:
        result = subprocess.run([self.args.niri, "msg", "-j", *request], env=self.env,
                                capture_output=True, text=True, timeout=10)
        return json.loads(result.stdout) if result.stdout.strip() else None

    def window_by_app_id(self, app_id: str) -> Optional[dict[str, Any]]:
        return next((w for w in self.niri_msg("windows") or [] if w.get("app_id") == app_id), None)

    # -- scenarios -------------------------------------------------------

    def scenario_text_editor_typing(self) -> list[tuple[str, int]]:
        binary = self.bin("rmac-text-editor")
        process = self.traced([str(binary)], "text-editor")
        window = self.wait_for(lambda: self.window_by_app_id("org.rmac.TextEditor"), 30)
        if window is None:
            raise StepFailed("Text Editor did not map a window")
        time.sleep(1.5)  # let the first frames (font/atlas load) settle out
        text = ("The quick brown fox jumps over the lazy dog. " * 3).strip()
        self.input.type_text(text, delay=0.03)
        time.sleep(1.0)
        process.send_signal(signal.SIGTERM)
        process.wait(8)
        return read_trace(self.traces["text-editor"])

    def scenario_files_scrolling(self) -> list[tuple[str, int]]:
        sandbox = self.work / "files-sandbox"
        sandbox.mkdir(exist_ok=True)
        for index in range(300):
            (sandbox / f"file-{index:03d}.txt").write_text("")
        binary = self.bin("rmac-files")
        process = self.traced([str(binary), "--path", str(sandbox)], "files")
        window = self.wait_for(lambda: self.window_by_app_id("org.rmac.Files"), 30)
        if window is None:
            raise StepFailed("Files did not map a window")
        time.sleep(1.5)
        self.input.move(OUTPUT_W * 0.65, OUTPUT_H * 0.55, OUTPUT_W, OUTPUT_H)
        for _ in range(40):
            self.input.scroll(vertical=6.0)
            time.sleep(0.05)
        time.sleep(1.0)
        process.send_signal(signal.SIGTERM)
        process.wait(8)
        return read_trace(self.traces["files"])

    def scenario_control_centre(self) -> list[tuple[str, int]]:
        binary = self.bin("rmac-quick-settings")
        process = self.traced([str(binary)], "quick-settings")
        endpoint = self.runtime / "rmac" / "shortcut-quick-settings.sock"
        if not self.wait_for(endpoint.exists, 20):
            raise StepFailed("Quick Settings did not register its shortcut endpoint")
        dispatcher = self.bin("rmac-shortcut-dispatch")
        time.sleep(0.5)
        result = subprocess.run([str(dispatcher), "quick-settings"], env=self.env,
                                capture_output=True, text=True, timeout=10)
        if result.returncode != 0:
            raise StepFailed(f"rmac-shortcut-dispatch quick-settings failed: {result.stderr[:200]}")
        time.sleep(1.0)
        self.input.key("escape")
        time.sleep(1.0)
        process.send_signal(signal.SIGTERM)
        process.wait(8)
        return read_trace(self.traces["quick-settings"])

    def scenario_mission_control(self) -> list[tuple[str, int]]:
        # The service was started once in start() and keeps writing to the
        # same file handle for its whole life (frame_trace.rs opens the file
        # once, in init()), so this scenario's window is a line-count mark
        # rather than a fresh file: unlinking the path here would not
        # affect writes already going to that open file descriptor.
        trace = self.traces["mission-control"]
        mark = len(read_trace(trace))
        self.input.key("ctrl-up")
        time.sleep(1.0)
        self.input.key("escape")
        time.sleep(1.0)
        return read_trace(trace)[mark:]

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
            "budgets": {
                "input_to_present_p95_ms": INPUT_TO_PRESENT_BUDGET_MS,
                "within_60hz_budget_share": 0.99,
                "within_120hz_budget_share": 0.95,
            },
            "scenarios": {},
        }
        scenarios = {
            "text-editor-typing": self.scenario_text_editor_typing,
            "files-scrolling": self.scenario_files_scrolling,
            "control-centre": self.scenario_control_centre,
            "mission-control": self.scenario_mission_control,
        }
        all_events: list[list[tuple[str, int]]] = []
        for name, step in scenarios.items():
            print(f"Running {name}...", flush=True)
            try:
                events = step()
            except StepFailed as error:
                report["scenarios"][name] = {"error": str(error)}
                print(f"FAILED {name}: {error}", file=sys.stderr)
                continue
            all_events.append(events)
            report["scenarios"][name] = summarize_scenario(events)
        report["overall"] = merge_events(all_events)
        self.finish()
        return report


# --------------------------------------------------------------------------
# Markdown rendering -- pure, given an already-built report dict.
# --------------------------------------------------------------------------


def _fmt_ms(value: Optional[float]) -> str:
    return "n/a" if value is None else f"{value:.1f} ms"


def _fmt_pct(value: Optional[float]) -> str:
    return "n/a" if value is None else f"{value * 100:.1f}%"


def render_markdown(report: dict[str, Any]) -> str:
    lines = [f"# Frame timing -- {report.get('captured_at', 'unknown')}", ""]
    lines.append(f"Profile: {report.get('profile', 'unknown')} binaries.")
    lines.append("")
    budgets = report.get("budgets", {})
    lines.append(
        f"Budgets: input-to-present p95 <= {budgets.get('input_to_present_p95_ms')} ms; "
        f">= {budgets.get('within_60hz_budget_share', 0) * 100:.0f}% of frames within the 16.7 ms "
        f"(60 Hz) budget; >= {budgets.get('within_120hz_budget_share', 0) * 100:.0f}% within the "
        "8.3 ms (120 Hz) budget."
    )
    lines.append("")
    lines.append("| Scenario | Frames | Inputs | Input->present p95 | <=16.7 ms | <=8.3 ms | Worst frame |")
    lines.append("|---|---:|---:|---:|---:|---:|---:|")
    for name, scenario in report.get("scenarios", {}).items():
        if "error" in scenario:
            lines.append(f"| {name} | -- | -- | -- | -- | -- | error: {scenario['error']} |")
            continue
        lines.append(
            f"| {name} | {scenario['frame_count']} | {scenario['input_count']} | "
            f"{_fmt_ms(scenario['input_to_present_p95_ms'])} | "
            f"{_fmt_pct(scenario['within_60hz_budget_share'])} | "
            f"{_fmt_pct(scenario['within_120hz_budget_share'])} | "
            f"{_fmt_ms(scenario['worst_frame_ms'])} |"
        )
    overall = report.get("overall", {})
    lines.append(
        f"| **Overall** | {overall.get('frame_count', 0)} | {overall.get('input_count', 0)} | "
        f"{_fmt_ms(overall.get('input_to_present_p95_ms'))} | "
        f"{_fmt_pct(overall.get('within_60hz_budget_share'))} | "
        f"{_fmt_pct(overall.get('within_120hz_budget_share'))} | "
        f"{_fmt_ms(overall.get('worst_frame_ms'))} |"
    )
    lines.append("")
    return "\n".join(lines)


# --------------------------------------------------------------------------
# main
# --------------------------------------------------------------------------


def parse_args() -> tuple[argparse.Namespace, list[str]]:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--niri", default="/usr/bin/niri")
    parser.add_argument("--bin-dir", action="append", default=[], required=False)
    parser.add_argument("--profile", default="unknown", help="free-text label recorded in the report")
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
        failed = [name for name, scenario in report["scenarios"].items() if "error" in scenario]
        return 1 if failed else 0
    if not args.bin_dir:
        raise SystemExit("--bin-dir is required (at least once)")
    argv = [a for a in sys.argv[1:]]
    return outer(args, argv)


if __name__ == "__main__":
    sys.exit(main())
