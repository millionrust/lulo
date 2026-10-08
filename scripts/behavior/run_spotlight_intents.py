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


def input_to_present_latencies(path: Path) -> list[int]:
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

    Empty (never a hard failure) if the trace was never written: some
    builds or renderers may not emit it, and typing is still the
    scenario's real test."""

    if not path.exists():
        return []
    events = []
    for line in path.read_text().splitlines()[1:]:
        parts = line.split(",")
        if len(parts) != 2 or parts[0] not in ("input", "present"):
            continue
        try:
            events.append((int(parts[1]), parts[0]))
        except ValueError:
            continue
    events.sort(key=lambda pair: pair[0])
    latencies = []
    pending: list[int] = []
    for moment, event in events:
        if event == "input":
            pending.append(moment)
        elif pending:
            latencies.extend(moment - input_moment for input_moment in pending)
            pending = []
    return latencies


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
        self.driver.action({"key": "cmd-space"})
        if not wait_until(lambda: "Spotlight Search" in self.showing(), 10):
            raise RuntimeError("Spotlight did not open")
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
        self.driver.session.pointer.key("return")
        dark = wait_until(lambda: theme_scheme(self.config_home) == "dark", 10)
        record["end_to_end"] = {"scheme_before": before, "scheme_after": theme_scheme(self.config_home)}
        if not dark:
            record["bug"] = "confirming did not switch the nested appearance to Dark"

    def _confirm_timer(self, record: dict) -> None:
        before = len(clock_timers(self.config_home))
        self.driver.session.pointer.key("return")
        created = wait_until(
            lambda: any(timer.get("duration") == 600_000 for timer in clock_timers(self.config_home)),
            10,
        )
        timers = clock_timers(self.config_home)
        record["end_to_end"] = {"timers_before": before, "timers_after": len(timers)}
        if not created:
            record["bug"] = f"no 10-minute timer in the nested Clock store: {timers}"

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
            if not expect_row:
                # Unsupported: give it the same wait a real row would need,
                # then require that none ever showed.
                wait_until(lambda: bool(self.assist_rows()), min(timeout, 6))
                rows = self.assist_rows()
                record.update(row=(rows[0] if rows else None), latency_ms=None, correct=not rows)
                if rows:
                    record["bug"] = f"a row appeared for an unsupported request: {rows}"
                return record

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
        self.driver.start()

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

        latencies = input_to_present_latencies(trace_path)
        self.facts["frame_trace_present"] = trace_path.exists()
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
