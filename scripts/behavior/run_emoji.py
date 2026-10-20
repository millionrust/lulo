#!/usr/bin/env python3
"""Emoji & Symbols (⌃⌘Space), end to end, in a private nested session.

A headless Sway with a nested niri that binds ⌃⌘Space to `rmac-emoji`
exactly as `packaging/rmac-session/shell.kdl` does (the journey Driver points
every `/usr/libexec/rmac/<name>` bind at --bin-dir), with the Text Editor
focused:

1. ⌃⌘Space opens the panel as the layer surface `rmac-emoji`.
2. Typing "tada" searches; Return inserts the first result into the Text
   Editor's document.
3. The choice shows under Frequently Used the next time.
4. ⌃⌘Space again, with the panel open, closes it.
5. Escape closes it and inserts nothing.

    python3 scripts/behavior/run_emoji.py --bin-dir DIR --niri NIRI
"""

from __future__ import annotations

import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / "parallel"))
import run_lulo  # noqa: E402
import run_lulo_journey  # noqa: E402

EMOJI = "rmac-emoji"
NAMESPACE = "rmac-emoji"
PARTY = "🎉"


def wait_until(predicate, timeout: float, interval: float = 0.1):
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
            args, work, {"title": "Emoji & Symbols", "steps": []}, work / "out")
        (work / "out").mkdir(exist_ok=True)
        self.facts: dict[str, object] = {}
        self.errors: list[str] = []
        self.state_home = Path(self.driver.session.env["XDG_STATE_HOME"])

    # ---- helpers ---------------------------------------------------------

    def layers(self) -> list[dict]:
        result = subprocess.run(
            [self.args.niri, "msg", "--json", "layers"], env=self.driver.session.env,
            capture_output=True, text=True, timeout=10, check=False)
        try:
            return json.loads(result.stdout)
        except ValueError:
            return []

    def panel_open(self) -> bool:
        return any(layer.get("namespace") == NAMESPACE for layer in self.layers())

    def document_text(self) -> str:
        pid = self.driver.apps["text-editor"].pid
        found = ""
        for node, _ in self.driver.accessible(pid):
            try:
                text = self.driver.text_of(node)
            except Exception:  # noqa: BLE001
                continue
            if PARTY in text:
                return text
            found = found or text
        return found

    def recents(self) -> str:
        path = self.state_home / "rmac" / "emoji-recents"
        return path.read_text() if path.exists() else ""

    def snapshot(self, name: str) -> None:
        try:
            self.driver.capture(self.work / "out" / f"{name}.png", full=True)
        except Exception as error:  # noqa: BLE001
            self.facts[f"{name}_snapshot_error"] = str(error)

    # ---- scenario --------------------------------------------------------

    def run(self) -> dict:
        self.driver.start()
        key = self.driver.session.pointer.key
        self.driver.launch({"launch": "text-editor"})
        wait_until(lambda: (self.driver.window() or {}).get("is_focused"), 15)
        time.sleep(1.0)

        key("ctrl-cmd-space")
        opened = wait_until(self.panel_open, 10)
        self.facts["opens"] = bool(opened)
        if not opened:
            self.errors.append("⌃⌘Space did not open the Emoji & Symbols panel")
            return {"facts": self.facts, "errors": self.errors}
        time.sleep(0.8)
        self.snapshot("panel-open")

        self.driver.session.pointer.type_text("tada")
        time.sleep(0.6)
        self.snapshot("panel-search")
        key("return")
        closed = wait_until(lambda: not self.panel_open(), 10)
        self.facts["closes_on_choice"] = bool(closed)
        if not closed:
            self.errors.append("choosing an emoji left the panel open")
        inserted = wait_until(lambda: PARTY in self.document_text(), 10)
        self.facts["inserted"] = bool(inserted)
        if not inserted:
            self.errors.append(f"the emoji never reached the document: {self.document_text()!r}")
        recents = self.recents()
        self.facts["recents"] = recents.strip()
        if PARTY not in recents:
            self.errors.append("the choice was not counted under Frequently Used")

        # The shortcut toggles the panel.
        key("ctrl-cmd-space")
        reopened = wait_until(self.panel_open, 10)
        time.sleep(0.8)
        self.snapshot("panel-recents")
        key("ctrl-cmd-space")
        toggled = wait_until(lambda: not self.panel_open(), 10)
        self.facts["reopens"] = bool(reopened)
        self.facts["toggle_closes"] = bool(toggled)
        if not reopened:
            self.errors.append("the panel did not open a second time")
        if not toggled:
            self.errors.append("pressing ⌃⌘Space again did not close the panel")

        # Escape closes it and inserts nothing.
        before = self.document_text()
        key("ctrl-cmd-space")
        wait_until(self.panel_open, 10)
        time.sleep(0.5)
        key("escape")
        escaped = wait_until(lambda: not self.panel_open(), 10)
        self.facts["escape_closes"] = bool(escaped)
        if not escaped:
            self.errors.append("Escape did not close the panel")
        time.sleep(0.5)
        if self.document_text() != before:
            self.errors.append("Escape changed the document")
        focused = (self.driver.window() or {}).get("is_focused")
        self.facts["focus_returned"] = bool(focused)
        if not focused:
            self.errors.append("the Text Editor did not get the keyboard back")
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
    except Exception as error:  # noqa: BLE001 - a crash must fail, never pass
        result = {"facts": scenario.facts,
                  "errors": scenario.errors + [f"{type(error).__name__}: {error}"]}
    finally:
        scenario.finish()
    status = "pass" if not result["errors"] else "fail"
    print(json.dumps({"scenario": "emoji", "status": status, **result}, indent=2, default=str),
          flush=True)
    if args.keep_shots:
        shutil.copytree(work / "out", args.keep_shots, dirs_exist_ok=True)
    return 0 if status == "pass" else 1


