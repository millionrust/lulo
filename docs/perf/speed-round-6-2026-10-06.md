# Speed round 6 — 2026-10-06

Mission Control (SPEED-03, 64–69 → 43–47 ms) is recorded separately in
[speed-round-6-mission-control-2026-10-06.md](speed-round-6-mission-control-2026-10-06.md).
This page is Settings (SPEED-02).

Branch `op/speed-6` on `integ` 9f60488b. Release profile (fat LTO) for the
changed binary only (`rmac-system-settings`), before = integ and after =
this branch, built back to back on the shared target. `run_speed_sweep.py
--only settings --only settings-panes` on a 1920×1080 nested output on the
reference laptop, 5 launches per sweep, two sweeps per side, alternating
before/after, run while holding the shared build lock so no build could
start mid-sweep (`ps` before the first sweep: nothing but packagekitd
above 1 % CPU). The other binaries the nested session needs came from the
installed packages, the same for both sides.

## Changes (SPEED-02)

1. **Sidebar and detail pane are cached GPUI views** (`f8c77f5d`).
   `Settings` stays the one model and the root view; two thin views render
   the sidebar and the pane from it and the root draws them with
   `Entity::cached`. Both observe `Settings`, so every `cx.notify()` on it
   repaints everything as before, and GPUI's window refreshes (focus,
   activation, resize, appearance) re-render cached views too. The new
   `notify_pane` repaints the pane and the root shell (toolbar, banner,
   sheets) and reuses the sidebar's layout and paint. A hardware rescan
   repaints the whole window only when the sidebar's rows change. While an
   assistive technology is connected the views draw uncached: GPUI rebuilds
   the AccessKit tree from each frame's prepaint, which a reused subtree
   skips.
2. **Launch loads repaint only a pane that shows them** (`5eb26541`). The
   ~25 snapshot loads Settings starts at launch go through
   `notify_if_showing(panes)`: the pane repaints only when it reads that
   data, a subpage or a search is showing, or the window-wide error banner
   has to appear, change or go (the root remembers the banner it drew).
3. **Stream refreshes likewise** (`d527a248`). Labelled probes (a
   throwaway build printing every `cx.notify()` site and every view render
   with its time) showed the last frame of a fresh window coming from the
   display stream refresh niri's window-open events trigger, ~100 ms after
   the activation frame, and the theme, input, locale, storage and privacy
   refreshes plus every watcher's first Available/Unavailable event
   repainting the whole window during launch. Those use the same gating.

With these, a fresh window presents exactly twice: its first frame and the
frame for the activation niri sends when it focuses the window. Loads that
land in between are folded into that second frame.

## Release before / after (ms, median of 5; two sweeps per side)

| | Before (integ) | After |
|---|---|---|
| Settings first frame | 267, 285 | 265, 288 |
| Settings settled | 523, 603 | **421, 448** |
| Settings settled, best launch | 422–490 | 317 |
| Settings pane switch (median of 6) | 14, 16 | 13, 14 |
| Settings pane switch, worst | 38, 59 | 21, 24 |

Raw reports: `speed-round-6-2026-10-06/settings-*.json`. The runtime
window-move checks (`Settings first-frame has no wallpaper inset`,
`… idle-after-map …`) pass with the new binary on the laptop, the same as
integ; the one failure there (`Settings title-bar double-click Zooms`) fails
identically with integ's binary in that environment.

## Not met

| Target | State | What is left |
|---|---|---|
| Settings settled < 300 ms | ~420–450 ms (release, 1080p) | Only the activation frame is left after the first frame. In the nested sweep niri focuses the window 75–180 ms after its first frame (it lists it at ~400 ms), and GPUI re-renders the whole window when a window's active state changes (the sidebar's selection turns accent blue and the traffic lights colour). Next: draw the first frame already active (a new toplevel is always focused in Lulo) and have the Wayland backend skip the active-status callback when the state does not change, so activation costs no frame. |
