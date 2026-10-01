# Performance release audit

I4 turns the existing prototype baseline into an exact Linux release gate.
`scripts/performance-budgets.json` is the reviewed authority and
`scripts/verify-performance-audit.py` binds results to it, the ten product
journeys, the H8 station matrix, and the exact candidate revision.

The 114 budgets cover all twelve packaged windowed applications, the combined
shell, and every product journey. Archive Utility is a no-window file task;
its success and cleanup need separate fixture-based job evidence rather than
an idle-window or first-frame measurement.

- warm first-frame launch uses three excluded warmups and 20 samples; simple
  apps must remain at or below 500 ms p95, while Files and Terminal retain the
  reviewed 900 ms allowance;
- a 60-second quiescent sample limits normal applications to 0.3% CPU and 12
  wakeups/minute. System Monitor may use 2.5% and 60 wakeups/minute while its
  five-second collector is active;
- each app is limited to 128 MiB idle RSS and 16 MiB absolute RSS growth over
  eight hours;
- the complete shell is limited to 0.5% idle CPU, 30 wakeups/minute, 256 MiB
  RSS, and 24 MiB absolute growth over eight hours; and
- every journey measures 50 ms input-response p95, 16 ms frame p95 at 60 Hz,
  8 ms frame p95 at 120 Hz, and no more than 1% missed frames at either rate
during a deterministic 60-second interaction trace.

The live shell sampler now captures every running shell service in one shared
60-second idle window and counts a process only once in the combined CPU
total, even if two service process trees overlap. A unit absent at baseline
but running at the final check, or a baseline unit whose main PID exits or
changes, invalidates the combined result. Earlier sequential per-service sums
cannot establish the combined-shell release budget.

A read-only 30-second sample of the installed top bar on 2026-09-28 recorded
0.133% one-core CPU, 0.033 context switches/s and 58.3 MiB PSS
([raw sample](perf/reference-laptop-2026-09-28-live-topbar-idle.json)). This
passes the top bar's individual 0.3% CPU limit, but covers only one surface
for half the required idle interval. The older sampler prints a 1.0% combined
threshold in this one-surface report; the reviewed release threshold is 0.5%,
and this report cannot decide that combined gate.

A separate read-only 75.62-second snapshot of all 16 installed shell service
roots and their two observed descendants measured 0.185% one-core CPU combined
([raw sample](perf/reference-laptop-2026-09-28-live-shell-idle.json)). The
processes kept the same start times, and a follow-up process-tree scan found
the same descendants. This is useful evidence under the 0.5% CPU threshold,
but its two-point sampling can miss a short-lived child; the full sampler has
not run on the installed desktop, and other shell budgets remain open.

The [2026-10-01 nested memory comparison](perf/reference-laptop-2026-10-01-memory-diet.md)
reads `smaps_rollup` rather than inferring memory from RSS alone. On the
reference Intel GPU, lazy OSD surfaces and release of the wallpaper's decoded
pixel cache reduced the sampled shell from 637.49 to 538.53 MiB summed RSS
and 399.77 to 361.98 MiB summed PSS. Nine of eleven opened app windows met
128 MiB RSS; Player (160.17 MiB) and System Settings (131.30 MiB) did not.
The 256 MiB combined shell RSS limit also still fails. These five-second idle
samples do not replace the release-profile, eight-hour soak gate.

The shared-window shell probe can be run without staging files on the PC:

```sh
ssh -o BatchMode=yes jacob@192.168.18.52 \
  'exec 9>/tmp/lulo-journey.lock; flock -w 15 9 && python3 - --skip-apps --idle-seconds 60 --json-output /dev/stdout' \
  < scripts/linux/measure-budgets.py
```

It reads the installed session and starts no apps. Save the JSON output only
after confirming the installed package revision; its result is not a substitute
for the remaining app, input, frame, soak and hardware-station measurements.

Validate the contract without building the workspace:

```sh
python3 scripts/verify-performance-audit.py
```

## Capturing a station

Use the release profile in the full Ubuntu 26.04 rmac/niri session with a clean
candidate revision. Stabilize power mode and thermals, disable unrelated user
workloads, use synthetic fixture data, and record the public H8 station ID
instead of hardware serials or model strings. The 60 and 120 Hz runs must use
real outputs at those refresh rates; a timer simulation is not evidence.

Generate one pending template per required H8 station outside the repository:

```sh
python3 scripts/verify-performance-audit.py \
  --print-template \
  --station amd64-intel-laptop \
  --revision "$(git rev-parse HEAD)" \
  > /absolute/review/path/amd64-intel-laptop.json
```

The historical `scripts/measure-baseline.py` covers seven prototype apps.
`scripts/linux/measure-budgets.py` now covers all twelve installed windowed
apps; it launches Player with a short silent WAV so its idle window starts
after playback. Use these launch/CPU/RSS methods as inputs, and measure every
budgeted app with the stricter protocol and release packages. Use platform-native
tracing for wakeups, presentation timestamps, missed frames, input-to-present
latency, and the eight-hour leak window. Measure quiescent idle separately from
intentional refresh or background work; Player must not be playing a fixture
during its idle window. Record the non-negative observed value
and change `status` to `pass` only when the trace was reviewed and the value
does not exceed its pinned limit.

For an exploratory CPU attribution sample on the Linux reference host, run
`python3 scripts/linux/sample-installed-performance.py --app rmac-text-editor
--idle-seconds 60 --thread-breakdown` while no build is active. This launches
one installed app in a private nested Sway session and reports the eight
busiest app-process threads during its idle window. The optional thread data
can distinguish a busy UI thread from a background worker; it does not show
which function, redraw, or input event caused the work. The percentages are
approximate, omit threads that were not present at both sample endpoints, and
are not release-gate evidence. The report includes binary provenance and
local process information, so keep it in private evidence storage.

`scripts/analyze-interaction-trace.py` summarizes a normalized native trace
export without injecting input or changing the host. Its format 1 JSON input
contains a 60-second window, paired input and first-visible-presentation
timestamps, and per-frame timestamp/duration/missed observations for one real
60 or 120 Hz output. It reports nearest-rank input-to-visible p95,
frame-duration p95, and missed-frame percentage. The analyzer requires at
least 20 paired interactions and 99% of the expected output-frame samples
for the trace duration and refresh rate, spanning the full window. It counts
unreported output slots as missed frames. Example:

```sh
python3 scripts/analyze-interaction-trace.py /absolute/review/path/trace.json
```

The `within_limits` field compares those measurements with the reviewed gate.
The output always marks `native_trace_review_required: true`: normalized JSON
can be synthesized, and this analyzer cannot prove event origin, output refresh,
or that the visible timestamp is the first response. Keep the native trace and
review its provenance before transferring values into station evidence. The
existing `RMAC_BENCHMARK_READY_FILE` marker measures an app's first content
frame at launch; it does not instrument input responses or subsequent frames.

Verify the exact station directory for the intended tier:

```sh
python3 scripts/verify-performance-audit.py \
  --evidence-dir /absolute/review/path \
  --tier alpha \
  --revision "$(git rev-parse HEAD)"
```

Alpha requires both named Alpha stations, Beta adds NVIDIA, and 1.0 requires
all five H8 stations. Missing, extra, stale, reordered, non-finite, negative,
pending, skipped, or over-budget measurements fail closed. The verifier also
requires the evidence revision to equal the current Git HEAD and refuses a
dirty checkout when checking station results. Template generation does not
require that final clean state. Keep raw traces, process IDs, paths,
environment dumps, device identities, and personal content
in ignored evidence storage. A valid summary is not proof without the reviewed
native trace.
