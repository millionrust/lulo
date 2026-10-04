"""A tiny parser for the `item!`/`submenu!` macro grammar used by
`crates/rmac-app-menu/src/lib.rs` to declare each first-party app's static
menu-bar table (`MenuSpec`/`ItemSpec`).

This is not a general Rust parser. It only understands the shapes those two
macros are invoked with:

    item!(LABEL, ACTION, SHORTCUT)
    item!(LABEL, ACTION, SHORTCUT, separator)
    submenu!(LABEL, ACTION, [CHILD, CHILD, ...])
    submenu!(LABEL, ACTION, [CHILD, CHILD, ...], separator)

and the `MenuSpec { label: ..., items: &[ ... ] }` entries of a
`const X_MENUS: &[MenuSpec] = &[ ... ];` table.
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field, replace


@dataclass
class MenuItem:
    label: str
    action: str
    shortcut: str
    separator_before: bool
    children: list["MenuItem"] = field(default_factory=list)

    def to_dict(self) -> dict:
        return {
            "label": self.label,
            "action": self.action,
            "shortcut": self.shortcut,
            "separator_before": self.separator_before,
            "children": [c.to_dict() for c in self.children],
        }


@dataclass
class Menu:
    label: str
    items: list[MenuItem]

    def to_dict(self) -> dict:
        return {"label": self.label, "items": [i.to_dict() for i in self.items]}


def _strip_literal(text: str) -> str:
    text = text.strip()
    if text.startswith('"') and text.endswith('"'):
        inner = text[1:-1]
        # Un-escape the handful of Rust string escapes a menu label,
        # action name or shortcut glyph could plausibly use (e.g. the
        # literal "\\" a Finder ▸ View ▸ Show All Tabs shortcut (⇧⌘\)
        # needs in valid Rust source) — a naive quote-strip would instead
        # compare the raw two-character "\\\\" against the Mac's single
        # backslash and report a false shortcut mismatch.
        return (
            inner.replace('\\"', '"')
            .replace("\\n", "\n")
            .replace("\\t", "\t")
            .replace("\\\\", "\\")
        )
    return text


def split_top_level(s: str, sep: str = ",") -> list[str]:
    """Split `s` on `sep`, ignoring separators nested inside (), [], {} or a
    string literal."""
    parts: list[str] = []
    depth = 0
    in_string = False
    current = []
    i = 0
    while i < len(s):
        ch = s[i]
        if in_string:
            current.append(ch)
            if ch == "\\":
                i += 1
                if i < len(s):
                    current.append(s[i])
            elif ch == '"':
                in_string = False
            i += 1
            continue
        if ch == '"':
            in_string = True
            current.append(ch)
        elif ch in "([{":
            depth += 1
            current.append(ch)
        elif ch in ")]}":
            depth -= 1
            current.append(ch)
        elif ch == sep and depth == 0:
            parts.append("".join(current))
            current = []
        else:
            current.append(ch)
        i += 1
    parts.append("".join(current))
    return [p for p in parts if p.strip() != ""]


def _find_matching_paren(s: str, open_idx: int) -> int:
    """`s[open_idx]` must be an opening bracket; return the index of its
    match."""
    opens = "([{"
    closes = ")]}"
    pair = dict(zip(opens, closes))
    want = pair[s[open_idx]]
    depth = 0
    in_string = False
    i = open_idx
    while i < len(s):
        ch = s[i]
        if in_string:
            if ch == "\\":
                i += 2
                continue
            if ch == '"':
                in_string = False
            i += 1
            continue
        if ch == '"':
            in_string = True
        elif ch in opens:
            depth += 1
        elif ch in closes:
            depth -= 1
            if depth == 0 and ch == want:
                return i
        i += 1
    raise ValueError("unbalanced brackets")


def _spelling_and_grammar_submenu(prefix: str) -> MenuItem:
    """Mirrors `spelling_and_grammar_submenu!` in `crates/rmac-app-menu/src/lib.rs`."""
    return MenuItem(
        "Spelling and Grammar",
        f"{prefix}::SpellingAndGrammarMenu",
        "",
        False,
        [
            MenuItem("Show Spelling and Grammar", f"{prefix}::ShowSpellingAndGrammar", "⌘:", False, []),
            MenuItem("Check Document Now", f"{prefix}::CheckDocumentNow", "⌘;", False, []),
            MenuItem("Check Spelling While Typing", f"{prefix}::ToggleCheckSpellingWhileTyping", "", True, []),
            MenuItem("Check Grammar With Spelling", f"{prefix}::ToggleCheckGrammarWithSpelling", "", False, []),
            MenuItem(
                "Correct Spelling Automatically",
                f"{prefix}::ToggleCorrectSpellingAutomatically",
                "",
                False,
                [],
            ),
        ],
    )


def _transformations_submenu(prefix: str) -> MenuItem:
    """Mirrors `transformations_submenu!` in `crates/rmac-app-menu/src/lib.rs`."""
    return MenuItem(
        "Transformations",
        f"{prefix}::TransformationsMenu",
        "",
        False,
        [
            MenuItem("Make Uppercase", f"{prefix}::TransformUppercase", "", False, []),
            MenuItem("Make Lowercase", f"{prefix}::TransformLowercase", "", False, []),
            MenuItem("Capitalise", f"{prefix}::TransformCapitalise", "", False, []),
        ],
    )


def _speech_submenu(prefix: str) -> MenuItem:
    """Mirrors `speech_submenu!` in `crates/rmac-app-menu/src/lib.rs`."""
    return MenuItem(
        "Speech",
        f"{prefix}::SpeechMenu",
        "",
        False,
        [
            MenuItem("Start Speaking", f"{prefix}::StartSpeaking", "", False, []),
            MenuItem("Stop Speaking", f"{prefix}::StopSpeaking", "", False, []),
        ],
    )


def _substitutions_submenu_with_master_toggle(prefix: str) -> MenuItem:
    """Mirrors `substitutions_submenu_with_master_toggle!`."""
    return MenuItem(
        "Substitutions",
        f"{prefix}::SubstitutionsMenu",
        "",
        False,
        [
            MenuItem("Smart Substitutions", f"{prefix}::ToggleSmartSubstitutions", "", False, []),
            MenuItem("Smart Copy/Paste", f"{prefix}::ToggleSmartCopyPaste", "", True, []),
            MenuItem("Smart Quotes", f"{prefix}::ToggleSmartQuotes", "", False, []),
            MenuItem("Smart Dashes", f"{prefix}::ToggleSmartDashes", "", False, []),
            MenuItem("Smart Links", f"{prefix}::ToggleSmartLinks", "", False, []),
            MenuItem("Text Replacement", f"{prefix}::ToggleTextReplacement", "", False, []),
        ],
    )


def _substitutions_submenu_with_lists_and_tags(prefix: str) -> MenuItem:
    """Mirrors `substitutions_submenu_with_lists_and_tags!`."""
    return MenuItem(
        "Substitutions",
        f"{prefix}::SubstitutionsMenu",
        "",
        False,
        [
            MenuItem("Show Substitutions", f"{prefix}::ShowSubstitutions", "", False, []),
            MenuItem("Smart Copy/Paste", f"{prefix}::ToggleSmartCopyPaste", "", True, []),
            MenuItem("Smart Quotes", f"{prefix}::ToggleSmartQuotes", "", False, []),
            MenuItem("Smart Lists", f"{prefix}::ToggleSmartLists", "", False, []),
            MenuItem("Smart Dashes", f"{prefix}::ToggleSmartDashes", "", False, []),
            MenuItem("Smart Links", f"{prefix}::ToggleSmartLinks", "", False, []),
            MenuItem("Smart Tags", f"{prefix}::ToggleSmartTags", "", False, []),
            MenuItem("Text Replacement", f"{prefix}::ToggleTextReplacement", "", False, []),
        ],
    )


def _substitutions_submenu_with_data_detectors(prefix: str) -> MenuItem:
    """Mirrors `substitutions_submenu_with_data_detectors!`."""
    return MenuItem(
        "Substitutions",
        f"{prefix}::SubstitutionsMenu",
        "",
        False,
        [
            MenuItem("Show Substitutions", f"{prefix}::ShowSubstitutions", "", False, []),
            MenuItem("Smart Copy/Paste", f"{prefix}::ToggleSmartCopyPaste", "", True, []),
            MenuItem("Smart Quotes", f"{prefix}::ToggleSmartQuotes", "", False, []),
            MenuItem("Smart Dashes", f"{prefix}::ToggleSmartDashes", "", False, []),
            MenuItem("Smart Links", f"{prefix}::ToggleSmartLinks", "", False, []),
            MenuItem("Data Detectors", f"{prefix}::ToggleDataDetectors", "", False, []),
            MenuItem("Text Replacement", f"{prefix}::ToggleTextReplacement", "", False, []),
        ],
    )


# Shared Edit-menu submenu macros defined once in `rmac-app-menu/src/lib.rs`
# and used by several apps' static tables (see the comment above them
# there). Each takes the app's action-namespace prefix as its first
# argument and expands to a fixed `MenuItem` tree; keep these in sync with
# that file when a label, shortcut or shape changes.
_SHARED_SUBMENU_BUILDERS = {
    "spelling_and_grammar_submenu": _spelling_and_grammar_submenu,
    "transformations_submenu": _transformations_submenu,
    "speech_submenu": _speech_submenu,
    "substitutions_submenu_with_master_toggle": _substitutions_submenu_with_master_toggle,
    "substitutions_submenu_with_lists_and_tags": _substitutions_submenu_with_lists_and_tags,
    "substitutions_submenu_with_data_detectors": _substitutions_submenu_with_data_detectors,
}


def parse_macro_call(call: str) -> MenuItem:
    """Parse a single `item!(...)`/`submenu!(...)` invocation, or a call to
    one of the shared submenu macros in `_SHARED_SUBMENU_BUILDERS`."""
    call = call.strip()
    names = "|".join(["item", "submenu", *_SHARED_SUBMENU_BUILDERS])
    m = re.match(rf"^({names})!\s*\(", call)
    if not m:
        raise ValueError(f"not a recognised macro call: {call[:60]!r}")
    kind = m.group(1)
    open_idx = call.index("(", m.end() - 1)
    close_idx = _find_matching_paren(call, open_idx)
    inner = call[open_idx + 1 : close_idx]
    args = split_top_level(inner, ",")

    if kind in _SHARED_SUBMENU_BUILDERS:
        prefix = _strip_literal(args[0])
        built = _SHARED_SUBMENU_BUILDERS[kind](prefix)
        if len(args) > 1 and args[1].strip() == "separator":
            built = replace(built, separator_before=True)
        return built

    if kind == "item":
        label = _strip_literal(args[0])
        action = _strip_literal(args[1])
        shortcut = _strip_literal(args[2]) if len(args) > 2 else ""
        separator = len(args) > 3 and args[3].strip() == "separator"
        return MenuItem(label, action, shortcut, separator, [])

    # submenu!
    label = _strip_literal(args[0])
    action = _strip_literal(args[1])
    children_src = args[2].strip()
    if children_src.startswith("[") and children_src.endswith("]"):
        children_src = children_src[1:-1]
    child_calls = split_top_level(children_src, ",")
    children = [parse_macro_call(c) for c in child_calls if c.strip()]
    separator = len(args) > 3 and args[3].strip() == "separator"
    return MenuItem(label, action, "", separator, children)


def parse_items_array(items_src: str) -> list[MenuItem]:
    """`items_src` is the text between the `&[` and `]` of a `MenuSpec.items`
    field."""
    calls = split_top_level(items_src, ",")
    return [parse_macro_call(c) for c in calls if c.strip()]


def parse_menu_spec_table(table_src: str) -> list[Menu]:
    """`table_src` is the text between the outer `&[` and the final `]` of a
    `const X_MENUS: &[MenuSpec] = &[ ... ];` declaration (i.e. a sequence of
    `MenuSpec { label: ..., items: &[ ... ] }` entries)."""
    entries = split_top_level(table_src, ",")
    menus: list[Menu] = []
    for entry in entries:
        entry = entry.strip()
        if not entry:
            continue
        mm = re.search(r"MenuSpec\s*\{", entry)
        if not mm:
            continue
        brace_idx = entry.index("{", mm.start())
        close_idx = _brace_match(entry, brace_idx)
        body = entry[brace_idx + 1 : close_idx]
        label_m = re.search(r"label\s*:\s*([A-Za-z_][\w:]*|\"[^\"]*\")", body)
        label_raw = label_m.group(1)
        label = _strip_literal(label_raw) if label_raw.startswith('"') else _resolve_const(label_raw)
        items_m = re.search(r"items\s*:\s*&\[", body)
        if not items_m:
            menus.append(Menu(label, []))
            continue
        items_open = body.index("[", items_m.start())
        items_close = _bracket_match(body, items_open)
        items_src = body[items_open + 1 : items_close]
        items = parse_items_array(items_src)
        menus.append(Menu(label, items))
    return menus


def _brace_match(s: str, open_idx: int) -> int:
    depth = 0
    in_string = False
    i = open_idx
    while i < len(s):
        ch = s[i]
        if in_string:
            if ch == "\\":
                i += 2
                continue
            if ch == '"':
                in_string = False
            i += 1
            continue
        if ch == '"':
            in_string = True
        elif ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                return i
        i += 1
    raise ValueError("unbalanced braces")


def _bracket_match(s: str, open_idx: int) -> int:
    depth = 0
    in_string = False
    i = open_idx
    while i < len(s):
        ch = s[i]
        if in_string:
            if ch == "\\":
                i += 2
                continue
            if ch == '"':
                in_string = False
            i += 1
            continue
        if ch == '"':
            in_string = True
        elif ch == "[":
            depth += 1
        elif ch == "]":
            depth -= 1
            if depth == 0:
                return i
        i += 1
    raise ValueError("unbalanced brackets")


# `label: APPLICATION_MENU` / `label: WINDOW_MENU` resolve to these Rust
# constants (see rmac-app-menu/src/lib.rs).
_KNOWN_CONSTS = {
    "APPLICATION_MENU": "Application",
    "WINDOW_MENU": "Window",
}


def _resolve_const(name: str) -> str:
    return _KNOWN_CONSTS.get(name, name)


def strip_line_comments(text: str) -> str:
    """Remove `// ...` line comments, leaving string literals untouched."""
    out = []
    in_string = False
    i = 0
    n = len(text)
    while i < n:
        ch = text[i]
        if in_string:
            out.append(ch)
            if ch == "\\" and i + 1 < n:
                out.append(text[i + 1])
                i += 2
                continue
            if ch == '"':
                in_string = False
            i += 1
            continue
        if ch == '"':
            in_string = True
            out.append(ch)
            i += 1
            continue
        if ch == "/" and i + 1 < n and text[i + 1] == "/":
            while i < n and text[i] != "\n":
                i += 1
            continue
        out.append(ch)
        i += 1
    return "".join(out)


def extract_const_table(source: str, const_name: str) -> str:
    """Return the `&[ ... ]` body text of `const <const_name>: &[MenuSpec] =
    &[ ... ];` from `source`."""
    pattern = re.compile(rf"const\s+{re.escape(const_name)}\s*:\s*&\[MenuSpec\]\s*=\s*&\[")
    m = pattern.search(source)
    if not m:
        raise ValueError(f"const table {const_name} not found")
    open_idx = source.rindex("[", 0, m.end())
    close_idx = _bracket_match(source, open_idx)
    return strip_line_comments(source[open_idx + 1 : close_idx])
