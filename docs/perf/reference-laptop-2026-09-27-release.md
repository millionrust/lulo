# Reference laptop performance budgets -- 2026-09-27T06:38:56Z

This run used release binaries built from `8ae3a9eb`, one app at a time on
the reference laptop's live niri session, with a separate temporary HOME,
config, data, state, and cache directory for each app. Benchmark marker and
XDG writes sit outside Files' watched HOME and its parent. No input was injected
and no Cargo process ran. Only the nine applications listed below were
selected. Apps needs its service launch path and Preview needs a document,
so the generic launch method did not measure them; shell surfaces were also
excluded. Clock starts on its World view with a moving second hand, and
Weather starts with search focused in the empty profile. The
[machine-readable report](reference-laptop-2026-09-27-release.json) records
each sample and its limits.

A prior, unmerged System Monitor experiment cached stable per-process search
and user strings by PID and start time. Its focused package tests (39/39) and
Clippy passed, but a 60-second sample under the previous benchmark layout
measured 4.25% idle CPU versus that layout's 3.70% baseline. The experiment
was discarded. These measurements are not directly comparable with this
report's corrected HOME layout or five-sample p95. The
[probe data](reference-laptop-2026-09-27-system-monitor-probe.json) is retained
so the same cache is not repeated without new profiling evidence.

A later Weather change aligns its clock refresh with minute boundaries, since
the displayed time has no seconds. Its focused 60-second sample measured 0.63%
idle CPU and 6.550 wake-ups/s versus this release binary's 0.70% and 6.483/s.
The change passed its focused release-mode test, but the sample does not prove
a material CPU reduction and the 0.3% budget still fails. The
[probe data](reference-laptop-2026-09-27-weather-minute-probe.json) records
the result; this report's table remains the release-package baseline.

A paired run of the original Weather candidate binary measured 0.70% CPU in
the empty first-run state, where search takes focus, and 0.033% with one saved
city and a fresh forecast cache. The large difference points to the focused
search caret rather than the minute tick; each state has one 60-second sample,
so this is directional evidence. First-run autofocus remains intact
([data](weather-idle-2026-09-27.json)).

A later paired Clock probe used the same candidate binary in two private
profiles: its default one-city World view measured 0.550% idle CPU, and an
empty-city World view measured 0.433%; both had 3.117 wake-ups/s. The
candidate table's 1.87% Clock sample was not reproduced, but both focused
runs still exceeded the 0.3% budget. Removing one city left the one-second
World ticker active, so this probe does not identify the remaining CPU cost
([data](reference-laptop-2026-09-27-clock-city-probe.json)).

The previous report measured Files at 7.60% idle CPU and 32.100 wake-ups/s.
Its marker and XDG directories were inside Files' watched HOME or parent,
so benchmark writes could trigger Files' watcher. The changed result makes
that earlier Files measurement unreliable; the exact cause of the difference
is not fully isolated. This corrected release-package run measures 4.57% and
27.333/s; an independent focused run with the same layout measured
4.58% and 27.433/s. A later focused repeat of the original release binary
measured 4.83% and 26.417/s
([data](reference-laptop-2026-09-27-files-repeat.json)). The earlier
30-second thread sample, which attributed 6.767% CPU to the main UI thread,
used the contaminated layout and cannot establish the source of the remaining
cost.

An unmerged Files watcher filter passed all 215 Finder package tests. In a
controlled 30-second sibling-folder churn comparison, the original and
filtered release binaries both measured 0.73% CPU (7.467 versus 8.100
wake-ups/s). That workload did not establish a performance benefit and its
lower absolute CPU is not comparable to the clean-idle 60-second runs above;
the filter was left out of `dev` pending stronger evidence.

A paired inotify and Wayland trace of the original release binary under the
corrected layout then decoded 102 `OPEN` events on Files' current directory in
15 seconds, alongside 144 frame requests and 216 surface commits. Files was
forwarding those access events to its reload loop, so reading a folder could
trigger another read. The `dev` fix ignores access-only watcher events before
queuing a reload. All 214 Finder package tests passed. A focused 60-second
release-binary sample of that fix measured **0.03% idle CPU and 0.050
wake-ups/s**, with 171.9 ms warm-launch p95
([data](reference-laptop-2026-09-27-files-access-probe.json)). In a separate
passive check, the fixed Files window made zero frame requests and commits in
five untouched seconds; creating a file in its private HOME produced five
frame requests and seven commits, and its named row appeared through AT-SPI.
Tracing perturbs timing, while the budget sample does not trace the process.
The table below remains the original `8ae3a9eb` release-package baseline; the
fixed binary was packaged later in the `0e3fa470` set.

After installing the later `0e3fa470` package set, a single exploratory
private-Sway sample of `/usr/bin/rmac-files` (SHA-256
`72370ea1034a95ae9b0b288f91ef75ea1287ca9a0e94dec3b756aa7b678613c5`,
`rmac-apps` `0.9.0~beta.1-38`) reached its first-frame marker in 350.4 ms.
Over 30.0 idle seconds it used 0.767% of one CPU core, 0.2 context
switches/s and 160.6 MiB PSS. This exceeds the 0.3% idle target in that
sample, but it is not directly comparable with the live-niri, 60-second,
five-launch results above. It did not inject input or inspect pixels. Run
`scripts/linux/sample-installed-performance.py --app rmac-files` to repeat
the private sample; a full installed-package budget run remains outstanding.

