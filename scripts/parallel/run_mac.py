#!/usr/bin/env python3
"""Record parallel journeys in owned Mac windows and disposable /tmp files."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "behavior"))
import record_mac  # noqa: E402

import journey  # noqa: E402
import fixtures  # noqa: E402
import mac_capture  # noqa: E402

record_mac.APPS["terminal"] = {"process": "Terminal", "bundle": "com.apple.Terminal"}

LOCK = Path("/tmp/mac-gui.lock")


def lock_gui() -> None:
    # Match the coordinator's mkdir lock protocol exactly.
    while True:
        try:
            LOCK.mkdir()
            return
        except FileExistsError:
            time.sleep(10)


def bounds(process: str, fallback=None) -> tuple[int, int, int, int]:
    script = f'''function run() {{
      var p=Application("System Events").processes.byName({json.dumps(process)});
      var w=p.attributes.byName("AXFocusedWindow").value();
      var x=w.attributes.byName("AXPosition").value();
      var s=w.attributes.byName("AXSize").value();
      return [x[0],x[1],s[0],s[1]].map(Math.round).join(" ");
    }}'''
    try:
        x, y, w, h = map(int, record_mac.osascript(script, js=True).split())
    except (record_mac.Stop, ValueError):
        if fallback is not None:
            return fallback
        raise
    if w < 50 or h < 50:
        raise record_mac.Stop("focused window has invalid capture bounds")
    return x, y, w, h


def click_named(run: record_mac.MacRun, label: str) -> None:
    if run.app == "files":
        run.select(label, double=False)
        return
    run.check_target()
    script = f'''function run() {{
      var p=Application("System Events").processes.byName({json.dumps(run.process)});
      var w=p.attributes.byName("AXFocusedWindow").value();
      function A(e,n) {{ try {{ return e.attributes.byName(n).value(); }} catch(_) {{ return null; }} }}
      var hit=null;
      function walk(e,d) {{
        if(hit || d<0) return;
        if(A(e,"AXTitle")==={json.dumps(label)} || A(e,"AXDescription")==={json.dumps(label)} || A(e,"AXValue")==={json.dumps(label)}) {{ hit=e; return; }}
        var children=A(e,"AXChildren")||[];
        for(var i=0;i<children.length;i++) walk(children[i],d-1);
      }}
      walk(w,14);
      if(!hit) return "none";
      var p0=A(hit,"AXPosition"),s=A(hit,"AXSize"),wp=A(w,"AXPosition"),ws=A(w,"AXSize");
      if(!p0||!s||!wp||!ws||s[0]<2||s[1]<2) return "none";
      var x=p0[0]+s[0]/2,y=p0[1]+s[1]/2;
      if(x<wp[0]||x>wp[0]+ws[0]||y<wp[1]||y>wp[1]+ws[1]) return "none";
      return [x,y].join(" ");
    }}'''
    where = record_mac.osascript(script, js=True)
    if where == "none":
        raise record_mac.Stop(f"no named control {label!r} inside our window")
    x, y = map(float, where.split())
    run.check_target()
    subprocess.run([sys.executable, str(HERE.parent / "behavior/mac_click.py"), "left", str(x), str(y)],
                   check=True, timeout=10)


def verify_save_destination(run: record_mac.MacRun) -> None:
    """Require the Save sheet's Where control to name our sandbox."""
    run.check_target()
    script = f'''function run() {{
      var p=Application("System Events").processes.byName("TextEdit");
      var w=p.attributes.byName("AXFocusedWindow").value();
      function A(e,n) {{ try {{ return e.attributes.byName(n).value(); }} catch(_) {{ return null; }} }}
      var found=false;
      function walk(e,d) {{
        if(d<0) return;
        var role=A(e,"AXRole");
        if((role==="AXPopUpButton"||role==="AXButton"||role==="AXStaticText") &&
           [A(e,"AXTitle"),A(e,"AXValue"),A(e,"AXDescription")].some(
             function(v) {{ return v==="sandbox" || v==={json.dumps(str(run.sandbox))}; }})) found=true;
        var children=A(e,"AXChildren")||[];
        for(var i=0;i<children.length;i++) walk(children[i],d-1);
      }}
      walk(w,12);
      return found ? "safe" : "unknown";
    }}'''
    if record_mac.osascript(script, js=True) != "safe":
        raise record_mac.Stop("Save destination is not verified as this run's sandbox")


def click_menu(run: record_mac.MacRun, path: list[str]) -> None:
    if run.app != "text-editor":
        raise record_mac.Stop("Open Recent is allowed only for TextEdit")
    if path[:2] != ["File", "Open Recent"] or len(path) != 3:
        raise record_mac.Stop("unapproved Mac menu path")
    if not any(p.stem == path[2] for p in run.sandbox.iterdir() if p.is_file()):
        raise record_mac.Stop("recent document is not in the sandbox")
    raw = record_mac.observe_raw(run.process, [], run.baseline)
    if raw.get("frontmost") != run.process or not raw.get("running"):
        raise record_mac.Stop("TextEdit is not frontmost for Open Recent")
    menu = f'menu bar item {record_mac.as_string(path[0])} of menu bar 1'
    submenu = f'menu item {record_mac.as_string(path[1])} of menu 1 of {menu}'
    record_mac.osascript(
        f'tell application "System Events" to tell process {record_mac.as_string(run.process)} to '
        f'click menu item {record_mac.as_string(path[2])} of menu 1 of {submenu}')


