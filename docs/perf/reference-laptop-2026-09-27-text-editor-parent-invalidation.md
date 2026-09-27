# Text Editor parent invalidation probe — 2026-09-27

The installed Text Editor sample reached **40.5% CPU** and **9.300 context
switches/s** during a 30-second private idle window. A same-source comparison
with the parent `EditorView` observer removed measured **39.1% CPU** and
**9.133 context switches/s**. Both runs used one launch, a three-second settle,
private HOME/XDG directories, nested Sway with the pixman software renderer,
and no injected input. The editor body stayed focused so the caret continued
its normal blink.

The parent observer converted every `InputState` notification into a full
`EditorView` notification. The pinned input component notifies on caret blink,
and the observer comment refers to a line/column status that no longer exists
in the current renderer. Removing this stale observer leaves the input's own
redraw and the existing text-change subscription intact. The single before
and after samples differ by only 1.4 percentage points (about 3.5% relative),
with wake-ups nearly unchanged. That reduction is inconclusive and does not
resolve the high idle cost.

The baseline executable was `/home/jacob/rmac-release/inputs-e8b589ac/rmac-text-editor`,
SHA-256 `e9e6c8a46c4128c803b33cf2339069b7341c1a1ab1fd72a65755109c8e7d09b5`,
from source `e8b589ac`. The patched executable was built from `7d348df6` plus
the observer removal, SHA-256
`b2b556eefb7a1d589b8c6e9e32b65adfda8bb14f9ed9f1eb166009a2847b45d8`. The
app source at `7d348df6` otherwise matches the `e8b589ac` source used for this
comparison. Full per-run values and limitations are in
[the JSON report](reference-laptop-2026-09-27-text-editor-parent-invalidation.json).

The paired runs were sequential and isolated. Startup timing, CPU and
context-switch values are exploratory single samples; the private software
renderer differs from the installed desktop. No live input or user data was
used.
