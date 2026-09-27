#!/usr/bin/env python3
"""Focused installed-build journeys in a disposable nested Lulo session.

Checks that the installed Terminal accepts keyboard input and renders the
resulting command output. The session uses a temporary HOME, private D-Bus,
and the existing nested niri harness; it does not touch the active desktop or
host power state.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))

import run_lulo  # noqa: E402
import run_power_dialogs  # noqa: E402


def load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def result(name: str, passed: bool, detail: str) -> dict[str, object]:
    row = {"name": name, "status": "pass" if passed else "fail", "detail": detail}
    print(f"{row['status'].upper()} {name}: {detail}", flush=True)
    return row


def private_home(session, name: str) -> None:
    home = session.out.parent / "homes" / name
    for sub in (".config", ".local/share", ".local/state", ".cache", "Desktop", "Documents"):
        (home / sub).mkdir(parents=True, exist_ok=True)
    (home / ".config/user-dirs.dirs").write_text(
        'XDG_DESKTOP_DIR="$HOME/Desktop"\nXDG_DOCUMENTS_DIR="$HOME/Documents"\n'
    )
    session.env.update({
        "HOME": str(home), "XDG_CONFIG_HOME": str(home / ".config"),
        "XDG_DATA_HOME": str(home / ".local/share"), "XDG_STATE_HOME": str(home / ".local/state"),
        "XDG_CACHE_HOME": str(home / ".cache"),
    })
    os.environ.update(session.env)


def wait_for(predicate, timeout: float = 12.0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.2)
    return None


def inner(args, work: Path) -> int:
    session = run_power_dialogs.Run(args, work)
    rows = []
    try:
        session.start()
        private_home(session, "terminal")
        terminal = Path(args.app_bin_dir) / "rmac-terminal"
        child = session.spawn([str(terminal)], "beta-terminal")
        window = wait_for(lambda: next((w for w in session.niri("windows") or []
                                        if w.get("app_id") == "org.rmac.Terminal"), None))
        rows.append(result("terminal_launch", window is not None and child.poll() is None,
                           "installed Terminal window appeared in the private compositor" if window else
                           "installed Terminal did not create a window"))
        if window:
            session.keys.type_text("echo RMAC_BETA_TERMINAL_OK\n", delay=0.01)
            journey = load_module("terminal_journey", ROOT / "scripts/linux/run-journey-terminal.py")
            grid = wait_for(lambda: journey.find_terminal_grid_node(timeout=0.2), timeout=8)
            def command_and_output():
                if grid is None:
                    return None
                text = grid.queryText().getText(0, -1)
                return text if text.count("RMAC_BETA_TERMINAL_OK") >= 2 else None

            value = wait_for(command_and_output, timeout=8)
            occurrences = value.count("RMAC_BETA_TERMINAL_OK") if value else 0
            rows.append(result("terminal_typed_roundtrip", value is not None,
                               "marker appeared in both the echoed command and executed output" if value is not None else
                               f"expected marker in command and output, observed {occurrences} occurrence(s) (grid_found={grid is not None})"))
        child.terminate()
        try:
            child.wait(timeout=5)
        except subprocess.TimeoutExpired:
            child.kill()
            child.wait(timeout=3)

    except Exception as error:  # keep a useful partial report on unexpected failures
        rows.append(result("harness_error", False, f"{type(error).__name__}: {error}"))
    finally:
        status = session.finish()
    (work / "beta-report.json").write_text(json.dumps({"format": 1, "results": rows}, indent=2) + "\n")
    return 1 if status or any(row["status"] == "fail" for row in rows) else 0


def outer(args) -> int:
    for tool in ("sway", "swaymsg", "dbus-run-session", "busctl"):
        if shutil.which(tool) is None:
            raise SystemExit(f"{tool} is required")
    work = Path(tempfile.mkdtemp(prefix="lulo-private-beta-"))
    try:
        env = run_lulo.isolated_environment(work)
        run_lulo.refuse_live_session(env)
        services = work / "dbus-services"
        services.mkdir()
        accessibility = Path("/usr/share/dbus-1/services/org.a11y.Bus.service")
        if accessibility.exists():
            shutil.copy(accessibility, services / accessibility.name)
        config = work / "session.conf"
        config.write_text(
            "<!DOCTYPE busconfig PUBLIC \"-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN\"\n"
            " \"http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd\">\n"
            f"<busconfig><type>session</type><listen>unix:dir={work}</listen><auth>EXTERNAL</auth>"
            f"<servicedir>{services}</servicedir><policy context=\"default\"><allow send_destination=\"*\" eavesdrop=\"true\"/>"
            "<allow eavesdrop=\"true\"/><allow own=\"*\"/></policy></busconfig>\n"
        )
        command = ["dbus-run-session", f"--config-file={config}", "--", sys.executable,
                   str(Path(__file__).resolve()), "--inner", str(work), *sys.argv[1:]]
        with open(work / "session.log", "w") as log:
            code = subprocess.call(command, env=env, stdout=log, stderr=subprocess.STDOUT)
        report = work / "beta-report.json"
        if report.exists():
            print(report.read_text(), end="")
        else:
            print((work / "session.log").read_text()[-4000:])
        return code
    finally:
        run_lulo.reap(work / "runtime")
        if not args.keep:
            run_lulo.remove_tree(work)
        else:
            print(f"kept private artifacts: {work}", file=sys.stderr)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--niri", default="/usr/bin/niri")
    parser.add_argument("--bin-dir", required=True, help="installed shell aliases (top-bar, dock, etc.)")
    parser.add_argument("--app-bin-dir", required=True, help="installed Lulo application binaries")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--inner", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.inner:
        return inner(args, args.inner)
    return outer(args)


if __name__ == "__main__":
    raise SystemExit(main())
