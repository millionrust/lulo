# Speed round 3 — 2026-10-06

Branch `op/speed-3` on `integ` 54b0a7f9 (rounds 1 and 2 merged). Same method
as round 2: `scripts/behavior/run_speed_sweep.py` on the reference laptop's
GPU in a private nested niri, no other builds running, medians of 3; before
and after both `--profile iterate` from the shared target, shell binaries
included, measured back to back.

## What the probes showed

Temporary timing builds (not committed):

- **Dock**: its runtime published nothing until all sources were ready.
  Settings arrived at 14 ms, compositor/displays/places at ~51 ms,
  appearance at ~79 ms, the application catalog at **~330 ms**; the Dock
  opened its surfaces right after.
- **Settings**: every frame drew 3 paths (the traffic-light glyphs, kept at
  opacity 0 until hovered). Each path batch clears, rasterizes and resolves
  a window-sized 4x MSAA texture, so every frame of every rmac window paid
  that pass; Settings' frames submitted in 17–40 ms. The window's repaints
  after the first frame came from load completions at ~320, ~420, ~550,
  ~620 ms and the launch Wi-Fi scan at ~1 s (it sleeps 750 ms for
  NetworkManager), plus one resize where the nested 800 pt output is shorter
  than Settings' default height.

## Changes

1. **Transparent paths skipped** (`0fa7b709`): the renderer skips a path
   batch whose paths are all fully transparent and creates the path
   textures only when a frame has a visible path (also saves ~5 window-sized
   buffers of GPU memory per window).
2. **1x1 outside-click catcher** (`877dbeec`): the panels' display-sized,
   transparent, input-only catcher gets a 1x1 buffer at buffer scale 1 that
   its `wp_viewport` stretches to the display, instead of a display-sized
   swapchain cleared and presented on every open.
3. **Dock pinned apps first** (`b310522a`): the catalog watcher publishes
   the pinned apps' entries (parsed alone, same precedence:
   `rmac_apps::discover_entries`) before the full catalog, which then
   reconciles running and recent apps.
4. **Settings Wi-Fi scan** (`7f93aada`): the launch scan repaints only when
   the Wi-Fi state the window shows changed.

## Before / after (ms)

| App / surface | First frame before | after | Settled before | after |
|---|---:|---:|---:|---:|
| Calculator | 100 | 81 | 120 | 100 |
| Text Editor | 89 | 83 | 132 | 114 |
| Calendar | 121 | 104 | 174 | 149 |
| Mail | 111 | 83 | 165 | 121 |
| Clock | 121 | 111 | 167 | 159 |
| Weather | 100 | 84 | 148 | 117 |
| Preview | 110 | 85 | 158 | 122 |
| Notes | 124 | 111 | 322 | 279 |
| System Monitor | 182 | 173 | 257 | 247 |
| Terminal | 102 | 84 | animates | animates |
| Files | 115 | 99 | 303 | 287 |
| Settings | 294 | **205** | 709 | 894 (not met) |
| Desktop | 157 | 130 | 235 | 235 |
| **Dock** | 348 | **217** | 394 | **246** |

Panels, trigger → present (three opens; median):

| Panel | Before | After |
|---|---|---|
| Spotlight | 78, 75, 49 (75) | 46, 24, 24 (**24**) |
| Control Centre | 139, 30, 29 (30) | 106, 24, 30 (**30**) |
| Notification Centre | 101, 65, 45 (65) | 60, 28, 22 (**28**) |
| Launchpad | 144, 54, 49 (54) | 59, 23, 25 (**25**) |
| Mission Control | 152, 67, 64 (67) | 105, 58, 58 (**58**) |
| App Switcher | 67, 31, 41 (41) | 64, 25, 31 (**31**) |
| Settings pane switch | 18 (max 65) | 16 (max 38) |

Settings' per-frame GPU time after: 4–20 ms (was 15–60).

## Not met / left

| Item | State |
|---|---|
| Settings settled < 300 ms | Not met: 0.85–1.0 s. The window still repaints three or four times as data loads land between ~300 and ~1000 ms (one is the harness's short output resizing it). Next: label the remaining late repaints (the probe saw unlabelled ones at ~0.85–1.05 s), gate background loads on the visible pane, and fix the first-frame layout cost (~35–50 ms CPU). |
| Dock first frame < 150 ms | 217 ms. Remaining before `open_window`: appearance (portal) at ~79 ms, then two surfaces' renderer set-up. Next: don't wait for the appearance source (it only sets reduced motion). |
| Mission Control < 50 ms | 58 ms: it maps a full-output overlay each open. |
| First Settings pane switch | Not investigated this round (38 ms max after, 65 before). |

Raw reports: `speed-round-3-2026-10-06/*.json`.
