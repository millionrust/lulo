#!/usr/bin/env python3
"""Build a feature inventory of the reference macOS apps, read entirely
through the Accessibility tree (System Events). No clicking, no toggling,
no touching the owner's documents, mail or accounts.

For each app in APPS:
  - launch it only if it is not already running, and quit it afterwards
    only if this script launched it;
  - read its full menu bar tree (titles, items, submenus, separators,
    shortcut glyphs, enabled state, checked state);
  - open its Settings window with Cmd+, read its control tree, close it
    with Cmd+W;
  - read its front window's toolbar item titles;
  - for Finder only: open a throwaway sandbox folder under /tmp and read
    the context menu for a file, a folder and the background with
    AXShowMenu + Escape (never a click);
  - for System Settings only: read the sidebar list, then select
    (not click) each row to navigate and read that pane's top-level
    labels, read-only throughout.

Output: one JSON file per app under tests/inventory/mac/<app>.json.

Safety:
  - Aborts before doing anything if the screen is locked.
  - Holds a simple file-lock (/tmp/mac-gui.lock) for the whole run so it
    never races a human or another script driving the same GUI.
  - Never recurses into "Recent Items", "Open Recent" or "Services": those
    AppKit-provided submenus are populated from the owner's own documents
    and app usage, not from the app's own static menu declaration.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import mac_applescript as asc  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parents[2]
OUT_DIR = REPO_ROOT / "tests/inventory/mac"
LOCK_DIR = Path("/tmp/mac-gui.lock")
SCRATCH = Path(os.environ.get("TMPDIR", "/tmp")) / "rmac-inventory"

# Mac app display name -> (process name as System Events sees it, whether
# it is always running and must never be quit).
APPS: dict[str, dict] = {
    "Finder": {"process": "Finder", "always_running": True},
    "TextEdit": {"process": "TextEdit"},
    "Preview": {"process": "Preview"},
    "Notes": {"process": "Notes"},
    "Terminal": {"process": "Terminal"},
    "Calculator": {"process": "Calculator"},
    "Activity Monitor": {"process": "Activity Monitor"},
    "System Settings": {"process": "System Settings"},
    "Clock": {"process": "Clock"},
    "Weather": {"process": "Weather"},
}

# AppKit/system-populated submenus that reflect the owner's own documents,
# app usage, device names or account, rather than the app's static menu
# declaration — never recursed. "Apple" is the system Apple menu: besides
# being OS chrome (see tests/inventory/allowlist.json), it contains
# "Log Out <full name>…", which would otherwise commit the owner's real
# name into this file for every app. "Import from iPhone" (Finder's
# context menu) similarly names the owner's own device.
DYNAMIC_PERSONAL_SUBMENUS = {
    "Recent Items",
    "Open Recent",
    "Services",
    "Apple",
    "Import from iPhone",
}

# AXMenuItemCmdModifiers bitmask (HIToolbox Menus.h): shift=1, option=2,
# control=4, no-command=8.
MOD_SHIFT, MOD_OPTION, MOD_CONTROL, MOD_NO_COMMAND = 1, 2, 4, 8

# AXMenuItemCmdVirtualKey codes worth naming explicitly when AXMenuItemCmdChar
# is empty or unprintable.
VIRTUAL_KEY_GLYPH = {
    123: "←",
    124: "→",
    125: "↓",
    126: "↑",
    49: "Space",
    51: "⌫",
    53: "⎋",
    36: "↩",
    48: "⇥",
}

CONTROL_CHAR_GLYPH = {
    "\x08": "⌫",
    "\x7f": "⌫",
    "\x1b": "⎋",
    "\r": "↩",
    "\n": "↩",
    "\t": "⇥",
    " ": "Space",
    "-": "−",  # Lulo's tables spell Zoom Out etc. with U+2212, not a hyphen.
}


def run_osascript(script: str, timeout: int = 30) -> str:
    SCRATCH.mkdir(parents=True, exist_ok=True)
    path = SCRATCH / "driver.applescript"
    path.write_text(script)
    try:
        result = subprocess.run(
            ["osascript", str(path)], capture_output=True, text=True, timeout=timeout
        )
    except subprocess.TimeoutExpired as error:
        # Every caller in this file catches RuntimeError (never
        # subprocess.TimeoutExpired) to decide "this one step failed, keep
        # going" vs. letting it escape `inventory_app` entirely — which
        # would skip that app's final `quit_app` and leave it running.
        # Normalise to RuntimeError so a slow step (a Services-heavy menu,
        # a slow pane) degrades the same way a script error does.
        raise RuntimeError(f"osascript timed out after {timeout}s") from error
    if result.returncode != 0:
        raise RuntimeError(f"osascript failed: {result.stderr.strip()}")
    return result.stdout


# ---------------------------------------------------------------------------
# Safety
# ---------------------------------------------------------------------------


def screen_is_locked() -> bool:
    """CGSSessionScreenIsLocked is present (and true) only while the screen
    is locked; it is absent from the IOConsoleUsers dict otherwise."""
    try:
        import plistlib

        raw = subprocess.run(
            ["ioreg", "-n", "Root", "-d1", "-a"], capture_output=True, check=True
        ).stdout
        data = plistlib.loads(raw)
        users = data.get("IOConsoleUsers", [{}])
        current = users[0] if users else {}
        return bool(current.get("CGSSessionScreenIsLocked", False))
    except Exception as error:  # noqa: BLE001 - fail closed (locked) on error
        print(f"could not determine screen-lock state ({error}); assuming locked", file=sys.stderr)
        return True


def acquire_gui_lock() -> None:
    waited = 0
    while True:
        try:
            LOCK_DIR.mkdir()
            return
        except FileExistsError:
            if waited == 0:
                print(f"waiting for GUI lock {LOCK_DIR} ...", file=sys.stderr)
            time.sleep(10)
            waited += 10


def release_gui_lock() -> None:
    try:
        LOCK_DIR.rmdir()
    except OSError:
        pass


# ---------------------------------------------------------------------------
# Shortcut glyph assembly
# ---------------------------------------------------------------------------


def build_shortcut(cmd_char: str, cmd_vk: str, cmd_mods: str) -> str:
    if not cmd_char and not cmd_vk:
        return ""
    try:
        mods = int(cmd_mods) if cmd_mods else 0
    except ValueError:
        mods = 0
    prefix = ""
    if mods & MOD_CONTROL:
        prefix += "⌃"
    if mods & MOD_OPTION:
        prefix += "⌥"
    if mods & MOD_SHIFT:
        prefix += "⇧"
    if not (mods & MOD_NO_COMMAND):
        prefix += "⌘"
    key = ""
    if cmd_char:
        key = CONTROL_CHAR_GLYPH.get(cmd_char, cmd_char)
    elif cmd_vk:
        try:
            key = VIRTUAL_KEY_GLYPH.get(int(cmd_vk), "")
        except ValueError:
            key = ""
    if not key:
        return ""
    return prefix + key


# ---------------------------------------------------------------------------
# Menu bar
# ---------------------------------------------------------------------------


def parse_menu_dump(raw: str) -> list[dict]:
    """Turn the tab-separated `dumpMenuBar` output into a nested tree."""
    roots: list[dict] = []
    stack: list[tuple[int, list[dict]]] = [(-1, roots)]
    for line in raw.splitlines():
        if not line:
            continue
        fields = line.split("\t")
        fields += [""] * (7 - len(fields))
        depth_s, title, cmd_char, cmd_vk, cmd_mods, enabled, mark = fields[:7]
        depth = int(depth_s)
        node = {
            "label": title if title else None,  # None marks a separator
            "shortcut": build_shortcut(cmd_char, cmd_vk, cmd_mods),
            "enabled": enabled == "1",
            "checked": mark not in ("", None),
            "children": [],
        }
        while stack and stack[-1][0] >= depth:
            stack.pop()
        parent_children = stack[-1][1]
        if title in DYNAMIC_PERSONAL_SUBMENUS:
            node["children_omitted"] = "dynamic/personal submenu, not read"
        else:
            parent_children = parent_children  # for clarity
        parent_children.append(node)
        if title not in DYNAMIC_PERSONAL_SUBMENUS:
            stack.append((depth, node["children"]))
        else:
            stack.append((depth, []))  # swallow children without recording them
    return roots


def dump_menu_bar(process: str) -> list[dict]:
    script = asc.driver(
        asc.DUMP_MENU_ITEMS_HANDLER,
        asc.DUMP_MENU_BAR_HANDLER,
        body=f'dumpMenuBar("{process}")',
    )
    raw = run_osascript(script, timeout=90)
    tree = parse_menu_dump(raw)
    # tree[i] is a top-level menu-bar title with its items as children.
    return [
        {"label": t["label"], "items": t["children"]}
        for t in tree
    ]


# ---------------------------------------------------------------------------
# Process lifecycle
# ---------------------------------------------------------------------------


def is_running(process: str) -> bool:
    script = f'tell application "System Events" to return exists process "{process}"'
    out = run_osascript(script).strip()
    return out == "true"


def launch(app_name: str) -> None:
    subprocess.run(["open", "-a", app_name], check=True)


def wait_until_running(process: str, timeout: float = 15.0) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if is_running(process):
            # Give the app a moment to finish building its menu bar.
            time.sleep(1.0)
            return True
        time.sleep(0.5)
    return False


def quit_app(process: str) -> None:
    script = f'tell application "{process}" to quit'
    try:
        run_osascript(script, timeout=15)
    except RuntimeError as error:
        print(f"could not quit {process}: {error}", file=sys.stderr)


# ---------------------------------------------------------------------------
# Settings window (Cmd+,)
# ---------------------------------------------------------------------------


def _window_titles(process: str) -> list[str]:
    script = f"""
