# Speed round 1 — 2026-10-06

Owner priority: "everything should be fast… that's the only reason we chose
Rust native". Branch `op/speed-1`. Reference laptop: 4 cores, 6 GB, Intel HD
5500 (Mesa `hasvk` Vulkan), quiet (no cargo or other agents running during
the measured runs, checked with `ps` first).

## Method

`scripts/behavior/run_speed_sweep.py` in a private nested niri on the GPU
path, never the owner's session. Fixed in this round, so the numbers are
real:

- **First frame** is now the first `present` row of the app's own
  RMAC_FRAME_TRACE (polled every 5 ms), not a `niri msg windows` poll that
  spawned a process every 20 ms and competed with the launch. "Window
  shown" (niri lists it) is still reported.
- **Settled** ignores `frame_callback` rows, which GPUI writes even for
  frames it doesn't draw; it is the last `present` before 250 ms without
  one. Before, idle frame callbacks kept the trace "busy" and inflated it.
- **Median of repeated launches** (3, or 7 for Files/Settings/Calculator),
  and one throwaway launch first to warm the private HOME's Mesa shader
  cache. Without that, whichever app ran first paid a one-off ~0.6 s
  pipeline compile. That artifact plus a loaded laptop is what made Files
  look like a 5 s launch on 2026-10-05; on a quiet laptop with a warm cache
  its first frame was 191 ms.
- **Harness gaps closed**: Launchpad runs as `rmac-app-drawer --service`
  (as the packaged unit does); App Switcher runs as `--service` with two
  apps open and is opened by its own `next` client; Settings pane
  switching is timed from the Down key's `input` row to the next `present`
  in Settings' own trace, with each switch confirmed by the window title.
  Panel opens count only presents after the trigger (before, any earlier
  present counted).
- **Startup phases**: `gpui_linux` now records `common_start/ready`,
  `open_window`, `renderer_ready` and `first_configure` in the frame trace.

Before = the published Beta 1 release packages (`rmac-apps`/`rmac-session`
`0.9.0~beta.1-38`, release profile, extracted privately, never installed).
After = this branch built with `--profile iterate` (opt-level 3, no LTO; a
release build of every app did not fit the laptop's time budget). Release
and iterate builds of the same `integ` code measured the same within noise
(Files 191/196 ms, Settings 298/313 ms, Calculator 164/170 ms), so the
difference below is the code change, not the profile.

## Where a cold launch went (Calculator, `integ`, trace clock)

| Phase | Time |
|---|---:|
| exec → GPUI backend init | 6 ms |
| font database (`LinuxCommon::new`) | 15 ms |
| app + rmac-ui init (theme, components, session) | 2 ms |
| **first window's GPU context + renderer** | **113 ms** |
| compositor configure | 19 ms |
| first draw + present | 7 ms |

Experiments with the same binary: keeping EGL out of the process cut the
GPU step to 63 ms; also loading only the Intel Vulkan driver cut it to
40 ms; disabling Mesa's shader cache raised it to 341 ms. Upstream
`gpui_wgpu` builds a Vulkan+GL instance, so every process initialised EGL
and every installed Vulkan driver before picking the Intel one.

## Fixes

1. **Vulkan-first GPU context** (`e8f89c51`, ADR 0013 amendment).
   `gpui_wgpu` is vendored into `shell/compat/gpui_wgpu` like `gpui_linux`;
   `WgpuRenderer::new` tries a Vulkan-only instance with hardware adapters
   first and falls back to upstream's Vulkan+GL selection only when that
   finds nothing, so GL-only and software machines still work. Applies to
   every app and every resident panel's first window.
2. **Settings app icons** (SET-114, `338aba42`): `rmac_ui::svg_icon` takes
   a shared `&Context`, so Notifications, Focus and Privacy rows rasterize
   app icons at row size off the UI thread instead of 2048 px masters.

## Before / after

Apps, median ms from spawn:

| App | First frame before | after | Window shown before | after | Settled before | after |
|---|---:|---:|---:|---:|---:|---:|
| Files | 191 | **150** | 275 | 237 | 390 | 340 |
| Text Editor | 176 | **129** | 199 | 151 | 214 | 170 |
| Settings | 298 | **267** | 421 | 422 | 962 | 910 |
| Calculator | 164 | **116** | 190 | 148 | 180 | 135 |
| Calendar | 211 | **145** | 259 | 198 | 256 | 194 |
| Mail | 180 | **140** | 234 | 196 | 229 | 188 |
| Clock | 202 | **151** | 251 | 213 | 249 | 203 |
| Weather | 172 | **124** | 252 | 202 | 221 | 161 |
| Preview | 196 | **145** | 246 | 202 | 243 | 198 |
| Notes | 207 | **150** | 258 | 205 | 380 | 326 |
| System Monitor | 263 | **214** | 324 | 271 | 338 | 287 |
| Terminal | 181 | **127** | 212 | 148 | animates | animates |

Panels, ms from trigger to the panel's next present (resident service, the
three opens in order; median of the three):

| Panel | Before | After |
|---|---|---|
| Spotlight | 140, 106, 90 (106) | **117**, 106, 85 (106) |
| Control Centre | 224, 97, 106 (106) | **126**, 85, 82 (85) |
| Notification Centre | 214, 77, 96 (96) | **127**, 112, 80 (112) |
| Launchpad | 232, 90, 101 (101) | **131**, 111, 101 (111) |
| Mission Control | 204, 83, 80 (83) | 264, 84, 83 (84) — shell bin not rebuilt |
| App Switcher | 148, 58, 56 (58) | 188, 74, 46 (74) — shell bin not rebuilt |
| Settings pane switch | 12 (max 19) | 16 (max 45) |

The first open of a resident panel creates its GPU context, so the fix
shows there: the first ⌘Space / Control Centre / Notification Centre /
Launchpad after login went from 214–232 ms to 117–131 ms. Later opens are
unchanged (±20 ms run-to-run noise). Mission Control and the App Switcher
are shell-workspace binaries that were not rebuilt for this run (release
binaries in both columns); they get the same fix with the next shell build.

Every app's first frame now beats the 300 ms macOS target with margin
(115–267 ms). Raw reports: `speed-round-1-2026-10-06/*.json`.

## What's left

| Item | Finding | Next step |
|---|---|---|
| Settings content | First frame 267 ms, but it keeps repainting until ~0.9 s as its snapshot loads land; its first `open_window` render alone takes 55–95 ms and each later draw/present 35–85 ms (vs ~5 ms for Calculator). Sidebar glyphs are GPUI `svg()` elements parsed and rasterized on the UI thread during paint. | Profile Settings' render (layout cost of the full sidebar + pane), batch the snapshot updates into fewer repaints, pre-rasterize sidebar glyphs. |
| Panels' later opens 85–110 ms | Each open creates a new layer-shell window: a new wgpu surface and all render pipelines (`create_pipelines` per window), plus a configure round trip. | Share pipelines per (device, format, alpha) in the vendored `gpui_wgpu`, or keep panel windows mapped-hidden. |
| Vulkan driver loading | Loading every installed ICD (asahi, nouveau, radeon, lavapipe, …) still costs ~23 ms per process. | Hardware-adaptive session setting that limits the loader to the machine's driver (with the fallback kept). |
| Font database | 15 ms per process, synchronous before the first window. | Load it on a background thread overlapping the GPU init. |
| FILES-62 | Not changed: Files' artwork already uses pre-sized SVG variants and Files' first frame is 150 ms. | Low priority. |
| Release-profile after numbers, shell bins | Not built this round. | Next release build; expect equal or better (LTO). |
