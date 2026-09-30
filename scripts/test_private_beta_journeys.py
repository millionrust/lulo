"""Pure checks for the private installed Terminal round-trip gate."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import sys
import unittest


SCRIPT = Path(__file__).parent / "behavior" / "run_private_beta_journeys.py"
SPEC = importlib.util.spec_from_file_location("private_beta_journeys", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
journeys = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = journeys
SPEC.loader.exec_module(journeys)


class TerminalRoundtripTests(unittest.TestCase):
    def test_accepts_command_echo_followed_by_exact_output_line(self):
        text = "$ echo RMAC_BETA_TERMINAL_OK\nRMAC_BETA_TERMINAL_OK\n"
        self.assertTrue(journeys.has_typed_command_roundtrip(text, "RMAC_BETA_TERMINAL_OK"))

    def test_rejects_duplicate_marker_on_command_line_without_output(self):
        text = "$ echo RMAC_BETA_TERMINAL_OK RMAC_BETA_TERMINAL_OK\n"
        self.assertFalse(journeys.has_typed_command_roundtrip(text, "RMAC_BETA_TERMINAL_OK"))

    def test_rejects_marker_output_before_command_echo(self):
        text = "RMAC_BETA_TERMINAL_OK\n$ echo RMAC_BETA_TERMINAL_OK\n"
        self.assertFalse(journeys.has_typed_command_roundtrip(text, "RMAC_BETA_TERMINAL_OK"))

    def test_rejects_marker_embedded_in_unrelated_output(self):
        text = "$ echo RMAC_BETA_TERMINAL_OK\nresult RMAC_BETA_TERMINAL_OK done\n"
        self.assertFalse(journeys.has_typed_command_roundtrip(text, "RMAC_BETA_TERMINAL_OK"))


if __name__ == "__main__":
    unittest.main()
