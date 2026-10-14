"""Shared vocabulary of the Mac-vs-Lulo robot (mac_walk.py, lulo_walk.py, diff.py).

A *state* is one thing the robot opened and recorded, keyed by
``(app, path)``. Both walkers write the same record:

    {"app": "Finder", "path": "menu/File", "kind": "menu",
     "title": "…", "window": [w, h] | None,
     "items":    [{"label", "enabled", "key", "checked", "submenu"}]   # menus
     "controls": [{"role", "label", "enabled", "f": [x, y, w, h]}]      # windows
     "shot": "<app>/<file>.jpg" | None}

Paths (the same on both sides):
    window/main               the app's first window
    menu/@app, menu/File      a menu-bar menu (``@app`` is the application menu)
    menu/File/Open Recent     a submenu
    item/File/Page Setup…     what a menu item opened (sheet, panel, window)
    settings, settings/<tab>  the app's Settings window and its tabs
    tab/<label>               a tab or view switch in the main window
    toolbar/<label>           what a toolbar button opened (menu, popover, sheet)
    context/<surface>         a context menu
    pane/<label>[/<sub>]      a System Settings pane
    shell/<surface>           the desktop shell (app "Shell")
"""

from __future__ import annotations

import json
import re
import unicodedata
from pathlib import Path
from typing import Any, Optional

REPO = Path(__file__).resolve().parents[3]
# Local, persistent, never committed (scripts/check-no-mac-captures.sh).
CAPTURES = Path("/Users/jake/Projects/rmac/.claude/mac-captures")
SUMMARIES = REPO / "tests/robot/mac"

# Mac app -> Lulo app (display name, binary, AppMenu2 bus name).
LULO_APPS: dict[str, dict[str, str]] = {
    "Finder": {"lulo": "Files", "bin": "rmac-files", "bus": "org.rmac.Files.Menu"},
    "TextEdit": {"lulo": "Text Editor", "bin": "rmac-text-editor", "bus": "org.rmac.TextEditor.Menu"},
    "Preview": {"lulo": "Preview", "bin": "rmac-preview", "bus": "org.rmac.Preview.Menu"},
    "Notes": {"lulo": "Notes", "bin": "rmac-notes", "bus": "org.rmac.Notes.Menu"},
    "Terminal": {"lulo": "Terminal", "bin": "rmac-terminal", "bus": "org.rmac.Terminal.Menu"},
    "Calculator": {"lulo": "Calculator", "bin": "rmac-calculator", "bus": "org.rmac.Calculator.Menu"},
    "Clock": {"lulo": "Clock", "bin": "rmac-clock", "bus": "org.rmac.Clock.Menu"},
    "Weather": {"lulo": "Weather", "bin": "rmac-weather", "bus": "org.rmac.Weather.Menu"},
    "System Settings": {"lulo": "Settings", "bin": "rmac-system-settings", "bus": "org.rmac.SystemSettings.Menu"},
    "Activity Monitor": {"lulo": "System Monitor", "bin": "rmac-system-monitor", "bus": "org.rmac.SystemMonitor.Menu"},
    "Mail": {"lulo": "Mail", "bin": "rmac-mail", "bus": "org.rmac.Mail.Menu"},
    "Calendar": {"lulo": "Calendar", "bin": "rmac-calendar", "bus": "org.rmac.Calendar.Menu"},
    "Archive Utility": {"lulo": "Archive Utility", "bin": "rmac-archive-utility", "bus": "org.rmac.ArchiveUtility.Menu"},
    "QuickTime Player": {"lulo": "Player", "bin": "rmac-player", "bus": "org.rmac.Player.Menu"},
}

# Common role vocabulary.
AX_ROLES = {
    "AXButton": "button", "AXMenuButton": "menubutton", "AXPopUpButton": "popup", "AXComboBox": "popup",
    "AXCheckBox": "checkbox", "AXSwitch": "checkbox", "AXRadioButton": "radio", "AXTextField": "textfield",
    "AXSecureTextField": "textfield", "AXTextArea": "textarea", "AXStaticText": "text", "AXHeading": "text",
    "AXSlider": "slider", "AXIncrementor": "stepper", "AXRow": "row", "AXCell": "row", "AXMenuItem": "menuitem",
    "AXMenuBarItem": "menuitem", "AXImage": "image", "AXLink": "link", "AXTabGroup": "tabgroup",
    "AXToolbar": "toolbar", "AXTable": "list", "AXOutline": "list", "AXList": "list", "AXBrowser": "list",
    "AXDisclosureTriangle": "disclosure", "AXColorWell": "colorwell", "AXLevelIndicator": "indicator",
    "AXProgressIndicator": "indicator", "AXValueIndicator": "indicator", "AXDateField": "textfield",
    "AXSegmentedControl": "group", "AXRadioGroup": "group",
}
ATSPI_ROLES = {
    "push button": "button", "button": "button", "toggle button": "checkbox", "check box": "checkbox",
    "switch": "checkbox", "radio button": "radio", "page tab": "radio", "combo box": "popup",
    "text": "textfield", "entry": "textfield", "password text": "textfield", "search field": "textfield",
    "text area": "textarea", "terminal": "textarea", "document text": "textarea",
    "label": "text", "static": "text", "heading": "text", "caption": "text",
    "slider": "slider", "spin button": "stepper", "list item": "row", "table row": "row", "tree item": "row",
    "table cell": "row", "column header": "button", "table column header": "button", "menu item": "menuitem", "check menu item": "menuitem", "radio menu item": "menuitem",
    "image": "image", "icon": "image", "link": "link", "page tab list": "tabgroup", "tool bar": "toolbar",
    "list": "list", "list box": "list", "table": "list", "tree": "list", "tree table": "list",
    "color chooser": "colorwell", "level bar": "indicator", "progress bar": "indicator",
}
CONTROL_ROLES = {"button", "menubutton", "popup", "checkbox", "radio", "textfield", "textarea", "slider",
                 "stepper", "link", "disclosure", "colorwell", "text", "row", "image", "indicator"}
