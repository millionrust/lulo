"""Behaviour-parity scenarios: loading, portable facts and comparison.

A scenario (tests/behavior/<area>/<name>.json) says what to set up, which
keys to press and which facts to observe. record_mac.py plays it on macOS
and writes <name>.mac.json; run_lulo.py plays it on Lulo inside a nested
compositor and compares against that file. Both sides describe what they
saw in the same portable vocabulary defined here, so the comparison never
has to know which toolkit produced a fact.

Only words and numbers are ever recorded: no captures, no absolute paths.
"""

from __future__ import annotations

import json
import re
from pathlib import Path
from typing import Any, Iterable, Optional

FORMAT = 1
REPO = Path(__file__).resolve().parents[2]
SCENARIO_ROOT = REPO / "tests" / "behavior"

APPS = {"files", "text-editor", "settings", "calculator", "desktop"}
STEP_KINDS = {
    "key",
    "type",
    "wait",
    "observe",
    "menu",
    "context",
    "select",
    "click_key",
    "focus_desktop",
}
FACTS = {"focus", "windows", "dialog", "menu", "selection", "files", "tabs", "display"}

# Mac AX roles and AT-SPI role names, both mapped to one small vocabulary.
AX_ROLES = {
    "AXTextField": "text-field",
    "AXTextArea": "text-area",
    "AXSearchField": "search-field",
    "AXComboBox": "combo-box",
    "AXButton": "button",
    "AXPopUpButton": "pop-up-button",
    "AXMenuButton": "pop-up-button",
    "AXCheckBox": "check-box",
    "AXRadioButton": "radio-button",
    "AXOutline": "list",
    "AXTable": "list",
    "AXList": "list",
    "AXBrowser": "list",
    "AXCollectionView": "list",
    "AXRow": "row",
    "AXCell": "cell",
    "AXStaticText": "text",
    "AXWindow": "window",
    "AXSheet": "dialog",
    "AXGroup": "group",
    "AXScrollArea": "group",
    "AXWebArea": "document",
    "AXSlider": "slider",
    "AXMenu": "menu",
    "AXMenuItem": "menu-item",
    "AXImage": "image",
}
ATSPI_ROLES = {
    "entry": "text-field",
    "password text": "text-field",
    "text": "text-area",
    "editbar": "text-field",
    "document text": "text-area",
    "paragraph": "text-area",
    "combo box": "combo-box",
    "push button": "button",
    "button": "button",
    "toggle button": "check-box",
    "check box": "check-box",
    "radio button": "radio-button",
    "list": "list",
    "list box": "list",
    "table": "list",
    "tree table": "list",
    "tree": "list",
    "tree item": "row",
    "list item": "row",
    "table row": "row",
    "table cell": "cell",
    "label": "text",
    "static": "text",
    "frame": "window",
    "window": "window",
    "dialog": "dialog",
    "alert": "dialog",
    "file chooser": "dialog",
    "panel": "group",
    "filler": "group",
    "section": "group",
    "grouping": "group",
    "scroll pane": "group",
    "document frame": "document",
    "document web": "document",
    "slider": "slider",
    "menu": "menu",
    "popup menu": "menu",
    "menu item": "menu-item",
    "check menu item": "menu-item",
    "radio menu item": "menu-item",
    "image": "image",
    "icon": "image",
}
# Roles that count as the same thing unless a scenario asks for "exact".
ROLE_CLASSES = [
    {"text-field", "text-area", "search-field", "combo-box"},
    {"list", "row", "cell"},
    {"window", "dialog"},
]


class ScenarioError(ValueError):
    pass


def normalize_ax_role(role: Optional[str], subrole: Optional[str] = None) -> Optional[str]:
    if role is None:
        return None
    if subrole == "AXSearchField":
        return "search-field"
    return AX_ROLES.get(role, role.removeprefix("AX").lower())


def normalize_atspi_role(role: Optional[str]) -> Optional[str]:
    if role is None:
        return None
    return ATSPI_ROLES.get(role, role.replace(" ", "-"))


