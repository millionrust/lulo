# Speed round 5 — 2026-10-06

Branch `op/speed-5` on `integ` 7444b1cb. Release profile (fat LTO) for the
changed binaries only (Dock, Mission Control), before = integ and after =
this branch, built back to back on the shared target under the build lock.
`run_speed_sweep.py` on a 1920×1080 nested output on the reference laptop,
nothing else building, 5 launches/opens per sweep (7 for the Mission Control
re-run), two sweeps per side.

## Changes

1. **Mission Control captures in-process** (`1ec57926`, `1ef0d95f`). Each
   open spawned `grim` (process start, new Wayland connection, the whole
   output as a PPM through a pipe). The service now keeps one Wayland
   connection on its own thread, started with the service, and copies the
   output with `zwlr_screencopy_manager_v1` into a reused memfd buffer; the
   picture keeps the buffer's own pixel layout (BGRX/RGBX) and window crops
   copy rows straight into GPUI's BGRA. No cursor, top row first, same as
   `grim -o`. A missing protocol, output or format, a failure, or a 2 s
   timeout falls back to `grim`. The thread blocks on its request channel
   while idle.
2. **Dock entries cached between logins** (`cc8f8913`, `3d55f7dd`). The
   entries the Dock shows before the full catalog (pinned, then recent) are
   kept in `$XDG_CACHE_HOME/rmac/dock-entries.json` with each desktop
   entry's mtime. A login uses them when every wanted ID is cached, every
   entry is unchanged and every icon exists; otherwise it parses just those
   entries and caches them. The full catalog then reconciles and refreshes
   the cache. Including the recent apps keeps the first snapshot the same
   size as the reconciled one; the reduced-motion wait is unchanged, so the
   surface size is right from the first frame.

## Release before / after (ms)

| | Before (integ) | After |
|---|---|---|
| Dock first frame, median of 5 | 204, 228 | **115, 133** |
| Dock first frame, warm launches | 190–232 | 110–168 |
| Dock first frame, first launch (empty cache, cold) | 772–823 | 740–758 |
| Dock settled | 248, 290 | 170, 177 |
| Mission Control open, median | 85, 90 (95 in a 7-open re-run) | 63–70 |

The first launch of each sweep is cold (no shader cache, empty entry cache);
every later launch in the same private HOME has a warm cache, like a login
after the first.

## Not met

| Target | State |
|---|---|
| Mission Control < 50 ms | 63–70 ms at 1080p. The capture is no longer the main cost: taking out the RGB conversion moved it only ~5 ms. What remains is mapping a new full-output overlay (surface, swapchain, configure) and its first frame each open. Next: keep the overlay surface between opens (unmapped by attaching a null buffer, which GPUI's Wayland backend cannot do yet) or make the overlay's first frame cheaper. |
| Settings split into per-pane entities, settled < 300 ms | Not started this round. A cached GPUI view for the sidebar needs every sidebar-affecting state change (selection, navigation, search, hardware availability, compact layout, focus) to notify that view, and each pane view needs the per-load mapping that broke the first-frame check in round 4. That is a larger change than this round's box; the design is in docs/parity.md (SPEED-02). |

Raw reports: `speed-round-5-2026-10-06/*.json`.
