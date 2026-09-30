#!/usr/bin/env python3
"""Beta 1 gate: 8-hour memory soak (docs/beta-checklist.md "Memory (8-hour
soak): per-app budget, no leak"; budgets in scripts/performance-budgets.json's
"memory" section).

    setsid nohup python3 scripts/behavior/run_memory_soak.py \\
        --bin-dir ~/rmac-release/inputs-20260929T1945 \\
        --output-dir ~/rmac-coord/soak-2026-09-29 \\
        --duration-hours 8 \\
        >~/rmac-coord/soak-2026-09-29/launcher.log 2>&1 &

This starts one instance each of ten release apps (Files, Text Editor,
System Settings, Calculator, Clock, Weather, System Monitor, Notes, Preview,
Terminal) plus three shell pieces that make sense headless (Wallpaper, Top
Bar, Dock -- unlike lock-coordinator/setup-assistant/update-check, none of
these three need the real systemd --user session, a timer, or first-login
state) inside a nested, headless niri, itself nested inside a headless Sway,
each with its own fresh XDG_RUNTIME_DIR and a temporary HOME/XDG set --
exactly run_lulo.py's and run_niri_minimize.py's isolation, reused here
unchanged (`isolated_environment`, `refuse_live_session`, `reap`,
`remove_tree`). Every --sample-interval-seconds (default 300s = 5 min) it
appends one JSON line per tracked process to <output-dir>/samples.jsonl:
RSS and VmSwap (`/proc/<pid>/status` VmRSS, VmSwap), PSS and its
anonymous/file/swap breakdown (`/proc/<pid>/smaps_rollup` Pss, Pss_Anon,
Pss_File, SwapPss), cumulative CPU time, thread count and fd count, each
summed over the process's whole descendant tree (a shell/child a Terminal
or Files spawns is still this app's memory). RSS and plain PSS fall when the
kernel reclaims a process's idle clean/file-backed pages under memory
pressure, even though the process has freed nothing; Pss_Anon + SwapPss is
the process's real private footprint (its anonymous resident pages plus the
anonymous pages the kernel has swapped out) and does not fall just because
the machine is busy -- see scripts/linux/analyze-soak.py, which evaluates
leak growth on that sum wherever a sample has it. Every
--activity-interval-seconds (default 1800s = 30 min) it drives one gentle
round of activity through the nested virtual input (wlinput.py, which
refuses to inject into the live session): a Files new-window/close, and a
short typed line in Text Editor, into a file under the run's own temporary
HOME -- never the owner's real files.

Analyse the resulting samples.jsonl with scripts/linux/analyze-soak.py.

This script never invokes cargo, never touches /tmp/lulo-cargo.lock, and
nices itself (default 10) so it does not compete with a concurrent build.
"""

from __future__ import annotations

import argparse
import fcntl
import json
import os
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

LAVAPIPE_GLOB = "/usr/share/vulkan/icd.d/*lvp*.json"
OUTPUT_W, OUTPUT_H = 1440, 900

# (tracked name, executable, niri app_id or None for a shell piece with no
# reliably-mapped window). Matches scripts/linux/measure-budgets.py's APPS
# table and each crate's org.rmac.*.desktop app_id.
SHELL_PIECES: tuple[tuple[str, str], ...] = (
    ("wallpaper", "rmac-wallpaper"),
    ("top-bar", "rmac-top-bar"),
    ("dock", "rmac-dock"),
)
APPS: tuple[tuple[str, str, str], ...] = (
    ("files", "rmac-files", "org.rmac.Files"),
    ("text-editor", "rmac-text-editor", "org.rmac.TextEditor"),
    ("system-settings", "rmac-system-settings", "org.rmac.SystemSettings"),
    ("calculator", "rmac-calculator", "org.rmac.Calculator"),
    ("clock", "rmac-clock", "org.rmac.Clock"),
    ("weather", "rmac-weather", "org.rmac.Weather"),
    ("system-monitor", "rmac-system-monitor", "org.rmac.SystemMonitor"),
    ("notes", "rmac-notes", "org.rmac.Notes"),
    ("preview", "rmac-preview", "org.rmac.Preview"),
    ("terminal", "rmac-terminal", "org.rmac.Terminal"),
)
WINDOW_TIMEOUT_SECONDS = 20.0


