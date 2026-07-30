"""Focused build-free tests for the real H4-H6 session journey boundary."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock


SCRIPT = Path(__file__).parent / "linux/run-session-journey.py"
SPECIFICATION = importlib.util.spec_from_file_location("session_journey", SCRIPT)
assert SPECIFICATION is not None and SPECIFICATION.loader is not None
journey = importlib.util.module_from_spec(SPECIFICATION)
sys.modules[SPECIFICATION.name] = journey
SPECIFICATION.loader.exec_module(journey)


def diagnostic_document() -> dict[str, object]:
    return {
        "components": [
            {
                "available": True,
                "healthy": True,
                "restart_budget_exhausted": False,
                "restarts": 0,
                "unit": unit,
            }
            for unit in journey.NORMAL_UNITS
        ],
        "format": 1,
        "observed_at_unix_ms": 42,
        "safe_mode": False,
        "safe_mode_trigger_unit": None,
        "shell_settings_recovery": "current",
    }


class SessionJourneyTests(unittest.TestCase):
    def test_reviewed_contract_is_exact_and_requires_real_login_boundaries(self):
        contract = journey.load_contract()
        self.assertEqual(contract, journey.expected_contract())
        self.assertEqual(contract["minimum_free_gib"], 15)
        self.assertEqual(
            contract["phases"],
            [
                "normal-crash-loop",
                "safe-login",
                "tty-restore",
                "recovered-rmac",
                "gnome-recovery",
            ],
        )
        self.assertEqual(contract["test_user"], "rmac-journey")

    def test_contract_drift_fails_closed(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "journey.json"
            changed = journey.expected_contract()
            changed["test_user"] = "real-user"
            path.write_text(json.dumps(changed), encoding="utf-8")
            with self.assertRaisesRegex(journey.JourneyError, "reviewed policy"):
                journey.load_contract(path)

    def test_diagnostics_accept_only_redacted_exact_component_schema(self):
        document = diagnostic_document()
        completed = journey.subprocess.CompletedProcess(
            [], 0, stdout=json.dumps(document).encode(), stderr=b""
        )
        with mock.patch.object(journey, "_run", return_value=completed):
            self.assertEqual(journey._diagnostics({}), document)

        document["components"][0]["main_pid"] = 1234
        completed = journey.subprocess.CompletedProcess(
            [], 0, stdout=json.dumps(document).encode(), stderr=b""
        )
        with (
            mock.patch.object(journey, "_run", return_value=completed),
            self.assertRaisesRegex(journey.JourneyError, "component diagnostics"),
        ):
            journey._diagnostics({})

        document = diagnostic_document()
        document["private"] = "/home/alice/private-secret"
        completed = journey.subprocess.CompletedProcess(
            [], 0, stdout=json.dumps(document).encode(), stderr=b""
        )
        with (
            mock.patch.object(journey, "_run", return_value=completed),
            self.assertRaisesRegex(journey.JourneyError, "fields are not exact"),
        ):
            journey._diagnostics({})

    def test_phase_order_rejects_reused_graphical_session(self):
        contract = journey.expected_contract()
        state = journey._new_state("session-1")
        with mock.patch.object(journey, "_write_json"):
            journey._complete(
                state,
                contract,
                "normal-crash-loop",
                "session-1",
                Path("/private/state"),
            )
            with self.assertRaisesRegex(journey.JourneyError, "login boundary"):
                journey._complete(
                    state,
                    contract,
                    "safe-login",
                    "session-1",
                    Path("/private/state"),
                )
        with self.assertRaisesRegex(journey.JourneyError, "out of order"):
            journey._require_order(state, contract, "recovered-rmac")

    def test_portal_roundtrip_is_synthetic_and_rechecks_backend_isolation(self):
        completed = journey.subprocess.CompletedProcess(
            [], 0, stdout=b"()\n", stderr=b""
        )
        with (
            mock.patch.object(
                journey, "_unit_state", side_effect=["active", "active"]
            ),
            mock.patch.object(journey, "_run", return_value=completed) as run,
        ):
            journey._portal_roundtrip({}, expect_rmac_backend=True)
        commands = [call.args[0] for call in run.call_args_list]
        self.assertEqual(len(commands), 2)
        self.assertIn(
            "org.freedesktop.portal.Notification.AddNotification", commands[0]
        )
        self.assertIn(
            "org.freedesktop.portal.Notification.RemoveNotification", commands[1]
        )
        self.assertIn("synthetic portal routing evidence", " ".join(commands[0]))

        with (
            mock.patch.object(
                journey, "_unit_state", side_effect=["inactive", "active"]
            ),
            mock.patch.object(journey, "_run", return_value=completed),
            self.assertRaisesRegex(journey.JourneyError, "wrong desktop backend"),
        ):
            journey._portal_roundtrip({}, expect_rmac_backend=False)

    def test_command_execution_is_argument_separated_and_bounded(self):
        completed = journey.subprocess.CompletedProcess(
            [], 0, stdout=b"active\n", stderr=b""
        )
        with mock.patch.object(
            journey.subprocess, "run", return_value=completed
        ) as run:
            result = journey._run(
                ["systemctl", "--user", "show", "rmac-dock.service"],
                tools={"systemctl": "/usr/bin/systemctl"},
            )
        self.assertIs(result, completed)
        self.assertEqual(
            run.call_args.args[0],
            [
                "/usr/bin/systemctl",
                "--user",
                "show",
                "rmac-dock.service",
            ],
        )
        self.assertIs(run.call_args.kwargs["stdin"], journey.subprocess.DEVNULL)

        excessive = journey.subprocess.CompletedProcess(
            [],
            0,
            stdout=b"x" * (journey.MAX_TOOL_OUTPUT_BYTES + 1),
            stderr=b"",
        )
        with (
            mock.patch.object(journey.subprocess, "run", return_value=excessive),
            self.assertRaisesRegex(journey.JourneyError, "excessive output"),
        ):
            journey._run(["systemctl"], tools={"systemctl": "/usr/bin/systemctl"})


if __name__ == "__main__":
    unittest.main()
