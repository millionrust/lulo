"""Fake PipeWire tools for the nested behaviour-test session.

The private nested session has no PipeWire or WirePlumber, so Control
Centre's Sound list, the top bar's Sound menu and Settings ▸ Sound saw "No
Output Device". `install(work)` writes `pw-dump` and `wpctl` stand-ins onto
a scratch PATH directory: rmac-audio runs those two programs by name, so
the nested apps read and change a small graph kept in a JSON state file
instead of a real sound server. Nothing outside `work` is touched.

Three graphs, chosen with `install(work, graph=…)` or switched live with
`fake_audio.py set-graph NAME` (with the state variable in the env):

- "pair": two synthetic outputs, "Lulo Speakers" (the default) and "Lulo
  HDMI Display";
- "laptop": the reference laptop's real `pw-dump` (67 objects, exactly one
  sink, "Built-in Audio Analog Stereo", with the real ALSA properties and
  two-entry Props), from
  `crates/rmac-audio/src/fixtures/pw-dump-laptop-full.json`;
- "none": the same real graph with its sink removed and no
  `default.audio.sink`, as WirePlumber leaves it with no output.

`wpctl set-default`, `set-volume` and `set-mute` change the state file and
wake every `pw-dump --monitor` through its own FIFO (no polling), which then
prints the changed graph the way PipeWire's monitor does.

Run as `fake_audio.py pw-dump|wpctl ARGS…` (the PATH wrappers do this).
"""

from __future__ import annotations

import copy
import json
import os
import sys
from pathlib import Path

SPEAKERS = {"id": 61, "name": "alsa_output.lulo-speakers", "description": "Lulo Speakers"}
HDMI = {"id": 62, "name": "alsa_output.lulo-hdmi", "description": "Lulo HDMI Display"}
STATE_VARIABLE = "LULO_FAKE_AUDIO_STATE"
LAPTOP_FIXTURE = (Path(__file__).resolve().parents[2] / "crates" / "rmac-audio" / "src"
                  / "fixtures" / "pw-dump-laptop-full.json")
LAPTOP_SINK = {"id": 52, "name": "alsa_output.pci-0000_00_1b.0.analog-stereo",
               "description": "Built-in Audio Analog Stereo"}
GRAPHS = ("pair", "laptop", "none")


def initial_state(graph_name: str) -> dict:
    if graph_name not in GRAPHS:
        raise ValueError(f"unknown fake audio graph {graph_name!r}")
    if graph_name == "pair":
        return {
            "graph": graph_name,
            "default": SPEAKERS["name"],
            "volume": {SPEAKERS["name"]: 0.5, HDMI["name"]: 0.7},
            "muted": {SPEAKERS["name"]: False, HDMI["name"]: False},
        }
    # The real sink's own level: linear 0.027001 is wpctl's 0.30.
    return {
        "graph": graph_name,
        "default": LAPTOP_SINK["name"] if graph_name == "laptop" else None,
        "volume": {LAPTOP_SINK["name"]: 0.3},
        "muted": {LAPTOP_SINK["name"]: False},
    }


def install(work: Path, graph_name: str = "pair") -> dict[str, str]:
    """Write the state file and PATH wrappers under `work`; return the env
    entries that point the nested session at them."""

    root = work / "fake-audio"
    bin_dir = root / "bin"
    (root / "monitors").mkdir(parents=True, exist_ok=True)
    bin_dir.mkdir(parents=True, exist_ok=True)
    state = root / "state.json"
    state.write_text(json.dumps(initial_state(graph_name)))
    script = Path(__file__).resolve()
    for tool in ("pw-dump", "wpctl"):
        wrapper = bin_dir / tool
        wrapper.write_text(f'#!/bin/sh\nexec "{sys.executable}" "{script}" {tool} "$@"\n')
        wrapper.chmod(0o755)
    return {
        "PATH": f"{bin_dir}:{os.environ.get('PATH', '/usr/bin:/bin')}",
        STATE_VARIABLE: str(state),
    }


def read_state(path: Path) -> dict:
    return json.loads(path.read_text())


def sinks(state: dict) -> list[dict]:
    graph_name = state.get("graph", "pair")
    if graph_name == "pair":
        return [SPEAKERS, HDMI]
    if graph_name == "laptop":
        return [LAPTOP_SINK]
    return []


def real_graph(state: dict) -> list[dict]:
    """The laptop's real dump with the state's default, volume and mute
    written into the objects PipeWire itself would change."""

    keep_sink = state.get("graph") == "laptop"
    result = []
    for item in copy.deepcopy(json.loads(LAPTOP_FIXTURE.read_text())):
        props = (item.get("info") or {}).get("props") or {}
        if props.get("media.class") == "Audio/Sink":
            if not keep_sink:
                continue
            name = props.get("node.name")
            linear = round(state["volume"][name] ** 3, 6)
            entry = item["info"]["params"]["Props"][0]
            entry["channelVolumes"] = [linear] * len(entry["channelVolumes"])
            entry["mute"] = state["muted"][name]
        if (item.get("props") or {}).get("metadata.name") == "default":
            entries = [entry for entry in item.get("metadata", [])
                       if entry.get("key") != "default.audio.sink"]
            if state.get("default"):
                entries.insert(0, {"subject": 0, "key": "default.audio.sink",
                                   "type": "Spa:String:JSON",
                                   "value": {"name": state["default"]}})
            item["metadata"] = entries
        result.append(item)
    return result