def run_one(path: Path, output: Path) -> dict:
    data = journey.load(path)
    name = path.stem
    target = output / name
    target.mkdir(parents=True, exist_ok=True)
    result = {"journey": name, "platform": "mac", "steps": [], "status": "passed"}
    if data.get("mac") == "unsafe":
        result.update(status="skipped_unsafe", reason=data.get("mac_reason", "Mac action is unsafe"))
        (target / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        return result
    current = None
    pending = None
    last_bounds = None
    scratch = Path(tempfile.mkdtemp(prefix="parallel-mac-capture-"))
    try:
        for index, step in enumerate(data["steps"]):
            action = next(iter(journey.ACTIONS.intersection(step)))
            if action == "wait":
                time.sleep(float(step[action]))
                continue
            if action == "shot":
                if current is None:
                    raise record_mac.Stop("shot before an owned app launched")
                destination = target / f"{len(result['steps']):02d}-{step[action]}.png"
                if pending and pending.get("key") == "cmd-w":
                    image_name = None
                    point_size = None
                else:
                    current.check_target()
                    last_bounds = bounds(current.process, last_bounds)
                    point_size = last_bounds[2:]
                    subprocess.run(["screencapture", "-x", "-R" + ",".join(map(str, last_bounds)),
                                    str(destination)], check=True, timeout=10, capture_output=True)
                    image_name = destination.name
                result["steps"].append({"name": step[action], "image": image_name,
                                        "action": pending, "point_size": point_size,
                                        **(timing or {})})
                continue
            if action == "launch":
                if current is not None:
                    current.cleanup()
                app = step[action]
                scenario = {"app": app, "steps": [], "setup": data.get("setup", {}),
                            "launch": {"folder": step.get("path", "."), "file": step.get("file")}}
                current = record_mac.MacRun(f"parallel-{name}", scenario, 0.1)
                last_bounds = None
                # Do not reuse an owner's Settings or Calculator window.
                if app in {"settings", "calculator", "terminal"} and record_mac.running(record_mac.APPS[app]["process"]):
                    raise record_mac.Stop(f"{app} is already running; owner window must not be touched")
                current.setup()
                fixtures.prepare(current.sandbox, data.get("setup", {}))
                # A launch has no owned region before dispatch. Measure full screen,
                # then switch captures to the proven focused window.
                full = lambda p: mac_capture.capture(p)
                timing = journey.measure(full, current.launch, scratch, probe=lambda: mac_capture.fingerprint())
            else:
                if current is None:
                    raise record_mac.Stop("input before launch")
                if action == "key":
                    if current.app == "text-editor" and step[action] in {"return", "enter"}:
                        dialog = record_mac.observe_raw(current.process, ["dialog"], current.baseline).get("dialog") or {}
                        if dialog.get("present") and (not pending or pending.get("type") != "$SANDBOX"):
                            raise record_mac.Stop("refusing Return in a Save sheet outside a verified sandbox")
                    if current.app == "files" and step[action] == "cmd-z":
                        if not pending or pending.get("key") != "cmd-backspace" or (current.sandbox / "renamed.txt").exists():
                            raise record_mac.Stop("Finder undo is not proven to target this sandbox move")
                    operation = lambda: current.key(step[action])
                elif action == "type":
                    value = step[action].replace("$SANDBOX", str(current.sandbox))
                    if current.app == "terminal" and not (value == f"cd {current.sandbox}" or value == "ls" or re.fullmatch(r"echo [a-zA-Z0-9-]+", value)):
                        raise record_mac.Stop("Terminal command is outside the read-only allowlist")
                    operation = lambda: current.type_text(value)
                elif action == "click":
                    if current.app == "text-editor" and step[action] == "Save":
                        verify_save_destination(current)
                    operation = (lambda: current.click_key(1, 0)) if current.app == "calculator" and step[action] == "2nd" else (lambda: click_named(current, step[action]))
                elif action == "menu":
                    operation = lambda: click_menu(current, step[action])
                elif action == "drag_window":
                    raise record_mac.Stop("Mac window drag is intentionally Lulo-only")
                else:
                    raise record_mac.Stop(f"unsupported Mac action {action}")
                last_bounds = bounds(current.process, last_bounds)
                region = last_bounds
                timing = journey.measure(lambda p: mac_capture.capture(p, region), operation, scratch,
                                         probe=lambda: mac_capture.fingerprint(region))
            pending = {action: step[action], "index": index}
    except (record_mac.Stop, subprocess.SubprocessError, OSError, ValueError) as error:
        result.update(status="failed", error=str(error))
    finally:
        if current is not None:
            result["cleanup"] = current.cleanup()
        shutil.rmtree(scratch, ignore_errors=True)
    (target / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("journeys", nargs="*")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("run_mac.py requires macOS")
    if journey.ROOT in args.output.resolve().parents:
        parser.error("screenshots must be outside the repository")
    args.output.mkdir(parents=True, exist_ok=True)
    failures = 0
    lock_gui()
    try:
        for path in journey.paths(args.journeys):
            result = run_one(path, args.output)
            print(f"{result['status']} {path.stem}: {len(result['steps'])} shots", flush=True)
            failures += result["status"] == "failed"
    finally:
        LOCK.rmdir()
    return int(bool(failures))


if __name__ == "__main__":
    raise SystemExit(main())