LULO_TITLE_SUFFIXES = re.compile(
    r"( — (Files|Text Editor|System Settings|Settings|Calculator))?( — Edited)?( — (Files|Text Editor|System Settings|Settings|Calculator))?$"
)


def lulo_window_title(title: Optional[str]) -> Optional[str]:
    """The title a Mac AX client would read. Lulo's toplevel titles carry the
    app name ("sandbox — Files") and an edited mark for the Dock, Mission
    Control and the switcher; the Mac's AXTitle has neither (the title bar
    draws "— Edited" separately)."""

    if title is None:
        return None
    return LULO_TITLE_SUFFIXES.sub("", title) or title


def selection_facts(value: Optional[str], start: Optional[int], end: Optional[int]) -> dict[str, Any]:
    """The selection part of a focus fact, derived identically on both sides."""

    if value is None or start is None or end is None:
        return {"selection": None, "selected_text": None, "selected_all": None}
    start, end = sorted((max(0, start), max(0, end)))
    return {
        "selection": [start, end],
        "selected_text": value[start:end],
        "selected_all": bool(value) and start == 0 and end == len(value),
    }


def finish_observation(scenario: dict[str, Any], name: str, facts: dict[str, Any]) -> dict[str, Any]:
    """Apply a scenario's "omit" list and "menu_until" to one observation,
    identically on both platforms."""

    until = scenario.get("menu_until")
    menu = facts.get("menu")
    if until and isinstance(menu, dict) and isinstance(menu.get("items"), list):
        items = menu["items"]
        for index, item in enumerate(items):
            if item == until or item.startswith(until + " ["):
                menu["items"] = items[: index + 1]
                break
    for key in scenario.get("omit", []):
        obs, fact, field = (key.split(".") + [None, None])[:3]
        if obs not in {name, "*"} or fact not in facts:
            continue
        if field is None:
            facts.pop(fact)
        elif isinstance(facts[fact], dict):
            facts[fact].pop(field, None)
    return facts


# --------------------------------------------------------------------------
# Loading
# --------------------------------------------------------------------------


def scenario_paths(root: Path = SCENARIO_ROOT, only: Iterable[str] = ()) -> list[Path]:
    wanted = set(only)
    paths = []
    for path in sorted(root.glob("*/*.json")):
        if path.name.endswith((".mac.json", ".lulo.json")):
            continue
        if wanted and scenario_id(path, root) not in wanted and path.stem not in wanted:
            continue
        paths.append(path)
    return paths


def scenario_id(path: Path, root: Path = SCENARIO_ROOT) -> str:
    return f"{path.parent.name}/{path.stem}"


def expectation_path(path: Path, side: str = "mac") -> Path:
    return path.with_name(f"{path.stem}.{side}.json")


def load(path: Path) -> dict[str, Any]:
    try:
        data = json.loads(path.read_text())
    except json.JSONDecodeError as error:
        raise ScenarioError(f"{path.name}: not valid JSON: {error}") from error
    validate(data, path.name)
    return data


def validate(data: dict[str, Any], label: str = "scenario") -> None:
    if not isinstance(data, dict):
        raise ScenarioError(f"{label}: a scenario is a JSON object")
    for key in ("title", "app", "steps"):
        if key not in data:
            raise ScenarioError(f"{label}: missing {key!r}")
    if data["app"] not in APPS:
        raise ScenarioError(f"{label}: app must be one of {sorted(APPS)}")
    setup = data.get("setup", {})
    for name in setup.get("files", {}):
        if name.startswith("/") or ".." in Path(name).parts:
            raise ScenarioError(f"{label}: setup file {name!r} must stay inside the sandbox")
    names = set()
    for index, step in enumerate(data["steps"]):
        kinds = STEP_KINDS & set(step)
        if len(kinds) != 1:
            raise ScenarioError(f"{label}: step {index} must have exactly one of {sorted(STEP_KINDS)}")
        if "observe" in step:
            name = step["observe"]
            if name in names:
                raise ScenarioError(f"{label}: observation {name!r} appears twice")
            names.add(name)
            facts = step.get("facts", [])
            unknown = set(facts) - FACTS
            if not facts or unknown:
                raise ScenarioError(f"{label}: observation {name!r} needs facts from {sorted(FACTS)}")
    if not names:
        raise ScenarioError(f"{label}: a scenario must observe something")


