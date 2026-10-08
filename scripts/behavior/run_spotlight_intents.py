#!/usr/bin/env python3
"""Spotlight's "Lulo can do this" rows, end to end, in a private session.

ADR 0024 phase 1. A private headless Sway + nested niri shell (the parallel
journeys' session), a private session bus that can D-Bus-activate
`rmac-intelligence-service`, and private XDG directories. Two modes:

**Fixture (default).** The model is stubbed (`RMAC_INTELLIGENCE_ENGINE=fixture`):
CI has no model file.

1. Lulo Intelligence **off** (the default): type "dark mode on" in
   Spotlight; no "Lulo can do this" row may appear, and the service must
   never start.
2. Turned **on**: the same query shows "Turn On Dark Mode — Lulo can do
   this"; the first Return only arms it ("Press Return again to confirm")
   and nothing changes; the second Return switches the appearance to Dark
   (the rmac theme store in the private config). Then the service exits by
   itself once idle: no process is left.

**`--real-model`.** Runs the *installed* binaries (normally `--bin-dir
/usr/libexec/rmac`) with `RMAC_INTELLIGENCE_ENGINE` unset, so the real
service loads the real `llama.cpp` engine and the owner's downloaded,
checksum-verified model. The nested session's `XDG_DATA_HOME` holds a
directory-level symlink to the owner's own
`$XDG_DATA_HOME/lulo/intelligence/models` (never a per-file symlink: the
service's `verify::verified_model` reads the `<sha256>.gguf` and
`<sha256>.verified` paths with `symlink_metadata`/`O_NOFOLLOW`, which
rejects a *leaf* symlink outright, so only the parent directory may be one;
every intermediate path component, including that one, is still resolved
by the kernel, so the final `<sha256>.gguf`/`.verified` files are seen as
the plain regular files they are in the owner's real directory). Nothing
under the owner's `$HOME` is ever written: the stamp the owner's own
fetcher wrote already matches the file's (size, mtime, inode) identity, so
`verified_model` only reads it. The owner's own `intelligence.json` and
model files are never touched; this run's `enabled: true` setting and its
prefix-cache state live entirely under the private `XDG_CONFIG_HOME`/
`XDG_CACHE_HOME` this scenario creates. It types a list of realistic
requests (correct phrasings, typos, unsupported requests, and plain
single-word searches that must never reach the model at all) into
Spotlight, records each row's text and latency from the last keystroke,
and confirms two actions end to end (dark mode, a Clock timer) in the
nested session only. CI never passes `--real-model`: there is no model
file there.

    python3 scripts/behavior/run_spotlight_intents.py --bin-dir DIR --niri NIRI
    python3 scripts/behavior/run_spotlight_intents.py --real-model --bin-dir /usr/libexec/rmac
"""

from __future__ import annotations

import argparse
import fcntl
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / "parallel"))
import run_lulo  # noqa: E402
import run_lulo_journey  # noqa: E402

QUERY = "dark mode on"
ROW = "Turn On Dark Mode, Lulo can do this"
ARMED = "Turn On Dark Mode, Press Return again to confirm"
SERVICE = "rmac-intelligence-service"
# Short, so the scenario can watch the service leave; the product uses 60 s.
IDLE_SECONDS = 4
# --real-model's own idle-exit wait: longer than the fixture's, since the
# real engine's worker thread has an actual model and context to drop.
REAL_IDLE_SECONDS = 10

# Spotlight row suffixes (crates/launcher-app/src/view/assist.rs SUBTITLE /
# CONFIRM_SUBTITLE), mirrored here rather than imported since this is Python
# driving the real accessibility tree, not Rust.
ASSIST_SUFFIX = ", Lulo can do this"
CONFIRM_SUFFIX = ", Press Return again to confirm"

# At least 20 realistic Spotlight requests (ADR 0024 §1 feature 1): the
# brief's own examples, a couple of typos, requests the closed intent list
# cannot do (must show no row), and plain single-word searches that must
# never reach the model at all (`rmac_intelligence::prompt::worth_asking`
# requires two or more words, a client-side gate that never calls the
# service). `confirm` marks the two requests this scenario carries out end
# to end; every other row is only observed, then dismissed with Escape, so
# nothing else actually changes anything.
REQUESTS: list[dict[str, object]] = [
    dict(text="turn on dark mode", want="Turn On Dark Mode"),
    dict(text="dark mode off pls", want="Turn On Light Mode"),
    dict(text="open notes", want="Open Notes"),
    dict(text="set a timer for 10 minutes", want="Start a 10-Minute Timer", confirm="timer"),
    dict(text="volume 30%", want="Set Volume to 30%"),
    dict(text="make it brighter", want="Turn Brightness Up"),
    dict(text="turn wifi off", want="Turn Wi-Fi Off"),
    dict(text="bluetooth on", want="Turn Bluetooth On"),
    dict(text="do not disturb on", want="Turn On Do Not Disturb"),
    dict(text="find my resume pdf", want_prefix="Search Files for"),
    dict(text="open the calculator app", want="Open Calculator"),
    dict(text="mute the sound", want="Mute Sound"),
    dict(text="trun on drak mode", want="Turn On Dark Mode", confirm="dark"),
    dict(text="opn notse", want="Open Notes"),
    dict(text="turn bluetooth off", want="Turn Bluetooth Off"),
    dict(text="increase the volume", want="Turn Volume Up"),
    dict(text="remind me to call mum at 5", want=None),
    dict(text="whats the weather", want=None),
    dict(text="write me an email", want=None),
    dict(text="Notes", skip=True),
    dict(text="calc", skip=True),
    # A phrasing the dev set shows the base model (pre-fine-tuning) gets
    # wrong (ADR 0024 "Phase 1 results", held-out misses): recorded, not
    # treated as a scenario bug, so this honestly reports the known gap
    # instead of asserting a result the ADR says not to expect yet.
    dict(text="turn of the wifi", want="Turn Wi-Fi Off", lenient=True),
]


