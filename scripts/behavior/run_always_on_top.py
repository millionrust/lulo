#!/usr/bin/env python3
"""Window ▸ Always on Top (CALC-08), against the patched niri, in a private
nested session.

Needs the niri built with `0004-window-always-on-top.patch` (`--niri`).
Calculator is dragged over Clock; the overlap is read from niri's own
screenshot, so what is checked is what is drawn:

1. Without Always on Top, focusing Clock raises it above Calculator.
2. With it on, focusing Clock leaves Calculator on top.
3. `niri msg windows` reports it (`is_always_on_top`), and turning it off
   lets Clock rise again.

    python3 scripts/behavior/run_always_on_top.py --bin-dir DIR --niri NIRI
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

CALC = "org.rmac.Calculator"
CLOCK = "org.rmac.Clock"


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
        self.driver = run_lulo_journey.Driver(
            args, work, {"title": "Always on Top", "steps": []}, work / "out")
        self.facts: dict[str, object] = {}
        self.errors: list[str] = []

    def niri(self, *arguments: str) -> str:
        result = subprocess.run(
            [self.args.niri, "msg", *arguments], env=self.driver.session.env,
            capture_output=True, text=True, timeout=10, check=False)
        return result.stdout

    def windows(self) -> dict[str, dict]:
        try:
            listed = json.loads(self.niri("--json", "windows"))
        except ValueError:
            return {}
        return {window.get("app_id"): window for window in listed}

    def action(self, *arguments: str) -> None:
        self.niri("action", *arguments)
        time.sleep(0.4)

    def overlap_pixels(self, box) -> list[tuple[int, int, int]]:
        image = self.driver.screenshot()
        scale = image.width / max(1, self.driver.session.width)
        left, top, right, bottom = (round(value * scale) for value in box)
        crop = image.crop((left, top, right, bottom)).resize((8, 8))
        return list(crop.getdata())

    @staticmethod
    def distance(a, b) -> float:
        return sum(abs(x - y) for p, q in zip(a, b) for x, y in zip(p, q)) / (len(a) * 3)

    def run(self) -> dict:
        self.driver.start()
        self.driver.launch({"launch": "calculator"})
        self.driver.launch({"launch": "clock"}) if "clock" in run_lulo_journey.BINARIES else None
        if "clock" not in run_lulo_journey.BINARIES:
            clock = self.driver.session.spawn(
                [str(Path(self.args.bin_dir) / "rmac-clock")], "clock")
            self.driver.apps["clock"] = clock
        if not wait_until(lambda: CALC in self.windows() and CLOCK in self.windows(), 30):
            self.errors.append(f"windows did not open: {sorted(self.windows())}")
            return {"facts": self.facts, "errors": self.errors}
        time.sleep(1.0)
        windows = self.windows()
        calc, clock = windows[CALC]["id"], windows[CLOCK]["id"]
        # Clock at the top left, Calculator dragged over its middle.
        self.action("move-floating-window", "--id", str(clock), "-x", "40", "-y", "40")
        self.action("move-floating-window", "--id", str(calc), "-x", "260", "-y", "120")
        windows = self.windows()
        c = windows[CALC]["layout"]
        box = (c["tile_pos_in_workspace_view"][0] + 40, c["tile_pos_in_workspace_view"][1] + 60,
               c["tile_pos_in_workspace_view"][0] + 160, c["tile_pos_in_workspace_view"][1] + 160)
        self.facts["probe_box"] = [round(v) for v in box]

        self.action("focus-window", "--id", str(calc))
        calc_on_top = self.overlap_pixels(box)
        self.action("focus-window", "--id", str(clock))
        clock_on_top = self.overlap_pixels(box)
        stacks = self.distance(calc_on_top, clock_on_top)
        self.facts["stack_difference"] = round(stacks, 1)
        if stacks < 8:
            self.errors.append("the overlap does not change with focus: the probe is not on both windows")

        self.action("focus-window", "--id", str(calc))
        self.action("set-window-always-on-top", "--id", str(calc), "--on", "true")
        pinned = bool(wait_until(lambda: self.windows().get(CALC, {}).get("is_always_on_top"), 5))
        self.facts["reported_pinned"] = pinned
        if not pinned:
            self.errors.append("niri does not report is_always_on_top after turning it on")
        self.action("focus-window", "--id", str(clock))
        still_top = self.overlap_pixels(box)
        kept = self.distance(still_top, calc_on_top)
        self.facts["pinned_difference"] = round(kept, 1)
        if kept > 3:
            self.errors.append("Calculator did not stay above Clock after Clock took focus")

        self.action("set-window-always-on-top", "--id", str(calc), "--on", "false")
        released = not self.windows().get(CALC, {}).get("is_always_on_top")
        self.facts["reported_released"] = released
        if not released:
            self.errors.append("niri still reports Always on Top after turning it off")
        self.action("focus-window", "--id", str(calc))
        self.action("focus-window", "--id", str(clock))
        lowered = self.overlap_pixels(box)
        back = self.distance(lowered, clock_on_top)
        self.facts["released_difference"] = round(back, 1)
        if back > 3:
            self.errors.append("Clock did not rise above Calculator again once it was turned off")
        try:
            self.driver.capture(self.work / "out" / "end.png", full=True)
        except Exception:  # noqa: BLE001
            pass
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
    print(json.dumps({"scenario": "always-on-top", "status": status, **result}, indent=2,
                     default=str), flush=True)
    return 0 if status == "pass" else 1


def outer(args) -> int:
    lock = open("/tmp/lulo-journey.lock", "w")
    fcntl.flock(lock, fcntl.LOCK_EX)
    work = Path(tempfile.mkdtemp(prefix="lulo-aot-"))
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
    parser.add_argument("--niri", required=True, help="the niri built with patch 0004")
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