def mac_keystroke(chord: str) -> tuple[Optional[str], Optional[int], list[str]]:
    """Map a scenario chord to System Events: (character, key code, modifiers)."""

    from wlinput import parse_chord  # the same parser Lulo uses

    mods, evdev = parse_chord(chord)
    mac_mods = [{"cmd": "command down", "shift": "shift down", "alt": "option down",
                 "ctrl": "control down"}[m] for m in mods]
    codes = {1: 53, 14: 51, 15: 48, 28: 36, 57: 49, 102: 115, 103: 126, 104: 116,
             105: 123, 106: 124, 107: 119, 108: 125, 109: 121, 111: 117, 96: 76,
             59: 122, 60: 120, 61: 99, 62: 118, 63: 96}
    if evdev in codes:
        return None, codes[evdev], mac_mods
    from wlinput import KEYCODES

    for char, code in KEYCODES.items():
        if code == evdev and len(char) == 1:
            return char, None, mac_mods
    raise ScenarioError(f"no Mac key for {chord!r}")


# --------------------------------------------------------------------------
# Comparison
# --------------------------------------------------------------------------

# Default rule per fact field. Scenarios override with "tolerance":
# {"<observation>.<fact>.<field>": rule}; "*" matches any observation.
DEFAULT_RULES = {
    "focus.role": "role-class",
    "focus.value": "exact",
    "focus.selection": "exact",
    "focus.selected_text": "exact",
    "focus.selected_all": "exact",
    "focus.label": "ignore",
    "windows.count": "exact",
    "windows.front": "exact",
    "windows.titles": "set",
    "dialog.present": "exact",
    "dialog.title": "text",
    "dialog.texts": "ignore",
    "dialog.buttons": "exact",
    "dialog.default": "exact",
    "menu.items": "exact",
    "selection.items": "set",
    "files.entries": "set",
    "tabs.titles": "exact",
    "display.value": "text",
}
RULES = {"exact", "set", "text", "role-class", "ignore", "subset", "present", "count"}


def _text(value: Any) -> Any:
    if not isinstance(value, str):
        return value
    value = value.replace("’", "'").replace("‘", "'").replace("“", '"').replace("”", '"')
    value = value.replace("…", "...").replace(" ", " ")
    return re.sub(r"\s+", " ", value).strip()


def _same_role(a: Any, b: Any) -> bool:
    if a == b:
        return True
    return any(a in group and b in group for group in ROLE_CLASSES)


def rule_for(tolerance: dict[str, str], observation: str, fact: str, field: str) -> str:
    for key in (f"{observation}.{fact}.{field}", f"*.{fact}.{field}", f"{observation}.{fact}", f"*.{fact}"):
        if key in tolerance:
            return tolerance[key]
    return DEFAULT_RULES.get(f"{fact}.{field}", "exact")


def field_matches(rule: str, expected: Any, actual: Any) -> bool:
    if rule == "ignore":
        return True
    if rule == "present":
        return (expected is None) == (actual is None)
    if rule == "count":
        return len(expected or []) == len(actual or [])
    if rule == "role-class":
        return _same_role(expected, actual)
    if rule == "text":
        return _text(expected) == _text(actual)
    if rule == "set":
        if expected is None or actual is None:
            return expected == actual
        return sorted(map(_text, expected)) == sorted(map(_text, actual))
    if rule == "subset":
        return set(map(_text, expected or [])) <= set(map(_text, actual or []))
    return expected == actual


