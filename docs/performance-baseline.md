# Application performance baseline

> Captured: 2026-07-10
>
> Source revision: `59235ff723ce99e6bb28280bcf089efc9efc52b2`
>
> Status: macOS prototype baseline; Linux and interactive frame evidence pending

## Environment

- Mac mini with Apple M1 (8 CPU and 8 GPU cores) and 16 GB memory
- macOS 26.4 (build 25E246), arm64
- 1920x1080 display at 120 Hz
- Rust release profile with LTO, one codegen unit, and stripped binaries
- No uncommitted tracked changes during capture

## Method

Run from the repository root:

```sh
python3 scripts/measure-baseline.py
```

For a focused regression measurement, pass `--package` one or more times. For
example: `python3 scripts/measure-baseline.py --package rmac-terminal`.

The harness builds all seven applications in release mode. For each app, it
performs one excluded warm-up followed by five measured launches. Startup is
the elapsed time from spawning the binary until `rmac-ui` reports completion of
the first GPUI frame. The reported p95 uses the nearest-rank method; with five
samples this is the slowest measured launch.

For the idle sample, the harness launches a fresh process, waits for its first
frame, allows three seconds to settle, then measures aggregate process-group
CPU time and RSS over ten seconds using `ps`. Build time is excluded. The raw
JSON is written under ignored `target/baselines/` output.

## Results

| Application | Warm median | Warm p95 | Idle CPU | Idle RSS |
|---|---:|---:|---:|---:|
| Activity Monitor | 164.5 ms | 166.3 ms | 2.70% | 80.0 MiB |
| App Drawer | 333.1 ms | 347.1 ms | 7.10% | 89.5 MiB |
| Finder | 132.4 ms | 136.1 ms | 0.80% | 73.7 MiB |
| Notes | 126.6 ms | 131.0 ms | 0.60% | 69.5 MiB |
| System Settings | 986.7 ms | 1,102.8 ms | 0.60% | 71.2 MiB |
| Terminal | 127.3 ms | 130.2 ms | 10.29% | 71.7 MiB |
| Text Editor | 126.4 ms | 130.8 ms | 0.60% | 67.7 MiB |

The table above is the immutable `59235ff` starting point. Terminal's original
idle result predates the event-driven redraw fix documented below.

## Budget comparison

The Phase 0 provisional warm-launch budgets are 500 ms p95 for simple apps and
900 ms for Finder and Terminal. Six applications pass their applicable budget.
System Settings fails the simple-app budget by 602.8 ms; its synchronous system
discovery must move off the first-frame path during its service-layer port.

The provisional idle CPU budget is 0.3% per normal app. None of the original
prototypes passes it. Terminal and App Drawer were the priority outliers, which
matched their known 33 ms and 120 ms redraw loops in the Phase 0 inventory.
Activity Monitor's periodic metric refresh is legitimate domain work, but its
current 2.70% result still needs profiling and an explicit active-refresh
budget. RSS is recorded as the starting point for setting per-app memory
budgets; no memory pass/fail threshold has been approved yet.

## Terminal event-driven redraw follow-up

Revision `2e40ffc3b1d776222a2f2184f4dde1439fcdfad4` replaced Terminal's
unconditional 33 ms timer with a bounded async channel. The blocking PTY reader
wakes GPUI only after model input; a channel capacity of one coalesces output
bursts while preserving the newest grid state. The shell fallback also changed
from macOS-specific `/bin/zsh` to portable `/bin/sh` when `$SHELL` is absent.

The same release-mode harness and machine produced this clean-revision result:

| Metric | Original `59235ff` | Event-driven `2e40ffc` | Change |
|---|---:|---:|---:|
| Warm startup median | 127.3 ms | 129.1 ms | +1.8 ms |
| Warm startup p95 | 130.2 ms | 135.7 ms | +5.5 ms |
| Idle CPU | 10.29% | 1.00% | -90.3% |
| Idle RSS | 71.7 MiB | 71.5 MiB | -0.2 MiB |

The startup difference is small relative to run-to-run launch noise and remains
well inside Terminal's 900 ms budget. The idle improvement is material but does
not yet meet the provisional 0.3% goal. The Linux reference-PC run must confirm
the event-driven behavior under Wayland and identify the remaining base cost.

## Limits and next evidence

These measurements are a reproducible comparison point, not release evidence.
They cover one macOS machine, a short idle window, direct binary launch, and no
automated user interaction. Five startup samples are sufficient to expose
large regressions but not to estimate a production-grade percentile.

The harness does not yet capture wakeups, interactive frame time, frame misses,
GPU utilization, packaged-launch overhead, cold-cache startup, or energy use.
Those require longer platform-native traces and a deterministic interaction
journey. Repeat the baseline on the Ubuntu reference machines after Phase 1,
then record interactive frame timing at 60 Hz, 120 Hz, and fractional scale.
