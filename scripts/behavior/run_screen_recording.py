#!/usr/bin/env python3
"""⇧⌘5 screen recording (SHOT-01), end to end, in a private nested session.

A private PipeWire, a nested niri, the Text Editor open, and the resident
`rmac-screenshot` service:

1. ⇧⌘5 opens the toolbar; Record Entire Screen, then Record starts a
   recording: the state file the menu bar's stop button follows appears and
   a movie starts growing on the Desktop.
2. `rmac-screenshot stop-recording` (what the stop button and ⌃⌘⎋ run)
   ends it; the state file goes, and the file is a real video of the whole
   screen (checked with ffprobe).
3. Record Selected Portion records only the selection: the video is the
   selection's size.

    python3 scripts/behavior/run_screen_recording.py --bin-dir DIR --niri NIRI
"""

from __future__ import annotations

import argparse
import fcntl
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / "parallel"))
import run_lulo  # noqa: E402
import run_lulo_journey  # noqa: E402

SERVICE = "rmac-screenshot"
# model.rs: the toolbar's measured geometry.
TOOLBAR_WIDTH = 474.5
TOOLBAR_HEIGHT = 50.0
TOOLBAR_BOTTOM = 97.0
RECORD_SCREEN_X = 221.75
RECORD_SELECTION_X = 269.75
RECORD_BUTTON_FROM_RIGHT = 7.0 + 78.5 / 2


def wait_until(predicate, timeout: float, interval: float = 0.2):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            value = predicate()
        except Exception:  # noqa: BLE001
            value = None
        if value:
            return value
        time.sleep(interval)
    return None


class Scenario:
    def __init__(self, args, work: Path):
        self.args = args
        self.work = work
        self.driver = run_lulo_journey.Driver(
            args, work, {"title": "Screen recording", "steps": []}, work / "out")
        self.facts: dict[str, object] = {}
        self.errors: list[str] = []
        self.runtime = Path(self.driver.session.env["XDG_RUNTIME_DIR"])
        self.desktop = Path(self.driver.session.env["HOME"]) / "Desktop"

    # ---- helpers ---------------------------------------------------------

    def state(self) -> Path:
        return self.runtime / "rmac" / "screen-recording"

    def recordings(self) -> list[Path]:
        return sorted(p for p in self.desktop.glob("Screen Recording*") if p.is_file())

    def service(self, word: str) -> None:
        subprocess.run([str(Path(self.args.bin_dir) / SERVICE), word],
                       env=self.driver.session.env, capture_output=True, timeout=15, check=False)

    def click_toolbar(self, x_from_left: float) -> None:
        session = self.driver.session
        left = (session.width - TOOLBAR_WIDTH) / 2
        top = session.height - TOOLBAR_BOTTOM - TOOLBAR_HEIGHT
        point = session.parent_point(left + x_from_left, top + TOOLBAR_HEIGHT / 2)
        session.pointer.click(*point, session.parent_width, session.parent_height)
        time.sleep(0.4)

    def click_record(self) -> None:
        self.click_toolbar(TOOLBAR_WIDTH - RECORD_BUTTON_FROM_RIGHT)

    def probe(self, path: Path) -> dict:
        result = subprocess.run(
            ["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries",
             "stream=codec_name,width,height", "-of", "json", str(path)],
            capture_output=True, text=True, timeout=30, check=False)
        try:
            streams = json.loads(result.stdout).get("streams") or [{}]
            return streams[0]
        except ValueError:
            return {}

    def record(self, name: str, button_x: float, expect_full: bool) -> None:
        before = set(self.recordings())
        self.service("toolbar")
        time.sleep(1.0)
        self.click_toolbar(button_x)
        self.click_record()
        started = wait_until(self.state().exists, 20)
        self.facts[f"{name}_started"] = bool(started)
        if not started:
            self.errors.append(f"{name}: no recording started (no state file)")
            return
        time.sleep(3.0)
        new = [p for p in self.recordings() if p not in before]
        grown = bool(new) and wait_until(lambda: new[0].stat().st_size > 2000, 10)
        self.facts[f"{name}_growing"] = bool(grown)
        if not grown:
            self.errors.append(f"{name}: no video was being written")
        self.service("stop-recording")
        stopped = wait_until(lambda: not self.state().exists(), 20)
        self.facts[f"{name}_stopped"] = bool(stopped)
        if not stopped:
            self.errors.append(f"{name}: stop-recording did not end the recording")
        time.sleep(1.0)
        if not new:
            return
        info = self.probe(new[0])
        self.facts[f"{name}_video"] = info
        if not info.get("codec_name"):
            self.errors.append(f"{name}: the file is not a readable video")
            return
        width, height = info.get("width"), info.get("height")
        session = self.driver.session
        screen = (round(session.width * self.scale), round(session.height * self.scale))
        if expect_full and (abs(width - screen[0]) > 2 or abs(height - screen[1]) > 2):
            self.errors.append(f"{name}: video is {width}x{height}, the screen is {screen}")
        if not expect_full and (width >= screen[0] or height >= screen[1]):
            self.errors.append(f"{name}: video is {width}x{height}, not a selection of {screen}")

    def run(self) -> dict:
        bins = Path(self.args.bin_dir)
        for daemon in ("pipewire", "wireplumber"):
            if subprocess.run(["which", daemon], capture_output=True).returncode == 0:
                self.driver.session.spawn([daemon], daemon)
        time.sleep(1.5)
        self.driver.start()
        self.scale = 1.0
        self.driver.launch({"launch": "text-editor"})
        time.sleep(1.0)
        self.driver.session.spawn([str(bins / SERVICE), "--service"], "screenshot",
                                  {"RMAC_FRAME_TRACE": str(self.work / "screenshot-trace.csv")})
        if not wait_until((self.runtime / "rmac" / "screenshot.sock").exists, 20):
            self.errors.append("the screenshot service did not open its socket")
            return {"facts": self.facts, "errors": self.errors}
        self.record("screen", RECORD_SCREEN_X, expect_full=True)
        self.record("selection", RECORD_SELECTION_X, expect_full=False)
        return {"facts": self.facts, "errors": self.errors}

    def finish(self) -> None:
        try:
            self.driver.session.finish()
        except Exception:  # noqa: BLE001
            pass


