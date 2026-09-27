"""Mock tests for the live shutdown confirmation probe; no AT-SPI required."""
from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

SCRIPT = Path(__file__).parent / "linux" / "probe_live_shutdown_cancel.py"
SPEC = importlib.util.spec_from_file_location("probe_live_shutdown_cancel", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
probe = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = probe
SPEC.loader.exec_module(probe)


class FakeBackend:
    def __init__(self, fail_at: str | None = None):
        self.state = "closed"
        self.actions: list[str] = []
        self.fail_at = fail_at

    def find(self, wanted_role: str, wanted_name: str):
        if self.state != "dialog" and wanted_role == "push button" and wanted_name == "menu":
            return "menu"
        if self.state == "menu" and wanted_role == "menu item" and wanted_name == "Shut Down…":
            return "shutdown-row"
        if self.state == "dialog" and wanted_role == "push button" and wanted_name in {"Cancel", "Shut Down"}:
            return wanted_name
        return None

    def wait(self, predicate, label: str, timeout: float = 0):
        result = predicate()
        if not result:
            raise probe.ProbeError(f"timed out waiting for {label}")
        return result

    def has_confirmation(self) -> bool:
        return self.state == "dialog"

    def activate_click(self, node: str) -> None:
        self.actions.append(node)
        if node == "menu":
            self.state = "closed" if self.state == "menu" else "menu"
        elif node == "shutdown-row":
            self.state = "dialog"
        elif node == "Cancel":
            self.state = "menu"
        elif node == "Shut Down":
            raise AssertionError("Confirm must never be activated")
        if self.fail_at == node:
            raise probe.ProbeError(f"mock failure at {node}")

    def cancel_confirmation(self, timeout: float = 8.0) -> bool:
        if self.state == "dialog":
            self.activate_click("Cancel")
            return True
        return False


def test_probe_activates_only_menu_rows_and_cancel():
    backend = FakeBackend()
    states: list[str] = []

    probe.run_probe(backend, states.append)

    assert backend.actions == ["menu", "shutdown-row", "Cancel", "menu"]
    assert backend.state == "closed"
    assert "state: confirmation visible; Confirm was not activated" in states


def test_probe_leaves_an_existing_system_menu_untouched():
    backend = FakeBackend()
    backend.state = "menu"
    try:
        probe.run_probe(backend, lambda _message: None)
    except probe.ProbeError as error:
        assert "already open" in str(error)
    else:
        raise AssertionError("existing menu must not be changed")
    assert backend.actions == []
    assert backend.state == "menu"


def test_failure_after_opening_dialog_still_cancels():
    backend = FakeBackend(fail_at="shutdown-row")

    try:
        probe.run_probe(backend, lambda _message: None)
    except probe.ProbeError:
        pass
    else:
        raise AssertionError("expected mock failure")

    assert backend.actions == ["menu", "shutdown-row", "Cancel", "menu"]
    assert backend.state == "closed"


def test_session_guard_requires_expected_wayland_display():
    original = probe.platform.system
    try:
        probe.platform.system = lambda: "Darwin"
        try:
            probe.discover_live_session("jacob")
        except probe.ProbeError as error:
            assert "requires the live Linux session" in str(error)
        else:
            raise AssertionError("non-Linux session must be refused")
    finally:
        probe.platform.system = original


def test_confirm_button_is_rejected_by_allowlist():
    class Node:
        name = "Shut Down"

        def getRoleName(self):
            return "push button"

    backend = object.__new__(probe.AtspiBackend)
    try:
        backend.activate_click(Node())
    except probe.ProbeError as error:
        assert "outside the safe allowlist" in str(error)
    else:
        raise AssertionError("Confirm must be rejected before querying its action")


def test_failure_after_menu_open_toggles_menu_closed_and_reports_it():
    backend = FakeBackend(fail_at="menu")
    states: list[str] = []

    try:
        probe.run_probe(backend, states.append)
    except probe.ProbeError:
        pass
    else:
        raise AssertionError("expected mock failure")

    assert backend.actions == ["menu", "menu"]
    assert backend.state == "closed"
    assert any("cleanup: system menu may remain open" in state for state in states)
