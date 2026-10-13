#!/usr/bin/env python3
"""Talk to Lulo (hold ⌥Space), end to end, in a private session.

ADR 0024 §8, Voice slice 1. A private headless Sway + nested niri shell (the
parallel journeys' session), a private session bus that D-Bus-activates
`rmac-voice-service` and `rmac-intelligence-service`, and private XDG
directories. There is no microphone: the voice service plays a recorded WAV
(`RMAC_VOICE_CAPTURE_FILE`) in real time instead, and both models are the
fixture engines (`RMAC_VOICE_ENGINE=fixture` hears the text beside the WAV;
`RMAC_INTELLIGENCE_ENGINE=fixture`): CI has no model files.

The nested niri binds Alt+Space to `rmac-shortcut-dispatch voice`, as the
session's fallback bindings do. A real session's dispatcher starts the
talk-key HUD through its systemd unit; the nested session has no user
manager, so this script starts `rmac-voice-hud` itself just before each
press, and the press reaches it through the same dispatch socket.

1. **Off**: holding ⌥Space shows "Turn on Talk to Lulo…", the voice service
   never starts and Spotlight never opens.
2. **On, held**: hold ⌥Space → the pill says "Listening…" → let go → it
   says "Working…" → Spotlight opens with "turn on dark mode" typed and the
   row "Turn On Dark Mode, Lulo can do this" → the first Return only asks
   for confirmation, the appearance is still Light → the second Return
   switches it to Dark.
3. **Escape** while holding: nothing reaches Spotlight.
4. **Unsupported**: "what's the weather like" opens Spotlight with the
   transcript as an ordinary search, and no Lulo row.
5. **Tap**: a quick press listens hands-free and ends after the pause.
6. Both services exit by themselves once idle.

    python3 scripts/behavior/run_voice.py --bin-dir DIR --niri NIRI
"""

from __future__ import annotations

import argparse
import fcntl
import json
import math
import os
from pathlib import Path
import random
import shutil
import struct
import subprocess
import sys
import tempfile
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / "parallel"))
import run_lulo  # noqa: E402
import run_lulo_journey  # noqa: E402
import run_spotlight_intents as intents  # noqa: E402
import run_window_move  # noqa: E402

VOICE = "rmac-voice-service"
INTELLIGENCE = "rmac-intelligence-service"
HUD = "rmac-voice-hud"
IDLE_SECONDS = 3
SPEECH = "turn on dark mode"
UNSUPPORTED = "what's the weather like"
ROW = "Turn On Dark Mode, Lulo can do this"
ARMED = "Turn On Dark Mode, Press Return again to confirm"
OFF_MESSAGE = "Turn on Talk to Lulo in System Settings ▸ Lulo Intelligence."
BINDING = ('    Alt+Space repeat=false hotkey-overlay-title="Talk to Lulo" '
           '{ spawn "/usr/libexec/rmac/rmac-shortcut-dispatch" "voice"; }\n')


def write_clip(path: Path) -> None:
    """A stand-in for speech: 0.3 s of near silence, 1.4 s of loud,
    syllable-shaped noise, 0.2 s of quiet. The fixture engine "hears" the
    sidecar text whenever the audio is not silent; the voice service's
    hands-free detector sees speech and then a pause."""

    rate = 16_000
    rng = random.Random(7)
    samples = []
    for index in range(int(rate * 1.9)):
        seconds = index / rate
        if 0.3 <= seconds < 1.7:
            envelope = 0.25 * (0.6 + 0.4 * math.sin(2 * math.pi * 4 * seconds) ** 2)
            value = envelope * (math.sin(2 * math.pi * 180 * seconds) + rng.uniform(-0.4, 0.4))
        else:
            value = rng.uniform(-0.002, 0.002)
        samples.append(max(-1.0, min(1.0, value)))
    data = b"".join(struct.pack("<h", int(sample * 32767)) for sample in samples)
    header = b"RIFF" + struct.pack("<I", 36 + len(data)) + b"WAVEfmt " + struct.pack(
        "<IHHIIHH", 16, 1, 1, rate, rate * 2, 2, 16) + b"data" + struct.pack("<I", len(data))
    path.write_bytes(header + data)


def set_voice(config_home: Path, enabled: bool, voice: bool) -> None:
    path = config_home / "rmac" / "intelligence.json"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps({"version": 1, "enabled": enabled, "voice": voice}) + "\n")


