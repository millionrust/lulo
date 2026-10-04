#!/usr/bin/env python3
"""Analyse an 8-hour memory soak's samples.jsonl (scripts/behavior/run_memory_soak.py)
into a docs/perf/-style JSON report for the Beta 1 gate "Memory (8-hour
soak): per-app budget, no leak" (docs/beta-checklist.md).

For each tracked process it computes the start/end/peak RSS and PSS, an
ordinary-least-squares growth slope in MiB/hour (RSS and PSS both use every
sample, not just the endpoints, so a couple of noisy points cannot swing the
result), and flags it as a suspected leak when either:

  * the slope is more than ~5%/hour of the starting value, sustained (the
    fit's R^2 is at least 0.5, so a flat trace with one noisy sample is not
    flagged) -- growth only: a falling trace (e.g. the kernel reclaiming
    idle pages under memory pressure) is never a leak, however steep; or
  * the peak exceeds scripts/performance-budgets.json's
    memory.application_rss_mib (128 MiB), or the end-minus-start growth (not
    shrinkage), scaled to what it would be over a full 8-hour window,
    exceeds memory.application_rss_growth_mib_8h (16 MiB).

RSS and PSS alone are meaningless under memory pressure: the kernel can
reclaim an idle process's clean/file-backed pages at any time, which shows
up as RSS/PSS falling even though the process has not freed anything. So
this script also evaluates a third, swap-aware metric wherever the samples
have it (scripts/behavior/run_memory_soak.py's smaps_rollup Pss_Anon +
SwapPss): the process's anonymous resident pages plus the anonymous pages
the kernel has swapped out. That sum is a process's real private footprint
-- it does not shrink just because the machine reclaimed cache -- and is
what "leak_suspected" is judged on when it is available: the per-app peak
budget and the growth budget then apply to the private footprint, and RSS/PSS
are reported for reference only.
Samples captured before that field existed simply report "no live samples
with this metric" for it and fall back to RSS/PSS.

The three shell pieces (wallpaper, top-bar, dock) are also reported combined
against memory.combined_shell_rss_mib / combined_shell_rss_growth_mib_8h.

    python3 scripts/linux/analyze-soak.py \\
        --samples ~/rmac-coord/soak-2026-09-29/samples.jsonl \\
        --json-output docs/perf/reference-laptop-2026-09-29-memory-soak.json \\
        --markdown-output docs/perf/reference-laptop-2026-09-29-memory-soak.md

This script is read-only: it never launches an app or touches the live
session; it only reads the JSONL the soak already wrote.
"""

from __future__ import annotations

import argparse
import json
import statistics
import sys
import time
from pathlib import Path
from typing import Any, Optional

SCRIPTS_DIR = Path(__file__).resolve().parents[1]
DEFAULT_BUDGETS = SCRIPTS_DIR / "performance-budgets.json"

SHELL_PIECES = frozenset({"wallpaper", "top-bar", "dock"})
LEAK_SLOPE_PERCENT_PER_HOUR = 5.0
LEAK_MIN_R_SQUARED = 0.5


class SoakAnalysisError(RuntimeError):
    pass


# --------------------------------------------------------------------------
# Pure computation -- unit-tested from scripts/test_analyze_soak.py.
# --------------------------------------------------------------------------


def load_samples(path: Path) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []
    with path.open() as handle:
        for line_number, line in enumerate(handle, start=1):
            line = line.strip()
            if not line:
                continue
            try:
                records.append(json.loads(line))
            except json.JSONDecodeError as error:
                raise SoakAnalysisError(f"{path}:{line_number}: not valid JSON") from error
    return records


def group_by_app(records: list[dict[str, Any]]) -> dict[str, list[dict[str, Any]]]:
    grouped: dict[str, list[dict[str, Any]]] = {}
    for record in records:
        app = record.get("app")
        if not isinstance(app, str):
            continue
        grouped.setdefault(app, []).append(record)
    for app_records in grouped.values():
        app_records.sort(key=lambda record: record.get("elapsed_seconds", 0.0))
    return grouped


