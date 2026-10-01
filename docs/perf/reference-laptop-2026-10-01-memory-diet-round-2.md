# Reference laptop memory diet, round 2 (2026-10-01)

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
