#!/usr/bin/env python3
"""Exercise Notes crash recovery in a disposable nested Linux session.

    python3 scripts/behavior/run_private_notes_recovery.py --bin-dir DIR [--keep]

The runner starts a private D-Bus session and headless Sway compositor, then
launches only the supplied ``rmac-notes`` binary with a fresh HOME and XDG
directories. It creates a throwaway note, types a unique marker, waits for a
private recovery draft to appear, sends SIGKILL only to that child PID, and
relaunches Notes. The app is expected to offer recovery and restore the marker.
It never connects to a live desktop or reads an existing Notes library.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from typing import Any

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import run_lulo  # noqa: E402


class JourneyError(RuntimeError):
    pass


def private_environment(work: Path) -> dict[str, str]:
    env = run_lulo.isolated_environment(work)
    run_lulo.refuse_live_session(env)
    if not Path(env["HOME"]).is_relative_to(work):
        raise JourneyError("HOME is outside the disposable run directory")
    if not Path(env["XDG_DATA_HOME"]).is_relative_to(work):
        raise JourneyError("XDG_DATA_HOME is outside the disposable run directory")
    return env


def kill_owned_process(process: subprocess.Popen[Any]) -> None:
    """Kill only the Notes child created by this runner."""
    if process.poll() is None:
        process.kill()
        process.wait(timeout=8)


def stop_owned_process_group(process: subprocess.Popen[Any], grace: float = 5) -> None:
    """Stop the isolated D-Bus session and its compositor/app descendants."""
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    deadline = time.monotonic() + grace
    if process.poll() is None:
        try:
            process.wait(timeout=grace)
        except subprocess.TimeoutExpired:
            pass
    # The dbus-run-session leader can exit before a service it activated.
    # Waiting only for that leader would leave descendants running against a
    # runtime directory that outer() is about to remove.
    while time.monotonic() < deadline:
        try:
            os.killpg(process.pid, 0)
        except ProcessLookupError:
            break
        time.sleep(0.05)
    try:
        os.killpg(process.pid, 0)
    except ProcessLookupError:
        return
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    if process.poll() is None:
        process.wait(timeout=8)


def pump() -> None:
    from gi.repository import GLib

    context = GLib.MainContext.default()
    for _ in range(200):
        if not context.iteration(False):
            break


def descendants(node, limit: int = 3000):
    stack = [node]
    while stack and limit:
        current = stack.pop()
        if current is None:
            continue
        limit -= 1
        yield current
        try:
            stack.extend(current.getChildAtIndex(i) for i in range(current.childCount - 1, -1, -1))
        except Exception:
            continue


def app_for_pid(pid: int):
    import pyatspi

    pump()
    desktop = pyatspi.Registry.getDesktop(0)
    for index in range(desktop.childCount):
        try:
            app = desktop.getChildAtIndex(index)
            if app.get_process_id() == pid:
                return app
        except Exception:
            continue
    return None


def wait_for(predicate, label: str, timeout: float = 15.0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.1)
    raise JourneyError(f"timed out waiting for {label}")


def find_button(app, label: str):
    for node in descendants(app):
        try:
            if _role(node) in {"push button", "button"} and (node.name or "") == label:
                return node
        except Exception:
            continue
    return None


def click_named(app, label: str) -> None:
    node = find_button(app, label)
    if node is None:
        raise JourneyError(f"accessible button not found: {label}")
    actions = node.queryAction()
    for index in range(actions.nActions):
        if actions.getName(index).lower() in {"click", "press"}:
            if actions.doAction(index):
                return
    raise JourneyError(f"button has no working Click action: {label}")


def _role(node) -> str:
    try:
        return node.getRoleName()
    except Exception:
        return ""


def wait_for_app(nested: run_lulo.Nested, process: subprocess.Popen[Any]):
    def current_app():
        if process.poll() is not None:
            return None
        # Require a window from this exact child as well as its AT-SPI
        # application. That prevents a stale registry entry alone from
        # satisfying readiness after a restart.
        if not any(window.get("pid") == process.pid for window in nested.windows()):
            return None
        return app_for_pid(process.pid)

    return wait_for(current_app,
                    f"Notes window for PID {process.pid}")


def launch_notes(binary: Path, env: dict[str, str], log_path: Path) -> subprocess.Popen[Any]:
    log = open(log_path, "a", encoding="utf-8")
    try:
        return subprocess.Popen([str(binary)], env=env, stdin=subprocess.DEVNULL,
                                stdout=log, stderr=subprocess.STDOUT, close_fds=True)
    finally:
        log.close()


def body_node(app):
    # Source publishes this multiline editor as Role::MultilineTextInput and
    # aria-label "Body". The repo's live AT-SPI acceptance script confirms
    # it appears as an "entry" with a Text interface.
    for node in descendants(app):
        try:
            if _role(node) == "entry" and (node.name or "") == "Body":
                node.queryText()
                return node
        except Exception:
            continue
    return None


def body_text(app) -> str:
    node = body_node(app)
    if node is None:
        return ""
    return node.queryText().getText(0, -1)


def recovered_marker_visible(app, marker: str) -> bool:
    return marker in body_text(app)


def persisted_marker_visible(data_root: Path, marker: str) -> bool:
    expected = marker.encode("utf-8")
    return any(expected in path.read_bytes() for path in data_root.glob("drafts/*.draft"))


def inner(work: Path, binary: Path) -> int:
    nested = run_lulo.Nested(work)
    child = None
    try:
        env = dict(nested.env)
        home = work / "home"
        data_root = Path(env["XDG_DATA_HOME"]) / "rmac" / "notes"
        if data_root.exists() or Path(env["HOME"]).resolve() != home.resolve():
            raise JourneyError("private Notes data root is not fresh")
        marker = "RMAC_RECOVERY_PRIVATE_7F3C"

        child = launch_notes(binary, env, work / "notes.log")
        app = wait_for_app(nested, child)
        create = wait_for(lambda: find_button(app, "New Note"), "New Note control")
        if create is None:
            raise JourneyError("New Note control is unavailable")
        click_named(app, "New Note")
        body = wait_for(lambda: body_node(app), "body editor")
        if not body.queryComponent().grabFocus():
            raise JourneyError("could not focus the private note body")
        nested.input.type_text(marker, delay=0.01)
        wait_for(lambda: marker in body_text(app), "typed marker in the private editor")
        wait_for(lambda: persisted_marker_visible(data_root, marker),
                 "persisted private draft containing the typed marker", 20)
        # Inspect only this synthetic marker's presence. Draft bytes never
        # enter output or logs; the test's private library has no owner data.
        kill_owned_process(child)
        child = launch_notes(binary, env, work / "notes-relaunch.log")
        app = wait_for_app(nested, child)
        restore = wait_for(lambda: find_button(app, "Restore"), "recovery Restore action", 20)
        if restore is None:
            raise JourneyError("recovery review did not offer Restore")
        click_named(app, "Restore")
        wait_for(lambda: recovered_marker_visible(app, marker), "recovered private marker", 20)
        print(json.dumps({"format": 1, "status": "pass",
                          "checks": ["private_note_created", "draft_persisted_before_kill",
                                     "recovery_review_shown", "restored_body_matches_marker"]}, indent=2))
        return 0
    except Exception as error:
        print(json.dumps({"format": 1, "status": "fail",
                          "error": f"{type(error).__name__}: {error}"}, indent=2))
        return 1
    finally:
        if child is not None:
            if child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    kill_owned_process(child)
        nested.close()


def outer(binary: Path, keep: bool) -> int:
    for tool in ("sway", "swaymsg", "dbus-run-session"):
        if shutil.which(tool) is None:
            raise SystemExit(f"preflight: {tool} is required")
    for module in ("gi", "pyatspi"):
        if importlib.util.find_spec(module) is None:
            raise SystemExit(f"preflight: Python module {module} is required")
    work = Path(tempfile.mkdtemp(prefix="rmac-private-notes-recovery-"))
    try:
        env = private_environment(work)
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
            "<allow eavesdrop=\"true\"/><allow own=\"*\"/></policy></busconfig>\n",
            encoding="utf-8",
        )
        command = ["dbus-run-session", f"--config-file={config}", "--", sys.executable,
                   str(Path(__file__).resolve()), "--inner", str(work), "--binary", str(binary)]
        session = subprocess.Popen(command, env=env, close_fds=True, start_new_session=True)
        try:
            return session.wait()
        finally:
            # A parent SIGTERM must not leave dbus-run-session, Sway, or Notes
            # running while the disposable runtime and HOME are removed below.
            stop_owned_process_group(session)
    finally:
        if (work / "runtime").exists():
            run_lulo.reap(work / "runtime")
        if keep:
            print(f"kept private artifacts: {work}", file=sys.stderr)
        else:
            run_lulo.remove_tree(work)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, help="directory containing rmac-notes")
    parser.add_argument("--keep", action="store_true", help="retain the disposable HOME and logs")
    parser.add_argument("--inner", type=Path, help=argparse.SUPPRESS)
    parser.add_argument("--binary", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args(argv)
    if not sys.platform.startswith("linux"):
        parser.error("this runner requires Linux")
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    if args.inner:
        if args.binary is None or not args.binary.is_file():
            parser.error("inner run requires an existing Notes binary")
        return inner(args.inner, args.binary.resolve())
    if args.bin_dir is None:
        parser.error("--bin-dir is required")
    binary = (args.bin_dir / "rmac-notes").resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        parser.error(f"executable not found: {binary}")
    return outer(binary, args.keep)


if __name__ == "__main__":
    raise SystemExit(main())
