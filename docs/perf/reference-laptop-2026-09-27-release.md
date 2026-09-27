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

The previous report measured Files at 7.60% idle CPU and 32.100 wake-ups/s.
Its marker and XDG directories were inside Files' watched HOME or parent,
creating filesystem events during measurement. This corrected run measures
4.57% and 27.333/s; an independent focused run with the same layout measured
4.58% and 27.433/s. The earlier 30-second thread sample, which attributed
6.767% CPU to the main UI thread, used the contaminated layout and cannot
establish the source of the remaining cost. Files still misses the 0.3% budget;
a frame/event-loop profile is needed before attributing a code fix. Separate
15-second diagnostics under the corrected layout counted 150 Files frame
requests and 225 Wayland surface commits (Notes: zero of each); a traced
Files process returned 54 successful inotify reads in 15 seconds. Protocol
logging and tracing perturb timing, so those counts identify active paths,
not benchmark rates.

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
