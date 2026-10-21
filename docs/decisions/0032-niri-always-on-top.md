# ADR 0032 — Carry a third niri patch so Window ▸ Always on Top is real

- **Status:** accepted 2026-10-20.
- **Amends:** ADR 0021 (the niri patch series), ADR 0007.
- **Scope:** `packaging/third-party/niri/debian/patches/0004-window-always-on-top.patch`,
  `crates/rmac-compositor` (`Action::SetAlwaysOnTop`, `Window::always_on_top`),
  `crates/rmac-compositor-niri`, `crates/rmac-compositor-system` (Windows),
  `shell/bins/rmac-menubar` (Window ▸ Always on Top).

## Context

macOS Calculator's Window menu has Always on Top (CALC-08): the window stays above every other
window until the item is chosen again. niri stacks floating windows by when they were last
raised, and has no way to pin one. The first version of the menu item only raised the window once,
which is not what the item means.

## Decision

1. **niri owns the state.** `0004-window-always-on-top.patch` (about 150 lines):
   - `niri msg action set-window-always-on-top --id N --on true|false` (an IPC action; there is no
     key bind for it);
   - `Window.is_always_on_top` in the IPC window (absent in an unpatched niri; clients default it
     to false, and the event stream re-sends the window when it changes);
   - the stacking rule in `FloatingSpace`: pinned windows keep the front of the stack, in the order
     they were pinned; activating or opening any other window can raise it only as far as the
     first window that is not pinned. A tiled window is made floating first. Lulo's windows all
     float, so this covers every app.
2. **The shell asks and reads, never guesses.** `rmac_compositor::Action::SetAlwaysOnTop { window,
   on }` maps to the niri action; `Window::always_on_top` is the checkmark's state, and the menu
   item toggles the opposite of what it reads.
3. **Windows** uses its own mechanism for the same domain action: `SetWindowPos` with
   `HWND_TOPMOST` / `HWND_NOTOPMOST` (no move, no resize, no activation), and the flag is
   `WS_EX_TOPMOST` on the window.
4. **Only Calculator shows the item**, as on the Mac (`tests/inventory/mac/Calculator.json`); the
   mechanism works for any window.

## Why not a window rule

niri's window rules are matched when a window opens, not toggled by the user, and none pins a
floating window above others.

## Consequences

The patch is rebased onto each niri release with the other two. If niri ever gains an
equivalent, this one is dropped and the adapter maps to it.
