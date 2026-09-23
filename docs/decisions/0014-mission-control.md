# ADR 0014 — Mission Control, App Exposé, Show Desktop, Spaces and hot corners are an rmac service

- **Status:** accepted 2026-09-23.
- **Scope:** `shell/bins/rmac-mission-control`, the ⌃↑ / ⌃↓ / F11 / ⌃← / ⌃→ binds and the
  `rmac-mission-control` layer rule in `packaging/rmac-session/shell.kdl`,
  `rmac-mission-control.service`, `rmac_compositor::Action::{NameWorkspace, UnnameWorkspace}`,
  `rmac_shell_settings::HotCornerSettings`, and the Hot Corners rows in System Settings ›
  Desktop & Dock.

## The question

⌃↑ used to open niri's overview, which zooms out the vertical workspace strip. It looks nothing
like macOS 26 Mission Control. That shows every window of the current Space, scaled into a grid
over the bare wallpaper, under a Spaces bar with a "Desktop" pill and a **+** button. niri also
owns ⌃← / ⌃→, and those walk into the hidden `rmac-parking` workspace. There is no App Exposé
(⌃↓), no Show Desktop (F11) and no configurable hot corners. `RMAC_COMP_PLAN.md` puts "rmac's own
Mission Control" among the things that need rmac's own compositor. How much of it can niri 26.04
carry today?

## What was measured

The owner's Mac (macOS 26.2, 1470 × 956 pt) was driven through Mission Control, the expanded
Spaces bar, a temporary second Desktop (removed afterwards), App Exposé and Show Desktop. Window
frames were read from the Dock process's accessibility tree, and colours from Retina captures.
`design-lab/mission-control.html` records every number. The main ones:

- The **Spaces bar** is 72 pt collapsed and 164 pt while the pointer is over it. It is filled
  with white at about 5 %, with hairlines on its top and bottom edges.
- The **Desktop pill** is 24 pt tall with its top at 40 pt, filled with white at about 15 %, with
  a 13 pt label. With one Space it reads "Desktop"; with several, "Desktop 1", "Desktop 2", and
  so on.
- **Space thumbnails** are 138 × 90 pt, with their top at 46 pt, on a 170 pt pitch, with 11 pt
  labels. The current Space has a 3 pt #5697F5 ring.
- The **remove button** is a 22 pt ⊗ centred on the hovered thumbnail's top-left corner.
- The **+ button** is a 32 pt circle whose centre is 34 pt from the right edge and 44 pt (or
  90 pt) from the top. It holds a 19 pt plus drawn with 2 pt strokes.
- **Window areas:** Mission Control uses x 20 … W − 20 and y (bar + 40) … H − 104. App Exposé
  uses y 52 … H − 100.
- **Hover:** a 5 pt ring 1 pt outside the window. In Mission Control a light capsule
  (#B6B7BE, 18 pt text) shows the window's title. App Exposé shows 13 pt titles below each
  window instead.
- **App Exposé layout:** one uniform scale, with the spare space shared evenly around each
  window and each row. `model::layout` reproduces the Mac's three measured terminal frames.
- **Backdrop:** the wallpaper is neither dimmed nor blurred, and the menu bar disappears.

## What niri 26.04 can and cannot do

- **Per-window pictures: not possible.** niri implements wlr-screencopy and ext-image-copy-capture
  only with *output* sources. There is no foreign-toplevel capture source. Its
  `screenshot-window` action always writes to the clipboard, which would also pollute clipboard
  history. PipeWire window casting through the Mutter ScreenCast D-Bus API would work, but it
  would mean a PipeWire consumer in the shell.
- **Workspaces:** niri can focus a workspace, move windows to it, and name or unname it. A named
  workspace survives while empty. niri always keeps one unnamed, empty "spare" workspace per
  output.
- **Backdrop:** a layer rule with `background-effect { xray true; blur false }` draws the
  wallpaper behind a surface and ignores the windows under it.
- **Touchpad gestures:** these are built in and cannot be configured. Three-finger vertical
  swipes switch niri workspaces, three-finger horizontal swipes scroll columns, and four-finger
  vertical swipes open niri's overview. niri has no gesture binds, and a client only receives
  pointer gestures over its own surface.

## Decision

1. **One resident service, one word per key.** This follows the app switcher's pattern (ADR
   0009). `rmac-mission-control --service` keeps a live compositor model, the application catalog
   and the shell settings. niri binds ⌃↑ `mission-control`, ⌃↓ `app-windows`, F11
   `show-desktop`, and ⌃← / ⌃→ `previous-space` / `next-space`. Each bind spawns the binary,
   which forwards the word over `$XDG_RUNTIME_DIR/rmac/mission-control.sock`.