def linear_regression(xs: list[float], ys: list[float]) -> tuple[float, float, float]:
    """Ordinary least squares. Returns (slope, intercept, r_squared).

    r_squared is 0.0 for fewer than three points or a zero-variance `ys`
    series (a flat trace is not "explained" by a slope; it simply has none).
    """

    n = len(xs)
    if n < 2:
        raise ValueError("at least two points are required for a regression")
    mean_x = sum(xs) / n
    mean_y = sum(ys) / n
    ss_xx = sum((x - mean_x) ** 2 for x in xs)
    ss_xy = sum((x - mean_x) * (y - mean_y) for x, y in zip(xs, ys))
    if ss_xx == 0:
        return 0.0, mean_y, 0.0
    slope = ss_xy / ss_xx
    intercept = mean_y - slope * mean_x
    if n < 3:
        return slope, intercept, 0.0
    ss_tot = sum((y - mean_y) ** 2 for y in ys)
    if ss_tot == 0:
        return slope, intercept, 0.0
    ss_res = sum((y - (slope * x + intercept)) ** 2 for x, y in zip(xs, ys))
    r_squared = max(0.0, 1.0 - ss_res / ss_tot)
    return slope, intercept, r_squared


def percent_per_hour(slope_per_second: float, baseline: float) -> float:
    if baseline <= 0:
        return 0.0
    return slope_per_second * 3600.0 / baseline * 100.0


def evaluate_process_series(
    name: str,
    records: list[dict[str, Any]],
    rss_budget_mib: float,
    growth_budget_mib_8h: float,
    metric_key: str,
    metric_label: str,
) -> dict[str, Any]:
    """Analyse one metric (rss_kib or pss_kib) across one process's samples."""

    alive_records = [record for record in records if record.get("alive") and metric_key in record]
    if not alive_records:
        return {
            "metric": metric_label,
            "sample_count": 0,
            "note": "no live samples with this metric",
        }
    xs_hours = [record["elapsed_seconds"] / 3600.0 for record in alive_records]
    ys_mib = [record[metric_key] / 1024.0 for record in alive_records]
    start_mib, end_mib = ys_mib[0], ys_mib[-1]
    peak_mib = max(ys_mib)
    duration_hours = xs_hours[-1] - xs_hours[0]

    slope_mib_per_hour = 0.0
    r_squared = 0.0
    if len(alive_records) >= 2:
        xs_seconds = [hours * 3600.0 for hours in xs_hours]
        slope_per_second, _intercept, r_squared = linear_regression(xs_seconds, ys_mib)
        slope_mib_per_hour = slope_per_second * 3600.0

    growth_mib = end_mib - start_mib
    growth_mib_scaled_to_8h = (growth_mib / duration_hours * 8.0) if duration_hours > 0 else 0.0
    slope_percent_per_hour = percent_per_hour(slope_mib_per_hour / 3600.0, start_mib)

    # A leak is sustained GROWTH only. A falling slope or negative
    # end-minus-start delta (e.g. the kernel reclaiming idle pages under
    # memory pressure) must never be flagged, however large the magnitude.
    sustained_growth = (
        slope_percent_per_hour > LEAK_SLOPE_PERCENT_PER_HOUR and r_squared >= LEAK_MIN_R_SQUARED
    )
    over_peak_budget = peak_mib > rss_budget_mib
    over_growth_budget = growth_mib_scaled_to_8h > growth_budget_mib_8h
    leak_suspected = sustained_growth or over_peak_budget or over_growth_budget

    return {
        "metric": metric_label,
        "sample_count": len(alive_records),
        "duration_hours": round(duration_hours, 3),
        "start_mib": round(start_mib, 1),
        "end_mib": round(end_mib, 1),
        "peak_mib": round(peak_mib, 1),
        "growth_mib": round(growth_mib, 1),
        "growth_mib_scaled_to_8h": round(growth_mib_scaled_to_8h, 1),
        "slope_mib_per_hour": round(slope_mib_per_hour, 3),
        "slope_percent_per_hour": round(slope_percent_per_hour, 2),
        "r_squared": round(r_squared, 3),
        "budget_rss_mib": rss_budget_mib,
        "budget_growth_mib_8h": growth_budget_mib_8h,
        "over_peak_budget": over_peak_budget,
        "over_growth_budget": over_growth_budget,
        "sustained_growth": sustained_growth,
        "leak_suspected": leak_suspected,
    }