def graph(state: dict) -> list[dict]:
    if state.get("graph", "pair") != "pair":
        return real_graph(state)
    objects: list[dict] = [{
        "id": 41,
        "type": "PipeWire:Interface:Metadata",
        "props": {"metadata.name": "default"},
        "metadata": [{
            "subject": 0,
            "key": "default.audio.sink",
            "type": "Spa:String:JSON",
            "value": {"name": state["default"]},
        }],
    }]
    for sink in (SPEAKERS, HDMI):
        # wpctl shows cubic volume; pw-dump reports the linear channel value.
        linear = round(state["volume"][sink["name"]] ** 3, 6)
        objects.append({
            "id": sink["id"],
            "type": "PipeWire:Interface:Node",
            "permissions": ["r", "w", "x", "m"],
            "info": {
                "props": {
                    "media.class": "Audio/Sink",
                    "node.name": sink["name"],
                    "node.description": sink["description"],
                },
                "params": {
                    "Props": [{
                        "volume": 1.0,
                        "mute": state["muted"][sink["name"]],
                        "channelVolumes": [linear, linear],
                        "channelMap": ["FL", "FR"],
                    }],
                },
            },
        })
    return objects


def print_graph(state_path: Path) -> None:
    sys.stdout.write(json.dumps(graph(read_state(state_path)), indent=2) + "\n")
    sys.stdout.flush()


def pw_dump(state_path: Path, args: list[str]) -> int:
    if "--monitor" not in args:
        print_graph(state_path)
        return 0
    # One FIFO per monitor; wpctl writes a byte to each after a change.
    fifo = state_path.parent / "monitors" / f"{os.getpid()}.fifo"
    os.mkfifo(fifo)
    try:
        # O_RDWR keeps a writer open, so reads block instead of seeing EOF.
        descriptor = os.open(fifo, os.O_RDWR)
        print_graph(state_path)
        while True:
            if not os.read(descriptor, 64):
                return 0
            print_graph(state_path)
    except (BrokenPipeError, KeyboardInterrupt):
        return 0
    finally:
        fifo.unlink(missing_ok=True)


def wake_monitors(state_path: Path) -> None:
    for fifo in (state_path.parent / "monitors").glob("*.fifo"):
        try:
            descriptor = os.open(fifo, os.O_WRONLY | os.O_NONBLOCK)
        except OSError:
            continue
        try:
            os.write(descriptor, b"x")
        except OSError:
            pass
        finally:
            os.close(descriptor)


def node_name(state: dict, target: str) -> str | None:
    if target in ("@DEFAULT_AUDIO_SINK@", "@DEFAULT_SINK@"):
        return state["default"]
    for sink in sinks(state):
        if target == str(sink["id"]):
            return sink["name"]
    return None


def wpctl(state_path: Path, args: list[str]) -> int:
    state = read_state(state_path)
    if len(args) < 2:
        print("fake wpctl: missing arguments", file=sys.stderr)
        return 2
    command, target = args[0], args[1]
    name = node_name(state, target)
    if name is None:
        print(f"fake wpctl: unknown node {target}", file=sys.stderr)
        return 1
    if command == "set-default":
        state["default"] = name
    elif command == "set-volume" and len(args) >= 3:
        state["volume"][name] = max(0.0, min(1.5, float(args[2].rstrip("%"))))
    elif command == "set-mute" and len(args) >= 3:
        value = args[2]
        state["muted"][name] = (not state["muted"][name]) if value == "toggle" else value == "1"
    else:
        print(f"fake wpctl: unsupported {' '.join(args)}", file=sys.stderr)
        return 2
    write_state(state_path, state)
    return 0


def write_state(state_path: Path, state: dict) -> None:
    temporary = state_path.with_suffix(".tmp")
    temporary.write_text(json.dumps(state))
    temporary.replace(state_path)
    wake_monitors(state_path)


def set_graph(state_path: Path, graph_name: str) -> None:
    """Swap the whole graph (an output plugged in or removed) and tell every
    running monitor, as PipeWire reports a device change."""

    write_state(state_path, initial_state(graph_name))


def main(argv: list[str]) -> int:
    state_path = Path(os.environ[STATE_VARIABLE])
    tool, args = argv[0], argv[1:]
    if tool == "pw-dump":
        return pw_dump(state_path, args)
    if tool == "wpctl":
        return wpctl(state_path, args)
    if tool == "set-graph" and len(args) == 1 and args[0] in GRAPHS:
        set_graph(state_path, args[0])
        return 0
    print(f"fake_audio: unknown tool {tool}", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
