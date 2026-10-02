#!/usr/bin/env python3
"""Assert the open Notification Center has a nonempty AT-SPI panel tree.

Run in a private compositor/D-Bus session with RMAC_NOTIFICATION_CENTER_PID
set to the process started by the test harness.
"""

from __future__ import annotations

import os
import json
from pathlib import Path

import atspi_assert_support as support
from assert_control_centre_accessibility import app_by_pid

SCENARIO = json.loads((Path(__file__).resolve().parents[1] / "tests/behavior/notification-center/accessibility.lulo.json").read_text())


def assert_tree(app) -> int:
    assert app is not None, "Notification Center did not register with AT-SPI"
    nodes = list(support.descendants(app))
    assert len(nodes) > 2, "Notification Center AT-SPI tree is empty"
    named = {(support.role(node), support.name(node)) for node in nodes}
    assert any(name == SCENARIO["panel"] for _, name in named), "panel group is unnamed"
    content = set(SCENARIO["states"])
    assert any(name in content or (role in {"group", "push button", "button"}
                                   and name != SCENARIO["panel"] and bool(name))
               for role, name in named), (
        "Notification Center has no accessible status or notification cards"
    )
    for node in nodes:
        if support.name(node) == SCENARIO["edit_button"]:
            assert "click" in support.actions(node), "Edit Widgets has no AT-SPI click"
    return len(nodes)


if __name__ == "__main__":
    pid = int(os.environ["RMAC_NOTIFICATION_CENTER_PID"])
    app = support.wait_for(lambda: app_by_pid(pid), "Notification Center AT-SPI application")
    support.wait_for(lambda: len(list(support.descendants(app))) > 2, "Notification Center populated tree")
    count = assert_tree(app)
    print(f"PASS Notification Center: {count} AT-SPI nodes")