def theme_scheme(config_home: Path) -> str | None:
    """The saved Light/Dark/Auto choice (crates/rmac-theme StoredPreferences)."""
    path = config_home / "rmac" / "theme.json"
    try:
        return json.loads(path.read_text()).get("preferences", {}).get("color_scheme")
    except (OSError, ValueError, AttributeError):
        return None


def clock_timers(config_home: Path) -> list[dict]:
    """The Clock store's timers (crates/clock/src/store.rs State), the
    nested session's own, never the owner's."""
    path = config_home / "rmac" / "clock.json"
    try:
        return json.loads(path.read_text()).get("timers", [])
    except (OSError, ValueError, AttributeError):
        return []


def start_light(config_home: Path) -> None:
    """A fresh session is Dark (the Mac reference runs Dark); start Light so
    "Turn On Dark Mode" has something to change."""
    path = config_home / "rmac" / "theme.json"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps({"version": 1, "preferences": {"color_scheme": "light"}}) + "\n")


def set_enabled(config_home: Path, enabled: bool) -> None:
    path = config_home / "rmac" / "intelligence.json"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps({"version": 1, "enabled": enabled}) + "\n")


def link_real_model(env: dict[str, str], owner_models_dir: str | None) -> Path:
    """Point this run's models directory at the owner's own, read-only, by a
    directory-level symlink, and return the directory that was linked.

    Only the directory is a symlink. `rmac_intelligence::verify::identity`
    stats the model file with `symlink_metadata` (a leaf symlink reads as
    "not a regular file" and is refused) and `read_stamp` opens the
    `.verified` stamp with `O_NOFOLLOW` for the same reason; both still
    resolve every *other* path component normally, so the owner's real
    `<sha256>.gguf`/`.verified` files, reached through this one symlinked
    directory, are seen exactly as the service sees them on the owner's own
    `XDG_DATA_HOME` -- this is the layout the real install uses, not a
    weaker stand-in for it.

    Nothing here is ever written to: the owner's own fetcher already wrote
    a `.verified` stamp that matches the file's (size, mtime, inode)
    identity (the owner verified and downloaded it before this scenario
    runs), so `verify::verified_model` only reads that stamp and never
    re-hashes or rewrites it.
    """

    owner_models = Path(owner_models_dir or "~/.local/share/lulo/intelligence/models").expanduser()
    if not owner_models.is_dir() or not any(owner_models.glob("*.verified")):
        raise SystemExit(
            f"--real-model needs an already-downloaded, verified model in {owner_models} "
            "(Settings ▸ Lulo Intelligence ▸ Download, done once by the owner)"
        )
    private_intelligence = Path(env["XDG_DATA_HOME"]) / "lulo" / "intelligence"
    private_intelligence.mkdir(parents=True, exist_ok=True)
    (private_intelligence / "models").symlink_to(owner_models, target_is_directory=True)
    return owner_models


def service_pids(executable: Path) -> list[int]:
    """Processes running exactly this private copy of the service."""
    pids = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            if Path(os.readlink(entry / "exe")) == executable:
                pids.append(int(entry.name))
        except OSError:
            continue
    return pids


def process_cpu_ticks(runtime_dir: Path) -> int:
    """Total utime+stime (clock ticks) of every process in this run's
    private session, by its unique `XDG_RUNTIME_DIR` -- the same way
    `run_lulo.reap` finds them. Comparing two samples a few seconds apart,
    once the session is otherwise idle, is this scenario's "zero CPU
    afterwards" check."""

    needle = f"XDG_RUNTIME_DIR={runtime_dir}".encode()
    total = 0
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            environ = (entry / "environ").read_bytes().split(b"\0")
            if needle not in environ:
                continue
            fields = (entry / "stat").read_text().rsplit(") ", 1)[1].split(" ")
            total += int(fields[11]) + int(fields[12])  # utime, stime
        except (OSError, IndexError, ValueError):
            continue
    return total


def trace_events(path: Path) -> list[tuple[int, str]]:
    """`(micros, event)` rows of a `RMAC_FRAME_TRACE` CSV, in time order;
    empty if the trace was never written."""

    if not path.exists():
        return []
    events = []
    for line in path.read_text().splitlines()[1:]:
        event, _, moment = line.rpartition(",")
        try:
            events.append((int(moment), event))
        except ValueError:
            continue
    events.sort(key=lambda pair: pair[0])
    return events