tell application "System Events"
	tell process "{process}"
		set out to ""
		repeat with w in windows
			try
				set out to out & (name of w) & linefeed
			end try
		end repeat
		return out
	end tell
end tell
"""
    raw = run_osascript(script, timeout=10)
    return [line for line in raw.splitlines() if line]


def dump_settings_window(process: str) -> dict:
    """Open Settings with Cmd+,, read it, close it with Cmd+W — but only if
    a genuinely new window appeared (many apps bind no action to Cmd+, at
    all, and blindly sending Cmd+W afterwards would close that app's
    existing, unrelated window instead)."""
    try:
        before = _window_titles(process)
    except RuntimeError as error:
        return {"present": False, "error": str(error), "controls": []}

    open_script = f"""
tell application "System Events"
	tell process "{process}"
		set frontmost to true
		delay 0.3
		keystroke "," using {{command down}}
	end tell
end tell
delay 0.8
"""
    try:
        run_osascript(open_script, timeout=15)
    except RuntimeError as error:
        return {"present": False, "error": str(error), "controls": []}

    try:
        after = _window_titles(process)
    except RuntimeError as error:
        return {"present": False, "error": str(error), "controls": []}

    new_titles = [t for t in after if t not in before]
    if len(after) <= len(before) and not new_titles:
        # Cmd+, did nothing: no Settings window for this app.
        return {"present": False, "controls": []}

    read_script = asc.driver(
        asc.DUMP_CONTROLS_HANDLER,
        body=f"""
