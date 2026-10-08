"""Windows' half of the cross-platform shell check: the fixed scene.

    python scripts/windows/shell_scene.py --bin-dir target/debug --output PNG

ADR 0023, "Phase 3 revised: shared shell views": the menu bar, the Dock and
the desktop are one view on Lulo OS and on Windows. This draws, on the
runner's desktop, the scene `scripts/behavior/run_shell_scene.py` draws in a
nested niri, so `scripts/compare_shell_scenes.py` can hold the two to the
same pixels: the screen at 100 % (1024 x 768 on the runner), the built-in
Lulo wallpaper, the clock at Thursday 8 October 9:41 AM, the status items'
fixed readings, the Dock pinning the nine Lulo apps that build for Windows,
nothing running, the Recycle Bin empty and an empty Desktop folder.

`lulo-session.exe` starts the Lulo layer with private profile folders and
`LULO_SHELL_TRACE=1`; once lulo-shell says it is ready and its surfaces had
time to decode their pictures, the screen is captured and `lulo-session
--stop` gives the desktop back.
"""

from __future__ import annotations

import argparse
import ctypes
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

SCENE_TIME = "2026-10-08T09:41"
SCENE_PINS = [
    "org.rmac.Files",
    "org.rmac.Notes",
    "org.rmac.TextEditor",
    "org.rmac.Terminal",
    "org.rmac.SystemSettings",
    "org.rmac.Calculator",
    "org.rmac.Clock",
    "org.rmac.Weather",
    "org.rmac.Preview",
]
READY_TIMEOUT_SECONDS = 90.0
SHERB_NOCONFIRMATION = 0x1
SHERB_NOPROGRESSUI = 0x2
SHERB_NOSOUND = 0x4


def empty_recycle_bin() -> None:
    shell32 = ctypes.windll.shell32
    shell32.SHEmptyRecycleBinW(None, None, SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND)


def scene_profile(root: Path) -> dict[str, str]:
    """Private Lulo profile folders holding the scene's Dock pins."""
    folders = {
        "XDG_CONFIG_HOME": root / "config",
        "XDG_DATA_HOME": root / "data",
        "XDG_STATE_HOME": root / "state",
        "XDG_CACHE_HOME": root / "cache",
        "RMAC_DESKTOP_DIR": root / "Desktop",
    }
    for folder in folders.values():
        folder.mkdir(parents=True, exist_ok=True)
    settings = folders["XDG_CONFIG_HOME"] / "rmac"
    settings.mkdir(parents=True, exist_ok=True)
    pins = ",".join(f'"{app}"' for app in SCENE_PINS)
    (settings / "shell.json").write_text(
        f'{{"version": 3, "settings": {{"pinned_apps": [{pins}]}}}}\n', encoding="utf-8"
    )
    return {name: str(path) for name, path in folders.items()}


def wait_for_ready(log: Path, timeout: float) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if log.exists() and "lulo-shell: ready at" in log.read_text(errors="replace"):
            return True
        time.sleep(0.25)
    return False


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--bin-dir", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--settle", type=float, default=8.0)
    args = parser.parse_args()

    from PIL import ImageGrab

    bin_dir = Path(args.bin_dir).resolve()
    output = Path(args.output).resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    empty_recycle_bin()
    work = Path(tempfile.mkdtemp(prefix="lulo-scene-"))
    environment = {
        **os.environ,
        **scene_profile(work),
        "RMAC_SHELL_SCENE": "1",
        "RMAC_SHELL_SCENE_TIME": SCENE_TIME,
        "LULO_SHELL_TRACE": "1",
    }
    log = work / "session.log"
    session = subprocess.Popen(
        [str(bin_dir / "lulo-session.exe")],
        env=environment,
        stdout=open(log, "w"),
        stderr=subprocess.STDOUT,
    )
    try:
        if not wait_for_ready(log, READY_TIMEOUT_SECONDS):
            print(f"FAIL lulo-shell did not get ready:\n{log.read_text(errors='replace')[-4000:]}")
            return 1
        time.sleep(args.settle)
        ImageGrab.grab().save(output)
        print(f"PASS captured the Windows scene to {output}")
        return 0
    finally:
        subprocess.run([str(bin_dir / "lulo-session.exe"), "--stop"], env=environment, timeout=60, check=False)
        try:
            session.wait(30)
        except subprocess.TimeoutExpired:
            session.kill()
        print(f"--- session log\n{log.read_text(errors='replace')[-4000:]}")


if __name__ == "__main__":
    sys.exit(main())