def input_to_present_latencies(path: Path, actions: list[float] | None = None) -> list[int]:
    """Microseconds from each keystroke (`input`) to the next presented
    frame (`present`), from a `RMAC_FRAME_TRACE` CSV: this scenario's
    "typing never stutters" measurement, matching the input-to-visible-
    response budget `docs/beta-checklist.md` already sets, while the model
    loads and answers in the background.

    A *gap* between `present` events is the wrong signal here: Spotlight
    legitimately presents nothing for seconds while idle, waiting on a
    cold model load with no keystroke to answer, and that silence is
    correct behaviour, not a stutter. An `input` event only exists because
    a key was actually pressed, so measuring input-to-present latency
    instead counts only what the user can feel, and never a quiet window.
    Several inputs typed before the next redraw (GPUI coalescing rapid
    keystrokes into one frame, which is normal) are each measured against
    that same next `present`, so an earlier key in the batch correctly
    shows a longer wait than the last one.

    A key that *closes* Spotlight (each request ends with Escape) draws no
    frame of its own: the window is gone. Its next `present` is the first
    frame of the next request's new window, opened half a second later by
    this script, so pairing the two measured the script's own pause, not
    a stutter (ADR 0024 "Phase 1.1": the 400-620 ms "hitch" once per
    request was exactly this). An input followed by `open_window` before
    any `present` is therefore not counted.

    Nor is a key that carries out a picked row (`actions`: time.monotonic()
    just before this script pressed it): its next frame waits for the
    action itself (switching the whole session to Dark), which is not
    typing; [`action_latencies`] reports those on their own.

    Empty (never a hard failure) if the trace was never written: some
    builds or renderers may not emit it, and typing is still the
    scenario's real test."""

    latencies = []
    pending: list[int] = []
    skip = action_inputs(path, actions or [])
    for moment, event in trace_events(path):
        if event == "input" and moment in skip:
            continue
        if event == "input":
            pending.append(moment)
        elif event == "open_window":
            pending = []
        elif event == "present" and pending:
            latencies.extend(moment - input_moment for input_moment in pending)
            pending = []
    return latencies


def action_inputs(path: Path, actions: list[float]) -> set[int]:
    """The trace times of the first `input` after each of `actions`."""

    events = trace_events(path)
    origin = next((moment for moment, event in events if event == "monotonic_origin"), None)
    if origin is None:
        return set()
    inputs = [moment for moment, event in events if event == "input"]
    found = set()
    for action in actions:
        start = action * 1e6 - origin
        later = [moment for moment in inputs if moment >= start]
        if later:
            found.add(later[0])
    return found


def action_latencies(path: Path, actions: list[float]) -> list[int]:
    """Microseconds from each action key (see `input_to_present_latencies`)
    to the next presented frame."""

    skip = action_inputs(path, actions)
    out = []
    pending = None
    for moment, event in trace_events(path):
        if event == "input" and moment in skip:
            pending = moment
        elif event == "present" and pending is not None:
            out.append(moment - pending)
            pending = None
        elif event == "open_window":
            pending = None
    return out


def assist_frames(path: Path) -> list[dict[str, float]]:
    """For every "Lulo can do this" row Spotlight inserted
    (`assist_row_applied`, crates/launcher-app/src/view/assist.rs), the
    frame that showed it: milliseconds from the row being applied to the
    view's `launcher_render`, from there to `draw_start` (layout, text
    shaping and paint on the UI thread) and to `present`."""

    events = trace_events(path)
    frames = []
    for index, (moment, event) in enumerate(events):
        if event != "assist_row_applied":
            continue
        render = draw = None
        for later, kind in events[index + 1:]:
            if kind == "launcher_render" and render is None:
                render = later
            elif kind == "draw_start" and render is not None and draw is None:
                draw = later
            elif kind == "present" and draw is not None:
                frames.append({
                    "to_render_ms": (render - moment) / 1000,
                    "render_ms": (draw - render) / 1000,
                    "present_ms": (later - draw) / 1000,
                    "total_ms": (later - moment) / 1000,
                })
                break
    return frames


def keystroke_to_row(path: Path, typed_at: list[float]) -> list[float | None]:
    """For each request (`typed_at`: `time.monotonic()` when this script
    finished typing it), milliseconds from the request's last keystroke
    reaching Spotlight (`input`) to the frame that showed its row
    (`present` after `assist_row_applied`), on the trace's own clock: no
    AT-SPI polling in it. `None` where no row was shown."""

    events = trace_events(path)
    origin = next((moment for moment, event in events if event == "monotonic_origin"), None)
    if origin is None:
        return [None for _ in typed_at]
    events = [(moment, event) for moment, event in events if event != "monotonic_origin"]
    results: list[float | None] = []
    for index, typed in enumerate(typed_at):
        start = typed * 1e6 - origin
        end = typed_at[index + 1] * 1e6 - origin if index + 1 < len(typed_at) else float("inf")
        last_input = None
        latency = None
        applied = False
        for moment, event in events:
            if moment > end:
                break
            if event == "input" and not applied and moment <= start + 50_000:
                last_input = moment
            elif event == "assist_row_applied" and moment >= start and last_input is not None:
                applied = True
            elif event == "present" and applied:
                latency = (moment - last_input) / 1000
                break
        results.append(latency)
    return results


