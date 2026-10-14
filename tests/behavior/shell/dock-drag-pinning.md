# Dock drag-and-drop keeping

Run `python3 scripts/behavior/run_dock_drag.py --bin-dir DIR` with `DIR`
holding the built `dock` and `rmac-app-drawer`. The runner starts a private
nested niri with the shipped shell configuration, one Dock and temporary XDG
directories, and sends pointer input only to that compositor. Every check
reads the shell settings the Dock saved (`$XDG_CONFIG_HOME/rmac/shell.json`),
so each one proves that the change persists.

macOS 26 behaviour (docs/dock-behaviour-2026-09-23.md):

1. A running app that is not kept, dragged left across the separator, opens a
   slot among the kept apps under the pointer; released between Files and
   Apps, it is kept second.
2. A kept app dragged past its neighbour's centre moves one place.
3. A kept app dragged about 1.5 tiles off the Dock shows "Remove"; released
   there it is no longer kept, and because it is running it stays in the
   Dock's running apps.
4. An app dragged out of Apps and released between Files and Apps is kept
   second. Dropping an app on the Trash does nothing.

On Windows `scripts/windows/shell_smoke.py` (`check_dock_drag`) makes the
same four drags with SendInput against `lulo-shell`, reading the Dock's tile
trace.
