"""Fail when a Lulo app on Windows used CPU while idle.

Usage: python scripts/windows/idle_gate.py <results.json> [--budget-ticks N]
           [--exempt APP ...]

Reads the JSON that `launch_smoke.py --results` writes and checks every app's
idle CPU (process time over the 20 s no-input window, in 15.6 ms scheduling
ticks) against the budget. With `gpui_windows`' parked frame loop
(docs/decisions/0025-vendor-gpui-windows.md) an idle window wakes nothing,
so the budget is one tick. The Lulo layer's two processes (`lulo-shell`
and `lulo-session`, ADR 0023 phase 3), measured by `launch_smoke.py
--shell`, are gated the same way. Terminal is exempt by default: its live shell
(ConPTY) has its own work. A missing results file passes with a note, so a
build failure, which the Windows job reports elsewhere, does not also fail
this gate.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

DEFAULT_BUDGET_TICKS = 1.0
DEFAULT_EXEMPT = ("rmac-terminal",)

# Every reading below is the app's own CPU: `launch_smoke.py` attributes
# the threads of WARP, Direct3D's software rasteriser that stands in for a
# GPU on the runner (`idle_renderer_ticks`), and they are left out. One
# full frame of a 1024 x 768 window costs WARP about 9 ticks whatever the
# window shows (op/win-settings measured Clock's World Clock with its map,
# cards and pins each removed, and all of them: 9.2 to 10.5 ticks per
# frame every time); on a real PC that is the GPU's work, not the CPU's.
# A frame that should not have happened still costs the app its own
# layout and paint on the main thread, so a poll or a failure to re-park
# still fails here.
#
# Clock's World Clock redraws once a minute while its window is active
# (minute precision, as on the Mac); the 20 s idle window has about a
# one-in-three chance of holding that boundary. Its own cost is gated per
# redraw by `--max-world-tick-ticks` (`launch_smoke.py --world-tick-check`,
# ten redraws in one window): since op/win-settings it is about one tick
# in CI's debug build, the main thread's frame plus the next minute's map
# painted ahead on a background thread. Four runs read 0.90, 0.90, 1.10
# and 2.00 ticks per redraw: thread times move a whole tick at a time, and
# the rasteriser's share is the thread pool's time less the app's own
# tasks', so one reading swings by about a tick. 3 (per redraw, and here
# for the one boundary) holds that swing and still fails the old
# behaviour, which re-rasterised the whole map on the CPU (4 to 5 ticks of
# its own in a debug build) and drew two frames: 19 and 28 ticks in all in
# runs 37633070594 and 37657567719, under a 48-tick allowance.
#
# The Lulo layer's menu bar shows the time to the minute too, so its
# window redraws once a minute, and the same one-in-three window catches
# it: run 37713970007 charged lulo-shell 2.00 ticks of its own for that
# one update (three frames: the clock and `gpui_windows`' settle).
PER_APP_BUDGET_TICKS: dict[str, float] = {
    "rmac-clock": 3.0,
    "lulo-shell": 2.0,
}


def own_ticks(measurement: dict) -> float | None:
    """The app's own idle CPU: its process time less the software
    rasteriser's share, or `None` with no reading."""
    ticks = measurement.get("idle_ticks")
    if ticks is None:
        return None
    return max(ticks - measurement.get("idle_renderer_ticks", 0.0), 0.0)


def idle_failures(
    results: dict[str, dict],
    budget_ticks: float,
    exempt: tuple[str, ...],
    per_app_budget: dict[str, float] | None = None,
) -> list[str]:
    """One message per app over budget, or with no idle reading."""
    failures = []
    per_app_budget = per_app_budget or {}
    for app, measurement in sorted(results.items()):
        if app in exempt:
            continue
        ticks = own_ticks(measurement)
        if ticks is None:
            failures.append(f"{app}: no idle CPU reading")
            continue
        app_budget = per_app_budget.get(app, budget_ticks)
        # Windows charges CPU time a whole 15.6 ms tick at a time, so a
        # reading is a whole number of ticks give or take rounding.
        if round(ticks) > app_budget:
            sources = ", ".join(
                f"{entry['count']} x {entry['source']}"
                for entry in measurement.get("idle_wake_sources", [])[:4]
            )
            failures.append(
                f"{app}: {ticks:.2f} ticks idle over the budget of {app_budget:g}"
                + (f" (wake-ups: {sources})" if sources else "")
            )
    return failures


def world_tick_failures(results: dict[str, dict], max_ticks: float | None) -> list[str]:
    """Clock's World Clock redraw cost (`launch_smoke.py --world-tick-check`)
    against `max_ticks` per redraw; nothing to check when it was not run."""
    if max_ticks is None:
        return []
    tick = results.get("rmac-clock", {}).get("world_tick")
    if tick is None:
        return []
    per_redraw = tick.get("own_ticks_per_redraw", tick.get("ticks_per_redraw"))
    if per_redraw is None:
        return ["rmac-clock: no World Clock redraw reading"]
    if per_redraw > max_ticks:
        return [
            f"rmac-clock: one World Clock redraw cost {per_redraw:.2f} ticks, "
            f"over the budget of {max_ticks:g}"
        ]
    return []


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("results", type=Path)
    parser.add_argument("--budget-ticks", type=float, default=DEFAULT_BUDGET_TICKS)
    parser.add_argument("--exempt", nargs="*", default=list(DEFAULT_EXEMPT))
    parser.add_argument(
        "--max-world-tick-ticks",
        type=float,
        help="Fail when one World Clock redraw (launch_smoke.py --world-tick-check) costs more.",
    )
    arguments = parser.parse_args()
    if not arguments.results.exists():
        print(f"idle gate: {arguments.results} is missing (the apps did not run); skipped")
        return 0
    results = json.loads(arguments.results.read_text(encoding="utf-8"))
    for app, measurement in sorted(results.items()):
        print(
            f"idle gate: {app}: {measurement.get('idle_ticks')} ticks "
            f"({measurement.get('idle_renderer_ticks', 0)} in the software rasteriser), "
            f"{measurement.get('idle_wakes')} wake-ups, launch {measurement.get('launch_ms')} ms"
        )
    failures = idle_failures(
        results, arguments.budget_ticks, tuple(arguments.exempt), PER_APP_BUDGET_TICKS
    )
    tick = results.get("rmac-clock", {}).get("world_tick")
    if tick is not None:
        print(
            f"idle gate: rmac-clock: {tick.get('own_ticks_per_redraw')} ticks of its own per "
            f"World Clock redraw, {tick.get('renderer_ticks_per_redraw')} in the software "
            f"rasteriser ({tick.get('ticks')} ticks over {tick.get('redraws')} redraws)"
        )
    failures += world_tick_failures(results, arguments.max_world_tick_ticks)
    for failure in failures:
        print(f"idle gate: FAIL: {failure}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
