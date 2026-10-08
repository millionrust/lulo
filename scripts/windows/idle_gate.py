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
(ConPTY) has its own work. A missing results file fails, and so does any
app named with `--expect` that has no reading: a build that never ran the
apps must not pass this gate.
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


def missing_failures(results: dict[str, dict], expected: list[str]) -> list[str]:
    """One message per expected app the results do not mention at all."""
    return [f"{app}: not measured (it did not run)" for app in expected if app not in results]


def memory_failures(results: dict[str, dict], budget_mb: float) -> list[str]:
    """The Lulo layer's shell over its idle working-set budget (an 8 GB PC
    runs it all day; ADR 0023). Only checked when the shell ran."""
    shell = results.get("lulo-shell")
    if shell is None:
        return []
    working_set = shell.get("idle_working_set_mb")
    if working_set is None:
        return ["lulo-shell: no idle memory reading"]
    if working_set > budget_mb:
        return [
            f"lulo-shell: {working_set:.1f} MB working set at idle, over the budget of {budget_mb:g} MB "
            f"(private {shell.get('idle_private_mb')} MB)"
        ]
    return []


def idle_private_failures(results: dict[str, dict], budget_mb: float | None) -> list[str]:
    """lulo-shell's private bytes at idle over `budget_mb`. Private bytes,
    not the working set, are what the owner's PC showed growing (107.8 MB
    at start on a Radeon laptop while the working set read 9-20 MB after
    the trims; WIN-OS-53). Only checked when the shell ran."""
    shell = results.get("lulo-shell")
    if shell is None or budget_mb is None:
        return []
    private = shell.get("idle_private_mb")
    if private is None:
        return ["lulo-shell: no idle private-memory reading"]
    if private > budget_mb:
        return [f"lulo-shell: {private:.1f} MB private at idle, over the budget of {budget_mb:g} MB"]
    return []


def after_use_failures(
    results: dict[str, dict], budget_mb: float | None, growth_mb: float | None
) -> list[str]:
    """lulo-shell's private memory once Spotlight and a menu have been used
    and let go of again (`after_use_private_mb`, 30 s after they closed):
    over `budget_mb`, or more than `growth_mb` above its reading after the
    first use (`after_spotlight_private_mb`; the idle reading when there is
    none), which is memory each use of a closed panel kept (WIN-OS-43). The
    first use itself loads Spotlight's catalogue and the shell libraries
    behind it once. Only checked when the shell ran."""
    shell = results.get("lulo-shell")
    if shell is None or (budget_mb is None and growth_mb is None):
        return []
    after = shell.get("after_use_private_mb")
    if after is None:
        return ["lulo-shell: no after-use memory reading"]
    failures = []
    if budget_mb is not None and after > budget_mb:
        failures.append(
            f"lulo-shell: {after:.1f} MB private after use, over the budget of {budget_mb:g} MB"
        )
    first = shell.get("after_spotlight_private_mb")
    if first is not None:
        reference, what = first, "after the first use"
    else:
        reference, what = shell.get("idle_private_mb"), "at idle"
    if growth_mb is not None and reference is not None and after - reference > growth_mb:
        failures.append(
            f"lulo-shell: {after:.1f} MB private after use, {after - reference:.1f} MB above its "
            f"{reference:.1f} MB {what} (allowed {growth_mb:g} MB): closed panels kept memory"
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
        "--shell-memory-mb",
        type=float,
        default=None,
        help="Also fail when lulo-shell's idle working set is over this many MB.",
    )
    parser.add_argument(
        "--shell-idle-private-mb",
        type=float,
        default=None,
        help="Fail when lulo-shell's private bytes at idle are over this many MB.",
    )
    parser.add_argument(
        "--shell-after-use-private-mb",
        type=float,
        default=None,
        help="Fail when lulo-shell's private bytes after Spotlight and a menu were used are over this.",
    )
    parser.add_argument(
        "--shell-after-use-growth-mb",
        type=float,
        default=None,
        help="Fail when lulo-shell's private bytes after use grew more than this over idle.",
    )
    parser.add_argument(
        "--expect",
        nargs="*",
        default=[],
        help="Apps (and lulo-shell/lulo-session) that must have a reading.",
    )
    parser.add_argument(
        "--max-world-tick-ticks",
        type=float,
        help="Fail when one World Clock redraw (launch_smoke.py --world-tick-check) costs more.",
    )
    arguments = parser.parse_args()
    if not arguments.results.exists():
        print(f"idle gate: FAIL: {arguments.results} is missing (the apps did not run)")
        return 1
    results = json.loads(arguments.results.read_text(encoding="utf-8"))
    for app, measurement in sorted(results.items()):
        print(
            f"idle gate: {app}: {measurement.get('idle_ticks')} ticks "
            f"({measurement.get('idle_renderer_ticks', 0)} in the software rasteriser), "
            f"{measurement.get('idle_wakes')} wake-ups, launch {measurement.get('launch_ms')} ms"
        )
    failures = missing_failures(results, arguments.expect)
    failures += idle_failures(
        results, arguments.budget_ticks, tuple(arguments.exempt), PER_APP_BUDGET_TICKS
    )
    if arguments.shell_memory_mb is not None:
        shell = results.get("lulo-shell", {})
        print(
            f"idle gate: lulo-shell memory at idle: working set {shell.get('idle_working_set_mb')} MB, "
            f"private {shell.get('idle_private_mb')} MB, peak {shell.get('peak_working_set_mb')} MB; "
            f"after Spotlight: {shell.get('after_spotlight_working_set_mb')} MB"
        )
        failures += memory_failures(results, arguments.shell_memory_mb)
    failures += idle_private_failures(results, arguments.shell_idle_private_mb)
    if (
        arguments.shell_after_use_private_mb is not None
        or arguments.shell_after_use_growth_mb is not None
    ):
        shell = results.get("lulo-shell", {})
        print(
            f"idle gate: lulo-shell private memory: {shell.get('idle_private_mb')} MB at idle, "
            f"{shell.get('after_spotlight_private_mb')} MB after the first use, "
            f"{shell.get('after_use_private_mb')} MB 30 s after Spotlight and a menu closed again "
            f"(working set {shell.get('after_use_working_set_mb')} MB)"
        )
        failures += after_use_failures(
            results, arguments.shell_after_use_private_mb, arguments.shell_after_use_growth_mb
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
