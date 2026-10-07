#!/usr/bin/env python3
"""Spotlight's "Lulo can do this" rows, end to end, in a private session.

ADR 0024 phase 1. A private headless Sway + nested niri shell (the parallel
journeys' session), a private session bus that can D-Bus-activate
`rmac-intelligence-service`, and private XDG directories. Two passes:

1. Lulo Intelligence **off** (the default): type "dark mode on" in
   Spotlight; no "Lulo can do this" row may appear, and the service must
   never start.
2. Turned **on**: the same query shows "Turn On Dark Mode — Lulo can do
   this"; the first Return only arms it ("Press Return again to confirm")
   and nothing changes; the second Return switches the appearance to Dark
   (the rmac theme store in the private config). Then the service exits by
   itself once idle: no process is left.

The model is stubbed (`RMAC_INTELLIGENCE_ENGINE=fixture`): CI has no model
file. The real model is measured separately on the reference laptop with
`rmac-intelligence-bench` (docs/decisions/0024-lulo-intelligence.md).

    python3 scripts/behavior/run_spotlight_intents.py --bin-dir DIR --niri NIRI
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

QUERY = "dark mode on"
ROW = "Turn On Dark Mode, Lulo can do this"
ARMED = "Turn On Dark Mode, Press Return again to confirm"
SERVICE = "rmac-intelligence-service"
# Short, so the scenario can watch the service leave; the product uses 60 s.
IDLE_SECONDS = 4


def theme_scheme(config_home: Path) -> str | None:
    """The saved Light/Dark/Auto choice (crates/rmac-theme StoredPreferences)."""
    path = config_home / "rmac" / "theme.json"
    try:
        return json.loads(path.read_text()).get("preferences", {}).get("color_scheme")
    except (OSError, ValueError, AttributeError):
        return None


def start_light(config_home: Path) -> None:
    """A fresh session is Dark (the Mac reference runs Dark); start Light so
    "Turn On Dark Mode" has something to change."""
    path = config_home / "rmac" / "theme.json"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps({"version": 1, "preferences": {"color_scheme": "light"}}) + "\n")


def set_enabled(config_home: Path, enabled: bool) -> None:
    path = config_home / "rmac" / "intelligence.json"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps({"version": 1, "enabled": enabled}) + "\n")


def service_pids(executable: Path) -> list[int]:
    """Processes running exactly this private copy of the service."""
    pids = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            if Path(os.readlink(entry / "exe")) == executable:
                pids.append(int(entry.name))
        except OSError:
            continue
    return pids


def wait_until(predicate, timeout: float, interval: float = 0.2):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(interval)
    return predicate()


class Scenario:
    def __init__(self, args, work: Path):
        self.args = args
        data = {"title": "Spotlight intents", "steps": []}
        self.driver = run_lulo_journey.Driver(args, work, data, work / "out")
        self.config_home = Path(self.driver.session.env["XDG_CONFIG_HOME"])
        self.service = Path(args.bin_dir) / SERVICE
        self.facts: dict[str, object] = {}

    def showing(self) -> set[str]:
        import pyatspi

        names = set()
        for node, _pid in self.driver.accessible():
            try:
                if node.name and node.getState().contains(pyatspi.STATE_SHOWING):
                    names.add(node.name)
            except Exception:  # noqa: BLE001
                continue
        return names

    def open_and_type(self) -> None:
        self.driver.action({"key": "cmd-space"})
        if not wait_until(lambda: "Spotlight Search" in self.showing(), 10):
            raise RuntimeError("Spotlight did not open")
        self.driver.session.pointer.type_text(QUERY)

    def close(self) -> None:
        self.driver.session.pointer.key("escape")
        time.sleep(0.3)
        if "Spotlight Search" in self.showing():
            self.driver.session.pointer.key("escape")
        wait_until(lambda: "Spotlight Search" not in self.showing(), 5)

    def run(self) -> dict:
        errors: list[str] = []
        start_light(self.config_home)
        self.driver.start()

        # 1. Off: no row, no service.
        set_enabled(self.config_home, False)
        self.open_and_type()
        # The launcher would ask 250 ms after typing stops; give it far more.
        time.sleep(3)
        rows = sorted(name for name in self.showing() if name.startswith("Turn On Dark Mode"))
        self.facts["off_rows"] = rows
        self.facts["off_service_started"] = bool(service_pids(self.service))
        if rows:
            errors.append(f"a Lulo Intelligence row appeared while it was off: {rows}")
        if self.facts["off_service_started"]:
            errors.append("the service started while Lulo Intelligence was off")
        self.close()

        # 2. On: the row, the confirmation, the change.
        set_enabled(self.config_home, True)
        before = theme_scheme(self.config_home)
        self.facts["scheme_before"] = before
        self.open_and_type()
        shown = wait_until(lambda: ROW in self.showing(), 15)
        self.facts["row_shown"] = bool(shown)
        if not shown:
            errors.append(f"no {ROW!r} row; showing: {sorted(self.showing())[:40]}")
        else:
            self.driver.session.pointer.key("return")
            armed = wait_until(lambda: ARMED in self.showing(), 5)
            self.facts["armed"] = bool(armed)
            time.sleep(0.5)
            self.facts["scheme_after_first_return"] = theme_scheme(self.config_home)
            if not armed:
                errors.append("the first Return did not ask for confirmation")
            if before != "light":
                errors.append(f"the session did not start Light ({before!r})")
            if theme_scheme(self.config_home) != before:
                errors.append("the appearance changed before confirmation")
            self.driver.session.pointer.key("return")
            dark = wait_until(lambda: theme_scheme(self.config_home) == "dark", 10)
            self.facts["scheme_after_confirm"] = theme_scheme(self.config_home)
            if not dark:
                errors.append("confirming did not switch the appearance to Dark")
            self.facts["spotlight_closed"] = bool(
                wait_until(lambda: "Spotlight Search" not in self.showing(), 5)
            )

        # 3. Idle: the service leaves by itself.
        self.facts["service_ran"] = bool(self.facts.get("row_shown"))
        gone = wait_until(lambda: not service_pids(self.service), IDLE_SECONDS + 10, 0.5)
        self.facts["service_exited_when_idle"] = bool(gone)
        if not gone:
            errors.append("the service was still running after its idle timeout")
        return {"facts": self.facts, "errors": errors}


def inner(args) -> int:
    work = Path(args.inner)
    scenario = Scenario(args, work)
    try:
        result = scenario.run()
    except Exception as error:  # noqa: BLE001 - a crash must fail, never pass
        result = {"facts": scenario.facts, "errors": [f"{type(error).__name__}: {error}"]}
    finally:
        session = scenario.driver.session
        if hasattr(session, "pointer"):
            session.finish()
        else:
            for process in reversed(session.children):
                if process.poll() is None:
                    process.terminate()
    status = "pass" if not result["errors"] else "fail"
    print(json.dumps({"scenario": "spotlight-intents", "status": status, **result}, indent=2),
          flush=True)
    return 0 if status == "pass" else 1


def outer(args) -> int:
    lock = open("/tmp/lulo-journey.lock", "w")
    fcntl.flock(lock, fcntl.LOCK_EX)
    work = Path(tempfile.mkdtemp(prefix="lulo-intents-"))
    try:
        env = run_lulo.isolated_environment(work)
        run_lulo.refuse_live_session(env)
        bins = Path(args.bin_dir).expanduser().resolve()
        links = work / "bins"
        links.mkdir()
        for source in bins.iterdir():
            if source.is_file() and source.name not in {"dock", "mission-control"}:
                # A copy of the service (not a link) so its /proc exe path is
                # unique to this run.
                if source.name == SERVICE:
                    target = links / source.name
                    target.write_bytes(source.read_bytes())
                    target.chmod(0o755)
                else:
                    (links / source.name).symlink_to(source)
        for alias, source in (("dock", "rmac-dock"), ("mission-control", "rmac-mission-control")):
            (links / alias).symlink_to(links / source)
        if not (links / SERVICE).is_file():
            raise SystemExit(f"{SERVICE} is not in {bins}")
        run_lulo.install_shortcut_dispatcher(env, links)
        config = run_lulo_journey.private_bus(work, env, links)
        services = work / "dbus-services"
        (services / "org.rmac.Intelligence1.service").write_text(
            "[D-BUS Service]\nName=org.rmac.Intelligence1\n" f"Exec={links / SERVICE}\n")
        # The bus passes its environment to what it activates.
        env["RMAC_INTELLIGENCE_ENGINE"] = "fixture"
        env["RMAC_INTELLIGENCE_IDLE_SECONDS"] = str(IDLE_SECONDS)
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
    parser.add_argument("--niri", default="/usr/bin/niri")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--inner", help=argparse.SUPPRESS)
    # run_lulo_journey.Driver reads these.
    parser.set_defaults(frame_only=False, full_too=False, dump_a11y=False, override_bin_dir=None)
    args = parser.parse_args()
    if args.inner:
        return inner(args)
    return outer(args)


if __name__ == "__main__":
    raise SystemExit(main())
