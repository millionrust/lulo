#!/usr/bin/env python3
"""The World Clock's sweeping second hand (CLOCK-10) draws only while Clock is
the active window, in a private nested session.

The frame trace (`RMAC_FRAME_TRACE`) counts what Clock draws in three
seconds: about 30 frames a second while it is active, almost none once
another window has the keyboard, and the sweep resumes when Clock is active
again. (The nested runners hold the hand still by default,
`RMAC_CLOCK_SECOND_HAND=still`; this one turns it on.)

    python3 scripts/behavior/run_clock_sweep.py --bin-dir DIR --niri NIRI
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

CLOCK = "org.rmac.Clock"
CALC = "org.rmac.Calculator"
WINDOW_SECONDS = 3.0


def wait_until(predicate, timeout: float, interval: float = 0.1):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            value = predicate()
        except Exception:  # noqa: BLE001
            value = None
        if value:
            return value
        time.sleep(interval)
    return None


class Scenario:
    def __init__(self, args, work: Path):
        self.args = args
        self.work = work
        self.trace = work / "clock-trace.csv"
        self.driver = run_lulo_journey.Driver(
            args, work, {"title": "Clock sweep", "steps": []}, work / "out")
        self.facts: dict[str, object] = {}
        self.errors: list[str] = []

    def draws(self) -> int:
        if not self.trace.exists():
            return 0
        return sum(1 for line in self.trace.read_text().splitlines()
                   if line.startswith("draw_start"))

    def windows(self) -> dict[str, dict]:
        result = subprocess.run(
            [self.args.niri, "msg", "--json", "windows"], env=self.driver.session.env,
            capture_output=True, text=True, timeout=10, check=False)
        try:
            return {w.get("app_id"): w for w in json.loads(result.stdout)}
        except ValueError:
            return {}

    def focus(self, app_id: str) -> None:
        window = self.windows().get(app_id)
        if window:
            subprocess.run(
                [self.args.niri, "msg", "action", "focus-window", "--id", str(window["id"])],
                env=self.driver.session.env, capture_output=True, timeout=10, check=False)
        time.sleep(1.0)

    def count(self) -> int:
        before = self.draws()
        time.sleep(WINDOW_SECONDS)
        return self.draws() - before

    def run(self) -> dict:
        self.driver.start()
        bins = Path(self.args.bin_dir)
        self.driver.session.spawn(
            [str(bins / "rmac-clock")], "clock",
            {"RMAC_FRAME_TRACE": str(self.trace), "RMAC_CLOCK_SECOND_HAND": "sweep"})
        if not wait_until(lambda: CLOCK in self.windows(), 30):
            self.errors.append("Clock did not open a window")
            return {"facts": self.facts, "errors": self.errors}
        self.focus(CLOCK)
        time.sleep(1.5)
        active = self.count()
        self.facts["active_frames"] = active
        if active < 45:
            self.errors.append(f"an active World Clock drew {active} frames in {WINDOW_SECONDS:.0f} s; "
                               "the second hand should sweep at about 30 a second")
        if active > 150:
            self.errors.append(f"an active World Clock drew {active} frames: the sweep is not capped")

        self.driver.session.spawn([str(bins / "rmac-calculator")], "calculator")
        if not wait_until(lambda: CALC in self.windows(), 30):
            self.errors.append("Calculator did not open a window")
            return {"facts": self.facts, "errors": self.errors}
        self.focus(CALC)
        time.sleep(1.0)
        inactive = self.count()
        self.facts["inactive_frames"] = inactive
        if inactive > 3:
            self.errors.append(f"Clock drew {inactive} frames in {WINDOW_SECONDS:.0f} s "
                               "while another window was active")

        self.focus(CLOCK)
        time.sleep(1.0)
        again = self.count()
        self.facts["resumed_frames"] = again
        if again < 45:
            self.errors.append(f"the sweep did not resume when Clock was active again ({again} frames)")
        return {"facts": self.facts, "errors": self.errors}

    def finish(self) -> None:
        try:
            self.driver.session.finish()
        except Exception:  # noqa: BLE001
            pass


def inner(args) -> int:
    work = Path(args.inner)
    scenario = Scenario(args, work)
    try:
        result = scenario.run()
    except Exception as error:  # noqa: BLE001
        result = {"facts": scenario.facts,
                  "errors": scenario.errors + [f"{type(error).__name__}: {error}"]}
    finally:
        scenario.finish()
    status = "pass" if not result["errors"] else "fail"
    print(json.dumps({"scenario": "clock-sweep", "status": status, **result}, indent=2,
                     default=str), flush=True)
    return 0 if status == "pass" else 1


def outer(args) -> int:
    lock = open("/tmp/lulo-journey.lock", "w")
    fcntl.flock(lock, fcntl.LOCK_EX)
    work = Path(tempfile.mkdtemp(prefix="lulo-sweep-"))
    try:
        env = run_lulo.isolated_environment(work)
        run_lulo.refuse_live_session(env)
        bins = Path(args.bin_dir).expanduser().resolve()
        links = work / "bins"
        links.mkdir()
        sources = {source.name: source for source in bins.iterdir()}
        if args.override_bin_dir:
            for source in Path(args.override_bin_dir).expanduser().resolve().iterdir():
                sources[source.name] = source
        for source in sources.values():
            if source.is_file() and source.name not in {"dock", "mission-control"}:
                (links / source.name).symlink_to(source)
        for alias, source in (("dock", "rmac-dock"), ("mission-control", "rmac-mission-control")):
            if (links / source).exists():
                (links / alias).symlink_to(links / source)
        for required in ("rmac-calculator", "rmac-clock"):
            if not (links / required).is_file():
                raise SystemExit(f"{required} is not in {bins}")
        run_lulo.install_shortcut_dispatcher(env, links)
        config = run_lulo_journey.private_bus(work, env, links)
        command = ["dbus-run-session", f"--config-file={config}", "--", sys.executable,
                   str(Path(__file__).resolve()), "--inner", str(work), "--bin-dir", str(links),
                   "--niri", args.niri]
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
    parser.add_argument("--override-bin-dir", help="take these binaries instead of --bin-dir's")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--inner", help=argparse.SUPPRESS)
    parser.set_defaults(frame_only=False, full_too=False, dump_a11y=False)
    args = parser.parse_args()
    if args.inner:
        return inner(args)
    return outer(args)


if __name__ == "__main__":
    raise SystemExit(main())
