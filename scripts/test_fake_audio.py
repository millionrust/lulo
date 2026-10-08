"""The nested session's fake pw-dump/wpctl keep one consistent graph."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent / "behavior"))
import fake_audio  # noqa: E402


class FakeAudioTests(unittest.TestCase):
    def setUp(self):
        self.work = tempfile.TemporaryDirectory()
        self.env = {**os.environ, **fake_audio.install(Path(self.work.name))}

    def tearDown(self):
        self.work.cleanup()

    def run_tool(self, *argv: str) -> subprocess.CompletedProcess:
        return subprocess.run(list(argv), env=self.env, capture_output=True, text=True, timeout=10)

    def default_sink(self) -> str:
        graph = json.loads(self.run_tool("pw-dump", "--no-colors").stdout)
        metadata = next(item for item in graph if item["type"] == "PipeWire:Interface:Metadata")
        return metadata["metadata"][0]["value"]["name"]

    def test_set_default_switches_the_output_by_node_id(self):
        self.assertEqual(self.default_sink(), fake_audio.SPEAKERS["name"])
        self.assertEqual(self.run_tool("wpctl", "set-default", str(fake_audio.HDMI["id"])).returncode, 0)
        self.assertEqual(self.default_sink(), fake_audio.HDMI["name"])
        self.assertNotEqual(self.run_tool("wpctl", "set-default", "999").returncode, 0)

    def test_volume_is_reported_as_the_linear_channel_value(self):
        self.run_tool("wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "0.40")
        graph = json.loads(self.run_tool("pw-dump", "--no-colors").stdout)
        speakers = next(item for item in graph if item.get("id") == fake_audio.SPEAKERS["id"])
        volumes = speakers["info"]["params"]["Props"][0]["channelVolumes"]
        self.assertAlmostEqual(volumes[0], 0.4 ** 3, places=5)

    def test_monitor_prints_the_graph_again_after_a_change(self):
        monitor = subprocess.Popen(["pw-dump", "--monitor", "--no-colors"], env=self.env,
                                   stdout=subprocess.PIPE, text=True)
        try:
            decoder = json.JSONDecoder()

            def next_graph(buffer: str) -> tuple[list, str]:
                while True:
                    try:
                        value, end = decoder.raw_decode(buffer.lstrip())
                        return value, buffer.lstrip()[end:]
                    except json.JSONDecodeError:
                        buffer += monitor.stdout.readline()

            first, rest = next_graph("")
            self.assertEqual(len(first), 3)
            monitors = Path(self.env[fake_audio.STATE_VARIABLE]).parent / "monitors"
            for _ in range(100):
                if list(monitors.glob("*.fifo")):
                    break
                time.sleep(0.05)
            self.run_tool("wpctl", "set-default", str(fake_audio.HDMI["id"]))
            second, _ = next_graph(rest)
            self.assertEqual(second[0]["metadata"][0]["value"]["name"], fake_audio.HDMI["name"])
        finally:
            monitor.kill()
            monitor.wait(5)


if __name__ == "__main__":
    unittest.main()