def outer(args) -> int:
    lock = open("/tmp/lulo-journey.lock", "w")
    fcntl.flock(lock, fcntl.LOCK_EX)
    work = Path(tempfile.mkdtemp(prefix="lulo-emoji-"))
    try:
        env = run_lulo.isolated_environment(work)
        run_lulo.refuse_live_session(env)
        bins = Path(args.bin_dir).expanduser().resolve()
        links = work / "bins"
        links.mkdir()
        sources = {source.name: source for source in bins.iterdir()}
        if args.override_bin_dir:
            overrides = Path(args.override_bin_dir).expanduser().resolve()
            for source in overrides.iterdir():
                sources[source.name] = source
        for source in sources.values():
            if source.is_file() and source.name not in {"dock", "mission-control"}:
                (links / source.name).symlink_to(source)
        for alias, source in (("dock", "rmac-dock"), ("mission-control", "rmac-mission-control")):
            if (links / source).exists():
                (links / alias).symlink_to(links / source)
        if not (links / EMOJI).is_file():
            raise SystemExit(f"{EMOJI} is not in {bins} or the override directory")
        run_lulo.install_shortcut_dispatcher(env, links)
        config = run_lulo_journey.private_bus(work, env, links)
        command = ["dbus-run-session", f"--config-file={config}", "--", sys.executable,
                   str(Path(__file__).resolve()), "--inner", str(work), "--bin-dir", str(links),
                   "--niri", args.niri]
        if args.keep_shots:
            command += ["--keep-shots", args.keep_shots]
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
    parser.add_argument("--niri", default="/usr/bin/niri")
    parser.add_argument("--override-bin-dir",
                        help="take rmac-emoji (and anything else here) from this folder")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--keep-shots", help="copy the screenshots to this folder")
    parser.add_argument("--inner", help=argparse.SUPPRESS)
    parser.set_defaults(frame_only=False, full_too=False, dump_a11y=False)
    args = parser.parse_args()
    if args.inner:
        return inner(args)
    return outer(args)


if __name__ == "__main__":
    raise SystemExit(main())
