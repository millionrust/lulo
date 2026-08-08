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
        "portal_service_active": True,
        "portal_bus_owned": True,
        "manager_environment_matches": True,
        "niri_ipc_succeeded": True,
        "niri_enabled_outputs": 1,
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

    def test_accepts_hardware_gpu_when_software_fallback_is_also_enumerated(self):
        host = passing_host(
            vulkan_summary=(
                "GPU0 deviceType = PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU\n"
                "GPU0 deviceName = Intel(R) HD Graphics 5500\n"
                "GPU1 deviceType = PHYSICAL_DEVICE_TYPE_CPU\n"
                "GPU1 deviceName = llvmpipe"
            )
        )
        self.assertEqual(evaluate(host), [])

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

    def test_rejects_missing_portal_frontend_authorities(self):
        failures = evaluate(
            passing_host(
                portal_service_active=False,
                portal_bus_owned=False,
            )
        )
        self.assertIn("xdg-desktop-portal.service is not active", failures)
        self.assertIn(
            "the desktop portal frontend does not own its session-bus name",
            failures,
        )

    def test_niri_gate_requires_manager_environment_ipc_and_enabled_output(self):
        failures = evaluate(
            passing_host(
                current_desktop="rmac:niri",
                manager_environment_matches=False,
                niri_ipc_succeeded=False,
                niri_enabled_outputs=0,
            ),
            expected_desktop="niri",
        )
        self.assertIn(
            "the user-manager graphical routing environment is stale",
            failures,
        )
        self.assertIn(
            "niri IPC did not return a valid bounded output snapshot",
            failures,
        )

        failures = evaluate(
            passing_host(
                current_desktop="rmac:niri",
                niri_ipc_succeeded=True,
                niri_enabled_outputs=0,
            ),
            expected_desktop="niri",
        )
        self.assertIn("niri reports no enabled output", failures)

    def test_desktop_tokens_are_exact_not_substring_matches(self):
        self.assertEqual(
            reference_preflight.desktop_tokens("rmac:niri;GNOME"),
            {"rmac", "niri", "gnome"},
        )
        failures = evaluate(passing_host(current_desktop="not-gnome"))
        self.assertIn("the untouched GNOME Wayland session is required", failures)

    def test_manager_environment_compares_only_exact_required_routing_values(self):
        environment = {
            "DBUS_SESSION_BUS_ADDRESS": "unix:path=/run/user/1000/bus",
            "NIRI_SOCKET": "/run/user/1000/niri.sock",
            "WAYLAND_DISPLAY": "wayland-1",
            "XDG_CURRENT_DESKTOP": "rmac:niri",
            "XDG_RUNTIME_DIR": "/run/user/1000",
            "XDG_SESSION_ID": "9",
            "XDG_SESSION_TYPE": "wayland",
        }
        output = (
            "\n".join(f"{key}={value}" for key, value in environment.items())
            + "\nPRIVATE_TOKEN=not-inspected\n"
        )
        self.assertTrue(
            reference_preflight.manager_environment_matches(output, environment)
        )
        self.assertFalse(
            reference_preflight.manager_environment_matches(
                output.replace("WAYLAND_DISPLAY=wayland-1", "WAYLAND_DISPLAY=wayland-0"),
                environment,
            )
        )

    def test_niri_output_parser_counts_enabled_and_rejects_malformed_geometry(self):
        output = """
        {
          "DP-1": {"logical": {"width": 1920, "height": 1080}},
          "DP-2": {"logical": null}
        }
        """
        self.assertEqual(reference_preflight.niri_enabled_output_count(output), 1)
        self.assertIsNone(
            reference_preflight.niri_enabled_output_count(
                '{"DP-1":{"logical":{"width":0,"height":1080}}}'
            )
        )
        self.assertIsNone(reference_preflight.niri_enabled_output_count("[]"))


if __name__ == "__main__":
    unittest.main()
