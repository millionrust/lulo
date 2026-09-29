#!/usr/bin/env python3
"""Sample all running Lulo shell units over the same 60-second idle window."""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import sys
import time
from pathlib import Path


def load_budgets():
    path = Path(__file__).with_name("measure-budgets.py")
    spec = importlib.util.spec_from_file_location("lulo_measure_budgets", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seconds", type=float, default=60)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.seconds <= 0:
        parser.error("--seconds must be positive")

    budgets = load_budgets()
    os.environ.update(
        budgets.discover_environment(dict(os.environ), Path(f"/run/user/{os.getuid()}"))
    )
    hertz = os.sysconf("SC_CLK_TCK")
    pids = {
        name: pid
        for name in budgets.SHELL_SURFACES
        if (pid := budgets.get_unit_main_pid(f"{name}.service")) is not None
    }
    before = {
        name: budgets.sample_process_tree(pid, hertz) for name, pid in pids.items()
    }
    start = time.monotonic()
    time.sleep(args.seconds)
    elapsed = time.monotonic() - start
    after = {
        name: budgets.sample_process_tree(pid, hertz) for name, pid in pids.items()
    }

    units = {}
    total_ticks = 0
    total_switches = 0
    for name, pid in pids.items():
        first, last = before[name], after[name]
        if first is None or last is None:
            units[name] = {"pid": pid, "running_throughout": False}
            continue
        ticks = last["cpu_ticks"] - first["cpu_ticks"]
        switches = (
            last["voluntary_ctxt_switches"]
            + last["nonvoluntary_ctxt_switches"]
            - first["voluntary_ctxt_switches"]
            - first["nonvoluntary_ctxt_switches"]
        )
        total_ticks += ticks
        total_switches += switches
        units[name] = {
            "pid": pid,
            "running_throughout": True,
            "cpu_percent_one_core": round(budgets.cpu_percent_from_ticks(ticks, hertz, elapsed), 3),
            "context_switches_per_second": round(switches / elapsed, 3),
        }
    report = {
        "captured_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "seconds": round(elapsed, 3),
        "budget_cpu_percent_one_core": 0.5,
        "cpu_percent_one_core": round(
            budgets.cpu_percent_from_ticks(total_ticks, hertz, elapsed), 3
        ),
        "context_switches_per_second": round(total_switches / elapsed, 3),
        "units": units,
        "note": "Context switches are a wake-up proxy; active units were sampled concurrently.",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
