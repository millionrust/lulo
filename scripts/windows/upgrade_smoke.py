"""A newer Lulo MSI must replace an older one's files (WIN-OS-54).

Usage: python scripts/windows/upgrade_smoke.py <session-exe> <work-dir>

Every build used to carry ProductVersion 0.9.0, so installing a newer MSI
over an older one changed nothing (msiexec exited 0 and the old exes
stayed). This builds three MSIs with packaging/windows/build-installer.sh
from the same set of small stand-in exes, each with its own marker appended
to rmac-calculator.exe:

1. build 100 (MSI 0.9.100), marker "build-100";
2. build 101 (MSI 0.9.101), marker "build-101": a newer build;
3. build 101 again, marker "restamp-101": the same version re-stamped.

It installs them in that order (`msiexec /i /quiet`) and after each checks
that the installed rmac-calculator.exe ends with that MSI's marker, that
exactly one product with Lulo's UpgradeCode is installed, and that its
version is the one just installed. Then it uninstalls and checks the
install folder is gone. The stand-ins keep the MSIs small; lulo-session.exe
is the real one (`<session-exe>`), since install and uninstall run it.

It also checks that an install brings Lulo back (`lulo-session
--after-install`, run by the MSI once it has finished). The session records
each start under HKCU\\Software\\Lulo\\Session (`LastStarted`), so:

- the fresh install (build 100) starts it;
- the upgrade to build 101, after Lulo mode was turned off (`LuloMode` 0),
  does not;
- the re-stamped build, with Lulo mode on (`LuloMode` 1), starts it again.

And an upgrade keeps the user's Start Lulo at Sign-In value (the installer
used to delete it on every install), which uninstall removes.

Exits non-zero naming every failure.
"""

from __future__ import annotations

import json
import os
import time
import winreg
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
WINDOWS = ROOT / "packaging" / "windows"
UPGRADE_CODE = "{2C6E6B8E-9B0C-4E4D-9E7A-9B0B5B6E6A9C}"
MARKED = "rmac-calculator.exe"
STAND_IN = Path(os.environ.get("WINDIR", r"C:\Windows")) / "System32" / "whoami.exe"
BUILDS = [(100, "build-100"), (101, "build-101"), (101, "restamp-101")]
SESSION_KEY = r"Software\Lulo\Session"
RUN_KEY = r"Software\Microsoft\Windows\CurrentVersion\Run"
# The Lulo mode set before each install (None: Lulo never ran here), and
# whether that install must start Lulo.
LULO_MODE_BEFORE = {"build-100": None, "build-101": 0, "restamp-101": 1}
STARTS = {"build-100": True, "build-101": False, "restamp-101": True}
START_TIMEOUT_SECONDS = 30.0


def install_dir() -> Path:
    return Path(os.environ["LOCALAPPDATA"]) / "Programs" / "Lulo"


def run(args: list[str], check: bool = True) -> subprocess.CompletedProcess:
    print("+", " ".join(args), flush=True)
    return subprocess.run(args, check=check, capture_output=True, text=True)


def bash() -> str:
    git_bash = Path(os.environ.get("ProgramFiles", r"C:\Program Files")) / "Git" / "bin" / "bash.exe"
    return str(git_bash) if git_bash.is_file() else "bash"


def stage(exes: Path, session_exe: Path, marker: str) -> None:
    apps = json.loads((WINDOWS / "apps.json").read_text(encoding="utf-8"))
    exes.mkdir(parents=True, exist_ok=True)
    for app in apps:
        target = exes / f"{app['bin']}.exe"
        source = session_exe if app["bin"] == "lulo-session" else STAND_IN
        shutil.copyfile(source, target)
    with open(exes / MARKED, "ab") as marked:
        marked.write(marker.encode("ascii"))


def build(work: Path, session_exe: Path, number: int, marker: str) -> Path:
    exes = work / marker
    stage(exes, session_exe, marker)
    msi = work / f"Lulo-{marker}.msi"
    result = run(
        [
            bash(),
            (WINDOWS / "build-installer.sh").as_posix(),
            exes.as_posix(),
            "0.9.0-beta.1",
            msi.as_posix(),
            str(number),
        ],
        check=False,
    )
    sys.stdout.write(result.stdout + result.stderr)
    if result.returncode != 0:
        raise SystemExit(f"building the {marker} MSI failed: exit {result.returncode}")
    return msi


def msiexec(mode: str, msi: Path) -> int:
    log = msi.with_suffix(".install.log" if mode == "/i" else ".uninstall.log")
    result = run(["msiexec", mode, str(msi), "/quiet", "/norestart", "/l*v", str(log)], check=False)
    if result.returncode != 0 and log.is_file():
        text = log.read_text(encoding="utf-16", errors="replace")
        sys.stderr.write("\n".join(text.splitlines()[-80:]) + "\n")
    return result.returncode