class Scenario(intents.BaseScenario):
    def __init__(self, args, work: Path):
        super().__init__(args, work)
        self.driver.data["title"] = "Talk to Lulo"
        self.errors: list[str] = []
        self.clip = Path(os.environ["RMAC_VOICE_CAPTURE_FILE"])
        self.voice = (Path(args.bin_dir) / VOICE).resolve()
        self.hud_trace = work / "hud-frame-trace.csv"
        self.launcher_trace = work / "launcher-frame-trace.csv"
        self.hud = None

    # ---- helpers ---------------------------------------------------------

    def say(self, text: str) -> None:
        """What the fixture voice engine will hear next."""
        self.clip.with_suffix(".txt").write_text(text + "\n")

    def start_hud(self) -> None:
        """Start the talk-key HUD the way its unit would, and wait for its
        dispatch socket."""
        socket = self.driver.session.runtime / "rmac" / "shortcut-voice.sock"
        self.hud = self.driver.session.spawn([str(Path(self.args.bin_dir) / HUD)], HUD)
        if not intents.wait_until(socket.exists, 10, 0.1):
            log = self.driver.session.logs / f"{HUD}.log"
            detail = log.read_text(errors="replace")[-500:] if log.exists() else ""
            raise RuntimeError(f"{HUD} never bound its dispatch socket: {detail}")

    def hud_gone(self) -> bool:
        return self.hud is None or self.hud.poll() is not None

    def hud_names(self) -> set[str]:
        import pyatspi

        names = set()
        if self.hud_gone():
            return names
        for node, _pid in self.driver.accessible(self.hud.pid):
            try:
                if node.name and node.getState().contains(pyatspi.STATE_SHOWING):
                    names.add(node.name)
            except Exception:  # noqa: BLE001
                continue
        return names

    def marks(self, after: float) -> list[str]:
        watcher = intents.TraceWatcher(self.hud_trace)
        watcher.wait_for("voice_", after, 0)
        return [event for moment, event in watcher.events if moment >= after]

    def hold(self) -> None:
        self.driver.session.pointer.hold("alt-space")

    def release(self) -> None:
        self.driver.session.pointer.release("alt-space")

    def spotlight_open(self) -> bool:
        return "Spotlight Search" in self.showing()

    def spotlight_query(self) -> str:
        """The text in Spotlight's search field (its accessible text; the
        field's name stays "Spotlight Search")."""
        import pyatspi

        for node, _pid in self.driver.accessible():
            try:
                if not node.getState().contains(pyatspi.STATE_SHOWING):
                    continue
                if node.getRoleName() in {"entry", "text", "search entry"}:
                    text = self.driver.text_of(node)
                    if text and text != "Spotlight Search":
                        return text
            except Exception:  # noqa: BLE001
                continue
        return ""

    # ---- the scenario ----------------------------------------------------

    def off(self) -> None:
        set_voice(self.config_home, True, False)
        self.say(SPEECH)
        self.start_hud()
        pressed = time.monotonic()
        self.hold()
        shown = intents.wait_until(lambda: OFF_MESSAGE in self.hud_names(), 8, 0.2)
        self.release()
        self.facts["off_message"] = bool(shown)
        if not shown:
            self.errors.append(f"no 'turn on' message while Talk to Lulo was off: {sorted(self.hud_names())}")
        intents.wait_until(self.hud_gone, 8, 0.2)
        self.facts["off_hud_exited"] = self.hud_gone()
        self.facts["off_marks"] = self.marks(pressed)
        self.facts["off_voice_started"] = bool(intents.service_pids(self.voice))
        if self.facts["off_voice_started"]:
            self.errors.append("the voice service started while Talk to Lulo was off")
        if self.spotlight_open():
            self.errors.append("Spotlight opened while Talk to Lulo was off")
            self.close()

    def held_request(self) -> None:
        set_voice(self.config_home, True, True)
        before = intents.theme_scheme(self.config_home)
        self.facts["scheme_before"] = before
        self.say(SPEECH)
        self.start_hud()
        pressed = time.monotonic()
        self.hold()
        listening = intents.wait_until(lambda: "Listening…" in self.hud_names(), 8, 0.1)
        self.facts["listening_shown_s"] = round(time.monotonic() - pressed, 2) if listening else None
        if not listening:
            self.errors.append(f"the pill never said Listening…: {sorted(self.hud_names())}")
        # Hold for the whole clip, as someone speaking would.
        time.sleep(max(0.0, 2.2 - (time.monotonic() - pressed)))
        self.facts["voice_service_while_held"] = bool(intents.service_pids(self.voice))
        released = time.monotonic()
        self.release()
        shown = intents.wait_until(lambda: ROW in self.showing(), 15, 0.2)
        self.facts["release_to_row_s"] = round(time.monotonic() - released, 2) if shown else None
        marks = self.marks(pressed)
        self.facts["held_marks"] = marks
        for wanted in ("voice_pressed", "voice_listening", "voice_holding", "voice_released",
                       "voice_transcript", "voice_handed_to_spotlight"):
            if not any(mark.startswith(wanted) for mark in marks):
                self.errors.append(f"the HUD never marked {wanted}: {marks}")
        if not shown:
            self.errors.append(f"no {ROW!r} row after letting go; showing: {sorted(self.showing())[:40]}")
            return
        query = self.spotlight_query()
        self.facts["spotlight_query"] = query
        typed = SPEECH in query.lower()
        self.facts["transcript_in_spotlight"] = typed
        if not typed:
            self.errors.append("Spotlight does not show the transcript")
        self.driver.session.pointer.key("return")
        armed = intents.wait_until(lambda: ARMED in self.showing(), 5)
        time.sleep(0.5)
        self.facts["armed"] = bool(armed)
        if not armed:
            self.errors.append("the first Return did not ask for confirmation")
        if intents.theme_scheme(self.config_home) != before:
            self.errors.append("the appearance changed before confirmation")
        self.driver.session.pointer.key("return")
        dark = intents.wait_until(lambda: intents.theme_scheme(self.config_home) == "dark", 10)
        self.facts["scheme_after"] = intents.theme_scheme(self.config_home)
        if not dark:
            self.errors.append("confirming did not switch the appearance to Dark")
        intents.wait_until(lambda: not self.spotlight_open(), 5)
        intents.wait_until(self.hud_gone, 5, 0.2)
        self.facts["held_hud_exited"] = self.hud_gone()
        if not self.hud_gone():
            self.errors.append("the HUD kept running after handing the transcript over")

    def cancelled(self) -> None:
        self.say(SPEECH)
        self.start_hud()
        pressed = time.monotonic()
        self.hold()
        intents.wait_until(lambda: "Listening…" in self.hud_names(), 8, 0.1)
        time.sleep(0.6)
        self.driver.session.pointer.key("escape")
        time.sleep(0.3)
        self.release()
        intents.wait_until(self.hud_gone, 8, 0.2)
        time.sleep(1)
        self.facts["cancel_marks"] = self.marks(pressed)
        self.facts["cancel_spotlight_opened"] = self.spotlight_open()
        if self.spotlight_open():
            self.errors.append("Escape still handed the transcript to Spotlight")
            self.close()
        handoff = self.driver.session.runtime / "rmac" / "voice-transcript.json"
        if handoff.exists():
            self.errors.append("a cancelled transcript was left for Spotlight")

    def unsupported(self) -> None:
        self.say(UNSUPPORTED)
        self.start_hud()
        self.hold()
        intents.wait_until(lambda: "Listening…" in self.hud_names(), 8, 0.1)
        time.sleep(2.0)
        self.release()
        opened = intents.wait_until(self.spotlight_open, 10, 0.2)
        time.sleep(2.5)
        names = self.showing()
        query = self.spotlight_query()
        self.facts["unsupported_spotlight"] = bool(opened)
        self.facts["unsupported_query"] = query
        self.facts["unsupported_query_shown"] = "weather" in query.lower()
        self.facts["unsupported_rows"] = sorted(
            name for name in names if name.endswith(intents.ASSIST_SUFFIX))
        if not opened:
            self.errors.append("an unsupported request did not open Spotlight")
        elif not self.facts["unsupported_query_shown"]:
            self.errors.append("Spotlight does not show the unsupported transcript")
        if self.facts["unsupported_rows"]:
            self.errors.append(f"an unsupported request got a Lulo row: {self.facts['unsupported_rows']}")
        self.close()

    def tapped(self) -> None:
        self.say(SPEECH)
        self.start_hud()
        pressed = time.monotonic()
        self.driver.session.pointer.key("alt-space")
        opened = intents.wait_until(self.spotlight_open, 15, 0.2)
        self.facts["tap_spotlight_s"] = round(time.monotonic() - pressed, 2) if opened else None
        marks = self.marks(pressed)
        self.facts["tap_marks"] = marks
        if not any(mark == "voice_hands_free" for mark in marks):
            self.errors.append(f"a tap did not listen hands-free: {marks}")
        if not opened:
            self.errors.append("a hands-free request never reached Spotlight")
        else:
            time.sleep(1)
            query = self.spotlight_query()
            self.facts["tap_query"] = query
            if SPEECH not in query.lower():
                self.errors.append(f"the hands-free transcript is not in Spotlight: {query!r}")
            self.close()

    def run(self) -> dict:
        original_spawn = self.driver.session.spawn

        def spawn_with_trace(argv, name, extra=None):
            if name == HUD:
                extra = {**(extra or {}), "RMAC_FRAME_TRACE": str(self.hud_trace)}
            if name == "rmac-launcher":
                extra = {**(extra or {}), "RMAC_FRAME_TRACE": str(self.launcher_trace)}
            return original_spawn(argv, name, extra)

        self.driver.session.spawn = spawn_with_trace
        intents.start_light(self.config_home)
        self.driver.start()

        self.off()
        self.held_request()
        self.cancelled()
        self.unsupported()
        self.tapped()

        gone = intents.wait_until(
            lambda: not intents.service_pids(self.voice) and not intents.service_pids(self.service),
            IDLE_SECONDS + 15, 0.5)
        self.facts["services_exited_when_idle"] = bool(gone)
        if not gone:
            self.errors.append("a service was still running after its idle timeout")
        return {"facts": self.facts, "errors": self.errors}


