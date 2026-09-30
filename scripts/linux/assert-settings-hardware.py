#!/usr/bin/env python3
"""Read-only AT-SPI check of hardware panes on the reference laptop.

Launches one disposable Settings process at a time. No AT-SPI actions or
input injection are used; only the accessibility tree is read.
"""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import atspi_assert_support as support  # noqa: E402


def app():
    return support.find_app("rmac-system-settings") or support.find_app("System Settings")


def names():
    found = app()
    if found is None:
        return set()
    return {support.name(node) for node in support.descendants(found) if support.name(node)}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    args = parser.parse_args()
    for key in ("WAYLAND_DISPLAY", "XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS"):
        if not os.environ.get(key):
            parser.error(f"{key} is required from the live graphical session")
    if app() is not None:
        parser.error("Settings is already running; refusing a second instance")

    checks = (
        ("mouse", {"Mouse", "Battery", "Trackpad", "Touchscreen", "Pointing stick speed", "Pointing stick acceleration", "Scroll with middle button"}),
        ("touchscreen", {"Touchscreen", "Battery", "Trackpad", "Mouse", "Use touch input on the display"}),
    )
    with tempfile.TemporaryDirectory(prefix="lulo-settings-hardware-") as temporary:
        environment = os.environ.copy()
        for key, folder in (("XDG_CONFIG_HOME", "config"), ("XDG_STATE_HOME", "state"), ("XDG_DATA_HOME", "data"), ("XDG_CACHE_HOME", "cache")):
            path = Path(temporary, folder)
            path.mkdir()
            environment[key] = str(path)
        for pane, required in checks:
            process = subprocess.Popen(
                [str(args.binary), "--pane", pane],
                env=environment,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                close_fds=True,
            )
            try:
                support.wait_for(app, f"Settings {pane} window")
                observed = support.wait_for(
                    lambda: (seen if required <= (seen := names()) else None),
                    f"{pane} hardware rows",
                )
                print(f"{pane}: {len(required)} required labels present; {len(observed)} accessible names read")
            finally:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
            support.wait_for(lambda: app() is None, f"{pane} process exit")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