The same private sampler then ran the other ten installed apps sequentially,
with 30-second idle windows and cleanup checked after each run. The
[per-app JSON and binary hashes](reference-laptop-2026-09-27-installed-private/)
are retained. The first-frame marker was reached for eight of ten apps in that
initial batch and for the other two in the corrected follow-up run:

| Installed app | Startup | Idle CPU | Context switches/s | PSS |
|---|---:|---:|---:|---:|
| App Drawer¹ | 282.4 ms | 0.033% | 1.000 | 183.9 MiB |
| Calculator | 290.7 ms | 0.000% | 0.000 | 140.6 MiB |
| Clock | 319.6 ms | 32.633% | 6.100 | 182.2 MiB |
| Notes | 1573.3 ms | 0.000% | 0.000 | 146.3 MiB |
| Preview¹ | 328.7 ms | 0.000% | 0.000 | 220.9 MiB |
| System Monitor | 405.3 ms | 18.233% | 3.500 | 169.1 MiB |
| System Settings | 395.7 ms | 0.000% | 0.033 | 167.4 MiB |
| Terminal | 395.3 ms | 0.000% | 0.000 | 144.5 MiB |
| Text Editor | 324.4 ms | 38.533% | 9.467 | 160.4 MiB |
| Weather | 318.3 ms | 42.466% | 10.200 | 156.2 MiB |

¹ App Drawer and Preview were measured in a later sequential private run after
the sampler adopted the startup smoke's supervised `--service --show` arguments
and generated PDF fixture. Their earlier `rmac-app-drawer.json` and
`rmac-preview.json` records document launcher failures, superseded by
`app-drawer.json` and `preview.json`. All eleven apps now have one exploratory
private sample. The large CPU figures and Notes startup delay require
live-session or repeated checks before they can be treated as release
regressions. Private Sway uses software rendering and the sampler performs no
interaction or pixel checks.

Measured against todo.md "Performance budgets": idle CPU <=0.3% per app and <=1% for all shell surfaces combined, no idle redraw, warm launch p95 <=500 ms (<=900 ms for Files/Terminal), memory recorded. No personal data is recorded: no hostnames, home-directory paths, window titles, or user names.

Idle window: 60.0 s; warm-launch repetitions: 5 (+1 warm-up); idle-redraw threshold: 0.5 wakeups/s.

## Shell surfaces

| Surface | Running | Idle CPU % | Budget | Wake-ups/s | Idle redraw? | PSS |
|---|---|---:|---|---:|---|---:|

## Applications

| App | Warm p95 | Budget | Interactive marker | Idle CPU % | Budget | Wake-ups/s | PSS | Frame timing |
|---|---:|---|---|---:|---|---:|---:|---|
| Apps | n/a | n/a | not_measured | n/a | n/a | n/a | n/a | not_measured |
| Calculator | 150.3 ms | <=500 ms ✓ | ready_file | 0.00% | <=0.3% ✓ | 0.000 | 41.8 MiB | not_measured |
| Clock | 167.0 ms | <=500 ms ✓ | ready_file | 1.87% | <=0.3% ✗ | 12.417 | 53.5 MiB | not_measured |
| Files | 196.2 ms | <=900 ms ✓ | ready_file | 4.57% | <=0.3% ✗ | 27.333 | 48.9 MiB | not_measured |
| Notes | 290.7 ms | <=500 ms ✓ | ready_file | 0.00% | <=0.3% ✓ | 0.000 | 48.4 MiB | not_measured |
| Preview | n/a | n/a | not_measured | n/a | n/a | n/a | n/a | not_measured |
| System Monitor | 228.0 ms | <=500 ms ✓ | ready_file | 4.45% | <=0.3% ✗ | 3.233 | 54.0 MiB | not_measured |
| System Settings | 231.3 ms | <=500 ms ✓ | ready_file | 0.05% | <=0.3% ✓ | 3.383 | 59.1 MiB | not_measured |
| Terminal | 160.3 ms | <=900 ms ✓ | ready_file | 0.00% | <=0.3% ✓ | 0.000 | 49.2 MiB | not_measured |
| Text Editor | 150.1 ms | <=500 ms ✓ | ready_file | 0.70% | <=0.3% ✗ | 6.133 | 48.4 MiB | not_measured |
| Weather | 156.1 ms | <=500 ms ✓ | ready_file | 0.70% | <=0.3% ✗ | 6.483 | 44.3 MiB | not_measured |

## Over budget

- System Monitor: idle CPU over budget
- Files: idle CPU over budget
- Text Editor: idle CPU over budget
- Clock: idle CPU over budget
- Weather: idle CPU over budget

## Method

- Idle CPU %: `/proc/<pid>/stat` utime+stime deltas summed over the surface's whole descendant process tree, divided by the elapsed wall time (one CPU-core-second == 100%).
- Wake-ups/s: `/proc/<pid>/status` voluntary_ctxt_switches + nonvoluntary_ctxt_switches deltas over the same window and tree; a rate above the configured threshold while nothing changes is flagged as a suspected idle redraw.
- Memory: PSS from `/proc/<pid>/smaps_rollup`'s `Pss:` line, not RSS.
- Warm launch -> interactive: elapsed time from spawn to `RMAC_BENCHMARK_READY_FILE` appearing when the app writes it, else a fallback to its window being mapped (reported as `window_mapped_fallback`, a weaker proxy for "interactive").
- Frame timing: not measured -- no cheap per-frame trace is available from this harness; see docs/performance-baseline.md.
