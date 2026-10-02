#!/usr/bin/env python3
"""Assert the open Control Centre publishes actionable AT-SPI controls.

Run in a private compositor and D-Bus session with RMAC_QUICK_SETTINGS_PID
set to the one process this test started. The interaction probe imports
assert_tree directly after opening the panel.
"""

from __future__ import annotations

import os
import json
from pathlib import Path

import atspi_assert_support as support

SCENARIO = json.loads((Path(__file__).resolve().parents[1] / "tests/behavior/control-centre/accessibility.lulo.json").read_text())


def app_by_pid(pid: int):
    desktop = support.pyatspi.Registry.getDesktop(0)
    for index in range(desktop.childCount):
        try:
            app = desktop.getChildAtIndex(index)
            if app is not None and app.get_process_id() == pid:
                return app
        except (LookupError, RuntimeError):
            continue
    return None


def assert_tree(app) -> int:
    assert app is not None, "Control Centre did not register with AT-SPI"
    nodes = list(support.descendants(app))
    assert len(nodes) > 2, "Control Centre AT-SPI tree is empty"
    named = {(support.role(node), support.name(node)): node for node in nodes}
    assert any(name == SCENARIO["panel"] for _, name in named), "panel group is unnamed"
    sliders = [node for node in nodes if support.role(node) == "slider"]
    assert sliders, "Control Centre has no exposed slider"
    for label in SCENARIO["required_sliders"]:
        assert any(support.name(node) == label for node in sliders), f"{label} slider is missing"
    for slider in sliders:
        assert support.name(slider), "unnamed slider"
        actions = support.actions(slider)
        assert "increment" in actions and "decrement" in actions, (
            f"{support.name(slider)} has no increment/decrement actions: {actions}"
        )
        value = slider.queryValue()
        assert value.minimumValue == SCENARIO["slider_minimum"] and value.maximumValue == SCENARIO["slider_maximum"], (
            f"{support.name(slider)} has wrong bounds"
        )
        assert value.minimumValue <= value.currentValue <= value.maximumValue, f"{support.name(slider)} has wrong value"
    for label in SCENARIO["required_toggles"]:
        matches = [node for node in nodes if support.name(node) == label]
        assert matches, f"{label} toggle is missing"
        assert any("click" in support.actions(node) for node in matches), (
            f"{label} toggle has no click action"
        )
    return len(nodes)


if __name__ == "__main__":
    pid = int(os.environ["RMAC_QUICK_SETTINGS_PID"])
    app = support.wait_for(lambda: app_by_pid(pid), "Control Centre AT-SPI application")
    support.wait_for(lambda: len(list(support.descendants(app))) > 2, "Control Centre populated tree")
    count = assert_tree(app)
    print(f"PASS Control Centre: {count} AT-SPI nodes")