# --------------------------------------------------------------------------
# Pure /proc parsing -- unit-tested from scripts/test_run_memory_soak.py
# without a live /proc tree.
# --------------------------------------------------------------------------


def parse_proc_stat_ppid_and_ticks(text: str) -> tuple[int, int, int]:
    """Return (ppid, utime_ticks, stime_ticks) from a `/proc/<pid>/stat` line."""

    text = text.strip()
    close_paren = text.rindex(")")
    rest = text[close_paren + 1 :].split()
    if len(rest) < 15:
        raise ValueError("not enough fields in /proc/<pid>/stat")
    return int(rest[1]), int(rest[11]), int(rest[12])


def parse_status_fields(text: str) -> dict[str, int]:
    """Return {"vm_rss_kib": ..., "threads": ..., "vm_swap_kib": ...} from
    `/proc/<pid>/status`. VmSwap defaults to 0 when the kernel omits it
    (e.g. a build without swap accounting) rather than raising, since it is
    supplementary to the required VmRSS/Threads fields."""

    fields: dict[str, int] = {}
    for line in text.splitlines():
        if line.startswith("VmRSS:"):
            fields["vm_rss_kib"] = int(line.split()[1])
        elif line.startswith("Threads:"):
            fields["threads"] = int(line.split()[1])
        elif line.startswith("VmSwap:"):
            fields["vm_swap_kib"] = int(line.split()[1])
    if "vm_rss_kib" not in fields or "threads" not in fields:
        raise ValueError("missing VmRSS or Threads in /proc/<pid>/status")
    fields.setdefault("vm_swap_kib", 0)
    return fields


def parse_smaps_rollup_pss_kib(text: str) -> int:
    for line in text.splitlines():
        if line.startswith("Pss:"):
            return int(line.split()[1])
    raise ValueError("no Pss line found in smaps_rollup")


def parse_smaps_rollup_fields(text: str) -> dict[str, int]:
    """Return {"pss_kib", "pss_anon_kib", "pss_file_kib", "swap_pss_kib"}
    from `/proc/<pid>/smaps_rollup`. Pss_Anon + SwapPss is a process's real
    private footprint -- its anonymous resident pages plus the anonymous
    pages the kernel has swapped out -- which does not shrink just because
    the machine is under memory pressure and reclaims clean/file-backed
    pages, unlike plain Pss or VmRSS. The three breakdown fields default to
    0 when absent (an older kernel without the Pss_Anon/Pss_File/SwapPss
    rollup lines) rather than raising, since Pss alone is still usable."""

    fields: dict[str, int] = {}
    for line in text.splitlines():
        if line.startswith("Pss:"):
            fields["pss_kib"] = int(line.split()[1])
        elif line.startswith("Pss_Anon:"):
            fields["pss_anon_kib"] = int(line.split()[1])
        elif line.startswith("Pss_File:"):
            fields["pss_file_kib"] = int(line.split()[1])
        elif line.startswith("SwapPss:"):
            fields["swap_pss_kib"] = int(line.split()[1])
    if "pss_kib" not in fields:
        raise ValueError("no Pss line found in smaps_rollup")
    fields.setdefault("pss_anon_kib", 0)
    fields.setdefault("pss_file_kib", 0)
    fields.setdefault("swap_pss_kib", 0)
    return fields


def cpu_seconds_from_ticks(ticks: int, hertz: int) -> float:
    if hertz <= 0:
        raise ValueError("hertz must be greater than zero")
    return ticks / hertz


# --------------------------------------------------------------------------
# Live /proc sampling.
# --------------------------------------------------------------------------


def _read_text(path: Path) -> Optional[str]:
    try:
        return path.read_text()
    except (OSError, PermissionError):
        return None


def build_descendant_set(root_pid: int) -> set[int]:
    """All live PIDs descended from (and including) `root_pid`, from one
    /proc scan. Mirrors scripts/linux/measure-budgets.py's helper of the
    same name; duplicated here so this script has no cross-module import
    on a hyphenated filename."""

    children: dict[int, list[int]] = {}
    all_pids: set[int] = set()
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        pid = int(entry.name)
        stat_text = _read_text(entry / "stat")
        if stat_text is None:
            continue
        try:
            ppid, _utime, _stime = parse_proc_stat_ppid_and_ticks(stat_text)
        except (ValueError, IndexError):
            continue
        all_pids.add(pid)
        children.setdefault(ppid, []).append(pid)

    if root_pid not in all_pids:
        return set()
    result = {root_pid}
    queue = [root_pid]
    while queue:
        current = queue.pop()
        for child in children.get(current, ()):
            if child not in result:
                result.add(child)
                queue.append(child)
    return result


