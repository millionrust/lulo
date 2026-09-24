# Reference laptop performance budgets -- 2026-09-24

> Status: **smoke validation only.** The real todo.md "Performance budgets"
> measurement run is **pending**. See "Why this is not the real run" below.

## What this is

`scripts/linux/measure-budgets.py` is a new, self-contained harness for
todo.md's "Performance budgets" table. Copied to the reference laptop and run
there (the same pattern as `scripts/linux/run-journey-launch.py`, see
[`docs/journey-suite.md`](../journey-suite.md)), it measures:

- **idle CPU %**, from `/proc/<pid>/stat` utime+stime deltas summed over each
  surface/app's whole descendant process tree, over a fixed window;
- **idle wake-ups/s**, from `/proc/<pid>/status`
  `voluntary_ctxt_switches` + `nonvoluntary_ctxt_switches` deltas over the
  same window -- a rate above a configurable threshold while nothing changes
  is flagged as a suspected idle redraw;
- **PSS memory**, from `/proc/<pid>/smaps_rollup`'s `Pss:` line (not RSS --
  PSS is what is comparable across the shell's many small processes, per the
  project's own GPUI/Linux performance notes);
- **warm launch -> interactive time**, preferring the app writing
  `RMAC_BENCHMARK_READY_FILE` (as `scripts/measure-baseline.py` already
  does), falling back to its window being mapped in
  `niri msg --json windows` when that marker is not yet implemented, and
  saying which one was used;
- **frame timing**: left as `not_measured` everywhere -- there is no cheap
  per-frame trace available from this harness (see
  [`docs/performance-baseline.md`](../performance-baseline.md) "Limits and
  next evidence").

It covers every shell surface named in the shared brief (`rmac-top-bar`,
`rmac-dock`, `rmac-wallpaper`, `rmac-launcher`, `rmac-quick-settings`,
`rmac-notification-center` + `-panel`, `rmac-osd`, `rmac-app-switcher`,
`rmac-screenshot`, `rmac-mission-control`, `rmac-session-supervisor`,
`rmac-shortcut-broker`, `rmac-focus`, `rmac-clipboard`) and every packaged
application (System Monitor, Apps, Files, Notes, System Settings, Terminal,
Text Editor), and emits both JSON and this Markdown summary in one command.
Full usage is in the script's module docstring.

## Why this is not the real run

The reference laptop was running a background `cargo build --profile
iterate` (an unrelated in-flight `rustc` build) for this entire session --
`pgrep -c rustc` reported 3 throughout. Per the shared brief, that means only
a short smoke run to validate the script, not the real measurement: idle
CPU/wake-up numbers on a laptop actively compiling are meaningless (an idle
app measured 12.99% CPU and 190.9 wake-ups/s here, only because the build was
saturating all cores, not because of any regression). The **real** run must
be repeated once `pgrep -c rustc` reports 0, using:

```sh
scp scripts/linux/measure-budgets.py jacob@<reference-pc>:/tmp/
ssh jacob@<reference-pc> '
  exec 9>/tmp/lulo-journey.lock
  flock -w 900 9 &&
  python3 /tmp/measure-budgets.py \
    --json-output /tmp/rmac-budgets.json \
    --markdown-output /tmp/rmac-budgets.md
'
```

with the default 60 s idle window and 5 warm-launch repetitions, and its
output committed to `docs/perf/` in place of this smoke record.

## What was validated

Two live, end-to-end smoke runs (raw JSON in
[`reference-laptop-2026-09-24.json`](reference-laptop-2026-09-24.json)),
each wrapped in the shared laptop's `flock`:

1. Shell-surface sampling (`rmac-top-bar`, `rmac-dock`) plus one app's full
   launch -> idle -> close cycle (Text Editor): confirmed `systemctl --user
   show -p MainPID` lookups, `/proc` sampling, and report emission all work
   against the live session.
2. A second app (Notes), after a fix below, to confirm binary discovery
   across both of rmac's install layouts.

Both live runs surfaced real bugs in the first draft, now fixed in the
committed script:

- **Process-group leak under contention.** The first Text Editor idle sample
  left a live `rmac-text-editor` process behind after `stop_app()` returned
  successfully: the tracked launcher pid exited on its own (unrelated to the
  `SIGTERM`) while a long-lived grandchild -- still a member of the same
  process group, but reparented to `systemd --user` -- kept running. Found
  by hand (`ps -eo pid,ppid,pgid,args`) immediately after the run and killed
  before it could interfere with anything else on the shared laptop.
  `stop_app()` now always re-scans by process-group membership (which
  survives reparenting, unlike a ppid-based descendant walk) after every
  stop and force-kills any survivor individually by pid, regardless of
  whether `wait()` reported success.
- **Wrong binary search path for apps.** `discover_binary()` only looked in
  `~/.local/libexec/rmac` and `/usr/libexec/rmac` (where the shell surface
  systemd units run from) and missed `/usr/bin`, where a packaged
  application without a dev override actually lives (`rmac-notes` measured
  "binary not found" until this was added).

No screenshots, home-directory paths, hostnames, or window titles are
recorded anywhere in this document or its JSON.

## Smoke numbers (not the budget record -- see above)

| Surface | Running | Idle CPU % | Wake-ups/s | PSS |
|---|---|---:|---:|---:|
| rmac-top-bar | yes | 2.33% | 28.664 | 56.3 MiB |
| rmac-dock | yes | 0.00% | 8.402 | 50.2 MiB |

| App | Warm launch | Interactive marker | Idle CPU % | Wake-ups/s | PSS |
|---|---:|---|---:|---:|---:|
| Text Editor | 4,043.7 ms | ready_file | 3.33% | 75.331 | 72.7 MiB |
| Notes | 698.8 ms | ready_file | 12.99% | 190.858 | 49.5 MiB |

All of the above fail the todo.md budgets purely because of the concurrent
build; none of it should be read as a regression or a pass/fail verdict.

## Pure-logic tests

`scripts/test_measure_budgets.py` covers `/proc` parsing (`stat`, `status`,
`smaps_rollup`), the CPU%/wake-up-rate math, budget evaluation, and the
Markdown renderer, without touching a live `/proc`, niri, or systemd session
(same pattern as `scripts/test_journey_launch.py`). 32 tests, run with:

```sh
python3 -m pytest scripts/test_measure_budgets.py
```