def compare(scenario: dict[str, Any], expected: dict[str, Any], actual: dict[str, Any]) -> list[dict[str, Any]]:
    """Every field that differs. Fields the Mac did not report are skipped."""

    tolerance = scenario.get("tolerance", {})
    mismatches = []
    exp_obs = expected.get("observations", {})
    act_obs = actual.get("observations", {})
    for name, facts in exp_obs.items():
        got = act_obs.get(name)
        if got is None:
            mismatches.append({"observation": name, "fact": "*", "field": "*",
                               "expected": "observed", "actual": actual.get("error") or "not reached",
                               "rule": "exact"})
            continue
        for fact, fields in facts.items():
            other = got.get(fact)
            if not isinstance(fields, dict):
                continue
            for field, value in fields.items():
                rule = rule_for(tolerance, name, fact, field)
                if rule not in RULES:
                    raise ScenarioError(f"unknown tolerance rule {rule!r}")
                seen = (other or {}).get(field) if isinstance(other, dict) else None
                if not field_matches(rule, value, seen):
                    mismatches.append({"observation": name, "fact": fact, "field": field,
                                       "expected": value, "actual": seen, "rule": rule})
    return mismatches


def describe(value: Any) -> str:
    if value is None:
        return "nothing"
    if isinstance(value, bool):
        return "yes" if value else "no"
    if isinstance(value, list):
        return "[" + ", ".join(describe(v) for v in value) + "]"
    if isinstance(value, dict):
        return "{" + ", ".join(f"{k}: {describe(v)}" for k, v in value.items()) + "}"
    if isinstance(value, str):
        return f"“{value}”"
    return str(value)


def report_lines(sid: str, scenario: dict[str, Any], mismatches: list[dict[str, Any]]) -> list[str]:
    if not mismatches:
        return [f"PASS  {sid}  {scenario['title']}"]
    lines = [f"FAIL  {sid}  {scenario['title']}"]
    for item in mismatches:
        lines.append(
            f"      {item['observation']}.{item['fact']}.{item['field']}: "
            f"Mac {describe(item['expected'])}, Lulo {describe(item['actual'])}"
        )
    return lines


# --------------------------------------------------------------------------
# docs/parity.md rows
# --------------------------------------------------------------------------

AREA_SECTIONS = {
    "files": "### Files",
    "text-editor": "### Text Editor",
    "settings": "### Settings",
    "calculator": "### Calculator",
    "desktop": "### Desktop",
}


def next_ids(parity_text: str, area: str, count: int) -> list[str]:
    heading = AREA_SECTIONS.get(area)
    lines = parity_text.splitlines()
    try:
        start = lines.index(heading)
    except ValueError:
        return [f"{area.upper()}-?"] * count
    section = []
    for line in lines[start + 1:]:
        if line.startswith("### ") or line.startswith("## "):
            break
        section.append(line)
    ids = re.findall(r"^\| ([A-Z]+)-(\d+) \|", "\n".join(section), re.M)
    if not ids:
        return [f"{area.upper()}-?"] * count
    prefix = ids[0][0]
    top = max(int(n) for p, n in ids if p == prefix)
    width = len(ids[0][1])
    return [f"{prefix}-{str(top + i + 1).zfill(width)}" for i in range(count)]


def parity_rows(parity_text: str, failures: list[tuple[str, dict[str, Any], list[dict[str, Any]]]]) -> dict[str, list[str]]:
    """Proposed rows per parity.md section, one per failing scenario, skipping
    scenarios a row already cites by id."""

    by_area: dict[str, list[tuple[str, dict[str, Any], list[dict[str, Any]]]]] = {}
    for sid, scenario, mismatches in failures:
        if f"behavior:{sid}" in parity_text:
            continue
        by_area.setdefault(scenario["app"], []).append((sid, scenario, mismatches))
    rows: dict[str, list[str]] = {}
    for area, items in by_area.items():
        ids = next_ids(parity_text, area, len(items))
        for row_id, (sid, scenario, mismatches) in zip(ids, items):
            mac = "; ".join(f"{m['observation']} {m['fact']} {m['field']} {describe(m['expected'])}" for m in mismatches[:3])
            lulo = "; ".join(f"{describe(m['actual'])}" for m in mismatches[:3])
            gap = f"{scenario['title']}. Mac: {mac}. / Lulo: {lulo}.".replace("|", "/")
            rows.setdefault(AREA_SECTIONS.get(area, area), []).append(
                f"| {row_id} | P1 | S | Missing | {gap} | `tests/behavior/{sid}.json` (behavior:{sid}) |"
            )
    return rows