# Window chrome both platforms draw differently; never diffed.
CHROME_SUBROLES = {"AXCloseButton", "AXMinimizeButton", "AXZoomButton", "AXFullScreenButton"}

# Menu items the walker may activate to capture what they open. Everything
# else is only looked at. An ellipsis item opens a sheet or panel by macOS
# convention; these extra labels open a panel or window without changing
# anything.
OPENER_LABELS = {
    "Show Fonts", "Show Colours", "Show Colors", "Show Inspector", "Show View Options", "Get Info",
    "Show Spelling and Grammar", "Show Substitutions", "Show Clipboard", "Show Inspector",
    "Find", "Show Info",
    "Show Inspector Window", "Show Info Window", "Show All Tabs", "Show Tab Overview",
}
# Never activated, never pressed, in any app (word match, case-insensitive).
DENY = re.compile(
    r"\b(delete|erase|empty|trash|send|reply|forward|redirect|bounce|junk|archive|shut ?down|restart|"
    r"log ?out|sign ?(in|out)|sleep|lock|unlock|install|update|upgrade|reset|quit|force|discard|revert|"
    r"move to|remove|burn|eject|encrypt|decrypt|passwords?|accounts?|share|sharing|airdrop|icloud|purchase|"
    r"buy|subscribe|report|feedback|signal|sample|spindump|diagnostics|kill|terminate|recording|record|"
    r"import|transfer|erase|apple account|apple id|family|payment|wallet|screen time|"
    r"new event|new reminder|new message|new note|new mailbox|new alarm|new timer|add|create|"
    r"run|execute|connect|disconnect|duplicate|rename|compress|copy|paste|cut|undo|redo|"
    r"clear|print|attach|insert|subscription|screenshot|take screenshot|from selection|from window|"
    r"from entire screen|app store|camera|microphone|dictation|facetime|new folder|new smart folder)\b",
    re.IGNORECASE,
)
# Ellipsis items that are still allowed although DENY matches a word.
ALLOW_DESPITE_DENY = {"Print…", "Page Setup…", "Rename…", "Connect to Server…", "Duplicate…"}
# Submenus whose contents are the owner's own data or the system's app list.
DYNAMIC_SUBMENUS = {"Services", "Open Recent", "Recent Items", "Import from iPhone or iPad", "Import from iPhone",
                    "Open With", "Share", "Recent Folders", "Recent Places", "Writing Tools", "AutoFill",
                    "Speech", "Insert from iPhone", "Insert from iPhone or iPad", "Revert To", "Move To", "Quick Actions", "Tags", "Window"}


def may_activate(label: str) -> bool:
    label = (label or "").strip()
    if not label:
        return False
    if label in ALLOW_DESPITE_DENY:
        return True
    if DENY.search(label):
        return False
    return (label.endswith("…") or label.endswith("...") or label in OPENER_LABELS
            or label.startswith("About "))


QUOTES = str.maketrans({"’": "'", "‘": "'", "“": '"', "”": '"', "−": "-", " ": " ", "‑": "-"})
BIDI = dict.fromkeys(map(ord, "‎‏‪‫‬‭‮⁦⁧⁨⁩"))
# American and British spellings both platforms use for the same thing.
SPELLING = {"color": "colour", "colors": "colours", "favorites": "favourites", "favorite": "favourite",
            "center": "centre", "minimize": "minimise", "organize": "organise", "customize": "customise",
            "behavior": "behaviour", "gray": "grey"}


def norm_label(label: Optional[str], app: str = "") -> str:
    """Comparable form of a label: case, quotes, ellipsis, app name and
    spelling folded, so 'About Finder…' and 'About Files' compare equal."""

    if not label:
        return ""
    text = unicodedata.normalize("NFC", str(label)).translate(BIDI).translate(QUOTES).strip()
    text = text.replace("...", "…").rstrip("…").rstrip(":").strip()
    names = {app, LULO_APPS.get(app, {}).get("lulo", "")} - {""}
    for name in sorted(names, key=len, reverse=True):
        text = re.sub(rf"\b{re.escape(name)}\b", "{app}", text)
    words = [SPELLING.get(w, w) for w in text.casefold().split()]
    return " ".join(words)


