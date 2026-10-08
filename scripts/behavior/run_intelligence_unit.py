#!/usr/bin/env python3
"""Lulo Intelligence's caller check under the real systemd user unit.

ADR 0024. On 2026-10-08 the installed service refused every request with
"the caller cannot be checked": its unit's `PrivateTmp=yes` made the user
manager run it in its own user namespace, and from there the kernel does not
let it read its callers' `/proc/<pid>/exe`. Earlier tests started the service
directly and never saw it. This check runs the service the way the product
does:

- the unit file from `crates/rmac-session/units/rmac-intelligence.service`,
  unchanged, under a real `systemd --user` manager, D-Bus activated through
  the real activation file (`SystemdService=rmac-intelligence.service`);
- a Lulo caller (`rmac-intelligence-bench`, beside the service as in a
  development install) both unconfined, like System Settings, and in a
  transient unit with Spotlight's own sandbox (`PrivateTmp=yes`, which on
  Ubuntu puts it in a user namespace under the `unprivileged_userns`
  AppArmor profile);

and checks that both Lulo callers get an answer, that `gdbus` and a copy of
the Lulo caller outside the trusted paths are refused as "not a Lulo
program", and that putting `PrivateTmp=yes` back on the service (a drop-in)
brings the production failure back, so this check would have caught it.

It needs a user manager it owns: GitHub's runners, where the workflow
enables lingering for the runner user. It refuses to run anywhere else; the
reference laptop's user manager is the owner's live session.

    python3 scripts/behavior/run_intelligence_unit.py --bin-dir DIR
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

REPO = Path(__file__).resolve().parents[2]
UNIT = REPO / "crates" / "rmac-session" / "units" / "rmac-intelligence.service"
ACTIVATION = REPO / "crates" / "rmac-intelligence-service" / "install" / "org.rmac.Intelligence1.service.in"
SERVICE = "rmac-intelligence-service"
BENCH = "rmac-intelligence-bench"
UNIT_NAME = "rmac-intelligence.service"
QUERY = "turn on dark mode"
ANSWER = '{"intent":"appearance","mode":"dark"}'


def run(command: list[str], env: dict[str, str], check: bool = False,
        timeout: float = 60) -> subprocess.CompletedProcess:
    result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=timeout)
    if check and result.returncode != 0:
        raise RuntimeError(f"{' '.join(command)} exited {result.returncode}: "
                           f"{result.stdout}{result.stderr}")
    return result


def wait_for(path: Path, timeout: float) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if path.exists():
            return True
        time.sleep(0.2)
    return path.exists()


class Check:
    def __init__(self, bins: Path):
        self.bins = bins
        self.home = Path.home()
        uid = os.getuid()
        runtime = Path(f"/run/user/{uid}")
        self.env = dict(os.environ)
        self.env.update({
            "XDG_RUNTIME_DIR": str(runtime),
            "DBUS_SESSION_BUS_ADDRESS": f"unix:path={runtime}/bus",
        })
        self.libexec = self.home / ".local" / "libexec" / "rmac"
        self.units = self.home / ".config" / "systemd" / "user"
        self.dropin = self.units / f"{UNIT_NAME}.d" / "regression.conf"
        self.activation = self.home / ".local" / "share" / "dbus-1" / "services" / "org.rmac.Intelligence1.service"
        self.config = self.home / ".config" / "rmac" / "intelligence.json"
        self.facts: dict[str, object] = {}
        self.errors: list[str] = []

    def systemctl(self, *arguments: str, check: bool = True) -> subprocess.CompletedProcess:
        return run(["systemctl", "--user", *arguments], self.env, check=check)

    def install(self) -> None:
        """What scripts/linux/install-session-units.sh installs, unchanged."""
        self.libexec.mkdir(parents=True, exist_ok=True)
        for name in (SERVICE, BENCH):
            shutil.copy2(self.bins / name, self.libexec / name)
            (self.libexec / name).chmod(0o755)
        self.units.mkdir(parents=True, exist_ok=True)
        shutil.copy2(UNIT, self.units / UNIT_NAME)
        self.activation.parent.mkdir(parents=True, exist_ok=True)
        self.activation.write_text(ACTIVATION.read_text().replace(
            "@RMAC_INTELLIGENCE_EXEC@", str(self.libexec / SERVICE)))
        self.config.parent.mkdir(parents=True, exist_ok=True)
        self.config.write_text(json.dumps({"version": 1, "enabled": True}) + "\n")
        # CI has no model file: the deterministic stand-in, through the same
        # parser. The bus activates through systemd, so the manager's
        # environment is what the service sees.
        self.systemctl("set-environment", "RMAC_INTELLIGENCE_ENGINE=fixture",
                       "RMAC_INTELLIGENCE_IDLE_SECONDS=5")
        self.systemctl("daemon-reload")
        # The bus reads its service directories at start; make it look again.
        run(["gdbus", "call", "--session", "--dest", "org.freedesktop.DBus",
             "--object-path", "/org/freedesktop/DBus",
             "--method", "org.freedesktop.DBus.ReloadConfig"], self.env, check=True)

    def stop_service(self) -> None:
        self.systemctl("stop", UNIT_NAME, check=False)
        self.systemctl("reset-failed", UNIT_NAME, check=False)

    def bench(self, executable: Path, confined: bool) -> str:
        command = [str(executable), "service", "--runs", "1", QUERY]
        if confined:
            # Spotlight's own sandbox (crates/rmac-session/units/rmac-launcher.service).
            command = ["systemd-run", "--user", "--quiet", "--wait", "--pipe", "--collect",
                       "-p", "NoNewPrivileges=yes", "-p", "PrivateTmp=yes",
                       "-E", f"DBUS_SESSION_BUS_ADDRESS={self.env['DBUS_SESSION_BUS_ADDRESS']}",
                       "-E", f"XDG_CONFIG_HOME={self.home / '.config'}", *command]
        result = run(command, self.env, timeout=90)
        return (result.stdout + result.stderr).strip()

    def gdbus(self) -> str:
        result = run(["gdbus", "call", "--session", "--dest", "org.rmac.Intelligence1",
                      "--object-path", "/org/rmac/Intelligence1",
                      "--method", "org.rmac.Intelligence1.Run", "intent", QUERY],
                     self.env, timeout=60)
        return (result.stdout + result.stderr).strip()

    def confinement(self) -> dict[str, str]:
        """The namespace and AppArmor label a Spotlight-like unit gets."""
        result = run(["systemd-run", "--user", "--quiet", "--wait", "--pipe", "--collect",
                      "-p", "NoNewPrivileges=yes", "-p", "PrivateTmp=yes",
                      "/bin/sh", "-c", "readlink /proc/self/ns/user; cat /proc/self/attr/current"],
                     self.env, timeout=30)
        lines = result.stdout.strip().splitlines() + ["", ""]
        return {"user_namespace": lines[0], "label": lines[1].strip()}

    def service_facts(self) -> dict[str, object]:
        shown = self.systemctl("show", UNIT_NAME, "-p", "MainPID", "-p", "ActiveState",
                               "-p", "PrivateTmp", "-p", "NoNewPrivileges", "-p",
                               "RestrictAddressFamilies", check=False).stdout
        facts: dict[str, object] = dict(
            line.split("=", 1) for line in shown.strip().splitlines() if "=" in line)
        pid = facts.get("MainPID", "0")
        if pid not in {"", "0"}:
            try:
                facts["user_namespace"] = os.readlink(f"/proc/{pid}/ns/user")
                facts["label"] = Path(f"/proc/{pid}/attr/current").read_text().strip()
            except OSError as error:
                facts["proc_error"] = str(error)
        return facts

    def expect_answer(self, name: str, output: str) -> None:
        self.facts[name] = output.splitlines()[-2:] if output else []
        if ANSWER not in output:
            self.errors.append(f"{name}: no answer from the service: {output[-400:]!r}")

    def expect_refused(self, name: str, output: str, reason: str) -> None:
        self.facts[name] = output[-400:]
        if "org.rmac.Intelligence1.Error.Refused" not in output or reason not in output:
            self.errors.append(f"{name}: expected a refusal naming {reason!r}: {output[-400:]!r}")

    def run(self) -> dict:
        self.facts["own_user_namespace"] = os.readlink("/proc/self/ns/user")
        self.facts["restrict_unprivileged_userns"] = Path(
            "/proc/sys/kernel/apparmor_restrict_unprivileged_userns").read_text().strip() \
            if Path("/proc/sys/kernel/apparmor_restrict_unprivileged_userns").exists() else None
        self.facts["dbus"] = run(["dbus-daemon", "--version"], self.env).stdout.splitlines()[:1]
        self.install()
        self.facts["spotlight_like_confinement"] = self.confinement()
        bench = self.libexec / BENCH

        # 1. The unit as shipped: Lulo callers answered, others refused.
        self.stop_service()
        self.expect_answer("lulo_caller_unconfined", self.bench(bench, confined=False))
        self.facts["service"] = self.service_facts()
        self.expect_answer("lulo_caller_confined_like_spotlight", self.bench(bench, confined=True))
        self.expect_refused("gdbus_caller", self.gdbus(), "the caller is not a Lulo program")
        with tempfile.TemporaryDirectory(prefix="lulo-not-trusted-") as directory:
            impostor = Path(directory) / "rmac-launcher"
            shutil.copy2(bench, impostor)
            impostor.chmod(0o755)
            output = self.bench(impostor, confined=False)
            self.facts["impostor_caller"] = output[-300:]
            if ANSWER in output or "refused" not in output:
                self.errors.append(f"a copy outside the trusted paths was answered: {output!r}")
        service = self.facts["service"]
        if service.get("user_namespace") != self.facts["own_user_namespace"]:
            self.errors.append(f"the service ran in another user namespace: {service}")

        # 2. The production bug: PrivateTmp= back on the service.
        self.stop_service()
        self.dropin.parent.mkdir(parents=True, exist_ok=True)
        self.dropin.write_text("[Service]\nPrivateTmp=yes\n")
        self.systemctl("daemon-reload")
        try:
            output = self.bench(bench, confined=False)
            self.facts["with_private_tmp_lulo_caller"] = output[-300:]
            self.facts["with_private_tmp_service"] = self.service_facts()
            if ANSWER in output:
                self.errors.append("with PrivateTmp=yes the service still answered: "
                                   "this check no longer reproduces the 2026-10-08 bug")
            self.expect_refused("with_private_tmp_gdbus", self.gdbus(),
                                "the caller cannot be checked")
        finally:
            self.stop_service()
            self.dropin.unlink()
            self.dropin.parent.rmdir()
            self.systemctl("daemon-reload")
        journal = run(["journalctl", "--user", "-u", UNIT_NAME, "--no-pager", "-o", "cat", "-n", "20"],
                      self.env)
        self.facts["service_log"] = journal.stdout.strip().splitlines()[-8:]
        return {"facts": self.facts, "errors": self.errors}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--bin-dir", required=True)
    args = parser.parse_args()
    if os.environ.get("GITHUB_ACTIONS") != "true":
        print("run_intelligence_unit.py drives this user's systemd manager; it runs only "
              "on GitHub's runners (see docs/behavior-suite.md)", file=sys.stderr)
        return 2
    runtime = Path(f"/run/user/{os.getuid()}")
    if not wait_for(runtime / "bus", 60) or not wait_for(runtime / "systemd" / "private", 60):
        print(f"no user manager and session bus in {runtime}", file=sys.stderr)
        return 2
    check = Check(Path(args.bin_dir).resolve())
    try:
        result = check.run()
    except Exception as error:  # noqa: BLE001 - a crash must fail, never pass
        result = {"facts": check.facts, "errors": [f"{type(error).__name__}: {error}"]}
    status = "pass" if not result["errors"] else "fail"
    print(json.dumps({"scenario": "intelligence-unit", "status": status, **result}, indent=2),
          flush=True)
    return 0 if status == "pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