tell application "System Events"
	tell process "{process}"
		set w to front window
		return my dumpControls(w, 0, 6)
	end tell
end tell
""",
    )
    try:
        raw = run_osascript(read_script, timeout=20)
        controls_error = None
    except RuntimeError as error:
        raw = ""
        controls_error = str(error)

    # We only reach this point once we have confirmed a genuinely new
    # window opened (see `new_titles` above) and it is now frontmost, so
    # Cmd+W is guaranteed to target it rather than some unrelated window.
    close_script = f"""
tell application "System Events"
	tell process "{process}"
		set frontmost to true
		delay 0.2
		keystroke "w" using {{command down}}
	end tell
end tell
"""
    try:
        run_osascript(close_script, timeout=10)
    except RuntimeError as error:
        print(f"could not close {process}'s Settings window: {error}", file=sys.stderr)

    controls = [
        {"depth": int(line.split("\t")[0]), "role": line.split("\t")[1], "label": line.split("\t")[2]}
        for line in raw.splitlines()
        if line
    ]
    result = {"present": True, "controls": controls}
    if controls_error:
        result["error"] = controls_error
    return result


# ---------------------------------------------------------------------------
# Toolbar
# ---------------------------------------------------------------------------


def dump_toolbar(process: str) -> dict:
    script = asc.driver(
        asc.DUMP_CONTROLS_HANDLER,
        body=f"""
