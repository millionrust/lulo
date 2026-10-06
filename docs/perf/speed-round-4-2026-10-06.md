# Speed round 4 — 2026-10-06

Branch `op/speed-4` on `integ` d7fa75c9 (rounds 1–3 merged). This round's
table is the **release profile** (fat LTO), for the two packages that
changed: `rmac-system-settings` and the Dock. Both sides were built back to
back on the shared target under the build lock: before = integ, after =
this branch. Measured on a **1920×1080** nested output (the sweep's new
default, so Settings is no longer resized by a short 1280×800 output), two
sweeps per side, 5 launches each, on the reference laptop with nothing else
building.

## Changes

1. **Sweep at 1080p** (`96ad26fa`): `run_speed_sweep.py --output` (default
   `1920x1080`).
2. **Dock doesn't wait for the appearance portal** (`d11e0965`): the first
   snapshot no longer needs the appearance source, which only sets reduced
   motion; it starts at the default and is reconciled when the portal
   answers. Unit test updated.
3. **Settings launch loads repaint only for the open pane** (`ea2e5573`):
   labelled probes (every `update` closure in Settings) showed a fresh
   window on General repainting at ~185, ~325 and ~410 ms as input, audio,
   displays, Bluetooth, login items, storage, VPN, GTK text and screen
   reader loads landed. Those 13 completions now call
   `Settings::notify_if_showing(panes)`, which repaints when one of the
   panes reading that data is open, or any subpage, a search or the
   window-wide error banner is showing. A pane opened later repaints on
   navigation and reads the latest values.

## Release before / after (ms, median of 5; two sweeps per side)

| | Before (integ) | After |
|---|---|---|
| Settings first frame | 278, 250 | 258, 306 |
| Settings settled | 687, 552 | 616, 582 |
| Dock first frame | 212, 222 | 206, 214 |
| Dock settled | 261, 269 | 263, 265 |
| Settings pane switch (median of 6) | 16, 18 | 16, 15 |

Per-launch values are in `speed-round-4-2026-10-06/*.json`. The first
launch of each sweep is cold (700–980 ms first frame, no shader-cache
warm-up because only the two changed binaries were built in release).

**Neither change moved its number measurably.** Settings run-to-run spread
(424–1030 ms settled) is larger than the change, and the Dock was not
waiting on appearance on this machine: the probe shows appearance answering
at ~67 ms and the pinned-apps catalog at ~130 ms, then ~85 ms of window and
renderer set-up for its two surfaces. Release and iterate builds measure
within noise of each other for start-up, as in round 1.

## Not met

| Target | State | What is left |
|---|---|---|
| Settings settled < 300 ms | ~550–690 ms (release, 1080p) | Repaints still come from loads the probe could not label (they use other closure shapes: system data/About, hardware capabilities, theme), and each Settings frame costs 30–50 ms of GPUI layout. Next: label those, and split the sidebar and each pane into their own views so a load repaints only its pane. |
| Dock first frame < 150 ms | ~210 ms | ~130 ms until the pinned apps' entries are read (the settings file, a scan of every application directory, the pinned entries' icons), then ~85 ms for two surfaces. Next: cache the pinned apps' resolved entries between logins and open the surfaces while they load. |
| Mission Control < 50 ms | 58 ms (round 3) | Not changed this round. Each open runs `grim` as a child process to capture the output (spawn, Wayland connect, screencopy, a PPM of the whole output through a pipe) before the overlay maps; that, not the overlay surface, dominates. Next: capture in-process with wlr-screencopy over a connection kept open by the service. |