def service_timings(path: Path) -> list[dict[str, float]]:
    """What the service measured for each answer Spotlight received, from
    the launcher's `assist_timing:` trace rows (assist.rs)."""

    timings = []
    for _moment, event in trace_events(path):
        if not event.startswith("assist_timing:"):
            continue
        fields = {}
        for pair in event.split(":")[1:]:
            key, _, value = pair.partition("=")
            try:
                fields[key] = float(value)
            except ValueError:
                continue
        timings.append(fields)
    return timings


class TraceWatcher:
    """Follows Spotlight's own `RMAC_FRAME_TRACE` file, so a request's
    answer is noticed from the launcher's `assist_reply` mark (a few file
    reads) instead of by walking the whole accessibility tree five times a
    second. That polling ran on the same two cores as the model and roughly
    doubled the service's measured time (ADR 0024 "Phase 1.1"); a user's
    session has no such poller. The row's text is still read from the
    accessibility tree, once, after the answer arrived."""

    def __init__(self, path: Path):
        self.path = path
        self.offset = 0
        self.buffer = ""
        self.origin: int | None = None
        self.events: list[tuple[float, str]] = []

    def _read(self) -> None:
        try:
            with self.path.open() as handle:
                handle.seek(self.offset)
                chunk = handle.read()
                self.offset = handle.tell()
        except OSError:
            return
        self.buffer += chunk
        *lines, self.buffer = self.buffer.split("\n")
        for line in lines:
            event, _, moment = line.rpartition(",")
            try:
                micros = int(moment)
            except ValueError:
                continue
            if event == "monotonic_origin":
                self.origin = micros
            elif self.origin is not None:
                self.events.append(((self.origin + micros) / 1e6, event))

    def wait_for(self, prefix: str, after: float, timeout: float) -> bool:
        """Whether an event starting with `prefix` happened after `after`
        (time.monotonic()), waiting up to `timeout` seconds."""

        deadline = time.monotonic() + timeout
        while True:
            self._read()
            if any(moment >= after and event.startswith(prefix) for moment, event in self.events):
                return True
            if time.monotonic() >= deadline:
                return False
            time.sleep(0.05)


def wait_until(predicate, timeout: float, interval: float = 0.2):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(interval)
    return predicate()


class BaseScenario:
    def __init__(self, args, work: Path):
        self.args = args
        data = {"title": "Spotlight intents", "steps": []}
        self.driver = run_lulo_journey.Driver(args, work, data, work / "out")
        self.config_home = Path(self.driver.session.env["XDG_CONFIG_HOME"])
        # `.resolve()` so a symlinked service binary (--real-model) compares
        # equal to the canonical path `/proc/<pid>/exe` always reports; a
        # copied-in-place binary (the fixture mode below) is already
        # canonical, so this is a no-op there.
        self.service = (Path(args.bin_dir) / SERVICE).resolve()
        self.facts: dict[str, object] = {}

    def showing(self) -> set[str]:
        import pyatspi

        names = set()
        for node, _pid in self.driver.accessible():
            try:
                if node.name and node.getState().contains(pyatspi.STATE_SHOWING):
                    names.add(node.name)
            except Exception:  # noqa: BLE001
                continue
        return names

    def assist_rows(self) -> list[str]:
        return sorted(name for name in self.showing() if name.endswith(ASSIST_SUFFIX))

    def open_and_type(self, text: str = QUERY) -> None:
        opened = time.monotonic()
        self.driver.action({"key": "cmd-space"})
        if not wait_until(lambda: "Spotlight Search" in self.showing(), 10):
            raise RuntimeError("Spotlight did not open")
        watcher = getattr(self, "watcher", None)
        if watcher is not None:
            # Keys sent before the compositor gives Spotlight keyboard focus
            # are lost ("open notes" arrived as "en notes"): wait for the
            # launcher's own focus_in on its trace first.
            watcher.wait_for("focus_window:org.rmac.Launcher", opened, 5)
            time.sleep(0.05)
        self.driver.session.pointer.type_text(text)

    def close(self) -> None:
        self.driver.session.pointer.key("escape")
        time.sleep(0.3)
        if "Spotlight Search" in self.showing():
            self.driver.session.pointer.key("escape")
        wait_until(lambda: "Spotlight Search" not in self.showing(), 5)

    def finish(self) -> None:
        session = self.driver.session
        if hasattr(session, "pointer"):
            session.finish()
        else:
            for process in reversed(session.children):
                if process.poll() is None:
                    process.terminate()


