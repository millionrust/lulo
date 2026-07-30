# Performance release audit

I4 turns the existing prototype baseline into an exact Linux release gate.
`scripts/performance-budgets.json` is the reviewed authority and
`scripts/verify-performance-audit.py` binds results to it, the ten product
journeys, the H8 station matrix, and the exact candidate revision.

The 89 budgets cover all seven applications, the combined shell, and every
product journey:

- warm first-frame launch uses three excluded warmups and 20 samples; simple
  apps must remain at or below 500 ms p95, while Files and Terminal retain the
  reviewed 900 ms allowance;
- a 60-second quiescent sample limits normal applications to 0.3% CPU and 12
  wakeups/minute. System Monitor may use 2.5% and 60 wakeups/minute while its
  two-second collector is active;
- each app is limited to 128 MiB idle RSS and 16 MiB absolute RSS growth over
  eight hours;
- the complete shell is limited to 0.5% idle CPU, 30 wakeups/minute, 256 MiB
  RSS, and 24 MiB absolute growth over eight hours; and
- every journey measures 50 ms input-response p95, 16 ms frame p95 at 60 Hz,
  8 ms frame p95 at 120 Hz, and no more than 1% missed frames at either rate
  during a deterministic 60-second interaction trace.

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

Reuse `scripts/measure-baseline.py` for application launch/CPU/RSS input, but
run with the stricter protocol and release packages. Use platform-native
tracing for wakeups, presentation timestamps, missed frames, input-to-present
latency, and the eight-hour leak window. Measure quiescent idle separately from
intentional refresh or background work. Record the non-negative observed value
and change `status` to `pass` only when the trace was reviewed and the value
does not exceed its pinned limit.

Verify the exact station directory for the intended tier:

```sh
python3 scripts/verify-performance-audit.py \
  --evidence-dir /absolute/review/path \
  --tier alpha \
  --revision "$(git rev-parse HEAD)"
```

Alpha requires both named Alpha stations, Beta adds NVIDIA, and 1.0 requires
all five H8 stations. Missing, extra, stale, reordered, non-finite, negative,
pending, skipped, or over-budget measurements fail closed. Keep raw traces,
process IDs, paths, environment dumps, device identities, and personal content
in ignored evidence storage. A valid summary is not proof without the reviewed
native trace.
