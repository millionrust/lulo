# Speed round 2 — 2026-10-06

Branch `op/speed-2`, on top of round 1 (docs/perf/speed-round-1-2026-10-06.md).
Same method: `scripts/behavior/run_speed_sweep.py` in a private nested niri on
the reference laptop's GPU, nothing else building (checked with `ps`),
medians of 3 launches/opens. Before = `integ` 1bc9dd7c (round 1 merged),
after = this branch; both built with `--profile iterate` on the shared
target, including the shell binaries (Dock, Desktop, Mission Control,
App Switcher) this time. "Step 1" is the branch before the GPU pre-warm.

The sweep now also times the desktop and the Dock cold start (both
layer-shell, so first frame comes from their frame trace) and puts a folder
and a document on the private Desktop.

## Changes

1. **Shared render pipelines** (`552c4b9a`, SPEED-03). Every window built
   its own bind group layouts and full pipeline set; panels open two
   windows per open (the panel and the outside-click catcher). `gpui_wgpu`
   now caches them per device and surface format in the GPU context.
2. **Only the present GPUs' Vulkan drivers** (`d6a8a341`, SPEED-04). Before
   the first instance, `gpui_wgpu` sets `VK_DRIVER_FILES` to the manifests
   whose name matches the PCI vendor of each DRM render node (Intel →
   `intel*`, AMD → `radeon`/`amd`, NVIDIA → `nvidia`/`nouveau`, VirtIO),
   and removes it once the instance exists so child processes see the
   normal search. An explicit driver choice, an unknown vendor, a render
   node without a PCI vendor, or no match leave the loader alone.
3. **Fonts in the background** (`d6a8a341`, SPEED-04). The system font scan
   runs on a thread started with the text system; the first call that
   needs fonts waits for it (nothing sees a partial database, so the first
   frame looks the same). rmac-ui's missing-UI-font check moved off the UI
   thread.
4. **GPU context pre-warm** (`e8e4c815`, SPEED-03). On a single-GPU
   machine the Vulkan instance, adapter and device are made on a thread as
   soon as the Wayland client connects; the first window takes the result,
   checks it against its surface, and otherwise falls back to the normal
   selection. Multi-GPU machines keep choosing per surface. No timers or
   polling: one thread, once, at start.

## Before / after (ms)

Apps, spawn → first frame, and → settled (last present before 250 ms
without one):

| App | First frame before | after | Settled before | after |
|---|---:|---:|---:|---:|
| Calculator | 119 | **85** | 139 | 102 |
| Text Editor | 125 | **95** | 164 | 135 |
| Calendar | 150 | **114** | 202 | 163 |
| Mail | 129 | **99** | 177 | 144 |
| Clock | 145 | **109** | 202 | 162 |
| Weather | 124 | **89** | 160 | 126 |
| Preview | 139 | **102** | 190 | 161 |
| Notes | 167 | **122** | 337 | 309 |
| System Monitor | 215 | **174** | 292 | 264 |
| Terminal | 125 | 117 | animates | animates |
| Files | 152 | 158 | 308 | 318 |
| Settings | 300 | **216** | 512 | 872 (see SPEED-02) |
| Desktop (wallpaper) | 216 | **161** | 270 | 217 |
| Dock | 408 | **355** | 443 | 391 |

Panels, trigger → the panel's next present, the three opens in order
(median):

| Panel | Before | Step 1 | After |
|---|---|---|---|
| Spotlight | 116, 101, 129 (116) | 73, 71, 65 (71) | 89, 60, 45 (**60**) |
| Control Centre | 122, 80, 72 (80) | 161, 36, 34 (36) | 111, 28, 41 (**41**) |
| Notification Centre | 122, 108, 74 (108) | 146, 40, 60 (60) | 97, 40, 30 (**40**) |
| Launchpad | 130, 90, 101 (101) | 181, 50, 51 (51) | 116, 54, 51 (**54**) |
| Mission Control | 175, 86, 84 (86) | 185, 63, 64 (64) | 129, 64, 66 (**66**) |
| App Switcher | 105, 46, 50 (50) | 127, 34, 29 (34) | 83, 37, 25 (**37**) |

Steady-state opens are now 25–66 ms (target ~50: Control Centre,
Notification Centre and App Switcher under it; Spotlight, Launchpad and
Mission Control just over). The first open after login is 83–129 ms; step 1
made it slower (the font scan and device creation moved onto the first
open), the pre-warm brought it back below the old numbers.

Settings pane switching: median 16–18 ms before and after (four runs),
except the first switch after launch, which is 35–39 ms after vs 13–15 ms
before (not explained yet).

Desktop time-to-icon (`run_desktop_first_paint.py --gpu --login-only`, 3
rounds, includes `grim` capture overhead): mean 857 → 761 ms. The CI number
(1.7–2.9 s) runs on lavapipe on a shared runner with a cold shader cache;
the desktop's own trace on the laptop shows its first frame at 161 ms, so
the CI figure is not the Intel path. The "icon in the very first captured
frame" check fails both before and after (pre-existing; the icon lands one
async decode later).

## Where Settings still goes (SPEED-02, open)

Probe builds (temporary timing, not committed):

- `Settings::render()` itself is 0.3–5 ms per frame (sidebar ≤ 5 ms, detail
  < 1 ms): building the element tree is not the cost.
- GPUI layout + paint of the frame: 15–55 ms on the first frame, 10–40 ms on
  later ones (Calculator ~1 ms). The initial draw inside `open_window`
  (renderer ready → first configure) takes 66–140 ms vs ~20 ms for
  Calculator: first-time text shaping and SVG glyph rasterization for the
  whole sidebar and pane.
- GPU side of the first frame: atlas upload 7–14 ms and `queue.submit`
  24–35 ms (Calculator 5 ms); later frames still submit 5–18 ms. This points
  at path rasterization (GPUI draws paths through a full-window MSAA
  intermediate texture) and at the window's size.
- "Settled" (0.5–1 s) is bounded by when the last async snapshot load lands
  (each repaints the whole window), so it varies run to run.

Next: find which Settings elements produce GPUI paths (and replace them with
quads/sprites), virtualize the sidebar list, cache sidebar glyphs per size,
and coalesce snapshot repaints.

## Also found

| Item | Finding |
|---|---|
| Dock (SPEED-05) | 355 ms to first frame, ~270 ms of it before `open_window`: the Dock waits for its runtime's first full snapshot (compositor, settings, application catalog, places, appearance) before opening any surface. |
| Outside-click catcher | Every panel open creates a second, display-sized transparent layer surface with its own swapchain; a 1×1 buffer scaled by `wp_viewport` would make it nearly free. |

Raw reports: `speed-round-2-2026-10-06/*.json`.
