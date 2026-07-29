# System Monitor application specification

This file describes the behavior currently implemented by
`rmac-system-monitor`. Metrics are shown only when backed by the local system;
the application does not fabricate unavailable per-process data.

## Core journeys

- Browse, search, select, and sort the live process table across CPU, memory,
  energy, and disk views.
- Inspect real aggregate history, per-core CPU usage, memory pressure, and
  system-wide network-interface counters.
- Choose visible process columns and retain that preference locally.
- Inspect a selected process, request Quit or Force Quit, review the exact PID
  and process name in a confirmation dialog, then confirm or cancel. Confirmation
  revalidates the PID, start time, and name before sending any signal.

## Platform authorities

- `sysinfo` supplies process, CPU, memory, disk-I/O, users, and network data.
- Linux CPU user/system/idle proportions use cumulative kernel CPU ticks.
- Network data is system-wide per interface. No per-process network metric is
  shown because the current authority cannot provide one reliably.
- Energy impact is explicitly an app-local approximation derived from real CPU
  and interval disk-I/O signals, not a kernel or hardware energy measurement.
- The isolated column domain owns stable identities, display metadata,
  canonical preference parsing, retired-path migration, and typed atomic
  load/save. The root view consumes only its validated visible-column set.
- The metric domain owns tab-to-sort policy, the Network summary-only rule,
  aggregate/interface snapshots, bounded 60-sample history, refresh interval,
  and exact byte/rate/duration presentation.
- The process-table domain owns the retained `System` authority, complete and
  bounded visible snapshots, filtering, sorting, column projection, PID-stable
  selection, row rendering, and row-scoped Quit/Force Quit actions. The root
  view coordinates that domain without owning its table-widget implementation.
- The view-render boundary owns the GPUI projection for summaries, histories,
  network rows, toolbar, process table, column chooser, inspector, confirmation,
  feedback, and shortcuts. It invokes controller operations without owning
  metric sampling, persistence, process identity, or signal-delivery policy.

## Sampling and failure states

- One retained `System` instance refreshes on a two-second off-render-path
  cadence so delta-based metrics remain meaningful.
- The selected PID is retained separately from the visible row index so a
  refresh cannot silently retarget a process action.
- Column-preference load/save failures appear in the visible error banner.
- Process disappearance, PID reuse, unsupported signals, permission rejection,
  and signal failure produce distinct visible results. A successful system call
  is described only as signal delivery, never as proof that the process exited.
- Linux opens a kernel pidfd before the confirmation-time identity refresh and
  sends `SIGTERM`/`SIGKILL` through that handle. A kernel without pidfd support
  fails closed as unavailable; it never falls back to a reuse-prone numeric PID.
  Non-Linux development retains the identity-checked `sysinfo` signal path.
- A targeted confirmation-time refresh cannot add a synthetic history sample or
  disturb the two-second delta cadence. History retains exactly 60 samples.

## Keyboard map

- `Cmd-F`: focus process search.
- `Cmd-Backspace`: request Quit for the selected process.
- `Shift-Cmd-Backspace`: request Force Quit for the selected process.
- `Enter`: confirm an open process-action dialog.
- `Escape`: cancel an open process-action dialog.

## Visual and accessibility states

- CPU, Memory, Energy, Disk, and Network tabs have distinct summaries backed by
  their corresponding sampled data.
- Selection, sorting, filtering, column chooser, inspector, confirmation,
  empty/unavailable data, and persistence-error states are visually distinct.
- All System Monitor labels, metrics, tables, inspector text, and banners
  follow rmac's bounded 100%, 115%, and 130% application text preference.
- Linux clipping, pidfd runtime, permission, and signal evidence; full keyboard
  traversal; accessible table semantics, roles/names/states/actions, live
  announcements; measured active refresh budget; and Orca evidence remain
  required by roadmap items G5 and I3.
