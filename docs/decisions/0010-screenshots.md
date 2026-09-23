# ADR 0010 — Screenshots are an rmac overlay over `grim`, not niri's screenshot UI

- **Status:** accepted 2026-09-23.
- **Scope:** `shell/bins/rmac-screenshot`, the ⇧⌘3/⇧⌘4/⇧⌘5 binds in
  `packaging/rmac-session/shell.kdl`, `rmac-screenshot.service`, and the Control Center
  Screenshot control.

## The question

macOS has three screenshot shortcuts. ⇧⌘3 captures the screen. ⇧⌘4 shows a crosshair with a live
readout (Space switches it to picking a window). ⇧⌘5 opens a toolbar with an adjustable selection
and an Options menu. Each capture plays the shutter sound and shows a floating thumbnail
bottom-right, and the file "Screenshot 2026-09-23 at 1.36.39 PM.png" lands only after the
thumbnail leaves. niri 26.04 has `screenshot`, `screenshot-screen` and `screenshot-window`
actions, but its interactive UI looks nothing like the Mac, it writes straight to
`screenshot-path`, and there is no thumbnail. Which backend should read the pixels, and who
draws the UI?

## Decision

1. **rmac draws every visible part.** `rmac-screenshot --service` is a resident GPUI
   layer-shell service, like the OSD and the app switcher. niri binds
   `Mod+Shift+3|4|5` (and `Mod+Ctrl+Shift+3|4` for the clipboard) and `Print` to
   `spawn rmac-screenshot <word>`. The spawned process sends that word over
   `$XDG_RUNTIME_DIR/rmac/screenshot.sock` and exits. ⇧⌘4 and ⇧⌘5 map one full-output
   overlay surface (`Layer::Overlay`, exclusive zone −1, exclusive keyboard) on the focused
   output. The thumbnail is a separate bottom-right surface without keyboard focus. All
   numbers come from `design-lab/screenshot.html`, which was measured on the owner's Mac.
2. **`grim` reads the pixels through niri's wlr-screencopy.** The overlay is removed first,
   and the service waits 120 ms for niri to present a frame without it. Then it runs
   `grim -o <output>` for a screen, or `grim -g "X,Y WxH"` in niri's global logical
   coordinates for a selection or window. `-c` is added when "Show Mouse Pointer" is on.
   Window rectangles come from niri IPC (`rmac_compositor::window_logical_rect`), which the
   Dock's minimize thumbnails already use with `grim -g`. grim is a small, stable, packaged
   tool (Ubuntu `grim`, installed on the reference laptop). It writes PNG to a path we choose.
   So the file can be staged under `$XDG_RUNTIME_DIR/rmac/screenshots` and handed off only
   after the thumbnail leaves, as macOS does.
3. **niri's own screenshot actions are not used.** They write to `screenshot-path` and copy to
   the clipboard every time, and they have no path argument we can rely on in 26.04. rmac
   could neither delay the file behind the thumbnail nor honour Save to Desktop / Documents /
   Downloads / Clipboard without racing niri's writer. niri's interactive `screenshot` UI is
   the thing being replaced.
4. **The clipboard uses `wl-copy --type image/png`.** GPUI's Linux clipboard cannot write
   images at this revision. The session package now depends on `grim` and `wl-clipboard`.
   If `wl-copy` is missing, the Clipboard destination is hidden.
5. **Files use macOS's exact name.** 12-hour clock without a leading zero, and U+202F before
   AM/PM ("Screenshot 2026-09-23 at 1.36.39 PM.png"). A clash becomes "… (2).png"; an
   existing file is never replaced. Save locations come from `user-dirs.dirs`.
6. **The shutter cue is `rmac_sound::play(Cue::Screenshot)`.** It plays once the capture
   succeeds and follows the Sound settings for interface effects.

## Consequences

- A window capture is the window's on-screen rectangle, not an isolated render with a shadow.
  An overlapping floating window would appear in it.
- The overlay covers only the focused output. On a multi-monitor setup, ⇧⌘4 cannot select on
  another display yet.
- The crosshair is the cursor theme's `crosshair`. GPUI cannot hide the pointer, so window mode
  keeps the arrow instead of the Mac's camera cursor.
- Omitted because rmac has no backend yet: the three record buttons (and the stop button in the
  menu bar); Mail, Preview and Other Location… in Options; dragging the thumbnail into another
  app; and the Markup window that opens on click (rmac opens the saved file with `xdg-open`).
- The timer delays the capture but draws no countdown. The Mac's countdown was not measured.
