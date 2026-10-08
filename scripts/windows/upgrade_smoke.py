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
is the real one (`<session-exe>`), since uninstall runs it.

Exits non-zero naming every failure.
"""

from __future__ import annotations

import json
import os
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

    code = msiexec("/x", msis[-1][2])
    if code != 0:
        failures.append(f"uninstalling failed: exit {code}")
    if install_dir().exists():
        failures.append(f"install directory still exists after uninstall: {install_dir()}")
    if installed_products():
        failures.append(f"Lulo products left after uninstall: {installed_products()}")

    if failures:
        print(f"\n{len(failures)} upgrade check(s) failed:", file=sys.stderr)
        for failure in failures:
            print(f"  - {failure}", file=sys.stderr)
        return 1
    print("\nupgrade checks passed: each newer or re-stamped MSI replaced the installed files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
