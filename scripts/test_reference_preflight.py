"""Unit tests for the Linux reference-PC preflight."""

import importlib.util
from pathlib import Path
import sys
import unittest


SCRIPT = Path(__file__).parent / "linux" / "reference-preflight.py"
SPEC = importlib.util.spec_from_file_location("reference_preflight", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
reference_preflight = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = reference_preflight
SPEC.loader.exec_module(reference_preflight)


def passing_host(**overrides):
    values = {
        "kernel": "Linux",
        "effective_uid": 1000,
        "session_type": "wayland",
        "current_desktop": "ubuntu:GNOME",
        "wayland_display": "wayland-0",
        "free_bytes": 40 * reference_preflight.GIB,
        "commands": {"cargo": True, "vulkaninfo": True},
        "vulkan_succeeded": True,
        "vulkan_summary": (
            "deviceType = PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU\n"
            "deviceName = Example GPU"
        ),
        "worktree_clean": True,
    }
    values.update(overrides)
    return reference_preflight.HostSnapshot(**values)


def evaluate(snapshot, expected_desktop="gnome"):
    return reference_preflight.evaluate_host(
        snapshot,
        expected_desktop=expected_desktop,
        minimum_free_bytes=25 * reference_preflight.GIB,
        required_commands=("cargo", "vulkaninfo"),
    )


class ReferencePreflightTests(unittest.TestCase):
    def test_accepts_clean_hardware_backed_gnome_wayland(self):
        self.assertEqual(evaluate(passing_host()), [])

    def test_accepts_rmac_prefixed_niri_session_for_niri_gate(self):
        host = passing_host(current_desktop="rmac:niri")
        self.assertEqual(evaluate(host, expected_desktop="niri"), [])

    def test_rejects_niri_for_untouched_gnome_baseline(self):
        failures = evaluate(passing_host(current_desktop="rmac:niri"))
        self.assertIn("the untouched GNOME Wayland session is required", failures)

    def test_rejects_x11_missing_wayland_and_root(self):
        failures = evaluate(
            passing_host(
                effective_uid=0,
                session_type="x11",
                wayland_display="",
            )
        )
        self.assertIn("run as the graphical test user, not root", failures)
        self.assertIn("XDG_SESSION_TYPE must be wayland", failures)
        self.assertIn("WAYLAND_DISPLAY is unavailable", failures)

    def test_rejects_every_known_software_vulkan_marker(self):
        for marker in reference_preflight.SOFTWARE_VULKAN_MARKERS:
            with self.subTest(marker=marker):
                failures = evaluate(passing_host(vulkan_summary=marker))
                self.assertIn(
                    "a software Vulkan renderer is not reference-PC evidence",
                    failures,
                )

    def test_rejects_unproven_vulkan_low_disk_and_missing_tools(self):
        host = passing_host(
            free_bytes=24 * reference_preflight.GIB,
            commands={"cargo": True, "vulkaninfo": False},
            vulkan_succeeded=False,
            vulkan_summary="",
        )
        failures = evaluate(host)
        self.assertIn("at least 25 GiB free is required before builds", failures)
        self.assertIn("required commands are missing: vulkaninfo", failures)
        self.assertIn("vulkaninfo --summary did not complete", failures)

    def test_rejects_tracked_edits_and_unknown_gpu_type(self):
        host = passing_host(
            vulkan_summary="deviceType = PHYSICAL_DEVICE_TYPE_OTHER",
            worktree_clean=False,
        )
        failures = evaluate(host)
        self.assertIn("an integrated or discrete Vulkan GPU was not proven", failures)
        self.assertIn(
            "tracked worktree changes make the evidence non-reproducible",
            failures,
        )

    def test_desktop_tokens_are_exact_not_substring_matches(self):
        self.assertEqual(
            reference_preflight.desktop_tokens("rmac:niri;GNOME"),
            {"rmac", "niri", "gnome"},
        )
        failures = evaluate(passing_host(current_desktop="not-gnome"))
        self.assertIn("the untouched GNOME Wayland session is required", failures)


if __name__ == "__main__":
    unittest.main()
