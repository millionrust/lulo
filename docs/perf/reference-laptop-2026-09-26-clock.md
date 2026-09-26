# Reference laptop Clock performance -- 2026-09-26

> Status: focused before/after measurement of Clock's idle ticker on the
> reference laptop (i5-5300U / HD 5500, Ubuntu 26.04, niri). The before binary
> was `~/rmac-dev-bin/rmac-clock`; the after binary was built from
> `a9bcb1bd` with the `iterate` profile. No Cargo process ran during either
> sample, and the application did not receive injected input.

Both runs used `scripts/linux/measure-budgets.py --app rmac-clock
--skip-surfaces`, the shared `/tmp/lulo-journey.lock`, one warm-up and five
launch samples, then a 30-second idle window after three seconds of settling.
The app used temporary XDG data, state, and config directories. Startup ended
at the app's `RMAC_BENCHMARK_READY_FILE` marker. CPU and wake-ups cover the
launched process group; memory is PSS from `/proc/<pid>/smaps_rollup`.

| Metric | Before | After | Budget |
|---|---:|---:|---:|
| Warm launch p95 | 255.9 ms | 183.5 ms | 500 ms |
| Warm launch median | 160.7 ms | 171.1 ms | — |
| Idle CPU | 3.767% | 0.467% | 0.3% |
| Idle wake-ups | 41.933/s | 3.200/s | 12/min |
| Idle PSS | 96.7 MiB | 95.3 MiB | 128 MiB |
| Frame timing | not measured | not measured | p95 ≤16 ms at 60 Hz |

The prior loop refreshed and notified GPUI every 250 ms, including static
tabs. The new loop checks static tabs every five seconds and redraws only when
the saved state changes; visible world clocks and running countdowns redraw
once per second, while an active stopwatch keeps its 33 ms cadence. The test
run confirms the active stopwatch cadence and static-tab behavior. Idle CPU
fell 87.6% and wake-ups fell 92.4%; launch and memory stayed within their
budgets. Idle CPU remains 0.167 percentage points over budget, and wake-ups
remain above 12/min, so Clock still fails its complete idle budget. The
remaining 3.2 wake-ups/s are consistent with GPUI's existing Linux idle-frame
recheck; this run does not attribute them conclusively. No per-frame trace is
available from this harness.

Raw, privacy-safe reports: [`before`](reference-laptop-2026-09-26-clock-before.json)
and [`after`](reference-laptop-2026-09-26-clock-after.json).
