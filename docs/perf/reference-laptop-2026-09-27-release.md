# Reference laptop performance budgets -- 2026-09-27T05:37:32Z

This run used release binaries built from `8ae3a9eb`, one app at a time on
the reference laptop's live niri session, with a separate temporary HOME,
config, data, state, and cache directory for each app. No input was injected
and no Cargo process ran. Only the nine applications listed below were
selected. Apps needs its service launch path and Preview needs a document,
so the generic launch method did not measure them; shell surfaces were also
excluded. Clock starts on its World view with a moving second hand, and
Weather starts with search focused in the empty profile. The
[machine-readable report](reference-laptop-2026-09-27-release.json) records
each sample and its limits.

Measured against todo.md "Performance budgets": idle CPU <=0.3% per app and <=1% for all shell surfaces combined, no idle redraw, warm launch p95 <=500 ms (<=900 ms for Files/Terminal), memory recorded. No personal data is recorded: no hostnames, home-directory paths, window titles, or user names.

Idle window: 60.0 s; warm-launch repetitions: 5 (+1 warm-up); idle-redraw threshold: 0.5 wakeups/s.

## Shell surfaces

| Surface | Running | Idle CPU % | Budget | Wake-ups/s | Idle redraw? | PSS |
|---|---|---:|---|---:|---|---:|

## Applications

| App | Warm p95 | Budget | Interactive marker | Idle CPU % | Budget | Wake-ups/s | PSS | Frame timing |
|---|---:|---|---|---:|---|---:|---:|---|
| Calculator | 144.2 ms | <=500 ms ✓ | ready_file | 0.00% | <=0.3% ✓ | 0.000 | 45.5 MiB | not_measured |
| Clock | 166.7 ms | <=500 ms ✓ | ready_file | 1.78% | <=0.3% ✗ | 12.283 | 55.7 MiB | not_measured |
| Files | 167.5 ms | <=900 ms ✓ | ready_file | 7.60% | <=0.3% ✗ | 32.100 | 54.2 MiB | not_measured |
| Notes | 250.6 ms | <=500 ms ✓ | ready_file | 0.00% | <=0.3% ✓ | 0.000 | 51.5 MiB | not_measured |
| System Monitor | 230.2 ms | <=500 ms ✓ | ready_file | 3.70% | <=0.3% ✗ | 3.150 | 56.9 MiB | not_measured |
| System Settings | 235.5 ms | <=500 ms ✓ | ready_file | 0.05% | <=0.3% ✓ | 2.383 | 61.8 MiB | not_measured |
| Terminal | 151.5 ms | <=900 ms ✓ | ready_file | 0.00% | <=0.3% ✓ | 0.000 | 52.4 MiB | not_measured |
| Text Editor | 150.8 ms | <=500 ms ✓ | ready_file | 0.63% | <=0.3% ✗ | 6.267 | 55.5 MiB | not_measured |
| Weather | 156.3 ms | <=500 ms ✓ | ready_file | 0.60% | <=0.3% ✗ | 6.233 | 47.4 MiB | not_measured |

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