class Scenario(BaseScenario):
    """The fixture pass CI runs: off shows nothing, on arms then confirms."""

    def run(self) -> dict:
        errors: list[str] = []
        start_light(self.config_home)
        self.driver.start()

        # 1. Off: no row, no service.
        set_enabled(self.config_home, False)
        self.open_and_type()
        # The launcher would ask 250 ms after typing stops; give it far more.
        time.sleep(3)
        rows = sorted(name for name in self.showing() if name.startswith("Turn On Dark Mode"))
        self.facts["off_rows"] = rows
        self.facts["off_service_started"] = bool(service_pids(self.service))
        if rows:
            errors.append(f"a Lulo Intelligence row appeared while it was off: {rows}")
        if self.facts["off_service_started"]:
            errors.append("the service started while Lulo Intelligence was off")
        self.close()

        # 2. On: the row, the confirmation, the change.
        set_enabled(self.config_home, True)
        before = theme_scheme(self.config_home)
        self.facts["scheme_before"] = before
        self.open_and_type()
        shown = wait_until(lambda: ROW in self.showing(), 15)
        self.facts["row_shown"] = bool(shown)
        if not shown:
            errors.append(f"no {ROW!r} row; showing: {sorted(self.showing())[:40]}")
        else:
            self.driver.session.pointer.key("return")
            armed = wait_until(lambda: ARMED in self.showing(), 5)
            self.facts["armed"] = bool(armed)
            time.sleep(0.5)
            self.facts["scheme_after_first_return"] = theme_scheme(self.config_home)
            if not armed:
                errors.append("the first Return did not ask for confirmation")
            if before != "light":
                errors.append(f"the session did not start Light ({before!r})")
            if theme_scheme(self.config_home) != before:
                errors.append("the appearance changed before confirmation")
            self.driver.session.pointer.key("return")
            dark = wait_until(lambda: theme_scheme(self.config_home) == "dark", 10)
            self.facts["scheme_after_confirm"] = theme_scheme(self.config_home)
            if not dark:
                errors.append("confirming did not switch the appearance to Dark")
            self.facts["spotlight_closed"] = bool(
                wait_until(lambda: "Spotlight Search" not in self.showing(), 5)
            )

        # 3. Idle: the service leaves by itself.
        self.facts["service_ran"] = bool(self.facts.get("row_shown"))
        gone = wait_until(lambda: not service_pids(self.service), IDLE_SECONDS + 10, 0.5)
        self.facts["service_exited_when_idle"] = bool(gone)
        if not gone:
            errors.append("the service was still running after its idle timeout")
        return {"facts": self.facts, "errors": errors}


