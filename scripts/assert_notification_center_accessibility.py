#!/usr/bin/env python3
"""Assert the open Notification Center has a nonempty AT-SPI panel tree.

Run in a private compositor/D-Bus session with RMAC_NOTIFICATION_CENTER_PID
set to the process started by the test harness.
"""

from __future__ import annotations

import os

import atspi_assert_support as support
from assert_control_centre_accessibility import app_by_pid


def assert_tree(app) -> int:
    assert app is not None, "Notification Center did not register with AT-SPI"
    nodes = list(support.descendants(app))
    assert len(nodes) > 2, "Notification Center AT-SPI tree is empty"
    named = {(support.role(node), support.name(node)) for node in nodes}
    assert any(name == "Notification Center" for _, name in named), "panel group is unnamed"
    content = {"No recent notifications", "Notification Center Unavailable", "Loading Notification Center…"}
    assert any(name in content or (role in {"group", "push button", "button"}
                                   and name != "Notification Center" and bool(name))
               for role, name in named), (
        "Notification Center has no accessible status or notification cards"
    )
    for node in nodes:
        if support.name(node) == "Edit Widgets":
            assert "click" in support.actions(node), "Edit Widgets has no AT-SPI click"
    return len(nodes)


if __name__ == "__main__":
    pid = int(os.environ["RMAC_NOTIFICATION_CENTER_PID"])
    app = support.wait_for(lambda: app_by_pid(pid), "Notification Center AT-SPI application")
    support.wait_for(lambda: len(list(support.descendants(app))) > 2, "Notification Center populated tree")
    count = assert_tree(app)
    print(f"PASS Notification Center: {count} AT-SPI nodes")