def with_private_footprint(records: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Add a derived `private_footprint_kib` = Pss_Anon + SwapPss to each
    record that has both fields (older samples, captured before
    run_memory_soak.py recorded them, are passed through unchanged and so
    are simply excluded by evaluate_process_series's `metric_key in record`
    filter -- reported as "no live samples with this metric", not as zero
    growth)."""

    augmented: list[dict[str, Any]] = []
    for record in records:
        if "pss_anon_kib" in record and "swap_pss_kib" in record:
            merged = dict(record)
            merged["private_footprint_kib"] = record["pss_anon_kib"] + record["swap_pss_kib"]
            augmented.append(merged)
        else:
            augmented.append(record)
    return augmented


def evaluate_process(
    name: str,
    records: list[dict[str, Any]],
    rss_budget_mib: float,
    growth_budget_mib_8h: float,
) -> dict[str, Any]:
    rss = evaluate_process_series(name, records, rss_budget_mib, growth_budget_mib_8h, "rss_kib", "rss")
    pss = evaluate_process_series(name, records, rss_budget_mib, growth_budget_mib_8h, "pss_kib", "pss")
    private_footprint = evaluate_process_series(
        name,
        with_private_footprint(records),
        rss_budget_mib,
        growth_budget_mib_8h,
        "private_footprint_kib",
        "private_footprint",
    )

    death = next((record for record in records if not record.get("alive")), None)
    cpu_records = [record for record in records if record.get("alive") and "cpu_seconds" in record]
    thread_records = [record for record in records if record.get("alive") and "threads" in record]
    fd_records = [record for record in records if record.get("alive") and "fds" in record]

    return {
        "app": name,
        "sample_count": len(records),
        "alive_at_end": bool(records) and bool(records[-1].get("alive")),
        "died_at_elapsed_seconds": death["elapsed_seconds"] if death is not None else None,
        "died_exit_code": death.get("exit_code") if death is not None else None,
        "rss": rss,
        "pss": pss,
        "private_footprint": private_footprint,
        "cpu_seconds_start": cpu_records[0]["cpu_seconds"] if cpu_records else None,
        "cpu_seconds_end": cpu_records[-1]["cpu_seconds"] if cpu_records else None,
        "threads_start": thread_records[0]["threads"] if thread_records else None,
        "threads_end": thread_records[-1]["threads"] if thread_records else None,
        "peak_threads": max((record["threads"] for record in thread_records), default=None),
        "fds_start": fd_records[0]["fds"] if fd_records else None,
        "fds_end": fd_records[-1]["fds"] if fd_records else None,
        "peak_fds": max((record["fds"] for record in fd_records), default=None),
        # Judge on the private footprint when the samples have it. RSS and
        # PSS also count shared libraries, GPU driver mappings and fonts,
        # and fall when the kernel reclaims cache, so they are reported but
        # only decide the verdict for samples that predate the footprint.
        "judged_on": "private_footprint" if private_footprint.get("sample_count") else "rss_pss",
        "leak_suspected": (
            private_footprint.get("leak_suspected", False)
            if private_footprint.get("sample_count")
            else rss.get("leak_suspected", False) or pss.get("leak_suspected", False)
        ),
    }


def combine_shell(
    grouped: dict[str, list[dict[str, Any]]],
    combined_rss_budget_mib: float,
    combined_growth_budget_mib_8h: float,
) -> Optional[dict[str, Any]]:
    """Sum RSS/PSS across the tracked shell pieces at each shared timestamp
    they were all sampled together (the soak samples every tracked process
    in the same pass, so elapsed_seconds lines up across shell pieces)."""

    shell_series = {name: records for name, records in grouped.items() if name in SHELL_PIECES}
    if not shell_series:
        return None
    by_elapsed: dict[float, dict[str, float]] = {}
    for name, records in shell_series.items():
        for record in records:
            if not record.get("alive"):
                continue
            elapsed = record.get("elapsed_seconds")
            if elapsed is None or "rss_kib" not in record or "pss_kib" not in record:
                continue
            bucket = by_elapsed.setdefault(
                elapsed,
                {"rss_kib": 0.0, "pss_kib": 0.0, "pss_anon_kib": 0.0, "swap_pss_kib": 0.0, "pieces": 0, "footprint_pieces": 0},
            )
            bucket["rss_kib"] += record["rss_kib"]
            bucket["pss_kib"] += record["pss_kib"]
            bucket["pieces"] += 1
            if "pss_anon_kib" in record and "swap_pss_kib" in record:
                bucket["pss_anon_kib"] += record["pss_anon_kib"]
                bucket["swap_pss_kib"] += record["swap_pss_kib"]
                bucket["footprint_pieces"] += 1
    # Only count a moment where every tracked shell piece was alive and
    # sampled, so a dead piece cannot understate the combined total.
    complete = {elapsed: bucket for elapsed, bucket in by_elapsed.items() if bucket["pieces"] == len(shell_series)}
    if not complete:
        return None
    ordered = sorted(complete.items())
    pseudo_records = []
    for elapsed, bucket in ordered:
        record: dict[str, Any] = {
            "alive": True,
            "elapsed_seconds": elapsed,
            "rss_kib": bucket["rss_kib"],
            "pss_kib": bucket["pss_kib"],
        }
        # Only carry the swap-aware fields when every piece reported them,
        # for the same reason a partial RSS/PSS sample point is excluded.
        if bucket["footprint_pieces"] == len(shell_series):
            record["pss_anon_kib"] = bucket["pss_anon_kib"]
            record["swap_pss_kib"] = bucket["swap_pss_kib"]
        pseudo_records.append(record)
    result = evaluate_process("shell_combined", pseudo_records, combined_rss_budget_mib, combined_growth_budget_mib_8h)
    result["pieces"] = sorted(shell_series)
    result["complete_sample_points"] = len(ordered)
    return result


def build_report(
    samples_path: Path,
    grouped: dict[str, list[dict[str, Any]]],
    budgets: dict[str, Any],
    note: Optional[str] = None,
) -> dict[str, Any]:
    memory_budgets = budgets.get("memory", {})
    app_rss_budget = float(memory_budgets.get("application_rss_mib", 128))
    app_growth_budget = float(memory_budgets.get("application_rss_growth_mib_8h", 16))
    shell_rss_budget = float(memory_budgets.get("combined_shell_rss_mib", 256))
    shell_growth_budget = float(memory_budgets.get("combined_shell_rss_growth_mib_8h", 24))

    apps: dict[str, Any] = {}
    for name, records in sorted(grouped.items()):
        if name in SHELL_PIECES:
            continue
        apps[name] = evaluate_process(name, records, app_rss_budget, app_growth_budget)

    shell_pieces: dict[str, Any] = {}
    for name in sorted(SHELL_PIECES):
        if name in grouped:
            shell_pieces[name] = evaluate_process(name, grouped[name], app_rss_budget, app_growth_budget)

    shell_combined = combine_shell(grouped, shell_rss_budget, shell_growth_budget)

    leaks_detected = sorted(name for name, app in apps.items() if app["leak_suspected"])
    leaks_detected += sorted(name for name, piece in shell_pieces.items() if piece["leak_suspected"])
    if shell_combined is not None and shell_combined["leak_suspected"]:
        leaks_detected.append("shell_combined")

    return {
        "schema_version": 2,
        "kind": "memory-soak",
        "captured_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "source_samples": str(samples_path),
        "note": note,
        "budgets": {
            "application_rss_mib": app_rss_budget,
            "application_rss_growth_mib_8h": app_growth_budget,
            "combined_shell_rss_mib": shell_rss_budget,
            "combined_shell_rss_growth_mib_8h": shell_growth_budget,
            "leak_slope_percent_per_hour": LEAK_SLOPE_PERCENT_PER_HOUR,
        },
        "apps": apps,
        "shell_pieces": shell_pieces,
        "shell_combined": shell_combined,
        "leaks_detected": leaks_detected,
        "within_budget": not leaks_detected,
    }


# --------------------------------------------------------------------------
# Markdown rendering.
# --------------------------------------------------------------------------


def _fmt_mib(value: Optional[float]) -> str:
    return "n/a" if value is None else f"{value:.1f} MiB"


def render_markdown_report(report: dict[str, Any]) -> str:
    lines: list[str] = []
    lines.append(f"# Reference laptop memory soak -- {report.get('captured_at', 'unknown')}")
    lines.append("")
    note = report.get("note")
    if note:
        lines.append(f"> **{note}**")
        lines.append("")
    lines.append(
        f"Source samples: `{report.get('source_samples', 'unknown')}`. Per-app budget: "
        f"{_fmt_mib(report['budgets']['application_rss_mib'])} idle RSS, "
        f"{_fmt_mib(report['budgets']['application_rss_growth_mib_8h'])} growth over 8h. "
        f"Combined shell budget: {_fmt_mib(report['budgets']['combined_shell_rss_mib'])} RSS, "
        f"{_fmt_mib(report['budgets']['combined_shell_rss_growth_mib_8h'])} growth over 8h. "
        f"A sustained slope over {report['budgets']['leak_slope_percent_per_hour']}%/hour "
        "(R^2 >= 0.5) is also flagged as a suspected leak -- growth only, never a falling trace. "
        "Where the samples have it, growth is also evaluated on the swap-aware private-footprint "
        "metric (Pss_Anon + SwapPss), the process's real private memory, which does not fall just "
        "because the kernel reclaimed idle pages under memory pressure."
    )
    lines.append("")
    lines.append("## Applications (RSS)")
    lines.append("")
    lines.append("| App | Samples | Start RSS | End RSS | Peak RSS | Growth/8h | Slope %/h | Leak? |")
    lines.append("|---|---:|---:|---:|---:|---:|---:|---|")
    for name, app in sorted(report.get("apps", {}).items()):
        rss = app.get("rss", {})
        lines.append(
            f"| {name} | {app.get('sample_count', 0)} | {_fmt_mib(rss.get('start_mib'))} | "
            f"{_fmt_mib(rss.get('end_mib'))} | {_fmt_mib(rss.get('peak_mib'))} | "
            f"{_fmt_mib(rss.get('growth_mib_scaled_to_8h'))} | "
            f"{rss.get('slope_percent_per_hour', 'n/a')} | "
            f"{'YES' if app.get('leak_suspected') else 'no'} |"
        )
    lines.append("")
    footprint_rows = [
        (name, app)
        for name, app in sorted(report.get("apps", {}).items())
        if app.get("private_footprint", {}).get("sample_count", 0) > 0
    ]
    if footprint_rows:
        lines.append("## Applications (private footprint: Pss_Anon + SwapPss)")
        lines.append("")
        lines.append("| App | Samples | Start | End | Peak | Growth/8h | Slope %/h | Leak? |")
        lines.append("|---|---:|---:|---:|---:|---:|---:|---|")
        for name, app in footprint_rows:
            footprint = app.get("private_footprint", {})
            lines.append(
                f"| {name} | {footprint.get('sample_count', 0)} | {_fmt_mib(footprint.get('start_mib'))} | "
                f"{_fmt_mib(footprint.get('end_mib'))} | {_fmt_mib(footprint.get('peak_mib'))} | "
                f"{_fmt_mib(footprint.get('growth_mib_scaled_to_8h'))} | "
                f"{footprint.get('slope_percent_per_hour', 'n/a')} | "
                f"{'YES' if footprint.get('leak_suspected') else 'no'} |"
            )
        lines.append("")
    else:
        lines.append(
            "## Applications (private footprint: Pss_Anon + SwapPss)\n\n"
            "No sample in this run recorded Pss_Anon/SwapPss (captured before "
            "run_memory_soak.py recorded them); growth is reported on RSS/PSS "
            "only above, which is not reliable evidence of no leak under memory "
            "pressure.\n"
        )
    combined = report.get("shell_combined")
    if combined is not None:
        rss = combined.get("rss", {})
        lines.append("## Shell pieces combined")
        lines.append("")
        lines.append(
            f"Start {_fmt_mib(rss.get('start_mib'))}, end {_fmt_mib(rss.get('end_mib'))}, "
            f"peak {_fmt_mib(rss.get('peak_mib'))}, growth/8h {_fmt_mib(rss.get('growth_mib_scaled_to_8h'))}, "
            f"slope {rss.get('slope_percent_per_hour', 'n/a')}%/h -- "
            f"{'LEAK SUSPECTED' if combined.get('leak_suspected') else 'within budget'}."
        )
        lines.append("")
    lines.append("## Leaks detected")
    lines.append("")
    leaks = report.get("leaks_detected", [])
    if leaks:
        for name in leaks:
            lines.append(f"- {name}")
    else:
        lines.append("- none")
    lines.append("")
    return "\n".join(lines)


# --------------------------------------------------------------------------
# Orchestration.
# --------------------------------------------------------------------------


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--samples", type=Path, required=True)
    parser.add_argument("--budgets", type=Path, default=DEFAULT_BUDGETS)
    parser.add_argument("--json-output", type=Path, required=True)
    parser.add_argument("--markdown-output", type=Path, default=None)
    parser.add_argument(
        "--note",
        type=str,
        default=None,
        help="free-form caveat embedded in the report, e.g. a confound that makes the run "
        "inconclusive for leak detection (concurrent build load, a busy shared machine, ...)",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        budgets = json.loads(args.budgets.read_text())
    except OSError as error:
        raise SoakAnalysisError(f"could not read budgets file {args.budgets}") from error
    records = load_samples(args.samples)
    if not records:
        raise SoakAnalysisError(f"{args.samples} contained no samples")
    grouped = group_by_app(records)
    report = build_report(args.samples, grouped, budgets, note=args.note)

    args.json_output.parent.mkdir(parents=True, exist_ok=True)
    args.json_output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(f"Wrote {args.json_output}")
    if report["leaks_detected"]:
        print(f"Suspected leaks / over-budget: {', '.join(report['leaks_detected'])}", file=sys.stderr)
    else:
        print("No leaks or over-budget processes detected.")

    if args.markdown_output is not None:
        args.markdown_output.parent.mkdir(parents=True, exist_ok=True)
        args.markdown_output.write_text(render_markdown_report(report))
        print(f"Wrote {args.markdown_output}")

    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except SoakAnalysisError as error:
        print(f"analyze-soak: {error}", file=sys.stderr)
        sys.exit(4)
    except KeyboardInterrupt:
        sys.exit(130)