def sample_tree(root_pid: int, hertz: int) -> Optional[dict[str, Any]]:
    """One point-in-time sample summed over root_pid's descendant tree."""

    pids = build_descendant_set(root_pid)
    if not pids:
        return None
    rss_kib = pss_kib = threads = fds = 0
    vm_swap_kib = pss_anon_kib = pss_file_kib = swap_pss_kib = 0
    cpu_ticks = 0
    for pid in pids:
        proc = Path(f"/proc/{pid}")
        stat_text = _read_text(proc / "stat")
        status_text = _read_text(proc / "status")
        rollup_text = _read_text(proc / "smaps_rollup")
        if stat_text is not None:
            try:
                _ppid, utime, stime = parse_proc_stat_ppid_and_ticks(stat_text)
                cpu_ticks += utime + stime
            except (ValueError, IndexError):
                pass
        if status_text is not None:
            try:
                fields = parse_status_fields(status_text)
                rss_kib += fields["vm_rss_kib"]
                threads += fields["threads"]
                vm_swap_kib += fields["vm_swap_kib"]
            except ValueError:
                pass
        if rollup_text is not None:
            try:
                rollup_fields = parse_smaps_rollup_fields(rollup_text)
                pss_kib += rollup_fields["pss_kib"]
                pss_anon_kib += rollup_fields["pss_anon_kib"]
                pss_file_kib += rollup_fields["pss_file_kib"]
                swap_pss_kib += rollup_fields["swap_pss_kib"]
            except ValueError:
                pass
        try:
            fds += len(os.listdir(proc / "fd"))
        except OSError:
            pass
    return {
        "process_count": len(pids),
        "rss_kib": rss_kib,
        "pss_kib": pss_kib,
        "vm_swap_kib": vm_swap_kib,
        "pss_anon_kib": pss_anon_kib,
        "pss_file_kib": pss_file_kib,
        "swap_pss_kib": swap_pss_kib,
        "cpu_seconds": cpu_seconds_from_ticks(cpu_ticks, hertz),
        "threads": threads,
        "fds": fds,
    }


# --------------------------------------------------------------------------
# Inner run: Sway, nested niri, the tracked processes, the sampling loop.
# --------------------------------------------------------------------------


