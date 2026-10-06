#!/usr/bin/env python3
"""Mission Control keeps its overlay surface between opens (SPEED-03): prove
the kept, unmapped overlay is harmless and every open is fresh.

    python3 scripts/behavior/run_mission_control_reopen.py --niri PATH \
        --bin-dir DIR [--bin-dir DIR ...] --json-output PATH [--opens N]

`--bin-dir` must hold `mission-control` (or `rmac-mission-control`) and
`rmac-calculator`. Uses run_speed_sweep.py's private session (headless Sway
hosting a nested niri with the shipped shell.kdl, temporary HOME and
XDG_RUNTIME_DIR, RMAC_FRAME_TRACE per process); never the live session.

With a Calculator window open, each of `--opens` rounds presses Ctrl+Up,
waits for Mission Control's first present, captures the output with grim,
presses Esc and checks, once the overlay has unmapped:

  - shown: the open changed the screen (the overlay really mapped again);
  - hidden: grim's capture matches the one taken before the open, so the
    kept overlay shows nothing, not even a transparent layer's pixels;
  - keyboard: a digit typed now reaches Calculator (an `input` row in its
    trace) and none reaches Mission Control;
  - pointer: a click on Calculator reaches Calculator, not Mission Control;
  - idle: over IDLE_S the hidden overlay presents nothing and the service
    uses less than IDLE_CPU_MS of CPU time.

Every open runs the service's capture afresh; `opens_ms` reports each open
the same way run_speed_sweep.py does (Ctrl+Up to the first present).
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Optional

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import run_speed_sweep as sweep  # noqa: E402
from run_frame_timing import read_trace  # noqa: E402

CALCULATOR_APP_ID = "org.rmac.Calculator"
IDLE_S = 3.0
IDLE_CPU_MS = 30.0
# A capture matches when at most this share of its bytes differ (a menu bar
# clock may tick between two captures; the overlay changes far more).
SAME_SCREEN_MAX_DIFF = 0.005


def count(events: list[tuple[str, int]], name: str) -> int:
    return sum(1 for event, _ in events if event == name)


def cpu_ms(pid: int) -> float:
    fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    ticks = int(fields[11]) + int(fields[12])
    return ticks * 1000.0 / os.sysconf("SC_CLK_TCK")


def diff_share(a: bytes, b: bytes) -> float:
    if len(a) != len(b) or not a:
        return 1.0
    step = 4096
    differing = 0
    for offset in range(0, len(a), step):
        chunk_a, chunk_b = a[offset:offset + step], b[offset:offset + step]
        if chunk_a != chunk_b:
            differing += sum(1 for x, y in zip(chunk_a, chunk_b) if x != y)
    return differing / len(a)


class Probe(sweep.Run):
    def grab(self, name: str) -> bytes:
        path = self.logs / f"{name}.ppm"
        subprocess.run(["grim", "-t", "ppm", str(path)], env=self.env, check=True, timeout=10)
        return path.read_bytes()

    def probe(self) -> dict[str, Any]:
        self.start()
        mission_control = self.bin("rmac-mission-control", "mission-control")
        calculator = self.bin("rmac-calculator")
        if mission_control is None or calculator is None:
            return {"error": "--bin-dir needs mission-control and rmac-calculator"}
        service, mc_trace = self.traced([str(mission_control), "--service"], "mission-control")
        self.wait_for(mc_trace.exists, timeout=20.0)
        _, calc_trace = self.traced([str(calculator)], "calculator")
        window = self.wait_for(lambda: self.window_by_app_id(CALCULATOR_APP_ID), timeout=20.0)
        if not window:
            return {"error": "Calculator did not open"}
        sweep.quiescence_wait(calc_trace, deadline=time.monotonic() + 5.0)
        time.sleep(1.0)
        width, height = (int(v) for v in self.args.output.split("x"))
        layout = window.get("layout") or {}
        pos = layout.get("tile_pos_in_workspace_view") or [width / 2 - 100, height / 2 - 100]
        size = layout.get("window_size") or layout.get("tile_size") or [200, 200]
        # Calculator's display, not a button, so a click changes nothing.
        target = (pos[0] + size[0] / 2, pos[1] + min(size[1] / 4, 60))

        rounds: list[dict[str, Any]] = []
        for index in range(self.args.opens):
            before = self.grab(f"before-{index}")
            baseline = count(read_trace(mc_trace), "present")
            start = time.monotonic()
            self.input.key("ctrl-up")
            presented = sweep.wait_for_present(mc_trace, baseline, start + 5.0)
            sweep.quiescence_wait(mc_trace, deadline=time.monotonic() + 2.0)
            shown = diff_share(before, self.grab(f"open-{index}"))
            self.input.key("escape")
            sweep.quiescence_wait(mc_trace, deadline=time.monotonic() + 2.0)
            time.sleep(0.5)
            hidden = diff_share(before, self.grab(f"closed-{index}"))

            mc_inputs = count(read_trace(mc_trace), "input")
            calc_inputs = count(read_trace(calc_trace), "input")
            self.input.key("7")
            time.sleep(0.4)
            keyboard_to_calculator = count(read_trace(calc_trace), "input") > calc_inputs
            self.input.key("escape")  # clear the digit again
            time.sleep(0.2)
            calc_inputs = count(read_trace(calc_trace), "input")
            self.input.click(target[0], target[1], width, height)
            time.sleep(0.4)
            pointer_to_calculator = count(read_trace(calc_trace), "input") > calc_inputs
            input_to_overlay = count(read_trace(mc_trace), "input") - mc_inputs
            sweep.quiescence_wait(calc_trace, deadline=time.monotonic() + 2.0)

            idle_presents = count(read_trace(mc_trace), "present")
            idle_cpu = cpu_ms(service.pid)
            time.sleep(IDLE_S)
            idle_presents = count(read_trace(mc_trace), "present") - idle_presents
            idle_cpu = cpu_ms(service.pid) - idle_cpu
            rounds.append({
                "open_ms": None if presented is None else (presented - start) * 1000.0,
                "shown_diff": shown,
                "hidden_diff": hidden,
                "keyboard_to_calculator": keyboard_to_calculator,
                "pointer_to_calculator": pointer_to_calculator,
                "input_rows_to_hidden_overlay": input_to_overlay,
                "hidden_presents": idle_presents,
                "hidden_cpu_ms": idle_cpu,
            })
        self.finish()

        failures: list[str] = []
        for index, entry in enumerate(rounds, 1):
            if entry["open_ms"] is None:
                failures.append(f"open {index}: no present after Ctrl+Up")
            if entry["shown_diff"] <= SAME_SCREEN_MAX_DIFF:
                failures.append(f"open {index}: the overlay did not change the screen")
            if entry["hidden_diff"] > SAME_SCREEN_MAX_DIFF:
                failures.append(f"open {index}: the closed overlay still changes the screen")
            if not entry["keyboard_to_calculator"]:
                failures.append(f"open {index}: typing after close did not reach Calculator")
            if not entry["pointer_to_calculator"]:
                failures.append(f"open {index}: a click after close did not reach Calculator")
            if entry["input_rows_to_hidden_overlay"]:
                failures.append(f"open {index}: the hidden overlay received input")
            if entry["hidden_presents"]:
                failures.append(f"open {index}: the hidden overlay presented frames")
            if entry["hidden_cpu_ms"] > IDLE_CPU_MS:
                failures.append(f"open {index}: the idle service used {entry['hidden_cpu_ms']:.0f} ms CPU")
        opens = [entry["open_ms"] for entry in rounds]
        return {
            "schema_version": 1,
            "captured_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "output": self.args.output,
            "opens_ms": opens,
            "open_ms_median": sweep.median(opens),
            "rounds": rounds,
            "failures": failures,
            "passed": not failures,
        }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--niri", default="/usr/bin/niri")
    parser.add_argument("--bin-dir", action="append", default=[])
    parser.add_argument("--json-output", type=Path, required=True)
    parser.add_argument("--output", default="1920x1080")
    parser.add_argument("--opens", type=int, default=3)
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--inner", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    # Fields run_speed_sweep.Run reads.
    args.app_env = []
    args.repeat = args.opens
    args.profile = "unknown"
    args.only = []
    return args


def main() -> int:
    args = parse_args()
    if args.inner:
        report = Probe(args, args.inner).probe()
        args.json_output.parent.mkdir(parents=True, exist_ok=True)
        args.json_output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
        return 0
    if not args.bin_dir:
        raise SystemExit("--bin-dir is required (at least once)")
    status = sweep.outer(args, sys.argv[1:], script=Path(__file__))
    if status != 0:
        return status
    report = json.loads(args.json_output.read_text())
    for failure in report.get("failures", []):
        print(f"FAIL {failure}", file=sys.stderr)
    if "error" in report:
        print(f"ERROR {report['error']}", file=sys.stderr)
        return 1
    print(f"opens_ms={report['opens_ms']} passed={report['passed']}")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
