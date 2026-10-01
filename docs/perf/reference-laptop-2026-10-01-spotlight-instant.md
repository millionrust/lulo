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
requests in the main runs used niri Spawn and `rmac-shortcut-dispatch`. GPUI
frame callbacks write the benchmark markers. Captures include `grim` overhead
and are only upper bounds, not dispatch-to-frame timings.

| Run | Resident settle | First dispatch to frame | Later dispatch to frame | Query echo to frame | Cached result to frame | Checks |
|---|---:|---:|---:|---:|---:|---:|
| Warmup window retained | 0 s | 67.4 ms | 52.4 ms | — | — | 30/30 |
| Warmup window retained | 5 s | 72.8 ms | 50.1 ms | 19.9 ms | 19.9 ms | 32/32 |
| Warmup window closed | 5 s | 65.1 ms | 62.8 ms | 22.7 ms | 22.8 ms | 32/32 |
| Warmup closed; freed pages trimmed | 5 s | 61.0 ms | 55.7 ms | 23.6 ms | 23.7 ms | 32/32 |
| Four-show run | 5 s | 79.0 ms | 64.2, 90.3, 89.5 ms | 22.6 ms | 22.6 ms | 36/36 |
| Final code (`9fd3a3fa`) | 5 s | 85.2 ms | 52.7, 84.4, 86.0 ms | 16.6 ms | 16.6 ms | 36/36 |

A separate direct-dispatch diagnostic bypassed niri Spawn for the first show:
65.5 ms to first frame, with later niri-dispatched shows at 55.1, 88.4 and
88.6 ms (36/36 checks). The 13.5 ms difference between that first show and
the 79.0 ms four-show run is indicative because they were separate nested
sessions.

The typing probe enters `cal` through the nested compositor's virtual
keyboard. The displayed numbers run from the launcher's input-change event to
the GPUI frame containing the echoed query and a nonempty cached result list.
The runner now fails above 150 ms on either the first or later Spotlight show
and above 50 ms for either typing measurement. The 150 ms gate allows CI
scheduling noise; the measured shows above met the stricter 100 ms product
target. These measurements do not cover the physical keyboard or an installed
systemd login.

The speed costs substantial idle memory on this GPU stack. A separate
five-second login sample with all ten Intel shell processes running measured
Spotlight at 104.64 MiB RSS / 46.15 MiB PSS / 21.92 MiB private dirty. The
earlier uninitialized resident launcher sample was 23.62 / 20.55 / 3.97 MiB.
The whole shell measured 575.98 MiB RSS / 325.96 MiB PSS / 153.08 MiB private
dirty, versus 499.85 / 329.40 / 134.38 MiB in the earlier report. RSS and
private dirty grew; PSS fell slightly across these separate runs as mapped
Vulkan pages were shared differently. The shell still exceeds its proposed
256 MiB RSS and 320 MiB PSS limits. An isolated launcher run measured 111.88
MiB RSS / 69.93 MiB PSS; that PSS overstates its share at a full login.
Closing the warmup surface reduced private dirty in an intermediate run, but
WGPU's shared device and Vulkan mappings remain resident. A one-time allocator
trim reduced isolated RSS/PSS by about 4 MiB. The incremental launcher memory
exceeds the few-MiB allowance suggested for keeping a surface ready; MEM-01
continues to track the budget.

Final-code validation: `rmac-launcher-app` unit tests 11/11, package-scoped
clippy with `-D warnings`, root and shell `cargo fmt --all -- --check`, and the
GPUI import and design-token guards passed. The private app behavior suite
matched 111/111 Mac scenarios on a quiet rerun; an earlier run under build
load matched 110/111 and its lone Files window-count case passed 1/1 alone.
Nested window movement/zoom passed 29/29 on rerun after an initial 27/29;
frame/repaint passed 15/15, power dialogs 51/51, and shutdown 38/38. The power
and shutdown runners used fake
`systemctl` paths and never operated the laptop's real power controls.