class Soak:
    def __init__(self, args: argparse.Namespace, work: Path) -> None:
        self.args = args
        self.env = dict(os.environ)
        run_lulo.refuse_live_session(self.env)
        self.runtime = Path(self.env["XDG_RUNTIME_DIR"])
        self.logs = work / "logs"
        self.logs.mkdir(exist_ok=True)
        self.output_dir = args.output_dir
        self.output_dir.mkdir(parents=True, exist_ok=True)
        self.hertz = os.sysconf("SC_CLK_TCK")
        self.children: list[subprocess.Popen] = []
        # name -> {"process": Popen, "app_id": Optional[str]}
        self.tracked: dict[str, dict[str, Any]] = {}
        self.stop_requested = False
        self.activity_round = 0

    def log(self, message: str) -> None:
        print(f"[{time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())}] {message}", flush=True)

    def spawn(self, argv: list[str], name: str, extra: dict[str, str] | None = None) -> subprocess.Popen:
        env = {**self.env, **(extra or {})}
        process = subprocess.Popen(
            argv, env=env, stdout=open(self.logs / f"{name}.log", "w"),
            stderr=subprocess.STDOUT, close_fds=True, start_new_session=True,
        )
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

    # -- compositors ---------------------------------------------------

    def start_compositors(self) -> None:
        self.locks = []
        for taken in ("wayland-0.lock", "wayland-1.lock"):
            handle = open(self.runtime / taken, "w")
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.locks.append(handle)
        sway_conf = self.logs / "sway.conf"
        sway_conf.write_text(
            "xwayland disable\ndefault_border none\n"
            f"output HEADLESS-1 mode {OUTPUT_W}x{OUTPUT_H} position 0 0\n"
        )
        self.spawn(
            ["sway", "--unsupported-gpu", "--config", str(sway_conf)], "sway",
            {"WLR_BACKENDS": "headless", "WLR_HEADLESS_OUTPUTS": "1",
             "WLR_LIBINPUT_NO_DEVICES": "1", "WLR_RENDERER": "pixman"},
        )
        self.sway_display = self.wait_for(lambda: next(
            (p.name for p in self.runtime.glob("wayland-*") if not p.name.endswith(".lock")), None))
        if not self.sway_display:
            raise SystemExit("sway did not start")

        config = self.logs / "niri.kdl"
        config.write_text((REPO / "packaging/rmac-session/shell.kdl").read_text(encoding="utf-8"))
        validate = subprocess.run(
            [self.args.niri, "validate", "-c", str(config)], env=self.env,
            capture_output=True, text=True, timeout=20,
        )
        if validate.returncode != 0:
            raise SystemExit(f"niri validate rejected shell.kdl: {validate.stderr[-500:]}")

        before = {p.name for p in self.runtime.glob("wayland-*")}
        self.spawn(
            [self.args.niri, "-c", str(config)], "niri",
            {"WAYLAND_DISPLAY": self.sway_display, "LIBGL_ALWAYS_SOFTWARE": "1"},
        )
        socket = self.wait_for(lambda: next(iter(self.runtime.glob("niri.*.sock")), None))
        display = self.wait_for(lambda: next(
            (p.name for p in self.runtime.glob("wayland-*")
             if not p.name.endswith(".lock") and p.name not in before), None))
        if not (socket and display):
            raise SystemExit("niri did not start")
        self.env.update({"WAYLAND_DISPLAY": display, "NIRI_SOCKET": str(socket)})
        subprocess.run(
            ["busctl", "--user", "set-property", "org.a11y.Bus", "/org/a11y/bus",
             "org.a11y.Status", "IsEnabled", "b", "true"],
            env=self.env, capture_output=True, timeout=10, check=False,
        )
        time.sleep(3)
        # Keys go to Sway, whose only window is niri; a virtual keyboard on
        # niri itself would bypass niri's own key handling (run_niri_minimize.py).
        self.keys = wlinput.Wayland({**self.env, "WAYLAND_DISPLAY": self.sway_display,
                                     "RMAC_BEHAVIOR_NESTED": "1"})

    def niri(self, *request: str):
        result = subprocess.run(
            [self.args.niri, "msg", "-j", *request], env=self.env,
            capture_output=True, text=True, timeout=10,
        )
        return json.loads(result.stdout) if result.stdout.strip() else None

    def window_by_app_id(self, app_id: str):
        for window in self.niri("windows") or []:
            if window.get("app_id") == app_id:
                return window
        return None

    # -- launching the tracked set --------------------------------------

    def binary(self, executable: str) -> Path:
        candidate = self.args.bin_dir / executable
        if not (candidate.is_file() and os.access(candidate, os.X_OK)):
            raise SystemExit(f"{executable} not found (or not executable) under {self.args.bin_dir}")
        return candidate

    def start_tracked(self) -> None:
        vk_icd = self.env.get("VK_ICD_FILENAMES", "")
        app_env = {"WAYLAND_DISPLAY": self.env["WAYLAND_DISPLAY"], "NIRI_SOCKET": self.env["NIRI_SOCKET"],
                   "VK_ICD_FILENAMES": vk_icd}
        for name, executable in SHELL_PIECES:
            process = self.spawn([str(self.binary(executable))], name, app_env)
            self.tracked[name] = {"process": process, "app_id": None}
            self.log(f"started shell piece {name} (pid {process.pid})")
        time.sleep(1.5)
        for name, executable, app_id in APPS:
            process = self.spawn([str(self.binary(executable))], name, app_env)
            self.tracked[name] = {"process": process, "app_id": app_id}
            self.log(f"started {name} (pid {process.pid})")
            mapped = self.wait_for(lambda aid=app_id: self.window_by_app_id(aid), WINDOW_TIMEOUT_SECONDS, 0.3)
            if mapped is None and process.poll() is None:
                self.log(f"warning: {name} has not mapped a window after {WINDOW_TIMEOUT_SECONDS:.0f}s (still running)")
            elif process.poll() is not None:
                self.log(f"warning: {name} exited during startup (status {process.returncode})")
        self.log(f"all {len(self.tracked)} tracked processes launched")

    # -- sampling --------------------------------------------------------

    def take_sample(self, elapsed_seconds: float, handle) -> tuple[int, int]:
        alive_count = dead_count = 0
        timestamp = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
        for name, info in self.tracked.items():
            process: subprocess.Popen = info["process"]
            alive = process.poll() is None
            record: dict[str, Any] = {
                "ts": timestamp,
                "elapsed_seconds": round(elapsed_seconds, 1),
                "app": name,
                "pid": process.pid,
                "alive": alive,
            }
            if alive:
                alive_count += 1
                sample = sample_tree(process.pid, self.hertz)
                if sample is None:
                    record["alive"] = False
                    dead_count += 1
                else:
                    record.update(sample)
            else:
                dead_count += 1
                if "exit_code" not in info:
                    info["exit_code"] = process.returncode
                    self.log(f"{name} (pid {process.pid}) is no longer running (exit code {process.returncode})")
                record["exit_code"] = info.get("exit_code")
            handle.write(json.dumps(record, sort_keys=True) + "\n")
        handle.flush()
        os.fsync(handle.fileno())
        return alive_count, dead_count

    # -- light, gentle activity every --activity-interval-seconds --------

    def light_activity(self) -> None:
        self.activity_round += 1
        self.log(f"activity round {self.activity_round}: begin")
        try:
            self._files_new_window_close()
        except Exception as error:  # noqa: BLE001
            self.log(f"activity round {self.activity_round}: Files step failed: {error}")
        try:
            self._text_editor_type()
        except Exception as error:  # noqa: BLE001
            self.log(f"activity round {self.activity_round}: Text Editor step failed: {error}")
        self.log(f"activity round {self.activity_round}: done")

    def _focus(self, app_id: str) -> Optional[dict[str, Any]]:
        window = self.window_by_app_id(app_id)
        if window is None:
            return None
        subprocess.run(
            [self.args.niri, "msg", "action", "focus-window", "--id", str(window["id"])],
            env=self.env, capture_output=True, timeout=10, check=False,
        )
        time.sleep(0.5)
        return window

    def _files_new_window_close(self) -> None:
        if self.tracked["files"]["process"].poll() is not None:
            return
        if self._focus("org.rmac.Files") is None:
            return
        self.keys.key("cmd-n")
        time.sleep(1.5)
        self.keys.key("cmd-w")
        time.sleep(0.5)

    def _text_editor_type(self) -> None:
        if self.tracked["text-editor"]["process"].poll() is not None:
            return
        if self._focus("org.rmac.TextEditor") is None:
            return
        self.keys.type_text(f"soak activity round {self.activity_round} at {time.strftime('%H:%M:%S')}\n")
        time.sleep(0.5)

    # -- run ---------------------------------------------------------------

    def run(self) -> int:
        self.start_compositors()
        self.start_tracked()
        signal.signal(signal.SIGTERM, self._on_signal)
        signal.signal(signal.SIGINT, self._on_signal)

        samples_path = self.output_dir / "samples.jsonl"
        events_path = self.output_dir / "events.log"
        started = time.monotonic()
        deadline = started + self.args.duration_hours * 3600
        next_sample = started
        next_activity = started + self.args.activity_interval_seconds if self.args.activity_interval_seconds > 0 else None
        sample_count = 0

        with open(samples_path, "a") as samples_handle:
            self.log(f"soak starting: writing to {samples_path}")
            while not self.stop_requested and time.monotonic() < deadline:
                now = time.monotonic()
                if now >= next_sample:
                    alive, dead = self.take_sample(now - started, samples_handle)
                    sample_count += 1
                    self.log(f"sample {sample_count}: {alive} alive, {dead} dead")
                    next_sample += self.args.sample_interval_seconds
                if next_activity is not None and now >= next_activity:
                    self.light_activity()
                    next_activity += self.args.activity_interval_seconds
                wake_targets = [next_sample] + ([next_activity] if next_activity is not None else []) + [deadline]
                sleep_for = max(0.2, min(wake_targets) - time.monotonic())
                time.sleep(min(sleep_for, 2.0))
            # A final sample at the natural or requested end.
            alive, dead = self.take_sample(time.monotonic() - started, samples_handle)
            sample_count += 1
            self.log(f"final sample {sample_count}: {alive} alive, {dead} dead")

        (self.output_dir / "done.json").write_text(json.dumps({
            "finished_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "elapsed_seconds": round(time.monotonic() - started, 1),
            "requested_duration_hours": self.args.duration_hours,
            "sample_count": sample_count,
            "activity_rounds": self.activity_round,
            "stopped_early": self.stop_requested,
        }, indent=2) + "\n")
        events_path.write_text("soak finished; per-process stdout/stderr logs copied to process-logs/\n")
        self._archive_process_logs()
        return self.finish()

    def _archive_process_logs(self) -> None:
        """Copy each process's log out of the temp work dir before it is
        removed, so a crash overnight still has a root cause in the morning."""

        import shutil

        destination = self.output_dir / "process-logs"
        destination.mkdir(exist_ok=True)
        for log_file in self.logs.glob("*.log"):
            try:
                shutil.copy2(log_file, destination / log_file.name)
            except OSError as error:
                self.log(f"could not archive {log_file.name}: {error}")

    def _on_signal(self, signum, _frame) -> None:
        self.log(f"received signal {signum}; finishing the current sample and stopping")
        self.stop_requested = True

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
        return 0