def menu_key(index: int, title: str) -> str:
    """The application menu is always the first one after Apple's."""

    return "@app" if index == 0 else title


# --------------------------------------------------------------- scrub ----

# Roles whose labels are content (files, notes, messages, processes), not
# chrome: kept locally, dropped from committed summaries.
CONTENT_ROLES = {"row", "text", "textfield", "textarea", "image", "link"}
CONTENT_APPS = {"Finder", "Notes", "Mail", "Calendar", "Terminal", "Weather", "Activity Monitor", "Preview",
                "TextEdit", "Shell"}


def personal_tokens() -> list[str]:
    """The owner's names on this Mac, never committed: the login name, the
    full name and its parts, and the computer's whole name (not its words:
    "Air" is also "Air Quality")."""

    import getpass
    import socket
    import subprocess

    tokens = {getpass.getuser(), socket.gethostname().split(".")[0]}
    for cmd, split in ((["id", "-F"], True), (["scutil", "--get", "ComputerName"], False),
                       (["scutil", "--get", "LocalHostName"], False)):
        try:
            out = subprocess.run(cmd, capture_output=True, text=True, timeout=5).stdout.strip()
        except Exception:  # noqa: BLE001
            out = ""
        if out:
            tokens.add(out)
            if split:
                tokens.update(part for part in re.split(r"[\s’'-]+", out) if len(part) >= 3)
    return sorted((t for t in tokens if len(t) >= 3), key=len, reverse=True)


def scrub_text(text: Optional[str], tokens: list[str]) -> Optional[str]:
    if not text:
        return text
    text = re.sub(r"[\w.+-]+@[\w-]+\.[\w.]+", "‹email›", text)
    for token in tokens:
        text = re.sub(rf"\b{re.escape(token)}", "‹owner›", text, flags=re.IGNORECASE)
    return text


def chrome_label(label: Optional[str], vocab: set[str]) -> str:
    """A label kept in a committed summary only when it is the app's own
    wording: a menu item label of some recorded app. Anything else (file
    names, forecasts, places, networks, events) becomes ‹content›."""

    if not label:
        return ""
    return label if label in vocab else "‹content›"


def committed_label(label: Optional[str], vocab: set[str], tokens: list[str]) -> str:
    return scrub_text(chrome_label(label, vocab), tokens) or ""


def summary_for_commit(state: dict[str, Any], tokens: list[str], vocab: set[str]) -> dict[str, Any]:
    """The committable part of a state: structure, menus and the apps' own
    wording. Window, pane, toolbar and shell surfaces keep a label only when
    it is in `vocab` (menu labels); settings windows keep scrubbed labels
    without digits, addresses or long text."""

    path = state["path"]
    out = {k: state.get(k) for k in ("app", "path", "kind", "window")}
    if state.get("items") is not None:
        items = state["items"]
        if path == "menu/Window" or path.startswith("shell/"):
            # The open-window list, Wi-Fi networks, devices: runtime content.
            ends = [i for i, row in enumerate(items) if row.get("label") in {"Bring All to Front", "Arrange in Front"}]
            items = items[: ends[-1] + 1] if ends else items
        rows = []
        for item in items:
            if item.get("dynamic"):
                continue
            row = {k: v for k, v in item.items() if k not in {"f", "p"}}
            label = row.get("label") or ""
            if path.startswith("shell/") and label != "-":
                row["label"] = committed_label(label, vocab, tokens)
            else:
                row["label"] = scrub_text(label, tokens)
            rows.append(row)
        out["items"] = rows
    if state.get("controls") is not None:
        settings = path.startswith("settings")
        rows = []
        for control in state["controls"]:
            label = control.get("label") or ""
            if settings:
                label = scrub_text(label, tokens) or ""
                if re.search(r"\d|@|‹", label) or len(label) > 60:
                    label = "‹content›"
            else:
                label = committed_label(label, vocab, tokens)
            rows.append({"role": control["role"], "label": label, "enabled": control.get("enabled", True),
                         "f": control.get("f")})
        out["controls"] = rows
    return out


def menu_vocabulary(states: list[dict[str, Any]]) -> set[str]:
    vocab = set()
    for state in states:
        if state["path"].startswith(("menu/", "context/")) and state["path"] != "menu/Window":
            vocab.update(row.get("label") for row in state.get("items") or [] if row.get("label"))
    return vocab


def load_states(path: Path) -> dict[tuple[str, str], dict[str, Any]]:
    states = {}
    if path.is_file():
        for line in path.read_text().splitlines():
            if line.strip():
                state = json.loads(line)
                states[(state["app"], state["path"])] = state
    return states
