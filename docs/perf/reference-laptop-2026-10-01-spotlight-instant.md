# Spotlight first-frame follow-up — reference laptop, 2026-10-01

The resident Spotlight process previously needed 402.2 ms from nested-niri
shortcut dispatch to its first presented frame. A direct-dispatch diagnostic
needed 315.6 ms, with about 300 ms inside GPUI's first `open_window`, before
the launcher view existed. This identifies renderer/device initialization on
the first Wayland window as the dominant cost; provider, recent-item and
settings work was not blocking that first frame.

Spotlight now opens a one-pixel, pointer-transparent, nonfocusing layer
surface during process startup, before it binds the shortcut endpoint. This
initializes GPUI's process-shared WGPU context at login. The surface closes
after its first frame; an off-thread, one-time glibc trim returns pages freed
by it. The actual Spotlight surface still opens on demand, receives exclusive
keyboard focus, and renders an empty search field while providers run on the
blocking pool. No timer or periodic wakeup was added to the idle launcher.

`scripts/behavior/run_cold_surfaces.py` ran in private nested Sway/niri with
temporary XDG directories and the laptop's Intel Vulkan driver. All shortcut
requests used niri Spawn and `rmac-shortcut-dispatch`. GPUI frame callbacks
write the benchmark markers. Captures include `grim` overhead and are only
upper bounds, not dispatch-to-frame timings.

| Run | Resident settle | First dispatch to frame | Later dispatch to frame | Query echo to frame | Cached result to frame | Checks |
|---|---:|---:|---:|---:|---:|---:|
| Warmup window retained | 0 s | 67.4 ms | 52.4 ms | — | — | 30/30 |
| Warmup window retained | 5 s | 72.8 ms | 50.1 ms | 19.9 ms | 19.9 ms | 32/32 |
| Warmup window closed | 5 s | 65.1 ms | 62.8 ms | 22.7 ms | 22.8 ms | 32/32 |
| Warmup closed; freed pages trimmed | 5 s | 61.0 ms | 55.7 ms | 23.6 ms | 23.7 ms | 32/32 |

The typing probe enters `cal` through the nested compositor's virtual
keyboard. The displayed numbers run from the launcher's input-change event to
the GPUI frame containing the echoed query and a nonempty cached result list.
The runner now fails above 150 ms on either the first or later Spotlight show
and above 50 ms for either typing measurement. The 150 ms gate allows CI
scheduling noise; the measured shows above met the stricter 100 ms product
target. These measurements do not cover the physical keyboard or an installed
systemd login.

The speed costs substantial idle memory on this GPU stack. At five seconds
after endpoint readiness, the final launcher used 111.88 MiB RSS, 69.93 MiB
PSS and 44.44 MiB private dirty. The earlier uninitialized resident launcher
sample was 23.62 / 20.55 / 3.97 MiB respectively; the measurements came from
different nested sessions, so their PSS figures are indicative. Closing the
warmup surface reduced private dirty in an intermediate run, but WGPU's shared
device and Vulkan mappings remain resident. A one-time allocator trim reduced
the measured RSS/PSS by about 4 MiB. This exceeds the few-MiB allowance
suggested for keeping a surface ready and worsens the shell login-memory
budget. The follow-up memory budget must account for this tradeoff.