class RealModelScenario(BaseScenario):
    """`--real-model`: the real engine, the owner's own downloaded model,
    typed one request at a time, as a user would."""

    def _enable_frame_trace(self, path: Path) -> None:
        """Only `rmac-launcher` (Spotlight) traces its frames: sharing one
        `RMAC_FRAME_TRACE` path across every resident process would have
        each one's `File::create` truncate what an earlier one wrote
        (shell/compat/gpui_linux .../frame_trace.rs opens for *create*, not
        append), so this wraps just that one spawn call instead of setting
        the variable for the whole private session."""

        original_spawn = self.driver.session.spawn

        def spawn_with_trace(argv, name, extra=None):
            if name == "rmac-launcher":
                extra = {**(extra or {}), "RMAC_FRAME_TRACE": str(path)}
            return original_spawn(argv, name, extra)

        self.driver.session.spawn = spawn_with_trace

    def _confirm_dark(self, record: dict) -> None:
        self.driver.session.pointer.key("return")
        armed = wait_until(lambda: any(row.endswith(CONFIRM_SUFFIX) for row in self.showing()), 5)
        if not armed:
            record["bug"] = "the first Return did not ask for confirmation"
            return
        time.sleep(0.3)
        before = theme_scheme(self.config_home)
        self.actions.append(time.monotonic())
        self.driver.session.pointer.key("return")
        dark = wait_until(lambda: theme_scheme(self.config_home) == "dark", 10)
        record["end_to_end"] = {"scheme_before": before, "scheme_after": theme_scheme(self.config_home)}
        if not dark:
            record["bug"] = "confirming did not switch the nested appearance to Dark"

    def _confirm_timer(self, record: dict) -> None:
        before = len(clock_timers(self.config_home))
        self.actions.append(time.monotonic())
        self.driver.session.pointer.key("return")
        created = wait_until(
            lambda: any(timer.get("duration") == 600_000 for timer in clock_timers(self.config_home)),
            10,
        )
        timers = clock_timers(self.config_home)
        record["end_to_end"] = {"timers_before": before, "timers_after": len(timers)}
        if not created:
            record["bug"] = f"no 10-minute timer in the nested Clock store: {timers}"

    watcher: "TraceWatcher | None" = None
    actions: list[float] = []

    def _warm_through_settings(self) -> None:
        """The product's own warm-up (ADR 0024 "Phase 1.1"): open System
        Settings ▸ Lulo Intelligence with the feature on and the model on
        disk, as a user does after the download; it says "Getting ready…"
        and has the service evaluate and save the prompt-prefix state. Then
        close Settings and let the service exit, so the first Spotlight
        request starts a fresh service that reads the state from disk."""

        cache = Path(self.driver.session.env["XDG_CACHE_HOME"]) / "lulo" / "intelligence"
        settings = Path(self.args.bin_dir) / "rmac-system-settings"
        if not settings.exists():
            settings = Path("/usr/bin/rmac-system-settings")
        # The pane's status text, as it renders it (render.rs marks each
        # frame's "Getting ready…" or "Downloaded"), on Settings' own trace.
        settings_trace = Path(self.args.inner) / "settings-frame-trace.csv"
        watcher = TraceWatcher(settings_trace)
        started = time.monotonic()
        process = self.driver.session.spawn([str(settings), "--pane", "intelligence"],
                                            "rmac-system-settings",
                                            {"RMAC_FRAME_TRACE": str(settings_trace)})
        saw = watcher.wait_for("intelligence_status:getting_ready", started, 30)
        states = wait_until(lambda: sorted(cache.glob("prefix-*.state")), 120, 0.5)
        warm_seconds = time.monotonic() - started
        record: dict[str, object] = {
            "showed_getting_ready": bool(saw),
            "state_saved": bool(states),
            "seconds": round(warm_seconds, 1),
        }
        if states:
            record["state_file_mode"] = oct(states[0].stat().st_mode & 0o777)
            record["cache_dir_mode"] = oct(cache.stat().st_mode & 0o777)
            record["state_mib"] = round(states[0].stat().st_size / 2**20, 1)
            # "Getting ready…" goes away once the state is saved.
            record["getting_ready_cleared"] = watcher.wait_for(
                "intelligence_status:downloaded", time.monotonic() - 1, 60)
            record["state_owner_only"] = record["state_file_mode"] == "0o600"
        process.terminate()
        try:
            process.wait(10)
        except subprocess.TimeoutExpired:
            process.kill()
        idle_timeout = getattr(self.args, "idle_seconds", None) or REAL_IDLE_SECONDS
        record["service_exited_after"] = bool(
            wait_until(lambda: not service_pids(self.service), idle_timeout + 15, 0.5))
        self.facts["warm_up"] = record

    def probe(self, spec: dict, cold: bool) -> dict:
        text = str(spec["text"])
        record: dict[str, object] = {"text": text, "cold": cold}
        try:
            if spec.get("skip"):
                # A single word: `prompt::worth_asking` never lets this reach
                # the model at all. Plain search already answers it.
                self.open_and_type(text)
                time.sleep(1.5)
                rows = self.assist_rows()
                record.update(row=(rows[0] if rows else None), latency_ms=None, correct=not rows)
                if rows:
                    record["bug"] = f"a Lulo Intelligence row appeared for a plain single-word search: {rows}"
                return record

            # Whether a row is wanted at all: a request with neither a
            # "want" (an exact title) nor a "want_prefix" (file search's
            # query is free text, so only its prefix is pinned) expects no
            # row, which is also true of the three explicit `want=None`
            # "unsupported" requests below.
            want = spec.get("want")
            want_prefix = spec.get("want_prefix")
            expect_row = want is not None or want_prefix is not None
            timeout = 40.0 if cold else 8.0
            self.open_and_type(text)
            t_key = time.monotonic()
            record["typed_at"] = t_key
            watcher = self.watcher
            if not expect_row:
                # Unsupported: wait for the answer (or the time a real row
                # would need), then require that no row showed.
                if watcher is not None:
                    watcher.wait_for("assist_reply", t_key, min(timeout, 6))
                    time.sleep(0.3)
                else:
                    wait_until(lambda: bool(self.assist_rows()), min(timeout, 6))
                rows = self.assist_rows()
                record.update(row=(rows[0] if rows else None), latency_ms=None, correct=not rows)
                if rows:
                    record["bug"] = f"a row appeared for an unsupported request: {rows}"
                return record

            if watcher is not None:
                applied = watcher.wait_for("assist_row_applied", t_key, timeout)
                latency_ms = (time.monotonic() - t_key) * 1000 if applied else None
                # The row reaches the accessibility tree with its frame.
                shown = applied and wait_until(lambda: bool(self.assist_rows()), 3, 0.1)
                if not shown:
                    latency_ms = None
            else:
                shown = wait_until(lambda: bool(self.assist_rows()), timeout)
                latency_ms = (time.monotonic() - t_key) * 1000 if shown else None
            rows = self.assist_rows()
            row_text = rows[0] if rows else None
            title = row_text[: -len(ASSIST_SUFFIX)] if row_text else None
            if title is None:
                correct = False
            elif want is not None:
                correct = title == want
            else:
                correct = title.startswith(str(want_prefix))
            record.update(row=row_text, latency_ms=latency_ms, correct=correct)
            lenient = bool(spec.get("lenient"))
            if title is None:
                if not lenient:
                    record["bug"] = (
                        f"expected a Lulo Intelligence row (wanted {want or want_prefix!r}); none appeared"
                    )
            elif not correct and not lenient:
                record["bug"] = f"wrong row {title!r}, wanted {want or want_prefix!r}"

            confirm = spec.get("confirm")
            if title is not None and confirm == "dark":
                self._confirm_dark(record)
            elif title is not None and confirm == "timer":
                self._confirm_timer(record)
            return record
        finally:
            self.close()

    def run(self) -> dict:
        errors: list[str] = []
        results: list[dict] = []
        start_light(self.config_home)
        set_enabled(self.config_home, True)
        trace_path = Path(self.args.inner) / "launcher-frame-trace.csv"
        self._enable_frame_trace(trace_path)
        self.watcher = TraceWatcher(trace_path)
        self.actions = []
        self.driver.start()
        if self.args.warm_first:
            self._warm_through_settings()
            warm = self.facts["warm_up"]
            if not (warm.get("state_saved") and warm.get("showed_getting_ready")
                    and warm.get("getting_ready_cleared") and warm.get("state_owner_only")):
                errors.append(f"System Settings did not warm the model up: {warm}")

        for index, spec in enumerate(REQUESTS):
            try:
                record = self.probe(spec, cold=(index == 0))
            except Exception as error:  # noqa: BLE001 - one bad request must not lose the rest
                record = {"text": spec.get("text"), "bug": f"{type(error).__name__}: {error}"}
                try:
                    self.close()
                except Exception:  # noqa: BLE001
                    pass
            results.append(record)
            if record.get("bug"):
                errors.append(f"{record.get('text')!r}: {record['bug']}")

        self.facts["results"] = results
        self.facts["cold_latency_ms"] = results[0].get("latency_ms") if results else None
        warm = [row["latency_ms"] for row in results[1:] if isinstance(row.get("latency_ms"), (int, float))]
        self.facts["warm_latency_ms"] = {
            "min": min(warm) if warm else None,
            "p50": sorted(warm)[len(warm) // 2] if warm else None,
            "max": max(warm) if warm else None,
            "count": len(warm),
        }

        latencies = input_to_present_latencies(trace_path, self.actions)
        self.facts["frame_trace_present"] = trace_path.exists()
        self.facts["action_key_to_present_ms"] = [
            round(latency / 1000) for latency in action_latencies(trace_path, self.actions)]
        typed = [row for row in results if isinstance(row.get("typed_at"), float)]
        for row, exact in zip(typed, keystroke_to_row(trace_path, [row["typed_at"] for row in typed])):
            row["keystroke_to_row_ms"] = None if exact is None else round(exact)
        exact_warm = sorted(row["keystroke_to_row_ms"] for row in typed[1:]
                            if isinstance(row.get("keystroke_to_row_ms"), (int, float)))
        self.facts["warm_keystroke_to_row_ms"] = {
            "min": exact_warm[0] if exact_warm else None,
            "p50": exact_warm[len(exact_warm) // 2] if exact_warm else None,
            "max": exact_warm[-1] if exact_warm else None,
            "count": len(exact_warm),
        }
        self.facts["cold_keystroke_to_row_ms"] = typed[0].get("keystroke_to_row_ms") if typed else None
        timings = service_timings(trace_path)
        self.facts["service_timing_ms"] = {
            key: (lambda values: {
                "p50": values[len(values) // 2] if values else None,
                "max": values[-1] if values else None,
            })(sorted(timing.get(key, 0.0) for timing in timings[1:]))
            for key in ("total_ms", "queued_ms", "rewind_ms", "prefill_ms", "decode_ms",
                        "request_tokens", "passes")
        }
        self.facts["service_timing_ms"]["warm_count"] = max(len(timings) - 1, 0)
        self.facts["service_timing_ms"]["first"] = timings[0] if timings else None
        frames = assist_frames(trace_path)

        def summary(values: list[float]) -> dict[str, float | None]:
            ordered = sorted(values)
            return {
                "count": len(ordered),
                "p50": ordered[len(ordered) // 2] if ordered else None,
                "max": ordered[-1] if ordered else None,
            }

        self.facts["assist_frame_ms"] = {
            key: summary([frame[key] for frame in frames])
            for key in ("to_render_ms", "render_ms", "present_ms", "total_ms")
        }
        # The frame that inserts the row is an ordinary frame: its UI-thread
        # work (layout, shaping, paint) stays inside one 60 Hz frame.
        slow_rows = [frame for frame in frames if frame["render_ms"] > 16.0]
        if slow_rows:
            errors.append(
                f"{len(slow_rows)} of {len(frames)} row insertions took over 16 ms of UI-thread "
                f"work (max {max(frame['render_ms'] for frame in slow_rows):.1f} ms)"
            )
        self.facts["input_to_present_us"] = {
            "count": len(latencies),
            "p50": sorted(latencies)[len(latencies) // 2] if latencies else None,
            "p95": sorted(latencies)[int(len(latencies) * 0.95)] if latencies else None,
            "max": max(latencies) if latencies else None,
        }
        # 100 ms, not the Beta budget's 50 ms: this path also carries
        # AT-SPI/dbus-run-session overhead the real session does not have,
        # so a looser bound still catches a genuine stutter without flagging
        # test-harness noise as one.
        stutter = [latency for latency in latencies if latency > 100_000]
        if stutter:
            errors.append(
                f"typing stuttered while Lulo Intelligence ran: {len(stutter)} of {len(latencies)} "
                f"keystrokes took over 100 ms to draw (max {max(stutter)} µs)"
            )

        self.close()
        runtime = Path(self.driver.session.env["XDG_RUNTIME_DIR"])
        idle_timeout = getattr(self.args, "idle_seconds", None) or REAL_IDLE_SECONDS
        gone = wait_until(lambda: not service_pids(self.service), idle_timeout + 15, 0.5)
        self.facts["service_exited_when_idle"] = bool(gone)
        if not gone:
            errors.append("the service was still running after its idle timeout")

        before = process_cpu_ticks(runtime)
        time.sleep(2)
        after = process_cpu_ticks(runtime)
        # CLK_TCK is 100 on every Lulo target; a handful of ticks over 2 s is
        # the idle compositor's own redraw, not the intelligence service.
        self.facts["cpu_ticks_after_idle_2s"] = after - before

        return {"facts": self.facts, "errors": errors}


def inner(args) -> int:
    work = Path(args.inner)
    scenario_cls = RealModelScenario if args.real_model else Scenario
    scenario = scenario_cls(args, work)
    try:
        result = scenario.run()
    except Exception as error:  # noqa: BLE001 - a crash must fail, never pass
        result = {"facts": scenario.facts, "errors": [f"{type(error).__name__}: {error}"]}
    finally:
        scenario.finish()
    status = "pass" if not result["errors"] else "fail"
    print(json.dumps({"scenario": "spotlight-intents", "status": status, **result}, indent=2),
          flush=True)
    return 0 if status == "pass" else 1


def outer(args) -> int:
    lock = open("/tmp/lulo-journey.lock", "w")
    fcntl.flock(lock, fcntl.LOCK_EX)
    work = Path(tempfile.mkdtemp(prefix="lulo-intents-"))
    try:
        env = run_lulo.isolated_environment(work)
        run_lulo.refuse_live_session(env)
        bins = Path(args.bin_dir).expanduser().resolve()
        links = work / "bins"
        links.mkdir()
        sources = {source.name: source for source in bins.iterdir()}
        if args.override_bin_dir:
            # Prefer freshly built programs; the shell falls back to --bin-dir.
            overrides = Path(args.override_bin_dir).expanduser().resolve()
            sources.update({source.name: source for source in overrides.glob("rmac-*")
                            if source.is_file() and os.access(source, os.X_OK)})
        # Fixture mode copies the service and Spotlight so their own
        # `/proc/<pid>/exe` is this run's private directory, which the
        # caller check's "a Lulo program beside me" rule accepts no matter
        # where --bin-dir came from (CI's --bin-dir is a build output
        # directory, never /usr/libexec/rmac). --real-model instead
        # symlinks them like everything else, so `/proc/<pid>/exe` resolves
        # to the real installed path and the caller check's hard-coded
        # `/usr/libexec/rmac` rule is the one actually exercised -- the
        # "test it the real way" this mode exists for.
        copy_in_place = set() if args.real_model else {SERVICE, "rmac-launcher"}
        for source in sources.values():
            if source.is_file() and source.name not in {"dock", "mission-control"}:
                if source.name in copy_in_place:
                    target = links / source.name
                    target.write_bytes(source.read_bytes())
                    target.chmod(0o755)
                else:
                    (links / source.name).symlink_to(source)
        for alias, source in (("dock", "rmac-dock"), ("mission-control", "rmac-mission-control")):
            (links / alias).symlink_to(links / source)
        if not (links / SERVICE).is_file():
            raise SystemExit(f"{SERVICE} is not in {bins}")
        run_lulo.install_shortcut_dispatcher(env, links)
        config = run_lulo_journey.private_bus(work, env, links)
        services = work / "dbus-services"
        (services / "org.rmac.Intelligence1.service").write_text(
            "[D-BUS Service]\nName=org.rmac.Intelligence1\n" f"Exec={links / SERVICE}\n")
        if args.real_model:
            # The real engine: no stub, the owner's own verified model.
            env.pop("RMAC_INTELLIGENCE_ENGINE", None)
            linked = link_real_model(env, args.owner_models_dir)
            print(f"--real-model: {linked} linked read-only into this private session", flush=True)
        else:
            env["RMAC_INTELLIGENCE_ENGINE"] = "fixture"
        idle_seconds = args.idle_seconds or (REAL_IDLE_SECONDS if args.real_model else IDLE_SECONDS)
        env["RMAC_INTELLIGENCE_IDLE_SECONDS"] = str(idle_seconds)
        command = ["dbus-run-session", f"--config-file={config}", "--", sys.executable,
                   str(Path(__file__).resolve()), "--inner", str(work), "--bin-dir", str(links),
                   "--niri", args.niri, "--idle-seconds", str(idle_seconds)]
        if args.real_model:
            command.append("--real-model")
        if args.warm_first:
            command.append("--warm-first")
        return subprocess.call(command, env=env, close_fds=True)
    finally:
        if run_lulo.reap(work / "runtime"):
            time.sleep(1)
            run_lulo.reap(work / "runtime")
        if args.keep:
            print(f"kept {work}", flush=True)
        else:
            run_lulo.remove_tree(work)
        lock.close()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--bin-dir", required=True)
    parser.add_argument("--niri", default="/usr/bin/niri")
    parser.add_argument("--override-bin-dir",
                        help="prefer the rmac-* programs here; the rest come from --bin-dir")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--real-model", action="store_true",
                        help="the real llama.cpp engine and the owner's own downloaded model, "
                             "instead of the fixture engine. Never used in CI: there is no model "
                             "file there. --bin-dir should be /usr/libexec/rmac (the installed "
                             "binaries), so the caller check's real trusted-directory rule runs.")
    parser.add_argument("--owner-models-dir",
                        help="the verified model directory to link read-only (--real-model only); "
                             "default $XDG_DATA_HOME/lulo/intelligence/models, or "
                             "~/.local/share/lulo/intelligence/models")
    parser.add_argument("--warm-first", action="store_true",
                        help="--real-model only: before the first request, open System Settings "
                             "> Lulo Intelligence so it warms the model up (\"Getting ready…\") "
                             "and wait for the service to exit, as after a download")
    parser.add_argument("--idle-seconds", type=int, default=None,
                        help="override the service's idle-exit timeout for this run")
    parser.add_argument("--inner", help=argparse.SUPPRESS)
    # run_lulo_journey.Driver reads these.
    parser.set_defaults(frame_only=False, full_too=False, dump_a11y=False)
    args = parser.parse_args()
    if args.inner:
        return inner(args)
    return outer(args)


if __name__ == "__main__":
    raise SystemExit(main())