tell application "System Events"
	tell process "{process}"
		set winCount to count of windows
		if winCount is 0 then return ""
		set w to window 1
		try
			set tb to toolbar 1 of w
		on error
			return ""
		end try
		return my dumpControls(tb, 0, 2)
	end tell
end tell
""",
    )
    try:
        raw = run_osascript(script, timeout=20)
    except RuntimeError as error:
        return {"present": False, "error": str(error), "items": []}
    items = [
        {"depth": int(line.split("\t")[0]), "role": line.split("\t")[1], "label": line.split("\t")[2]}
        for line in raw.splitlines()
        if line
    ]
    return {"present": bool(items), "items": items}


# ---------------------------------------------------------------------------
# Finder context menus
# ---------------------------------------------------------------------------


#: Locates the list view's outline/table from a Finder window, as a chain
#: of UI-element indices confirmed against macOS 26.2: window -> first
#: split group -> its 3rd child (the content split group) -> its 1st
#: child (the content scroll area) -> its 1st child (the AXOutline).
_FINDER_TABLE_PATH = """
set lvl1 to UI elements of w
set sg to item 1 of lvl1
set lvl2 to UI elements of sg
set sg2 to item 3 of lvl2
set lvl3 to UI elements of sg2
set scrollArea to item 1 of lvl3
set tbl to item 1 of (UI elements of scrollArea)
"""

# Reads the first AXMenu found anywhere under `w` after AXShowMenu fires.
# AXShowMenu's resulting menu is not a direct child of the row/table that
# requested it; it only turns up via a full-tree walk (confirmed against
# macOS 26.2). The first AXMenu in document order is the context menu
# itself; later ones are its own submenus (Open With, Quick Actions) and,
# incidentally, the toolbar's own "Customise…" menu, which the caller's
# `dumpMenuItems` already represents as a nested child read from the right
# starting node.
_READ_FIRST_MENU = """
delay 0.5
set allContents to entire contents of w
set menuList to {}
repeat with el in allContents
	try
		if role of el is "AXMenu" then set end of menuList to el
	end try
end repeat
set out to ""
if (count of menuList) > 0 then
	set targetMenu to item 1 of menuList
	set out to my dumpMenuItems(targetMenu, 0)
end if
key code 53
return out
"""


def dump_finder_context_menus() -> dict:
    sandbox = SCRATCH / "finder-context-sandbox"
    if sandbox.exists():
        import shutil

        shutil.rmtree(sandbox)
    sandbox.mkdir(parents=True)
    (sandbox / "sample.txt").write_text("inventory fixture\n")
    (sandbox / "Subfolder").mkdir()

    open_script = f"""
tell application "Finder"
	activate
	set targetFolder to (POSIX file "{sandbox}") as alias
	open targetFolder
	delay 0.5
	set the bounds of the front window to {{100, 100, 900, 700}}
end tell
tell application "System Events"
	tell process "Finder"
		set frontmost to true
		delay 0.3
		keystroke "2" using {{command down}}
	end tell
end tell
delay 0.5
"""
    run_osascript(open_script, timeout=15)

    menus: dict[str, list[dict]] = {}
    for label, file_name in (("file", "sample.txt"), ("folder", "Subfolder")):
        script = asc.driver(
            asc.DUMP_MENU_ITEMS_HANDLER,
            body=f"""
tell application "System Events"
	tell process "Finder"
		set w to front window
		{_FINDER_TABLE_PATH}
		set target to missing value
		repeat with r in (UI elements of tbl)
			if role of r is "AXRow" then
				set cellKids to UI elements of (item 1 of (UI elements of r))
				repeat with k in cellKids
					if role of k is "AXTextField" then
						try
							if value of k is "{file_name}" then set target to r
						end try
					end if
				end repeat
			end if
		end repeat
		if target is missing value then return ""
		select target
		delay 0.4
		perform action "AXShowMenu" of tbl
		{_READ_FIRST_MENU}
	end tell
