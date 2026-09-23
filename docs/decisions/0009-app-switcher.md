# ADR 0009 — ⌘Tab is a per-application rmac switcher, not niri's window MRU

- **Status:** accepted 2026-09-23.
- **Scope:** `shell/bins/rmac-app-switcher`, the `Mod+Tab` binds in
  `packaging/rmac-session/shell.kdl`, and `rmac-app-switcher.service`.

## The question

macOS ⌘Tab switches **applications**: hold ⌘, tap Tab to walk a row of app icons in
most-recently-used order, release ⌘ to bring the chosen app (all of its windows) forward. While
the row is up, ⌘Q quits and ⌘H hides the selected app. niri 26.04's `recent-windows` switcher is
per **window** and draws window previews, so it cannot match. How does rmac detect "⌘ held, then
released" on niri, and where does the switcher live?

## Decision

1. **A resident service plus a one-word dispatch command.** `rmac-app-switcher --service` runs
   under systemd like the OSD. It keeps a live compositor model (`rmac_compositor_niri::watch`),
   the application catalog (`rmac_apps::discover`), and its own app-activation order. niri binds
   `Mod+Tab` / `Mod+Shift+Tab` to `spawn rmac-app-switcher next|previous`. The spawned process
   sends that word over a user-private datagram socket
   (`$XDG_RUNTIME_DIR/rmac/app-switcher.sock`) and exits. So each press costs an exec and one
   datagram. It never starts a GPUI app or reads the window list per press.
2. **Key-hold detection comes from the overlay's own keyboard focus.** On the first `next`, the
   service maps an overlay layer-shell surface with `KeyboardInteractivity::Exclusive`. Because
   that surface has keyboard focus, the compositor sends it `wl_keyboard.modifiers`. The surface
   receives ⌘ as held on enter and cleared on release. That is GPUI's `ModifiersChanged`. Release
   of ⌘ activates the selection. Further ⌘Tab presses are still consumed by niri's bind and
   arrive as `next`. Tab, arrows, Esc, Return, ⌘Q, and ⌘H arriving as key events are handled on
   the surface too, so a compositor that forwards the key instead still works.
3. **Races are closed explicitly.**
   - A quick ⌘Tab tap can release ⌘ before the surface gets focus. The enter event then reports
     no modifiers and no change event fires. 80 ms after focus, the view checks
     `window.modifiers()`, and if ⌘ is already up it switches immediately.
   - A tap must not flash the panel. The surface maps at 1 × 1 and only grows to the panel size
     after 120 ms.
   - An invisible exclusive surface must never hold the keyboard. It closes if it has not gained
     focus within 1 s, and it cancels if it loses focus.
4. **Order and activation are pure, tested model code** (`src/model.rs`):
   - **Grouping:** windows group by app id.
   - **Order:** the focused app comes first. Next come apps this session saw activated, in order.
     Everything else follows by niri's per-window focus timestamps. The shell's own helper
     windows (`dev.rmac.*`, Launcher, Quick Settings, Notification Center) are excluded.
   - **Selection:** it starts on the second app, or on the last app for ⌘⇧Tab.
   - **Activating an app** restores a hidden (parked) app to its recorded workspaces. Otherwise it
     focuses every window on the same workspace as the app's most recent window, finishing on that
     window. niri raises a floating window when it is focused, so the whole app comes forward.
   - **⌘H** parks the app's visible windows through `ParkingStore`, exactly like the menu bar's
     Hide.
   - **⌘Q** closes all of the app's windows, like the menu bar's Quit.
5. **⌘` stays niri's.** Cycling the current app's windows is what niri's `recent-windows` already
   does with `filter="app-id"`. Listing only `Mod+grave` / `Mod+Shift+grave` there also replaces
   niri's default Alt/Super+Tab window switcher.

## Alternatives rejected

- **niri `recent-windows` (status quo).** It is per window, shows previews, and has no Quit or
  Hide. It can't be made to look or behave like the Mac.
- **Spawning a fresh GPUI process per ⌘Tab.** Startup and the first catalog/compositor read would
  put hundreds of milliseconds between the press and the panel, so the release would often land
  before the window existed.
- **Reading the keyboard directly (evdev / `libinput` in the service).** It needs `input`-group
  access, bypasses the compositor's own key handling (and the lock screen), and duplicates what
  the Wayland focus model already delivers to a focused surface.
- **Polling niri for modifier state.** niri's IPC exposes no keyboard modifier state.
- **Folding the switcher into the Dock process.** A crash in either would take down both, and the
  Dock's per-output surfaces don't need exclusive keyboard focus. Separate crash domains are the
  session supervisor's rule (`docs/session-supervisor.md`).

## Consequences

- One more resident component (`rmac-app-switcher.service`). It is non-essential: if it exhausts
  its restart budget, the rest of the desktop keeps running and only ⌘Tab stops working.
- The switcher's look is measured from the owner's Mac. The panel is 176 pt tall with a 56 pt
  radius, 128 pt icons, a 6 pt gap, and 24 pt padding. The selection plate is 120 pt with a 32 pt
  radius. The name label is 13 pt semibold under the selected icon.
  `design-lab/switcher-osd.html` holds the numbers. niri's layer rule gives the surface its blur
  and the matching corner radius.
- If a future niri version stops routing bound keys past an exclusive layer surface, the
  in-surface Tab handling becomes the path. No design change is needed.
