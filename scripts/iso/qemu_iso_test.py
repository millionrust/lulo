#!/usr/bin/env python3
"""Boot the Lulo OS ISO in QEMU and prove it works (ADR 0029).

Modes:

  live       Boot the ISO and wait until the Lulo session is up (the probe
             in the image reports it on the serial port). Checks that
             "Install Lulo OS" is in the Dock.
  install    Boot the ISO, open "Install Lulo OS" from the live session and
             drive Calamares through its real pages (screenshots + OCR):
             Erase disk, a user "Lulo CI", Install. Leaves an installed disk.
  installed  Boot that disk (no ISO), sign in at GDM, and wait for the Lulo
             session.

Firmware: "uefi-sb" (OVMF with Secure Boot enforced and Microsoft keys) or
"bios" (SeaBIOS).

Screenshots and serial logs go to --out. They are CI artifacts only.

    python3 scripts/iso/qemu_iso_test.py --mode live --firmware uefi-sb \
        --iso lulo-os-X-amd64.iso --out results/live-uefi
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path

OVMF_CODE = "/usr/share/OVMF/OVMF_CODE_4M.secboot.fd"
OVMF_VARS = "/usr/share/OVMF/OVMF_VARS_4M.ms.fd"
SCREEN = (1280, 800)
CI_USER_NAME = "Lulo CI"
CI_PASSWORD = "lulo-ci-2026"
MARKER = "LULO-CI: "


class TestFailure(RuntimeError):
    pass


# --- OCR ----------------------------------------------------------------------


@dataclass(frozen=True)
class Word:
    text: str
    left: int
    top: int
    width: int
    height: int

    @property
    def right(self) -> int:
        return self.left + self.width

    @property
    def middle_y(self) -> float:
        return self.top + self.height / 2


@dataclass(frozen=True)
class Match:
    text: str
    x: int
    y: int
    left: int
    top: int
    right: int
    bottom: int


def normalise(text: str) -> str:
    return re.sub(r"[^0-9a-z]+", "", text.lower())


def parse_tesseract_tsv(tsv: str, scale: float = 1.0) -> list[Word]:
    """Words from `tesseract ... tsv` output, in screen pixels."""
    words: list[Word] = []
    lines = tsv.splitlines()
    if not lines:
        return words
    header = lines[0].split("\t")
    index = {name: position for position, name in enumerate(header)}
    for line in lines[1:]:
        fields = line.split("\t")
        if len(fields) != len(header):
            continue
        text = fields[index["text"]].strip()
        if not text or not normalise(text):
            continue
        try:
            confidence = float(fields[index["conf"]])
        except ValueError:
            continue
        if confidence < 30:
            continue
        words.append(
            Word(
                text=text,
                left=round(int(fields[index["left"]]) / scale),
                top=round(int(fields[index["top"]]) / scale),
                width=round(int(fields[index["width"]]) / scale),
                height=round(int(fields[index["height"]]) / scale),
            )
        )
    return words


def cluster_lines(words: list[Word]) -> list[list[Word]]:
    """Group words into visual lines: similar height, close horizontally.

    Tesseract's own line ids split a UI into odd blocks (a button next to a
    label), so phrases are matched on geometry instead.
    """
    lines: list[list[Word]] = []
    for word in sorted(words, key=lambda item: (item.top, item.left)):
        placed = False
        for line in lines:
            last = line[-1]
            same_row = abs(last.middle_y - word.middle_y) <= max(last.height, word.height) * 0.6
            near = 0 <= word.left - last.right <= max(last.height, word.height) * 1.8
            if same_row and near:
                line.append(word)
                placed = True
                break
        if not placed:
            lines.append([word])
    for line in lines:
        line.sort(key=lambda item: item.left)
    return lines


def find_phrase(words: list[Word], phrase: str, exact_line: bool = False) -> list[Match]:
    """Every place the words of `phrase` appear in order on one line."""
    wanted = [normalise(part) for part in phrase.split() if normalise(part)]
    matches: list[Match] = []
    if not wanted:
        return matches
    for line in cluster_lines(words):
        texts = [normalise(word.text) for word in line]
        if exact_line and texts != wanted:
            continue
        for start in range(len(line) - len(wanted) + 1):
            if texts[start : start + len(wanted)] == wanted:
                chosen = line[start : start + len(wanted)]
                left = min(word.left for word in chosen)
                right = max(word.right for word in chosen)
                top = min(word.top for word in chosen)
                bottom = max(word.top + word.height for word in chosen)
                matches.append(
                    Match(
                        text=" ".join(word.text for word in chosen),
                        x=(left + right) // 2,
                        y=(top + bottom) // 2,
                        left=left,
                        top=top,
                        right=right,
                        bottom=bottom,
                    )
                )
    return matches


def ocr(png: Path) -> list[Word]:
    """OCR a screenshot at 2x (UI text is small) with tesseract."""
    from PIL import Image

    scale = 2.0
    big = png.with_suffix(".ocr.png")
    with Image.open(png) as image:
        image.convert("L").resize(
            (int(image.width * scale), int(image.height * scale)), Image.LANCZOS
        ).save(big)
    result = subprocess.run(
        ["tesseract", str(big), "-", "--psm", "11", "tsv"],
        check=True,
        capture_output=True,
        text=True,
    )
    big.unlink(missing_ok=True)
    return parse_tesseract_tsv(result.stdout, scale)


# --- QEMU -----------------------------------------------------------------------


QCODES = {
    " ": "spc",
    "-": "minus",
    "=": "equal",
    ".": "dot",
    ",": "comma",
    "/": "slash",
    ";": "semicolon",
    "'": "apostrophe",
    "\n": "ret",
    "\t": "tab",
}
SHIFTED = {"_": "minus", "+": "equal", "!": "1", "@": "2", "#": "3", "?": "slash", ":": "semicolon"}


def qcodes_for(char: str) -> list[str]:
    if char.isalpha() and char.isascii():
        return (["shift"] if char.isupper() else []) + [char.lower()]
    if char.isdigit():
        return [char]
    if char in QCODES:
        return [QCODES[char]]
    if char in SHIFTED:
        return ["shift", SHIFTED[char]]
    raise ValueError(f"no key for {char!r}")


class Qmp:
    def __init__(self, path: Path):
        deadline = time.monotonic() + 30
        while True:
            try:
                self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                self.sock.connect(str(path))
                break
            except OSError:
                if time.monotonic() > deadline:
                    raise
                time.sleep(0.5)
        self.file = self.sock.makefile("rwb")
        self._read()  # greeting
        self.command("qmp_capabilities")

    def _read(self) -> dict:
        line = self.file.readline()
        if not line:
            raise TestFailure("QEMU closed the QMP connection")
        return json.loads(line)

    def command(self, name: str, **arguments) -> dict:
        message = {"execute": name}
        if arguments:
            message["arguments"] = arguments
        self.file.write(json.dumps(message).encode() + b"\n")
        self.file.flush()
        while True:
            reply = self._read()
            if "event" in reply:
                continue
            if "error" in reply:
                raise TestFailure(f"QMP {name}: {reply['error']}")
            return reply.get("return", {})


class Machine:
    def __init__(self, args: argparse.Namespace, out: Path):
        self.args = args
        self.out = out
        self.qmp_path = out / "qmp.sock"
        self.serial = out / "serial0.log"
        self.log = out / "serial1.log"
        self.shots = 0
        self.serial_offset = 0
        self.markers: list[str] = []
        self.process: subprocess.Popen | None = None
        self.qmp: Qmp | None = None
        self.last_periodic = 0.0

    def command_line(self) -> list[str]:
        args = self.args
        command = ["qemu-system-x86_64", "-name", f"lulo-{args.mode}", "-nodefaults"]
        if Path("/dev/kvm").exists():
            command += ["-enable-kvm", "-cpu", "host"]
        command += ["-smp", "4", "-m", str(args.memory)]
        if args.firmware == "uefi-sb":
            vars_file = Path(args.vars)
            if not vars_file.exists():
                shutil.copy(OVMF_VARS, vars_file)
            command += [
                "-machine", "q35,smm=on",
                "-global", "driver=cfi.pflash01,property=secure,value=on",
                "-drive", f"if=pflash,format=raw,unit=0,file={OVMF_CODE},readonly=on",
                "-drive", f"if=pflash,format=raw,unit=1,file={vars_file}",
            ]
        else:
            command += ["-machine", "q35"]
        if args.mode in ("live", "install"):
            command += ["-drive", f"file={args.iso},media=cdrom,if=ide,readonly=on"]
        if args.mode in ("install", "installed"):
            command += ["-drive", f"file={args.disk},if=virtio,format=qcow2"]
        command += [
            *self.display_device(),
            "-device", "qemu-xhci",
            "-device", "usb-tablet",
            "-netdev", "user,id=net0",
            "-device", "virtio-net-pci,netdev=net0",
            "-chardev", f"file,id=ser0,path={self.serial}",
            "-chardev", f"file,id=ser1,path={self.log}",
            "-serial", "chardev:ser0",
            "-serial", "chardev:ser1",
            "-qmp", f"unix:{self.qmp_path},server=on,wait=off",
            "-fw_cfg", f"name=opt/org.lulo.ci,string={args.mode}",
            "-boot", "order=dc" if args.mode != "installed" else "order=c",
            "-no-reboot",
        ]
        return command

    def display_device(self) -> list[str]:
        """The guest GPU and QEMU's display for it.

        niri refuses software EGL renderers (llvmpipe), so a plain virtual
        framebuffer ("std", "virtio") gets no picture from the Lulo session.
        "virgl" gives the guest an accelerated virtio GPU, backed on the
        runner by Mesa's llvmpipe through QEMU's GTK display on Xvfb
        (DISPLAY must be set); screenshots then come from Xvfb.
        """
        size = f"xres={SCREEN[0]},yres={SCREEN[1]}"
        if self.args.display == "virgl":
            return ["-device", f"virtio-vga-gl,{size}", "-display", "gtk,gl=on,full-screen=on,show-menubar=off,zoom-to-fit=off"]
        if self.args.display == "std":
            return ["-device", f"VGA,{size},edid=on", "-display", "none"]
        return ["-device", f"virtio-vga,{size}", "-display", "none"]

    def start(self) -> None:
        command = self.command_line()
        (self.out / "qemu-command.txt").write_text(" ".join(command) + "\n")
        self.process = subprocess.Popen(command, stdout=open(self.out / "qemu.log", "w"), stderr=subprocess.STDOUT)
        self.qmp = Qmp(self.qmp_path)
        print(f"qemu started (pid {self.process.pid})", flush=True)

    def stop(self) -> None:
        if self.process and self.process.poll() is None:
            try:
                assert self.qmp
                self.qmp.command("quit")
            except Exception:
                self.process.kill()
            try:
                self.process.wait(timeout=30)
            except subprocess.TimeoutExpired:
                self.process.kill()

    def alive(self) -> bool:
        return self.process is not None and self.process.poll() is None

    # Screen

    def screenshot(self, name: str) -> Path:
        self.shots += 1
        path = self.out / f"{self.shots:03d}-{name}.png"
        if self.args.display == "virgl":
            subprocess.run(["import", "-window", "root", str(path)], check=False,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        else:
            assert self.qmp
            self.qmp.command("screendump", filename=str(path), format="png")
        for _ in range(20):
            if path.exists() and path.stat().st_size > 0:
                break
            time.sleep(0.1)
        return path

    def periodic(self) -> None:
        if time.monotonic() - self.last_periodic >= self.args.shot_interval:
            self.last_periodic = time.monotonic()
            self.screenshot("progress")

    def screen_words(self, name: str = "ocr") -> tuple[Path, list[Word]]:
        path = self.screenshot(name)
        return path, ocr(path)

    # Input

    def click(self, x: int, y: int) -> None:
        assert self.qmp
        position = [
            {"type": "abs", "data": {"axis": "x", "value": int(x * 32767 / (SCREEN[0] - 1))}},
            {"type": "abs", "data": {"axis": "y", "value": int(y * 32767 / (SCREEN[1] - 1))}},
        ]
        self.qmp.command("input-send-event", events=position)
        time.sleep(0.15)
        self.qmp.command("input-send-event", events=[{"type": "btn", "data": {"down": True, "button": "left"}}])
        time.sleep(0.08)
        self.qmp.command("input-send-event", events=[{"type": "btn", "data": {"down": False, "button": "left"}}])
        time.sleep(0.4)

    def key(self, *codes: str) -> None:
        assert self.qmp
        self.qmp.command("send-key", keys=[{"type": "qcode", "data": code} for code in codes])
        time.sleep(0.06)

    def type_text(self, text: str) -> None:
        for char in text:
            self.key(*qcodes_for(char))

    # Serial markers

    def poll_serial(self) -> None:
        if not self.serial.exists():
            return
        with open(self.serial, "rb") as handle:
            handle.seek(self.serial_offset)
            data = handle.read()
        if b"\n" not in data:
            return
        complete = data[: data.rindex(b"\n") + 1]
        self.serial_offset += len(complete)
        for raw in complete.decode(errors="replace").splitlines():
            if MARKER in raw:
                marker = raw[raw.index(MARKER) + len(MARKER) :].strip()
                self.markers.append(marker)
                print(f"[serial] {marker}", flush=True)

    def wait_marker(self, prefix: str, timeout: float, fail_on: tuple[str, ...] = ("session-timeout", "installer-missing")) -> str:
        deadline = time.monotonic() + timeout
        seen = 0
        while time.monotonic() < deadline:
            self.poll_serial()
            for marker in self.markers[seen:]:
                if marker.startswith(prefix):
                    return marker
                if marker.startswith(fail_on):
                    self.screenshot("failure")
                    raise TestFailure(f"probe reported: {marker}")
            seen = len(self.markers)
            if not self.alive():
                raise TestFailure(f"QEMU exited while waiting for {prefix!r}")
            self.periodic()
            time.sleep(2)
        self.screenshot("timeout")
        raise TestFailure(f"no {prefix!r} from the probe within {timeout:.0f}s")

    def wait_text(
        self,
        phrase: str,
        timeout: float,
        exact_line: bool = False,
        avoid: tuple[str, ...] = (),
        min_count: int = 1,
    ) -> Match:
        """Wait until `phrase` is on screen `min_count` times; returns the
        lowest match (buttons sit at the bottom, the step list on the left)."""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if not self.alive():
                raise TestFailure(f"QEMU exited while waiting for {phrase!r}")
            self.poll_serial()
            path, words = self.screen_words(normalise(phrase)[:24] or "ocr")
            for bad in avoid:
                if find_phrase(words, bad):
                    raise TestFailure(f"screen shows {bad!r} while waiting for {phrase!r} ({path.name})")
            matches = find_phrase(words, phrase, exact_line=exact_line)
            if len(matches) >= min_count:
                path.rename(path.with_name(path.stem + "-found.png"))
                return max(matches, key=lambda match: match.y)
            path.unlink(missing_ok=True)
            self.shots -= 1
            time.sleep(2)
        self.screenshot("missing-" + normalise(phrase)[:24])
        raise TestFailure(f"{phrase!r} did not appear within {timeout:.0f}s")

    def click_text(self, phrase: str, timeout: float = 120, **options) -> Match:
        match = self.wait_text(phrase, timeout, **options)
        print(f"click {phrase!r} at {match.x},{match.y}", flush=True)
        self.click(match.x, match.y)
        return match


def parse_marker(marker: str) -> dict[str, str]:
    return dict(part.split("=", 1) for part in marker.split() if "=" in part)


def expect(condition: bool, message: str) -> None:
    if not condition:
        raise TestFailure(message)


# --- Scenarios ------------------------------------------------------------------


def check_boot(machine: Machine, live: bool) -> None:
    boot = parse_marker(machine.wait_marker("boot ", timeout=600))
    firmware = "uefi" if machine.args.firmware == "uefi-sb" else "bios"
    expect(boot.get("firmware") == firmware, f"booted with firmware {boot.get('firmware')}, expected {firmware}")
    if firmware == "uefi":
        expect(boot.get("secureboot") == "1", "Secure Boot was not enabled in the guest")
    expect(boot.get("live") == ("1" if live else "0"), f"live={boot.get('live')}, expected {int(live)}")


def scenario_live(machine: Machine) -> None:
    check_boot(machine, live=True)
    session = parse_marker(machine.wait_marker("session-up", timeout=1000))
    time.sleep(8)  # let the Dock and the wallpaper settle for the screenshot
    machine.screenshot("live-session")
    expect(session.get("installer-entry") == "1", "the Install Lulo OS app is missing")
    expect(session.get("installer-pinned") == "1", "Install Lulo OS is not in the Dock")
    machine.wait_marker("done", timeout=120)


def scenario_install(machine: Machine) -> None:
    check_boot(machine, live=True)
    machine.wait_marker("session-up", timeout=1000)
    machine.wait_marker("installer-launched", timeout=180)
    failure = ("Installation Failed",)

    # Welcome, Location, Keyboard: the defaults are fine. The page labels are
    # only waited for briefly, so a reworded label costs time, not the run.
    machine.wait_text("Next", timeout=240)
    machine.screenshot("calamares-welcome")
    for label in ("Region", "Keyboard Model"):
        machine.click_text("Next", avoid=failure)
        try:
            machine.wait_text(label, timeout=60)
        except TestFailure as error:
            print(f"note: {error}", flush=True)
    machine.click_text("Next", avoid=failure)

    machine.click_text("Erase disk", timeout=180)
    time.sleep(2)
    machine.screenshot("partition-erase")
    machine.click_text("Next", avoid=failure)

    # Users: the fields' placeholders, else the questions above them.
    name_field = None
    for phrase, below in (("Your Full Name", 0), ("What is your name?", 32)):
        try:
            match = machine.wait_text(phrase, timeout=60 if not below else 20)
            name_field = (match.x, match.y + below)
            break
        except TestFailure:
            continue
    expect(name_field is not None, "the users page did not appear")
    machine.click(*name_field)
    machine.type_text(CI_USER_NAME)
    time.sleep(1)
    _, words = machine.screen_words("users-name")
    password = [match for match in find_phrase(words, "Password", exact_line=True)]
    if password:
        first = min(password, key=lambda match: match.left)
        machine.click(first.x, first.y)
    else:
        machine.key("tab")  # login name, filled in from the full name
        machine.key("tab")  # computer name, filled in too
        machine.key("tab")
    machine.type_text(CI_PASSWORD)
    machine.key("tab")
    machine.type_text(CI_PASSWORD)
    time.sleep(1)
    machine.screenshot("users-filled")
    machine.click_text("Next", avoid=failure)

    # The summary page: "Install" is both a step in the list and, now, the
    # button that replaced Next.
    machine.wait_text("Install", timeout=60, min_count=2)
    machine.screenshot("summary")
    machine.click_text("Install", avoid=failure, min_count=2)
    machine.click_text("Install now", timeout=60)

    started = time.monotonic()
    machine.wait_text("All done", timeout=machine.args.install_timeout, avoid=("Installation Failed",))
    print(f"install finished in {time.monotonic() - started:.0f}s", flush=True)
    machine.screenshot("install-done")
    time.sleep(5)


def scenario_installed(machine: Machine) -> None:
    check_boot(machine, live=False)
    machine.click_text(CI_USER_NAME, timeout=600)
    time.sleep(2)
    machine.type_text(CI_PASSWORD + "\n")
    session = parse_marker(machine.wait_marker("session-up", timeout=600))
    time.sleep(8)
    machine.screenshot("installed-session")
    expect(session.get("installer-entry") == "0", "the installed system still has Install Lulo OS")
    expect(session.get("user") == "lulo", f"signed in as {session.get('user')}, expected lulo")
    machine.wait_marker("done", timeout=120)


SCENARIOS = {"live": scenario_live, "install": scenario_install, "installed": scenario_installed}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--mode", choices=sorted(SCENARIOS), required=True)
    parser.add_argument("--firmware", choices=("uefi-sb", "bios"), default="uefi-sb")
    parser.add_argument("--iso", type=Path)
    parser.add_argument("--disk", type=Path, help="qcow2 disk for install/installed")
    parser.add_argument("--vars", default="", help="OVMF variables file (kept between install and installed)")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--display", choices=("virgl", "virtio", "std"), default="virgl")
    parser.add_argument("--memory", type=int, default=4096)
    parser.add_argument("--shot-interval", type=float, default=45)
    parser.add_argument("--install-timeout", type=float, default=3600)
    args = parser.parse_args()

    args.out.mkdir(parents=True, exist_ok=True)
    if not args.vars:
        args.vars = str(args.out / "OVMF_VARS.fd")
    if args.mode in ("live", "install") and not (args.iso and args.iso.exists()):
        parser.error("--iso is required")
    if args.mode in ("install", "installed") and not args.disk:
        parser.error("--disk is required")
    if args.mode == "install" and not args.disk.exists():
        subprocess.run(["qemu-img", "create", "-f", "qcow2", str(args.disk), "24G"], check=True)

    machine = Machine(args, args.out)
    started = time.monotonic()
    machine.start()
    result = 0
    try:
        SCENARIOS[args.mode](machine)
        print(f"PASS {args.mode} ({args.firmware}) in {time.monotonic() - started:.0f}s", flush=True)
    except TestFailure as error:
        print(f"FAIL {args.mode} ({args.firmware}): {error}", flush=True)
        result = 1
    finally:
        machine.poll_serial()
        (args.out / "markers.txt").write_text("\n".join(machine.markers) + "\n")
        machine.stop()
    return result


if __name__ == "__main__":
    os.umask(0o022)
    sys.exit(main())
