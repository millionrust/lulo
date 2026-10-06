# Speed round 6: Mission Control open (2026-10-06)

Branch `op/speed-6-mc`, based on `integ` 9f60488b. Release profile (fat LTO)
for `mission-control` only. Before is integ and after is this branch, built
back to back on the shared target under the build lock. Measured with
`run_speed_sweep.py --only mission-control --repeat 7` on a 1920×1080 nested
output on the reference laptop. Before and after alternated (before, after,
before, after), and the build lock was held so no build ran during the
sweeps. The only busy process on the laptop was packagekitd.

## Change

Every open used to map a new full-output overlay: a new `wl_surface` and
layer surface, a new Vulkan swapchain, a first configure, then the first
frame. That took about 40 ms of the 63–70 ms open. Mission Control now keeps
that surface between opens:

1. **GPUI's Wayland backend can unmap a layer-shell window and map it again**
   (`gpui_linux::set_layer_window_mapped` and
   `request_layer_window_configure`, in `shell/compat/gpui_linux`, ADR 0013).
   - **Unmapping** commits a null buffer. The surface, its swapchain and the
     GPUI window stay alive. The unmap resets the layer surface's
     double-buffered state, and smithay (niri) drops it to defaults, so the
     backend sets anchor, size, layer, keyboard interactivity, margins and
     exclusive zone again right away. Any later commit is then valid.
   - **While unmapped**, the window asks for no frame callbacks. A frame
     callback that still arrives does nothing, and `draw` never attaches a
     buffer. The hidden overlay costs no GPU or CPU and wakes nothing.
   - **Mapping** commits without a buffer and draws on the configure that
     follows. If that configure already arrived while unmapped, it draws at
     once. The first frame re-renders the whole scene.
2. **Mission Control keeps its last overlay unmapped** (`model::SurfaceKey`)
   and maps it again when the next scene is on the same output at the same
   size. Any other output or size gets a new surface. As soon as the command
   arrives, the service asks for the kept surface's configure. That round
   trip, about 15 ms on this nested niri, then overlaps the screen capture.
   Each open still runs the in-process screencopy capture from round 5, so
   every thumbnail is fresh. On close, the pictures still leave the GPU
   atlas.

## Release before / after (ms, Ctrl+Up to first present)

| Sweep | Before (integ) median | After median | Before warm opens | After warm opens |
|---|---|---|---|---|
| 1 | 63.9 | **47.0** | 57–65 | 43–56 |
| 2 | 69.0 | **43.0** | 61–71 | 42–52 |

Each sweep's first open is cold: no shader cache, because an MC-only sweep
skips the Calculator warm-up. It took 351–367 ms before and 315–320 ms after.
That open still creates the surface. The other six opens reuse it.

What is left in an open, timed with a temporary trace on this nested setup:

- Key to command: about 20–30 ms. This covers sway, nested niri's
  keybinding, the spawned `mission-control` client (about 5 ms exec) and the
  datagram to the service.
- Capture: about 25–40 ms. This is mostly niri rendering the output for
  screencopy, which uses software rendering in this nested setup.
- Map to first present: about 1 ms, down from about 40–55 ms for a new
  surface.

## Kept overlay checks

`scripts/behavior/run_mission_control_reopen.py` opens and closes Mission
Control five times over a Calculator window in the private nested session.
It passed every round (`speed-round-6-2026-10-06/mc-reopen-after.json`):

- Every open changes the screen.
- After close, grim's capture matches the one taken before the open, byte for
  byte.
- A typed digit and a click reach Calculator.
- No input row reaches Mission Control.
- Over 3 s the hidden overlay presents nothing and the service uses 0 ms of
  CPU.

## Cost

While hidden, the overlay keeps its swapchain: three 1920×1080 BGRA images,
about 24 MiB of GPU-visible memory on this laptop. Before, the swapchain was
freed on every close. The atlas pictures are still released.

Raw reports: `speed-round-6-2026-10-06/mc-{before,after}-release-{1,2}.json`
and `mc-reopen-after.json`.