2. **Pictures come from one output read before the overlay maps.** `grim -t ppm -o OUTPUT -` reads
   the focused output. Every window that nothing covers is cut out of that picture. Stacking
   order counts floating windows over tiled ones, then focus time, because niri raises a floating
   window when it is focused. A window with anything on top of it is drawn as a plate with its
   app icon, because its screen pixels are not its own. The same picture, scaled down, becomes the
   current Space's thumbnail. The service remembers that thumbnail while the Space's windows stay
   the same. A Space without a picture shows the wallpaper with window plates where its windows
   sit.
3. **The overlay** is an exclusive-keyboard, full-screen layer surface over the menu bar. niri
   backs it with the bare wallpaper (xray), which hides the real windows. The first frame puts
   each picture exactly where its window was. Over 350 ms (the existing motion token), the
   pictures fly to `model::layout` while the Spaces bar slides down. Clicking a window focuses it
   and flies everything back. Clicking empty space, pressing Esc or pressing ⌃↑ again leaves. The
   arrow keys and Return pick a window. The bar grows while hovered, and the windows re-flow into
   the smaller area.
4. **Spaces are niri workspaces, excluding `rmac-parking` and niri's spare workspace.**
   - **+** names the spare workspace `rmac-space-<id>` (the new `NameWorkspace` action), so it
     persists as a new Space. If the user is standing on the spare workspace, it is pinned first
     and the next spare workspace is named once niri creates it.
   - **⊗** moves the Space's windows to the Space before it (or after it, for the first Space),
     then unnames it (the new `UnnameWorkspace` action), so niri deletes it.
   - **⌃← / ⌃→** walk the same list without wrapping, so the parking workspace can no longer be
     reached from the keyboard.
5. **Show Desktop focuses the output's spare, empty workspace**, and F11 again returns. It returns
   only if the user is still looking at the desktop. niri cannot move a group of windows
   off-screen and put them back, so this replaces the Mac's slide-to-the-edges motion.
6. **Hot corners** are `ShellSettings.hot_corners` (top-left, top-right, bottom-left,
   bottom-right). System Settings › Desktop & Dock › Hot Corners edits them.
   - **Offered actions:** Mission Control, Application Windows, Desktop, Notification Centre, Apps
     and Lock Screen. The first three run inside the service. The others go through
     `rmac-shortcut-dispatch`.
   - **Surfaces:** each configured corner of each display gets a transparent 1 × 1 overlay
     surface. The corner fires when the pointer enters it and re-arms when the pointer leaves.
   - **Delay:** there is none. None could be measured without launching Notes on the owner's Mac,
     and niri's own hot corner also fires on arrival (value S).
   - **Omitted:** Quick Note (the Mac's default), the screen saver actions and display sleep have
     no rmac backend, so they are not offered. Every corner defaults to off.

## Alternatives rejected

- **Keep niri's overview.** It cannot draw a Spaces bar, a hover title, App Exposé, or the Mac's
  layout.
- **`screenshot-window` per window.** Every call writes to the clipboard, and the clipboard
  history service would record each capture.
- **PipeWire window casts for live thumbnails.** These are live and per-window. The cost is a
  PipeWire/DMA-BUF consumer, a portal session per window, and a screen-sharing indicator every
  time Mission Control opens. This is the path to take if live thumbnails become the priority.
- **Parking windows for Show Desktop.** Every window would appear in the Dock as a minimized tile.
- **A dimmed or blurred backdrop.** The Mac shows neither. With xray and no blur, niri draws the
  wallpaper pixel for pixel.

## Consequences and known differences

- **Thumbnails are still.** They are pictures from the moment Mission Control opened, not live.
  Opening takes one output read (about a frame or two) longer than on the Mac.
- **The Dock is hidden while the overlay is up.** It shares the Top layer below the overlay. The
  Mac keeps its Dock visible.
- **Mission Control opens on the focused display only.**
- **Gestures remain niri's.** Three-finger left and right swipes do not switch Spaces. niri's
  three-finger vertical swipe still switches niri workspaces, including the parking workspace,
  and its four-finger swipe still opens niri's overview. Fixing this needs the rmac compositor
  (`RMAC_COMP_PLAN.md` M5).
- **Not implemented:** dragging windows between Spaces, full-screen apps as Spaces, App Exposé's
  row of minimized windows (not measured), and the Mac's "Hot Corners…" sheet. The four pop-ups
  sit directly in the pane instead.
- The service is non-essential. If it fails, ⌃↑, ⌃↓, F11 and ⌃← / ⌃→ stop working, and the rest
  of the desktop keeps running.