end tell
""",
        )
        try:
            raw = run_osascript(script, timeout=20)
            menus[label] = parse_menu_dump(raw)
        except RuntimeError as error:
            menus[label] = []
            print(f"finder context menu ({label}) failed: {error}", file=sys.stderr)
            run_osascript('tell application "System Events" to key code 53', timeout=5)

    # Best effort: on macOS 26.2, asking AXShowMenu for the background menu
    # (no row selected) sometimes still answers with a single-item menu
    # instead of the true empty-selection menu, apparently because nothing
    # ever performed a real mouse-down to update the table's internal
    # "clicked row". Recorded as-is; treat a background result that looks
    # like a selection menu (has "Rename"/"Move to Bin") with suspicion.
    background_script = asc.driver(
        asc.DUMP_MENU_ITEMS_HANDLER,
        body=f"""
tell application "System Events"
	tell process "Finder"
		set w to front window
		{_FINDER_TABLE_PATH}
		set value of attribute "AXSelectedRows" of tbl to {{}}
		delay 0.3
		perform action "AXShowMenu" of tbl
		{_READ_FIRST_MENU}
	end tell
end tell
""",
    )
    try:
        raw = run_osascript(background_script, timeout=20)
        background_rows = parse_menu_dump(raw)
        top_labels = {r["label"] for r in background_rows if r["label"]}
        # Telltale signs AXShowMenu answered with a per-item menu instead
        # of the true empty-selection one (observed on macOS 26.2: clearing
        # AXSelectedRows does not reliably reset the table's internal
        # "last clicked row" that AXShowMenu actually consults). Recording
        # this as the background menu would corrupt the diff, so drop it
        # instead of reporting it as ground truth.
        if {"Rename", "Move to Bin", "Duplicate"} & top_labels:
            menus["background"] = []
            print(
                "finder context menu (background): AXShowMenu answered with a "
                "selection menu instead of the empty-selection one; discarded "
                "(known macOS 26.2 AX quirk, see mac_inventory.py)",
                file=sys.stderr,
            )
        else:
            menus["background"] = background_rows
    except RuntimeError as error:
        menus["background"] = []
        print(f"finder context menu (background) failed: {error}", file=sys.stderr)
        run_osascript('tell application "System Events" to key code 53', timeout=5)

    close_script = """
tell application "System Events"
	tell process "Finder"
		set frontmost to true
		delay 0.2
		keystroke "w" using {command down}
	end tell
end tell
"""
    try:
        run_osascript(close_script, timeout=10)
    except RuntimeError as error:
        print(f"could not close the Finder sandbox window: {error}", file=sys.stderr)

    import shutil

    shutil.rmtree(sandbox, ignore_errors=True)
    return menus


# ---------------------------------------------------------------------------
# System Settings sidebar + panes
# ---------------------------------------------------------------------------


# Best-effort label for a sidebar row: a plain AXStaticText child, or one
# nested one level deeper inside an AXCell (NSOutlineView rows are laid out
# either way depending on the app). Falls back to "".
_ROW_LABEL_HANDLER = r"""
on rowLabel(r)
	tell application "System Events"
		try
			return name of (static text 1 of r)
		end try
		try
			return value of (static text 1 of r)
		end try
		try
			set kids to UI elements of r
			repeat with kid in kids
				try
					set kkids to UI elements of kid
					repeat with kkid in kkids
						if role of kkid is "AXStaticText" then
							try
								return name of kkid
							on error
								return value of kkid
							end try
						end if
					end repeat
				end try
			end repeat
		end try
	end tell
	return ""
end rowLabel
"""


def dump_system_settings() -> dict:
    # The sidebar is an AXOutline/AXTable somewhere under the window, behind
    # an unknown number of split groups/scroll areas; `findList` searches
    # for it instead of assuming a fixed path (unverified against a live
    # run as of this writing — mac_inventory.py's Finder-specific paths
    # were confirmed against macOS 26.2 interactively, this one was not;
    # check tests/inventory/mac/System Settings.json's `sidebar` after the
    # next run and fix the search here if it is still empty).
    sidebar_script = asc.driver(
        asc.FIND_LIST_HANDLER,
        _ROW_LABEL_HANDLER,
        body="""
tell application "System Events"
	tell process "System Settings"
		set w to front window
		set theList to my findList(w, 0, 10)
		if theList is missing value then return ""
		set out to ""
		repeat with r in (rows of theList)
			set out to out & (my rowLabel(r)) & linefeed
		end repeat
		return out
	end tell
