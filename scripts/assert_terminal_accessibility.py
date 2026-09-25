#!/usr/bin/env python3
"""Assert Terminal publishes its screen text and caret over AT-SPI (ACC-01).

Run against one rmac-terminal started with temporary XDG_* directories and a
SHELL that prints RMAC_TERMINAL_MARKER and then shows the prompt
RMAC_TERMINAL_PROMPT (see scripts/linux/run-content-accessibility.sh). Checks:

- the grid is one `terminal` node with an AT-SPI Text interface;
- its text holds the marker, one line per screen line;
- the caret sits right after the prompt, at the live insertion point;
- line navigation (TEXT_BOUNDARY_LINE_START) returns one line, not the
  whole screen.
"""

from __future__ import annotations

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import atspi_assert_support as support  # noqa: E402

APP = os.environ.get("RMAC_TERMINAL_APP", "rmac-terminal")
MARKER = os.environ.get("RMAC_TERMINAL_MARKER", "rmac-a11y-marker")
PROMPT = os.environ.get("RMAC_TERMINAL_PROMPT", "a11y$ ")
DUMP = os.environ.get("RMAC_A11Y_DUMP")


def terminal_node():
    app = support.find_app(APP)
    if app is None:
        return None
    for node in support.nodes_with(app, "terminal"):
        if support.has_text(node):
            text, _, _ = support.text_of(node)
            if MARKER in text and PROMPT in text:
                return node
    return None


terminal = support.wait_for(terminal_node, f"a terminal node showing {MARKER!r} and the prompt")
if DUMP:
    support.dump(support.find_app(APP), DUMP)

text, caret, selections = support.text_of(terminal)
assert support.name(terminal) == "Terminal", f"terminal named {support.name(terminal)!r}"
assert 0 <= caret <= len(text), f"caret {caret} outside text of {len(text)} characters"
before_caret = text[:caret]
assert before_caret.endswith(PROMPT), (
    f"caret {caret} is not right after the prompt: ...{before_caret[-20:]!r}"
)
marker_line, start, end = support.line_at(terminal, text.index(MARKER))
assert marker_line.rstrip("\n") == MARKER, f"marker line reads {marker_line!r}"
assert text[start:end] == marker_line, "line offsets disagree with the text"
prompt_line, _, _ = support.line_at(terminal, caret)
assert prompt_line.startswith(PROMPT), f"caret line reads {prompt_line!r}"
lines = text.count("\n") + 1
assert lines > 2, f"expected one line per screen row, got {lines}"
assert not selections, f"no selection was made, but AT-SPI reports {selections}"

print(
    "AT-SPI Terminal: "
    f"{len(text)} characters over {lines} lines, caret {caret} after the prompt, "
    f"line navigation returns single lines, focused={'focused' in support.states(terminal)}"
)