# --------------------------------------------------------------------------
# Outer run: the isolated environment (shared with run_lulo.py).
# --------------------------------------------------------------------------


def outer(args: argparse.Namespace, argv: list[str]) -> int:
    for tool in ("sway", "niri", "dbus-run-session", "busctl"):
        if subprocess.run(["which", tool], capture_output=True).returncode != 0:
            raise SystemExit(f"{tool} is required")
    try:
        os.nice(args.nice)
    except OSError:
        pass
    args.output_dir.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix="lulo-soak-"))
    try:
        env = run_lulo.isolated_environment(work)
        run_lulo.refuse_live_session(env)
        for key in ("WLR_BACKENDS", "WLR_HEADLESS_OUTPUTS", "WLR_LIBINPUT_NO_DEVICES", "WLR_RENDERER",
                    "LIBGL_ALWAYS_SOFTWARE"):
            env.pop(key, None)  # set per process instead: niri, Sway and GPUI differ
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
        with open(args.output_dir / "session.log", "w") as log:
            status = subprocess.call(command, env=env, close_fds=True, stderr=log, stdout=log)
        return status
    finally:
        if run_lulo.reap(work / "runtime"):
            time.sleep(1.0)
            run_lulo.reap(work / "runtime")
        if args.keep:
            print(f"kept {work}", file=sys.stderr)
        else:
            run_lulo.remove_tree(work)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--niri", default="/usr/bin/niri")
    parser.add_argument("--bin-dir", type=Path, required=True,
                         help="directory with the release rmac-* binaries (e.g. ~/rmac-release/inputs-<tag>)")
    parser.add_argument("--output-dir", type=Path, required=True,
                         help="persistent directory for samples.jsonl/done.json (e.g. ~/rmac-coord/soak-<date>)")
    parser.add_argument("--duration-hours", type=float, default=8.0)
    parser.add_argument("--sample-interval-seconds", type=float, default=300.0)
    parser.add_argument("--activity-interval-seconds", type=float, default=1800.0,
                         help="0 disables the periodic Files/Text Editor activity")
    parser.add_argument("--nice", type=int, default=10)
    parser.add_argument("--keep", action="store_true", help="keep the temporary work dir on exit (debugging)")
    parser.add_argument("--inner", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.duration_hours <= 0:
        parser.error("--duration-hours must be greater than zero")
    if args.sample_interval_seconds <= 0:
        parser.error("--sample-interval-seconds must be greater than zero")
    if args.activity_interval_seconds < 0:
        parser.error("--activity-interval-seconds must be zero or greater")
    args.bin_dir = args.bin_dir.expanduser().resolve()
    args.output_dir = args.output_dir.expanduser().resolve()
    return args


def main() -> int:
    args = parse_args()
    if args.inner:
        return Soak(args, args.inner).run()
    argv = [a for a in sys.argv[1:] if a != "--keep"]
    return outer(args, argv)


if __name__ == "__main__":
    sys.exit(main())
