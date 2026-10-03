#!/usr/bin/env python3
"""Check the Internet Accounts chooser over AT-SPI without signing in.

Run against one Settings instance on a private bus with temporary XDG dirs.
This script only opens and cancels the sheet; it never creates an account.
"""

from __future__ import annotations

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import atspi_assert_support as support  # noqa: E402

APP = os.environ.get("RMAC_SETTINGS_APP", "rmac-system-settings")


def app():
    return support.find_app(APP)


def clickable(label):
    return support.wait_for(
        lambda: next(
            (
                node
                for node in support.descendants(app())
                if support.name(node) == label
                and "click" in support.actions(node)
            ),
            None,
        ),
        f"the actionable {label} control",
    )


assert support.click(clickable("Internet Accounts"))
assert support.click(clickable("Add Account…"))

for name in (
    "iCloud · App-specific password",
    "Microsoft · Outlook, Hotmail, Microsoft 365",
    "Google · Gmail, Google Workspace",
    "Yahoo · App-specific password",
    "Other Mail Account… · IMAP and SMTP",
    "Other Calendar Account… · CalDAV",
):
    clickable(name)

entry = support.wait_for(
    lambda: next(
        (
            node
            for node in support.nodes_with(app(), "entry")
            if support.name(node) == "Email address"
        ),
        None,
    ),
    "the email address entry",
)
assert support.has_text(entry), "email address entry has no Text interface"
clickable("Continue")
assert support.click(clickable("Cancel"))
support.wait_for(
    lambda: not any(
        support.name(node) == "Google · Gmail, Google Workspace"
        for node in support.descendants(app())
    ),
    "the account sheet to close",
)
print("AT-SPI Internet Accounts: 6 named provider actions, email entry, Continue, Cancel")
