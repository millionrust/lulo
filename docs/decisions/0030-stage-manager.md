# ADR 0030 — Stage Manager on niri

- **Status:** accepted 2026-10-22; built behind its off-by-default switch (see "As built").
- **Scope:** a Stage Manager mode inside `shell/bins/rmac-mission-control` (the resident
  window service, `src/stage.rs`), Control Centre's Stage Manager control, Desktop & Dock's "Desktop & Stage
  Manager" group (docs/parity.md CC-07, DOCK-23), `rmac-compositor` window actions.

## Context

macOS 26's Stage Manager shows the current app's windows ("the stage") in the middle of the
screen and the other recently used apps as a strip of live thumbnails ("sets") down the left
edge. Clicking a set brings its windows to the stage and sends the stage's windows to the strip.
Dragging a thumbnail onto the stage adds that window to the current set. Settings: show recent
apps in Stage Manager, show windows from an app all at once or one at a time, and show items on
the desktop in Stage Manager.

Lulo's windows are niri floating windows. Mission Control already owns everything Stage Manager
needs: the compositor event stream, window frames, screen pictures of windows (MC-01), the
parking workspace (ADR 0021) and floating-frame actions (WIN-01).

## Decision

1. **Sets are parked windows.** A set is a group of windows (an app's windows by default). Only
   the stage's set stays on the Space; the other sets' windows move to the parking workspace,
   exactly as minimising does, so they take no screen and no input. No niri patch is needed: the
   strip is drawn by Lulo, not by moving windows off-screen.

2. **The strip is a Lulo layer surface.** `rmac-mission-control` draws the strip on a left-edge
   layer surface (Top layer, exclusive zone 0, so windows can still go under it) from the window
   pictures it already keeps, tilted as macOS draws them, up to six sets in recency order. It
   updates only on compositor events; nothing polls. With reduced motion (low-spec PCs) the
   pictures are flat and the swaps cut instead of animating.

3. **Swapping a set** restores the chosen set's windows from the parking workspace to the Space
   (their parking records hold where they were) and parks the stage's. The stage keeps a margin
   for the strip: a window that would sit under it is nudged right with the WIN-01 frame actions,
   and its frame is restored when Stage Manager turns off.

4. **The switch** is Control Centre's Stage Manager circle (the slot Low Power stands in for
   today) and Desktop & Dock's switch; both write `stage_manager.enabled` in shell settings, which
   the resident service watches. Turning it off restores every parked set window.

5. **What is left out at first:** grouping windows of different apps into one set by drag (the
   data model allows it), and the desktop-items option (desktop icons already hide under windows).

## Consequences

- Every set swap is two batches of existing compositor actions; Dock minimised tiles must ignore
  windows parked by Stage Manager (the parking record carries the reason).
- Thumbnails are as fresh as the last time the window was on screen, as on macOS.
- Work items: settings field and Control Centre circle; strip surface; set model and swap;
  stage margin; Desktop & Dock group (DOCK-23); a nested scenario that swaps two sets.

## As built (2026-10-22)

- **Sets wait on their own hidden workspace, `rmac-stage`**, not the parking workspace: a set's
  windows are running windows, so the Dock must not show them as minimised and Restore must not
  pick them up. `shell.kdl` names it after `rmac-parking`; `rmac_compositor::is_hidden_workspace`
  keeps both out of Spaces, Mission Control, the window placer and the Dock's restore target.
- **The model** (`stage.rs`, unit-tested) runs on each compositor event while Stage Manager is on:
  when an app's window on a Space takes the focus, the other apps' windows on that Space move to
  `rmac-stage`; when a window waiting there takes the focus (the Dock, Command-Tab), its set moves
  back and the focus follows. Each move is asked once until it lands. A click on a set in the strip
  moves it straight back, so niri never shows the stage workspace. A wallpaper click with Stage
  Manager on sends the stage's set to the strip, as on macOS, and "Click wallpaper to reveal
  desktop" now offers macOS's Always / Only in Stage Manager (a saved "never" reads as the latter).
  Turning it off, or the service starting with it off, brings every waiting window back.
- **The strip** is a Top-layer surface anchored to the left edge, centred between the menu bar
  and the Dock, up to six sets with the most recent at the top: the set's last window picture
  when Mission Control has one, otherwise its app icon on a window plate, with the app's other
  windows peeking out behind. It is redrawn only on compositor and settings events.
- **The switches:** Control Centre's Stage Manager circle (first, as macOS 26 orders it; Low Power
  moves to the end) and Desktop & Dock's "Stage Manager" and "Show recent apps in Stage Manager"
  write `stage_manager` in shell settings. Lulo OS only: Windows keeps its own window management.
- **Not yet:** the stage margin that nudges windows out from under the strip, tilted live
  thumbnails, "Show windows from an application: One at a Time", dragging a thumbnail onto the
  stage to group windows, and showing desktop items in Stage Manager. Choosing a set from the Dock
  or Command-Tab briefly shows the stage workspace before the set comes back (a niri patch that
  focuses without switching workspace would remove it).