def inner(args) -> int:
    work = Path(args.inner)
    scenario = Scenario(args, work)
    try:
        result = scenario.run()
    except Exception as error:  # noqa: BLE001
        result = {"facts": scenario.facts,
                  "errors": scenario.errors + [f"{type(error).__name__}: {error}"]}
    finally:
        scenario.finish()
    status = "pass" if not result["errors"] else "fail"
    print(json.dumps({"scenario": "screen-recording", "status": status, **result}, indent=2,
                     default=str), flush=True)
    return 0 if status == "pass" else 1


def outer(args) -> int:
    lock = open("/tmp/lulo-journey.lock", "w")
    fcntl.flock(lock, fcntl.LOCK_EX)
    work = Path(tempfile.mkdtemp(prefix="lulo-rec-"))
    try:
        env = run_lulo.isolated_environment(work)
        run_lulo.refuse_live_session(env)
        bins = Path(args.bin_dir).expanduser().resolve()
        links = work / "bins"
        links.mkdir()
        sources = {source.name: source for source in bins.iterdir()}
        if args.override_bin_dir:
            for source in Path(args.override_bin_dir).expanduser().resolve().iterdir():
                sources[source.name] = source
        for source in sources.values():
            if source.is_file() and source.name not in {"dock", "mission-control"}:
                (links / source.name).symlink_to(source)
        for alias, source in (("dock", "rmac-dock"), ("mission-control", "rmac-mission-control")):
            if (links / source).exists():
                (links / alias).symlink_to(links / source)
        for required in (SERVICE, "rmac-text-editor"):
            if not (links / required).is_file():
                raise SystemExit(f"{required} is not in {bins} or the override directory")
        run_lulo.install_shortcut_dispatcher(env, links)
        config = run_lulo_journey.private_bus(work, env, links)
        command = ["dbus-run-session", f"--config-file={config}", "--", sys.executable,
                   str(Path(__file__).resolve()), "--inner", str(work), "--bin-dir", str(links),
                   "--niri", args.niri]
        return subprocess.call(command, env=env, close_fds=True)
    finally:
        if run_lulo.reap(work / "runtime"):
            time.sleep(1)
            run_lulo.reap(work / "runtime")
        if args.keep:
            print(f"kept {work}", flush=True)
        else:
            run_lulo.remove_tree(work)
        lock.close()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--bin-dir", required=True)
    parser.add_argument("--override-bin-dir", help="take rmac-screenshot from this folder")
    parser.add_argument("--niri", default="/usr/bin/niri")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--inner", help=argparse.SUPPRESS)
    parser.set_defaults(frame_only=False, full_too=False, dump_a11y=False)
    args = parser.parse_args()
    if args.inner:
        return inner(args)
    return outer(args)


if __name__ == "__main__":
    raise SystemExit(main())