def inner(args) -> int:
    work = Path(args.inner)
    scenario = Scenario(args, work)
    try:
        result = scenario.run()
    except Exception as error:  # noqa: BLE001 - a crash must fail, never pass
        result = {"facts": scenario.facts, "errors": scenario.errors + [f"{type(error).__name__}: {error}"]}
    finally:
        scenario.finish()
    status = "pass" if not result["errors"] else "fail"
    print(json.dumps({"scenario": "voice", "status": status, **result}, indent=2, default=str),
          flush=True)
    return 0 if status == "pass" else 1


def outer(args) -> int:
    lock = open("/tmp/lulo-journey.lock", "w")
    fcntl.flock(lock, fcntl.LOCK_EX)
    work = Path(tempfile.mkdtemp(prefix="lulo-voice-"))
    try:
        env = run_lulo.isolated_environment(work)
        run_lulo.refuse_live_session(env)
        bins = Path(args.bin_dir).expanduser().resolve()
        links = work / "bins"
        links.mkdir()
        # The services and their callers side by side, so the caller checks'
        # "a Lulo program beside me" rule accepts them.
        copy_in_place = {VOICE, INTELLIGENCE, HUD, "rmac-launcher"}
        sources = {source.name: source for source in bins.iterdir()}
        if args.override_bin_dir:
            overrides = Path(args.override_bin_dir).expanduser().resolve()
            for name in copy_in_place | {"rmac-shortcut-dispatch"}:
                if (overrides / name).is_file():
                    sources[name] = overrides / name
        for source in sources.values():
            if source.is_file() and source.name not in {"dock", "mission-control"}:
                if source.name in copy_in_place:
                    target = links / source.name
                    target.write_bytes(source.read_bytes())
                    target.chmod(0o755)
                else:
                    (links / source.name).symlink_to(source)
        for alias, source in (("dock", "rmac-dock"), ("mission-control", "rmac-mission-control")):
            (links / alias).symlink_to(links / source)
        for required in (VOICE, HUD, INTELLIGENCE):
            if not (links / required).is_file():
                raise SystemExit(f"{required} is not in {bins}")
        run_lulo.install_shortcut_dispatcher(env, links)
        config = run_lulo_journey.private_bus(work, env, links)
        services = work / "dbus-services"
        (services / "org.rmac.Voice1.service").write_text(
            f"[D-BUS Service]\nName=org.rmac.Voice1\nExec={links / VOICE}\n")
        (services / "org.rmac.Intelligence1.service").write_text(
            f"[D-BUS Service]\nName=org.rmac.Intelligence1\nExec={links / INTELLIGENCE}\n")
        clip = work / "clip.wav"
        write_clip(clip)
        env["RMAC_VOICE_CAPTURE_FILE"] = str(clip)
        env["RMAC_VOICE_ENGINE"] = "fixture"
        env["RMAC_VOICE_IDLE_SECONDS"] = str(IDLE_SECONDS)
        env["RMAC_INTELLIGENCE_ENGINE"] = "fixture"
        env["RMAC_INTELLIGENCE_IDLE_SECONDS"] = str(IDLE_SECONDS)
        # The nested niri binds the talk key as the session's fallback does.
        source_root = work / "voice-source"
        shell_kdl = source_root / "packaging/rmac-session/shell.kdl"
        shell_kdl.parent.mkdir(parents=True)
        original = (run_window_move.REPO / "packaging/rmac-session/shell.kdl").read_text()
        if "binds {\n" not in original:
            raise SystemExit("shell.kdl has no binds block")
        shell_kdl.write_text(original.replace("binds {\n", "binds {\n" + BINDING, 1))
        env["RMAC_VOICE_SHELL_SOURCE"] = str(source_root)
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
    parser.add_argument("--override-bin-dir",
                        help="take the voice and intelligence services, the HUD, Spotlight "
                             "and the dispatcher from here instead")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--inner", help=argparse.SUPPRESS)
    parser.set_defaults(frame_only=False, full_too=False, dump_a11y=False)
    args = parser.parse_args()
    if args.inner:
        # The private shell.kdl with the talk key bound (see outer()).
        source = os.environ.get("RMAC_VOICE_SHELL_SOURCE")
        if source:
            run_window_move.REPO = Path(source)
        return inner(args)
    return outer(args)


if __name__ == "__main__":
    raise SystemExit(main())
