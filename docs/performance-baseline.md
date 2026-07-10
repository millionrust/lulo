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
900 ms for Finder and Terminal. At the immutable starting revision, six
applications pass and System Settings fails the simple-app budget by 602.8 ms.
The follow-up below moves its discovery off the first-frame path and clears the
failure.

The provisional idle CPU budget is 0.3% per normal app. None of the original
prototypes passes it. Terminal and App Drawer were the priority outliers, which
matched their known 33 ms and 120 ms redraw loops in the Phase 0 inventory.
Activity Monitor's periodic metric refresh is legitimate domain work; the
follow-up below profiles it and defines a separate active-refresh budget. RSS
is recorded as the starting point for setting per-app memory budgets; no memory
pass/fail threshold has been approved yet.

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

## App Drawer event-driven redraw follow-up

Revision `72b2e22c1545691d98f1c067525cf4b1f757d5ed` removed App Drawer's
unconditional 120 ms redraw timer. Search input already notified its observer,
and the asynchronous icon pass already notified GPUI after updating the model,
so the timer repainted unchanged state between real events.

After warming the persistent icon cache, two consecutive clean-revision runs
reported 0.70% idle CPU. The later run produced this comparison:

| Metric | Original `59235ff` | Event-driven `72b2e22` | Change |
|---|---:|---:|---:|
| Warm startup median | 333.1 ms | 365.7 ms | +32.6 ms |
| Warm startup p95 | 347.1 ms | 387.7 ms | +40.6 ms |
| Idle CPU | 7.10% | 0.70% | -90.1% |
| Idle RSS | 89.5 MiB | 89.9 MiB | +0.4 MiB |

Startup remains inside the 500 ms simple-app budget. Cold icon-cache discovery
is a separate active workload: it overlapped early three-second settling
windows and produced unstable idle samples while the cache was being populated.
The stable figure above therefore describes quiescent idle, not first-run icon
indexing. App Drawer still misses the provisional 0.3% idle goal; Linux catalog
events and icon-cache lifecycle need separate measurements during its
`rmac-apps` port.

## System Settings asynchronous discovery follow-up

Revision `a1de00cccb27f4388a11733051e3bbac4aaa5882` moved read-only hardware
discovery off the first-frame path. Persisted controls and the settings shell
render immediately with an explicit loading banner; a background task gathers
the system snapshot, applies it atomically, and requests one redraw.

A second clean-revision run, after the release build and first launch were
warm, produced this comparison:

| Metric | Original `59235ff` | Async discovery `a1de00c` | Change |
|---|---:|---:|---:|
| Warm startup median | 986.7 ms | 147.8 ms | -838.9 ms |
| Warm startup p95 | 1,102.8 ms | 149.2 ms | -953.6 ms |
| Idle CPU | 0.60% | 0.90% | +0.30 pp |
| Idle RSS | 71.2 MiB | 72.1 MiB | +0.9 MiB |

Warm p95 improved by 86.5% and now passes the 500 ms simple-app budget with
350.8 ms of headroom. The small idle and RSS increases require Linux
confirmation; the snapshot has no polling loop, but its completion latency is
not yet a separate harness metric. The Linux service adapters should report
their sections independently so one slow subsystem cannot hold every hardware
pane in the loading state.

## Activity Monitor refresh follow-up

Revision `7e36406e854b5d0f975c443ddf9cd9bcd88d8483` kept the two-second
sampling cadence while narrowing each collection pass. It requests CPU usage
without frequency data, loads the stable user table once, requests command and
user metadata only when not already cached, avoids a discarded disk
aggregation, and clones at most the 300 visible rows after sorting.

The first post-link run reproduced the original 2.70% CPU result. Three
subsequent clean-revision ten-second samples reported 1.40%, 2.50%, and 1.70%
(2.10% median), showing that this short window is sensitive to collector and
host warmup. The last of those runs produced this same-window comparison:

| Metric | Original `59235ff` | Bounded refresh `7e36406` | Change |
|---|---:|---:|---:|
| Warm startup median | 164.5 ms | 161.8 ms | -2.7 ms |
| Warm startup p95 | 166.3 ms | 163.0 ms | -3.3 ms |
| Active-refresh CPU | 2.70% | 1.70% | -37.0% |
| RSS | 80.0 MiB | 79.3 MiB | -0.7 MiB |

A native ten-second stack sample across five refreshes found the active work in
macOS process-enumeration calls (`sysctl` and `proc_pidinfo`); the main event
loop otherwise slept. Disabling `sysinfo`'s parallel collector increased CPU to
3.90%, so that experiment was rejected.

Activity Monitor is not a quiescent app and is exempt from the normal 0.3%
idle target while monitoring. Its provisional active-refresh budget is 2.5%
CPU averaged over 30 seconds at the two-second cadence. Two clean-revision
30-second samples measured 1.03% and 2.07%, both passing. The Linux reference
PC must repeat the same cadence and window, record process count, and verify
that collection does not cause visible frame stalls before this budget becomes
a cross-platform release threshold.

## Finder event-driven watcher follow-up

Revision `b08aa153baa6d12c84140690d3a862e0d7cd5b49` replaced Finder's 600 ms
atomic-flag polling bridge with a capacity-one event channel. Watcher bursts
wait for a 200 ms quiet edge, continuous churn is capped at one reload every
two seconds, and watcher-triggered reloads reuse cached volume-space data
instead of launching `df`. Navigation and Finder's own file operations still
refresh immediately.

The final clean-revision run produced this comparison:

| Metric | Original `59235ff` | Event-driven `b08aa15` | Change |
|---|---:|---:|---:|
| Warm startup median | 132.4 ms | 129.2 ms | -3.2 ms |
| Warm startup p95 | 136.1 ms | 132.9 ms | -3.2 ms |
| Idle CPU | 0.80% | 0.80% | no change |
| Idle RSS | 73.7 MiB | 73.6 MiB | -0.1 MiB |

The event-driven design removes periodic wakeups without regressing the
real-home measurement, but the starting directory was not quiescent during
these samples and real events still require directory reads. Finder therefore
still misses the provisional 0.3% normal-app target. The Linux reference pass
must measure both a controlled unchanged directory and a deterministic event
burst so idle wakeups, event latency, and active rescan cost are reported
separately.

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
