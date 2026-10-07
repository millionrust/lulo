"""Fail when a Lulo app on Windows used CPU while idle.

Usage: python scripts/windows/idle_gate.py <results.json> [--budget-ticks N]
           [--exempt APP ...]

Reads the JSON that `launch_smoke.py --results` writes and checks every app's
idle CPU (process time over the 20 s no-input window, in 15.6 ms scheduling
ticks) against the budget. With `gpui_windows`' parked frame loop
(docs/decisions/0025-vendor-gpui-windows.md) an idle window wakes nothing,
so the budget is one tick. Terminal is exempt by default: its live shell
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


def idle_failures(
    results: dict[str, dict], budget_ticks: float, exempt: tuple[str, ...]
) -> list[str]:
    """One message per app over budget, or with no idle reading."""
    failures = []
    for app, measurement in sorted(results.items()):
        if app in exempt:
            continue
        ticks = measurement.get("idle_ticks")
        if ticks is None:
            failures.append(f"{app}: no idle CPU reading")
            continue
        # Windows charges CPU time a whole 15.6 ms tick at a time, so a
        # reading is a whole number of ticks give or take rounding.
        if round(ticks) > budget_ticks:
            sources = ", ".join(
                f"{entry['count']} x {entry['source']}"
                for entry in measurement.get("idle_wake_sources", [])[:4]
            )
            failures.append(
                f"{app}: {ticks:.2f} ticks idle over the budget of {budget_ticks:g}"
                + (f" (wake-ups: {sources})" if sources else "")
            )
    return failures


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("results", type=Path)
    parser.add_argument("--budget-ticks", type=float, default=DEFAULT_BUDGET_TICKS)
    parser.add_argument("--exempt", nargs="*", default=list(DEFAULT_EXEMPT))
    arguments = parser.parse_args()
    if not arguments.results.exists():
        print(f"idle gate: {arguments.results} is missing (the apps did not run); skipped")
        return 0
    results = json.loads(arguments.results.read_text(encoding="utf-8"))
    for app, measurement in sorted(results.items()):
        print(
            f"idle gate: {app}: {measurement.get('idle_ticks')} ticks, "
            f"{measurement.get('idle_wakes')} wake-ups, launch {measurement.get('launch_ms')} ms"
        )
    failures = idle_failures(results, arguments.budget_ticks, tuple(arguments.exempt))
    for failure in failures:
        print(f"idle gate: FAIL: {failure}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
