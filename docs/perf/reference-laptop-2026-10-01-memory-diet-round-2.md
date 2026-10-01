# Reference laptop memory diet, round 2 (2026-10-01)

The follow-up section below supersedes this round's shortcut-surface startup
policy and login memory sample.

This is a short idle comparison, not an eight-hour leak test. The same Intel
HD Graphics 5500 client path, private HOME/XDG and D-Bus, headless Sway and
nested niri as [round 1](reference-laptop-2026-10-01-memory-diet.md) were used.
`scripts/behavior/measure_memory.py --gpu intel` sampled each process's
`/proc/<pid>/smaps_rollup` five seconds after launch. Player opened a
0.1-second WAV and was sampled after playback. Only one application ran at a
time. Both runs recorded zero SwapPss. The before sample used `ae7fba6e`;
the after sample used rebuilt worktree binaries before this round's commit.

| Sample | Before RSS | After RSS | Before PSS | After PSS | Before private dirty | After private dirty |
|---|---:|---:|---:|---:|---:|---:|
| Shell resident processes (12 → 8) | 540.29 | 460.62 | 363.50 | 293.37 | 141.67 | 128.47 |
| Player after audio EOF | 160.19 | 121.75 | 98.81 | 62.97 | 36.18 | 28.27 |
| System Settings at initial pane | 132.20 | 132.23 | 74.78 | 75.39 | 28.67 | 27.60 |

Values are MiB. The shell reduction is primarily four surfaces no longer
launched at login: Spotlight, App Drawer, Quick Settings and Notification
Center's panel. Notification delivery remains resident. Their systemd units
start on a missing shortcut socket; the dispatch helper retries the original
command, and activation waits briefly for compositor and seat snapshots.
After a window closes, a configurable idle timer (`RMAC_SURFACE_IDLE_SECONDS`,
default 300, range 1–3600) exits the surface service. In nested niri, all four
first dispatches painted and all four services exited after a one-second test
idle timeout: 17/17 lifecycle checks passed. The test used nested niri's Spawn
IPC for the two fallback shortcut commands and direct dispatch for the other
two. Virtual keyboard injection did not trigger the compositor bindings, so
the physical ⌘Space and ⌘⌃N chords were not established by this test.

The four measured cold spawn-to-paint **upper bounds** were 742.6, 650.7,
1026.3 and 452.1 ms respectively. They include process startup, socket
readiness and screenshot capture. This run does **not** establish the requested
<150 ms first-open target. It also does not benchmark the installed systemd
activation path; the private compositor started the service binaries directly.

Settings now loads the applications catalog only in panes that need it and
stops its background application watcher when those panes close. Wallpaper
previews use the packaged thumbnail when present, and the preview watcher is
released outside Wallpaper. The initial-pane private reduction is 1.07 MiB;
the 128 MiB RSS target remains exceeded by 4.23 MiB. Player starts libmpv
off the UI thread, releases decoded audio state and its decoder threads at
EOF, and drops its video render target when playback ends. A later Play
restarts the audio backend. The after sample had 26 threads versus 39 before.

RSS counts shared file pages in every process. In the after sample, shell
`Pss_File` was 164.10 MiB of its 293.37 MiB PSS; Settings had 47.53 MiB
`Pss_File` and 27.60 MiB private dirty, and Player had 34.43 MiB `Pss_File`
and 28.27 MiB private dirty. For the next release gate, a proposed
shared-page-aware *idle* budget is ≤96 MiB PSS and ≤48 MiB private dirty per
open app, and ≤320 MiB aggregate PSS and ≤144 MiB aggregate private dirty for
the resident shell. The old 128/256 MiB RSS figures remain reported for
comparison; this sample passes Player's 128 MiB RSS limit but fails Settings'
and the combined shell's. The proposed PSS/private numbers are an engineering
budget, not a substitute for a swap-aware eight-hour soak or installed release
binary measurement.

An independent read-only `/proc` snapshot of the owner's *installed*, heavily
swapped session found 23 shell processes both times. Before: 153.06 MiB RSS,
78.50 MiB PSS, 42.12 MiB private dirty and 201.65 MiB SwapPss. After:
144.91 MiB RSS, 70.34 MiB PSS, 35.95 MiB private dirty and 207.95 MiB
SwapPss. These installed binaries were not replaced, so this variation is not
an effect measurement; PSS plus SwapPss stayed near 278–280 MiB. No input or
control request was sent to the live session.

