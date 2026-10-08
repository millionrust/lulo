"""A private, real PipeWire + WirePlumber for one nested behaviour run.

`fake_audio.py` replays recorded `pw-dump` output; this starts the real
sound server instead, so rmac-audio's own parsing runs against what
PipeWire and WirePlumber actually publish. Both daemons live entirely in the
nested session's private XDG_RUNTIME_DIR (its socket is
`$XDG_RUNTIME_DIR/pipewire-0`), with private config and state homes, and
WirePlumber's hardware monitors (ALSA, Bluetooth, cameras) disabled, so the
run never opens a sound card or touches the owner's live PipeWire. Outputs
are null sinks created and removed with `pw-cli`, so a run can go from 0 to
1 to 2 outputs and back the way plugging a device in does.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import time
from pathlib import Path

WIREPLUMBER_CONF = """\
wireplumber.profiles = {
  main = {
    hardware.audio = disabled
    hardware.bluetooth = disabled
    hardware.video-capture = disabled
    support.reserve-device = disabled
    monitor.alsa.reserve-device = disabled
    support.logind = disabled
    support.portal-permissionstore = disabled
    script.client.access-portal = disabled
  }
}
"""

PIPEWIRE_CONF = """\
context.properties = {
  module.x11.bell = false
  module.jackdbus-detect = false
}
"""


def available() -> bool:
    return all(shutil.which(tool) for tool in ("pipewire", "wireplumber", "pw-cli", "pw-dump", "wpctl"))


class RealAudio:
    def __init__(self, env: dict[str, str], work: Path) -> None:
        runtime = env.get("XDG_RUNTIME_DIR", "")
        if not runtime or runtime.startswith("/run/user/"):
            raise SystemExit("real_audio: refusing to start outside a private XDG_RUNTIME_DIR")
        root = work / "real-audio"
        config = root / "config"
        (config / "wireplumber" / "wireplumber.conf.d").mkdir(parents=True, exist_ok=True)
        (config / "pipewire" / "pipewire.conf.d").mkdir(parents=True, exist_ok=True)
        (config / "wireplumber" / "wireplumber.conf.d" / "lulo-behaviour.conf").write_text(WIREPLUMBER_CONF)
        (config / "pipewire" / "pipewire.conf.d" / "lulo-behaviour.conf").write_text(PIPEWIRE_CONF)
        (root / "state").mkdir(exist_ok=True)
        self.env = {key: value for key, value in env.items() if not key.startswith("PIPEWIRE_")}
        self.env.update({
            "PIPEWIRE_RUNTIME_DIR": runtime,
            "XDG_CONFIG_HOME": str(config),
            "XDG_STATE_HOME": str(root / "state"),
            # WirePlumber's GLib would otherwise start gvfs on the bus.
            "GIO_USE_VFS": "local",
        })
        self.log = root / "daemons.log"
        self.processes: list[subprocess.Popen] = []
        self.sinks: dict[str, int] = {}

    def start(self) -> None:
        log = open(self.log, "a")
        self.processes.append(subprocess.Popen(["pipewire"], env=self.env, stdout=log,
                                               stderr=subprocess.STDOUT, close_fds=True))
        socket = Path(self.env["PIPEWIRE_RUNTIME_DIR"]) / "pipewire-0"
        deadline = time.monotonic() + 10
        while not socket.exists() and time.monotonic() < deadline:
            time.sleep(0.1)
        if not socket.exists():
            raise RuntimeError(f"private PipeWire did not start; see {self.log}")
        self.processes.append(subprocess.Popen(["wireplumber"], env=self.env, stdout=log,
                                               stderr=subprocess.STDOUT, close_fds=True))
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if self.metadata_ready():
                return
            time.sleep(0.2)
        raise RuntimeError(f"private WirePlumber did not publish its metadata; see {self.log}")

    def dump(self) -> list[dict]:
        result = subprocess.run(["pw-dump", "--no-colors"], env=self.env, capture_output=True,
                                text=True, timeout=10, check=False)
        return json.loads(result.stdout) if result.returncode == 0 and result.stdout.strip() else []

    def metadata_ready(self) -> bool:
        # WirePlumber publishes its settings metadata once its policy is
        # loaded; the "default" metadata only appears with the first node.
        return any((item.get("props") or {}).get("metadata.name") == "sm-settings"
                   for item in self.dump())

    def node_id(self, name: str) -> int | None:
        for item in self.dump():
            props = (item.get("info") or {}).get("props") or {}
            if props.get("node.name") == name:
                return item["id"]
        return None

    def default_sink(self) -> str | None:
        for item in self.dump():
            if (item.get("props") or {}).get("metadata.name") != "default":
                continue
            for entry in item.get("metadata") or []:
                if entry.get("key") == "default.audio.sink":
                    value = entry.get("value")
                    return value.get("name") if isinstance(value, dict) else None
        return None

    def volume(self) -> float | None:
        """wpctl's (cubic) volume of the default output."""
        result = subprocess.run(["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"], env=self.env,
                                capture_output=True, text=True, timeout=10, check=False)
        parts = result.stdout.split()
        try:
            return float(parts[1]) if result.returncode == 0 and len(parts) >= 2 else None
        except ValueError:
            return None

    def add_sink(self, name: str, description: str) -> int:
        properties = (f"{{ factory.name=support.null-audio-sink node.name={name} "
                      f"node.description=\"{description}\" media.class=Audio/Sink "
                      "audio.position=[ FL FR ] object.linger=true monitor.channel-volumes=true }")
        subprocess.run(["pw-cli", "create-node", "adapter", properties], env=self.env,
                       capture_output=True, text=True, timeout=10, check=True)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            node = self.node_id(name)
            if node is not None:
                self.sinks[name] = node
                return node
            time.sleep(0.1)
        raise RuntimeError(f"null sink {name} did not appear")

    def remove_sink(self, name: str) -> None:
        node = self.sinks.pop(name, None)
        if node is None:
            return
        subprocess.run(["pw-cli", "destroy", str(node)], env=self.env, capture_output=True,
                       text=True, timeout=10, check=False)
        deadline = time.monotonic() + 10
        while self.node_id(name) is not None and time.monotonic() < deadline:
            time.sleep(0.1)

    def set_sinks(self, wanted: list[tuple[str, str]]) -> None:
        names = {name for name, _ in wanted}
        for name in list(self.sinks):
            if name not in names:
                self.remove_sink(name)
        for name, description in wanted:
            if name not in self.sinks:
                self.add_sink(name, description)
        if wanted:
            deadline = time.monotonic() + 10
            while self.default_sink() is None and time.monotonic() < deadline:
                time.sleep(0.1)

    def stop(self) -> None:
        for process in reversed(self.processes):
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(5)
        self.processes.clear()
