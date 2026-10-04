#!/usr/bin/env python3
"""Stage CI runtime binaries and merge isolated runner results."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import run_lulo  # noqa: E402
import scenario  # noqa: E402

SHELL_BINARIES = {
    "top-bar": "rmac-top-bar",
    "dock": "rmac-dock",
    "wallpaper": "rmac-wallpaper",
    "app-switcher": "rmac-app-switcher",
    "mission-control": "rmac-mission-control",
    "osd": "rmac-osd",
    "screenshot": "rmac-screenshot",
}
REQUIRED_APPS = {name for choices in run_lulo.APP_BINARIES.values() for name in choices if name.startswith("rmac-")}
REQUIRED_APPS.discard("rmac-wallpaper")
REQUIRED = REQUIRED_APPS | {"rmac-file-chooser", "rmac-shortcut-dispatch"}
JOURNEYS = {f"{number:02d}-{name}" for number, name in enumerate(
    ("files", "text-editor", "settings", "calculator", "preview", "notes",
     "terminal", "shell", "menu-bar", "spotlight"), 1)}
CHECKS = {"menu-dismiss", "power-dialogs", "window-move", "idle-cpu"} | JOURNEYS
SHARDS = 8


def stage(root: Path, shell: Path, output: Path) -> None:
    output.mkdir(parents=True, exist_ok=True)
    for source in sorted(root.glob("rmac-*")):
        if source.is_file() and os.access(source, os.X_OK):
            shutil.copy2(source, output / source.name)
    for binary, alias in SHELL_BINARIES.items():
        source = shell / binary
        if not source.is_file():
            raise FileNotFoundError(source)
        # Store the installed name once; each playback runner recreates the
        # legacy shell names as symlinks after downloading the artifact.
        shutil.copy2(source, output / alias)
    missing = sorted(REQUIRED - {path.name for path in output.iterdir()})
    if missing:
        raise RuntimeError(f"runtime artifact lacks binaries: {', '.join(missing)}")
    print(f"Staged {sum(path.is_file() for path in output.iterdir())} binaries")


def prepare(directory: Path) -> None:
    for binary, alias in SHELL_BINARIES.items():
        source = directory / alias
        if not source.is_file():
            raise FileNotFoundError(source)
        (directory / binary).symlink_to(alias)


def summarize(root: Path, output: Path) -> int:
    expected = {f"scenario-{index}" for index in range(SHARDS)} | CHECKS
    reports = {path.parent.name: json.loads(path.read_text())
               for path in root.glob("*/status.json")}
    problems = [f"missing result: {name}" for name in sorted(expected - reports.keys())]
    problems += [f"unexpected result: {name}" for name in sorted(reports.keys() - expected)]
    scenarios = {}
    for name, report in sorted(reports.items()):
        if report.get("exit_code") != 0:
            problems.append(f"{name}: runner exited {report.get('exit_code')}")
        if name.startswith("scenario-"):
            result_file = root / name / "behavior-results.json"
            if not result_file.exists():
                problems.append(f"{name}: missing behavior-results.json")
                continue
            for result in json.loads(result_file.read_text()).get("results", []):
                sid = result["scenario"]
                if sid in scenarios:
                    problems.append(f"duplicate scenario: {sid}")
                scenarios[sid] = result
                if result["status"] != "pass":
                    detail = result.get("mismatches") or result.get("lulo", {}).get("error")
                    problems.append(f"{sid}: {result['status']} ({str(detail)[:240]})")
    wanted = {scenario.scenario_id(path) for path in run_lulo.runnable_scenarios([])}
    problems += [f"missing scenario: {sid}" for sid in sorted(wanted - scenarios.keys())]
    problems += [f"unexpected scenario: {sid}" for sid in sorted(scenarios.keys() - wanted)]
    passed = sum(result["status"] == "pass" for result in scenarios.values())
    lines = [f"# Lulo runtime validation", "",
             f"Scenarios: **{passed}/{len(wanted)}** passed; runtime checks: "
             f"**{sum(reports.get(name, {}).get('exit_code') == 0 for name in CHECKS)}/{len(CHECKS)}** passed.", ""]
    if problems:
        lines += ["## Regressions", ""] + [f"- {problem}" for problem in problems]
    else:
        lines.append("All recorded scenarios and runtime checks passed.")
    output.write_text("\n".join(lines) + "\n")
    print(output.read_text())
    return int(bool(problems))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    stage_parser = sub.add_parser("stage")
    stage_parser.add_argument("root", type=Path)
    stage_parser.add_argument("shell", type=Path)
    stage_parser.add_argument("output", type=Path)
    prepare_parser = sub.add_parser("prepare")
    prepare_parser.add_argument("directory", type=Path)
    summary_parser = sub.add_parser("summarize")
    summary_parser.add_argument("results", type=Path)
    summary_parser.add_argument("output", type=Path)
    args = parser.parse_args()
    if args.command == "stage":
        stage(args.root, args.shell, args.output)
        return 0
    if args.command == "prepare":
        prepare(args.directory)
        return 0
    return summarize(args.results, args.output)


if __name__ == "__main__":
    raise SystemExit(main())