def installed_products() -> list[tuple[str, str]]:
    """(product code, version) of every installed product with Lulo's
    UpgradeCode, from Windows Installer itself."""
    script = (
        "$i = New-Object -ComObject WindowsInstaller.Installer; "
        f"foreach ($p in $i.RelatedProducts('{UPGRADE_CODE}')) "
        "{ $p + ' ' + $i.ProductInfo($p, 'VersionString') }"
    )
    result = run(["powershell", "-NoProfile", "-Command", script], check=False)
    products = []
    for line in result.stdout.splitlines():
        parts = line.split()
        if len(parts) == 2:
            products.append((parts[0], parts[1]))
    return products


def read_value(path: str, name: str):
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, path) as key:
            return winreg.QueryValueEx(key, name)[0]
    except OSError:
        return None


def set_value(path: str, name: str, value, kind=winreg.REG_DWORD) -> None:
    with winreg.CreateKey(winreg.HKEY_CURRENT_USER, path) as key:
        winreg.SetValueEx(key, name, 0, kind, value)


def delete_value(path: str, name: str) -> None:
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, path, 0, winreg.KEY_SET_VALUE) as key:
            winreg.DeleteValue(key, name)
    except OSError:
        pass


def prepare_lulo_mode(mode) -> None:
    """Lulo mode as the user left it, and no start recorded yet."""
    delete_value(SESSION_KEY, "LastStarted")
    if mode is None:
        delete_value(SESSION_KEY, "LuloMode")
    else:
        set_value(SESSION_KEY, "LuloMode", mode)


def session_started(expected: bool) -> bool:
    """Whether lulo-session recorded a start: waited for when one is
    expected (the MSI does not wait for it), watched for a while when not."""
    deadline = time.monotonic() + (START_TIMEOUT_SECONDS if expected else 10.0)
    while time.monotonic() < deadline:
        if read_value(SESSION_KEY, "LastStarted") is not None:
            return True
        time.sleep(0.5)
    return read_value(SESSION_KEY, "LastStarted") is not None


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        sys.stderr.write(__doc__ or "")
        return 2
    session_exe = Path(argv[0]).resolve()
    work = Path(argv[1]).resolve()
    if not session_exe.is_file():
        raise SystemExit(f"no such file: {session_exe}")
    work.mkdir(parents=True, exist_ok=True)
    failures: list[str] = []
    if installed_products():
        failures.append(f"a Lulo product was installed before the test: {installed_products()}")

    msis = [(number, marker, build(work, session_exe, number, marker)) for number, marker in BUILDS]
    for number, marker, msi in msis:
        prepare_lulo_mode(LULO_MODE_BEFORE[marker])
        if marker == "build-101":
            # The user's Start Lulo at Sign-In, which an upgrade must keep.
            session = install_dir() / "lulo-session.exe"
            set_value(RUN_KEY, "Lulo", f'"{session}"', winreg.REG_SZ)
        code = msiexec("/i", msi)
        if code != 0:
            failures.append(f"installing {msi.name} failed: exit {code}")
            continue
        installed = install_dir() / MARKED
        tail = installed.read_bytes()[-len(marker) :] if installed.is_file() else b""
        if tail != marker.encode("ascii"):
            failures.append(
                f"after {msi.name}, the installed {MARKED} is not that MSI's (ends {tail!r})"
            )
        products = installed_products()
        versions = [version for _, version in products]
        if versions != [f"0.9.{number}"]:
            failures.append(f"after {msi.name}, installed Lulo products are {products}")
        print(f"{msi.name}: {MARKED} ends {tail!r}; products {products}", flush=True)
        started = session_started(STARTS[marker])
        if started != STARTS[marker]:
            failures.append(
                f"after {msi.name} with Lulo mode {LULO_MODE_BEFORE[marker]}, Lulo "
                f"{'started' if started else 'did not start'}"
            )
        print(f"{msi.name}: Lulo {'started' if started else 'not started'}", flush=True)
        if marker != "build-100" and read_value(RUN_KEY, "Lulo") is None:
            failures.append(f"{msi.name} removed the user's Start Lulo at Sign-In value")

    code = msiexec("/x", msis[-1][2])
    if code != 0:
        failures.append(f"uninstalling failed: exit {code}")
    if install_dir().exists():
        failures.append(f"install directory still exists after uninstall: {install_dir()}")
    if installed_products():
        failures.append(f"Lulo products left after uninstall: {installed_products()}")
    if read_value(RUN_KEY, "Lulo") is not None:
        failures.append("Start Lulo at Sign-In survived uninstall")
        delete_value(RUN_KEY, "Lulo")
    if read_value(SESSION_KEY, "LuloMode") is not None:
        failures.append("Lulo mode survived uninstall")

    if failures:
        print(f"\n{len(failures)} upgrade check(s) failed:", file=sys.stderr)
        for failure in failures:
            print(f"  - {failure}", file=sys.stderr)
        return 1
    print(
        "\nupgrade checks passed: each newer or re-stamped MSI replaced the installed files "
        "and brought Lulo back unless it had been turned off"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
