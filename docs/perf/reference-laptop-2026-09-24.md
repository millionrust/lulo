# Reference laptop performance budgets -- 2026-09-24

> Status: **real measurement of the shell surfaces; applications measured
> under build contention.** This replaces the earlier smoke-only record.

Run: `scripts/linux/measure-budgets.py --binary-dir ~/rmac-dev-bin` on the
reference laptop (i5-5300U / HD 5500 / 6.7 GB, niri, dev builds from the
coordinator's 09:51 IST deploy of `dev` at 5bea40c), under the shared screen
lock, started 2026-09-24 04:30:23Z (10:00 IST). Raw data:
[`reference-laptop-2026-09-24.json`](reference-laptop-2026-09-24.json).

## Contention

- `pgrep -c rustc` was **0** when the run started and stayed 0 for the whole
  shell-surface phase (10:00-10:16 IST). **Surface numbers are clean.**
- A coordinator `cargo build --profile iterate -p rmac-system-settings`
  started at 10:16:07 IST, at the moment the application phase began, and ran
  through it (`rustc` = 1 at the end). Per-process idle CPU and wake-ups of
  an idle app are that app's own counters and remain indicative, but **warm
  launch times are inflated** by the four-core contention. Re-run
  `--skip-surfaces` with `rustc` at 0 before quoting launch numbers.
- Some surfaces still ran the old Sep-20 `/usr/libexec/rmac` build at the
  time: `rmac-session-supervisor`, `rmac-shortcut-broker`, `rmac-focus`
  (no dev drop-ins for them). Everything else was the dev build.

## Reading the numbers

- Every visible GPUI layer surface (top bar, Dock, wallpaper, OSD) shows about
  4 wake-ups/s per window even when nothing changes. That is the idle-frame
  re-check in the vendored `gpui_linux` (`idle_check_delay`, 16 ms growing to
  250 ms, `shell/compat/gpui_linux/src/linux/wayland/window.rs`, ADR 0013):
  cheap, but not zero, so the harness flags it as an idle redraw. Getting to
  zero needs GPUI to tell the platform when a window becomes dirty instead of
  the platform asking every 250 ms.
- `rmac-top-bar` is the real outlier: **1.98% idle CPU, 24 wake-ups/s**, more
  than the whole rest of the shell together. Its timerfds show a one-second
  re-armed GPUI timer on top of the 250 ms idle check. Cause not yet found;
  see docs/system-audit-2026-09-24.md (P1).
- `rmac-shortcut-broker` (0.28%, 23 wake-ups/s) was re-asking a
  GlobalShortcuts portal that refuses it every 2 s; fixed on the
  `system-audit` branch (exponential back-off to 5 min), pending deploy.
- `rmac-session-supervisor` (7 wake-ups/s, old build) and `rmac-focus`
  (3.8/s, old build) should be re-measured on dev builds.
- System Settings idles at **24.95% CPU / 164 wake-ups/s**: a real defect,
  independent of the build (the contention cannot put CPU time on an idle
  process's own counters).
- Apps: the harness launched `rmac-app-drawer`, which exits 0 at once because
  the resident drawer service already owns it; Preview opened no window
  within 20 s when started without a file (it asks for one through the
  FileChooser portal, which on this laptop still resolves to GNOME's).

## Shell surfaces

| Surface | Running | Idle CPU % | Budget | Wake-ups/s | Idle redraw? | PSS |
|---|---|---:|---|---:|---|---:|
| rmac-app-switcher | yes | 0.00% | <=0.3% ✓ | 0.000 | no | 6.8 MiB |
| rmac-clipboard | yes | 0.02% | <=0.3% ✓ | 0.517 | yes | 2.6 MiB |
| rmac-dock | yes | 0.12% | <=0.3% ✓ | 8.000 | yes | 52.8 MiB |
| rmac-focus | yes | 0.02% | <=0.3% ✓ | 3.817 | yes | 2.0 MiB |
| rmac-launcher | yes | 0.02% | <=0.3% ✓ | 0.000 | no | 17.8 MiB |
| rmac-mission-control | yes | 0.02% | <=0.3% ✓ | 0.000 | no | 6.0 MiB |
| rmac-notification-center | yes | 0.03% | <=0.3% ✓ | 0.000 | no | 12.5 MiB |
| rmac-notification-center-panel | yes | 0.02% | <=0.3% ✓ | 0.000 | no | 12.4 MiB |
| rmac-osd | yes | 0.03% | <=0.3% ✓ | 4.000 | yes | 28.6 MiB |
| rmac-quick-settings | yes | 0.03% | <=0.3% ✓ | 0.000 | no | 12.8 MiB |
| rmac-screenshot | yes | 0.02% | <=0.3% ✓ | 0.000 | no | 7.2 MiB |
| rmac-session-supervisor | yes | 0.07% | <=0.3% ✓ | 7.067 | yes | 0.4 MiB |
| rmac-shortcut-broker | yes | 0.28% | <=0.3% ✓ | 22.983 | yes | 1.8 MiB |
| rmac-top-bar | yes | 1.98% | <=0.3% ✗ | 24.250 | yes | 61.9 MiB |
| rmac-wallpaper | yes | 0.13% | <=0.3% ✓ | 4.150 | yes | 85.6 MiB |
| **All shell surfaces combined** | -- | 2.78% | <=1.0% ✗ | -- | -- | -- |

## Applications

| App | Warm p95 | Budget | Interactive marker | Idle CPU % | Budget | Wake-ups/s | PSS | Frame timing |
|---|---:|---|---|---:|---|---:|---:|---|
| Apps | n/a | n/a | not_measured | n/a | n/a | n/a | n/a | not_measured |
| Calculator | 378.8 ms | <=500 ms ✓ | ready_file | 0.02% | <=0.3% ✓ | 4.017 | 43.0 MiB | not_measured |
| Clock | 390.3 ms | <=500 ms ✓ | ready_file | 1.58% | <=0.3% ✗ | 36.333 | 54.8 MiB | not_measured |
| Files | 425.8 ms | <=900 ms ✓ | ready_file | 5.32% | <=0.3% ✗ | 60.383 | 54.3 MiB | not_measured |
| Notes | 719.1 ms | <=500 ms ✗ | ready_file | 0.03% | <=0.3% ✓ | 4.017 | 50.2 MiB | not_measured |
| Preview | n/a | n/a | not_measured | n/a | n/a | n/a | n/a | not_measured |
| System Monitor | 532.3 ms | <=500 ms ✗ | ready_file | 3.13% | <=0.3% ✗ | 24.683 | 56.4 MiB | not_measured |
| System Settings | 519.7 ms | <=500 ms ✗ | ready_file | 24.95% | <=0.3% ✗ | 164.479 | 79.8 MiB | not_measured |
| Terminal | 584.0 ms | <=900 ms ✓ | ready_file | 0.02% | <=0.3% ✓ | 4.033 | 51.5 MiB | not_measured |
| Text Editor | 465.8 ms | <=500 ms ✓ | ready_file | 0.65% | <=0.3% ✗ | 20.083 | 49.5 MiB | not_measured |
| Weather | 253.4 ms | <=500 ms ✓ | ready_file | 0.67% | <=0.3% ✗ | 19.633 | 46.8 MiB | not_measured |

## Over budget

- rmac-top-bar: idle CPU over budget
- rmac-top-bar: suspected idle redraw
- rmac-dock: suspected idle redraw
- rmac-wallpaper: suspected idle redraw
- rmac-osd: suspected idle redraw
- rmac-session-supervisor: suspected idle redraw
- rmac-shortcut-broker: suspected idle redraw
- rmac-focus: suspected idle redraw
- rmac-clipboard: suspected idle redraw
- all shell surfaces combined: idle CPU over budget
- System Monitor: warm launch p95 over budget
- System Monitor: idle CPU over budget
- Files: idle CPU over budget
- Notes: warm launch p95 over budget
- System Settings: warm launch p95 over budget
- System Settings: idle CPU over budget
- Text Editor: idle CPU over budget
- Clock: idle CPU over budget
- Weather: idle CPU over budget

## Method

- Idle CPU %: `/proc/<pid>/stat` utime+stime deltas summed over the surface's whole descendant process tree, divided by the elapsed wall time (one CPU-core-second == 100%).
- Wake-ups/s: `/proc/<pid>/status` voluntary_ctxt_switches + nonvoluntary_ctxt_switches deltas over the same window and tree; a rate above the configured threshold while nothing changes is flagged as a suspected idle redraw.
- Memory: PSS from `/proc/<pid>/smaps_rollup`'s `Pss:` line, not RSS.
- Warm launch -> interactive: elapsed time from spawn to `RMAC_BENCHMARK_READY_FILE` appearing when the app writes it, else a fallback to its window being mapped (reported as `window_mapped_fallback`, a weaker proxy for "interactive").
- Frame timing: not measured -- no cheap per-frame trace is available from this harness; see docs/performance-baseline.md.