The 11 affected root packages passed 293 unit tests (zero failures, three
ignored doctests), and package-scoped clippy with `-D warnings` passed,
including tests. Root and shell formatting checks passed. The full app
behavior suite passed 111/111. Nested niri window movement passed 19/19,
frame and wallpaper repaint 15/15, power dialogs 51/51, shutdown 38/38,
and the new cold-surface lifecycle runner 17/17. The power and shutdown
runners used fake `systemctl` paths; they did not operate the laptop's real
power controls. The local Python behavior-harness tests passed 36/36.

An eight-hour soak, a release-profile rerun, an installed systemd activation
measurement, and a reliable physical-chord latency measurement remain open.

## Follow-up: keep the primary shortcuts resident

The round-2 cold-start trade-off was too slow for the primary shortcuts.
`e43ec0df` restores Spotlight and Quick Settings to the login target with
`Restart=on-failure`, and removes their idle-exit hook. Apps and the
Notification Center panel still start on first use. Their idle exit now defaults
to 1,800 seconds after the last window closes; `RMAC_SURFACE_IDLE_SECONDS` can
set 1–86,400 seconds. App Drawer's catalog watcher and recents read, and both
panels' application catalog scans, start only after the first presented frame.
The scans and App Drawer's macOS icon extraction run off the UI thread.

The same private nested Intel path and `--profile iterate` binaries were used.
The first-frame numbers below come from `prepare_surface_window`'s benchmark
marker, observed by the runner; they include endpoint dispatch and, for Apps
and Notification Center, process startup. The screenshot numbers are upper
bounds that also include `grim` capture. The old round-2 numbers used only
screenshots, so compare them with the new screenshot column, not the marker.

| Surface | Login policy | Round-2 screenshot upper bound | Follow-up first frame | Follow-up screenshot upper bound |
|---|---|---:|---:|---:|
| Spotlight | Resident | 742.6 ms | 402.2 ms | 655.8 ms |
| Apps | On demand | 650.7 ms | 142.7 ms | 643.5 ms |
| Quick Settings | Resident | 1026.3 ms | 119.0 ms | 993.4 ms |
| Notification Center panel | On demand | 452.1 ms | 165.6 ms | 469.3 ms |

These are the final run's values. A preceding run on the same production code
recorded first frames of 352.1, 136.0, 118.8 and 136.4 ms respectively, so
the timing varies and Notification Center did not stay below 150 ms. Spotlight
is still well above the desired 150 ms first-frame target. A settled,
direct-dispatch diagnostic was 315.6 ms; instrumentation found approximately
300 ms inside GPUI's first `open_window` call, before the application view was
constructed. Deferring its provider work and changing layer-shell keyboard
interactivity did not reduce that time, so neither experiment was retained.
The screenshot upper bounds remain high because capture itself is slow. This
test did not measure a physical ⌘Space chord or installed systemd activation.

At five seconds idle, the corrected login sample contains 10 shell processes:
499.85 MiB RSS, 329.40 MiB PSS and 134.38 MiB private dirty, with zero SwapPss.
Spotlight accounts for 23.62 MiB RSS / 20.55 MiB PSS / 3.97 MiB private dirty;
Quick Settings accounts for 20.37 / 17.29 / 3.14 MiB. Compared with the
round-2 eight-process sample this is +39.23 MiB RSS, +36.03 MiB PSS and
+5.91 MiB private dirty. The proposed 320 MiB shell PSS budget is exceeded by
9.40 MiB. The memory harness now excludes only Apps and the Notification Center
panel when sampling a login state.

The cold-surface runner passed 27/27 checks, including first-frame markers,
visible first paint, resident survival after dismissal, same-process warm
reopen for the two on-demand panels, and exit after a three-second test idle
interval. The six affected root packages passed 146 tests with zero failures
and two ignored doctests. Package-scoped clippy with `-D warnings` and the root
and shell formatting checks passed. The full app behavior suite matched
111/111 scenarios; nested window movement passed 19/19, frame and wallpaper
repaint 15/15, power dialogs 51/51, and shutdown 38/38. The power and shutdown
runners used fake `systemctl` paths and did not operate the laptop's real power
controls. The default 30-minute interval was not waited out in a real login.
