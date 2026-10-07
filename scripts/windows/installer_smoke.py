"""Install, verify, launch, uninstall and re-verify Lulo's Windows MSI.

Usage: python scripts/windows/installer_smoke.py <path-to-msi>

ADR 0023's "Installer" section promises a real, double-click installer, so
this is a round trip through what a person actually does, entirely silent
(`/quiet`) for CI:

1. `msiexec /i <msi> /quiet` -- a per-user install, no admin prompt.
2. Check that every app's exe landed under
   `%LOCALAPPDATA%\\Programs\\Lulo` and that its Start Menu shortcut exists
   under `%APPDATA%\\Microsoft\\Windows\\Start Menu\\Programs\\Lulo`.
3. Launch one app (rmac-calculator, the simplest) from its shortcut, the
   way a person would double-click it from Start, and check the process
   actually starts.
4. Seed a dummy `RmacClockAlarm-*` scheduled task (the real ones are only
   created when Clock actually schedules an alarm, which this script does
   not do) to prove the uninstall step that removes them really runs.
5. `msiexec /x <msi> /quiet` and check everything is gone: the install
   directory, the Start Menu shortcuts, every "Open with" registry entry
   this installer wrote, the Add/Remove Programs entry, and the scheduled
   task from step 4.

Exits non-zero (naming every failure) if any check fails.
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
import time
import winreg
from pathlib import Path

APPS = [
    {"id": "calculator", "bin": "rmac-calculator", "name": "Calculator"},
    {"id": "notes", "bin": "rmac-notes", "name": "Notes"},
    {"id": "text-editor", "bin": "rmac-text-editor", "name": "Text Editor"},
    {"id": "preview", "bin": "rmac-preview", "name": "Preview"},
    {"id": "clock", "bin": "rmac-clock", "name": "Clock"},
    {"id": "weather", "bin": "rmac-weather", "name": "Weather"},
    {"id": "terminal", "bin": "rmac-terminal", "name": "Terminal"},
]
ASSOCIATED_EXES = ("rmac-text-editor.exe", "rmac-preview.exe")
DUMMY_TASK = "RmacClockAlarm-installer-smoke-test"
LAUNCH_APP = "calculator"
PROCESS_TIMEOUT_SECONDS = 30.0
PROCESS_POLL_SECONDS = 0.5


def install_dir() -> Path:
    return Path(os.environ["LOCALAPPDATA"]) / "Programs" / "Lulo"


def start_menu_dir() -> Path:
    return Path(os.environ["APPDATA"]) / "Microsoft" / "Windows" / "Start Menu" / "Programs" / "Lulo"


def run(*args: str, check: bool = True) -> subprocess.CompletedProcess:
    print("+", " ".join(args))
    return subprocess.run(args, check=check, capture_output=True, text=True)


def msiexec(mode: str, msi: Path) -> None:
    log = msi.with_suffix(".install.log" if mode == "/i" else ".uninstall.log")
    result = run(
        "msiexec", mode, str(msi), "/quiet", "/norestart", "/l*v", str(log), check=False
    )
    if result.returncode != 0:
        sys.stderr.write(result.stdout + result.stderr)
        if log.is_file():
            # The interesting lines are usually near the end (the actual
            # failing action and its return code); the full log can be
            # megabytes of per-file progress noise.
            text = log.read_text(encoding="utf-16", errors="replace")
            sys.stderr.write("\n--- tail of " + str(log) + " ---\n")
            sys.stderr.write("\n".join(text.splitlines()[-200:]))
            sys.stderr.write("\n")
        raise SystemExit(f"msiexec {mode} {msi} failed: exit {result.returncode}")


def is_process_running(image_name: str) -> bool:
    result = run("tasklist", "/FI", f"IMAGENAME eq {image_name}", "/NH", check=False)
    return image_name.lower() in result.stdout.lower()


def kill_process(image_name: str) -> None:
    run("taskkill", "/IM", image_name, "/F", check=False)


def applications_key_exists(exe_name: str) -> bool:
    try:
        key = winreg.OpenKey(winreg.HKEY_CURRENT_USER, f"Software\\Classes\\Applications\\{exe_name}")
        winreg.CloseKey(key)
        return True
    except FileNotFoundError:
        return False


def arp_entry_names() -> list[str]:
    """Every Add/Remove Programs DisplayName under this user's own
    (per-user install) uninstall key."""
    names = []
    base = r"Software\Microsoft\Windows\CurrentVersion\Uninstall"
    try:
        root = winreg.OpenKey(winreg.HKEY_CURRENT_USER, base)
    except FileNotFoundError:
        return names
    with root:
        index = 0
        while True:
            try:
                subkey_name = winreg.EnumKey(root, index)
            except OSError:
                break
            index += 1
            try:
                with winreg.OpenKey(root, subkey_name) as subkey:
                    names.append(winreg.QueryValueEx(subkey, "DisplayName")[0])
            except (FileNotFoundError, OSError):
                continue
    return names


def scheduled_task_exists(name_pattern: str) -> bool:
    result = run("schtasks", "/query", "/fo", "csv", check=False)
    return any(name_pattern in line for line in result.stdout.splitlines())


def seed_dummy_clock_task() -> bool:
    result = run(
        "schtasks", "/create", "/tn", DUMMY_TASK, "/tr", "cmd.exe",
        "/sc", "once", "/st", "23:59", "/f", check=False,
    )
    return result.returncode == 0


def wait_until(predicate, timeout: float, poll: float) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return True
        time.sleep(poll)
    return predicate()


def check_installed(failures: list[str]) -> None:
    for app in APPS:
        exe = install_dir() / f"{app['bin']}.exe"
        if not exe.is_file():
            failures.append(f"missing installed exe: {exe}")
        shortcut = start_menu_dir() / f"{app['name']}.lnk"
        if not shortcut.is_file():
            failures.append(f"missing Start Menu shortcut: {shortcut}")
    for exe_name in ASSOCIATED_EXES:
        if not applications_key_exists(exe_name):
            failures.append(f"missing \"Open with\" registration for {exe_name}")
    if "Lulo" not in arp_entry_names():
        failures.append("no \"Lulo\" entry in Add/Remove Programs after install")


def check_launch(failures: list[str]) -> None:
    app = next(a for a in APPS if a["id"] == LAUNCH_APP)
    shortcut = start_menu_dir() / f"{app['name']}.lnk"
    image_name = f"{app['bin']}.exe"
    os.startfile(str(shortcut))  # noqa: S606 -- the whole point of this check
    started = wait_until(
        lambda: is_process_running(image_name), PROCESS_TIMEOUT_SECONDS, PROCESS_POLL_SECONDS
    )
    if not started:
        failures.append(f"{image_name} did not appear in the process list after launching its shortcut")
    kill_process(image_name)


def check_uninstalled(failures: list[str]) -> None:
    if install_dir().exists():
        failures.append(f"install directory still exists: {install_dir()}")
    if start_menu_dir().exists():
        failures.append(f"Start Menu folder still exists: {start_menu_dir()}")
    for exe_name in ASSOCIATED_EXES:
        if applications_key_exists(exe_name):
            failures.append(f"\"Open with\" registration for {exe_name} survived uninstall")
    if "Lulo" in arp_entry_names():
        failures.append("\"Lulo\" still listed in Add/Remove Programs after uninstall")
    if scheduled_task_exists("RmacClockAlarm-"):
        failures.append("a RmacClockAlarm-* scheduled task survived uninstall")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("msi", type=Path)
    args = parser.parse_args(argv)
    msi = args.msi.resolve()
    if not msi.is_file():
        raise SystemExit(f"no such file: {msi}")

    failures: list[str] = []

    print(f"installing {msi} silently")
    msiexec("/i", msi)
    check_installed(failures)

    print(f"launching {LAUNCH_APP} from its Start Menu shortcut")
    check_launch(failures)

    print(f"seeding a dummy {DUMMY_TASK!r} scheduled task")
    if not seed_dummy_clock_task():
        print(f"warning: could not create {DUMMY_TASK!r}; its removal cannot be checked", file=sys.stderr)

    print(f"uninstalling {msi} silently")
    msiexec("/x", msi)
    check_uninstalled(failures)

    if failures:
        print(f"\n{len(failures)} installer check(s) failed:", file=sys.stderr)
        for failure in failures:
            print(f"  - {failure}", file=sys.stderr)
        return 1

    print("\nall installer checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