end tell
""",
    )
    try:
        raw = run_osascript(sidebar_script, timeout=20)
    except RuntimeError as error:
        return {"sidebar": [], "error": str(error)}
    sidebar = [line for line in raw.splitlines() if line]
    panes = {}
    for row_name in sidebar:
        select_script = asc.driver(
            asc.FIND_LIST_HANDLER,
            _ROW_LABEL_HANDLER,
            asc.DUMP_CONTROLS_HANDLER,
            body=f"""
tell application "System Events"
	tell process "System Settings"
		set w to front window
		set theList to my findList(w, 0, 10)
		if theList is missing value then return ""
		repeat with r in (rows of theList)
			if (my rowLabel(r)) is "{row_name}" then
				select r
				delay 0.4
				exit repeat
			end if
		end repeat
		return my dumpControls(w, 0, 4)
	end tell
end tell
""",
        )
        try:
            raw_pane = run_osascript(select_script, timeout=20)
        except RuntimeError as error:
            panes[row_name] = {"error": str(error)}
            continue
        labels = []
        for line in raw_pane.splitlines():
            if not line:
                continue
            parts = line.split("\t")
            label = parts[2] if len(parts) > 2 else ""
            if label and label not in labels:
                labels.append(label)
        panes[row_name] = labels
    return {"sidebar": sidebar, "panes": panes}


# ---------------------------------------------------------------------------
# Driver
# ---------------------------------------------------------------------------


def inventory_app(app_name: str, info: dict) -> dict:
    process = info["process"]
    already_running = is_running(process)
    launched_by_us = False
    if not already_running:
        print(f"launching {app_name} ...")
        launch(app_name)
        if not wait_until_running(process):
            # It may still have launched under a process name this script
            # did not anticipate; try to quit it by app name too so we
            # never leave a window open behind us.
            quit_app(app_name)
            return {"app": app_name, "error": f"{app_name} did not launch in time"}
        launched_by_us = True
    else:
        print(f"{app_name} already running, reusing it")

    data: dict = {"app": app_name, "source": "mac-accessibility-tree"}
    try:
        data["menu_bar"] = dump_menu_bar(process)
    except RuntimeError as error:
        data["menu_bar_error"] = str(error)

    try:
        data["toolbar"] = dump_toolbar(process)
    except RuntimeError as error:
        data["toolbar"] = {"present": False, "error": str(error)}

    try:
        data["settings"] = dump_settings_window(process)
    except RuntimeError as error:
        data["settings"] = {"present": False, "error": str(error)}

    if app_name == "Finder":
        try:
            data["context_menus"] = dump_finder_context_menus()
        except RuntimeError as error:
            data["context_menus"] = {"error": str(error)}

    if app_name == "System Settings":
        try:
            data["settings_app"] = dump_system_settings()
        except RuntimeError as error:
            data["settings_app"] = {"error": str(error)}

    if launched_by_us and not info.get("always_running"):
        print(f"quitting {app_name} (we launched it)")
        quit_app(process)

    return data


def main() -> int:
    only = sys.argv[1:] or list(APPS)
    unknown = [name for name in only if name not in APPS]
    if unknown:
        print(f"unknown app(s): {unknown}; choose from {list(APPS)}", file=sys.stderr)
        return 2

    if screen_is_locked():
        print("the screen is locked; aborting without touching the GUI", file=sys.stderr)
        return 1

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    acquire_gui_lock()
    try:
        for app_name in only:
            if screen_is_locked():
                print("screen locked mid-run; stopping", file=sys.stderr)
                return 1
            print(f"=== {app_name} ===")
            info = APPS[app_name]
            was_running_before = is_running(info["process"])
            try:
                data = inventory_app(app_name, info)
            except Exception as error:  # noqa: BLE001 - keep going per app
                data = {"app": app_name, "error": f"{type(error).__name__}: {error}"}
                # inventory_app's own `finally`-less body did not get to
                # its closing `quit_app` call; if we are the reason this
                # app is running, do not leave it open behind us.
                if not was_running_before and not info.get("always_running"):
                    quit_app(info["process"])
            out_path = OUT_DIR / f"{app_name}.json"
            out_path.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n")
            print(f"wrote {out_path.relative_to(REPO_ROOT)}")
    finally:
        release_gui_lock()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
