# Activity Monitor application specification

This file describes the behavior currently implemented by
`rmac-activity-monitor`. Metrics are shown only when backed by the local system;
the application does not fabricate unavailable per-process data.

## Core journeys

- Browse, search, select, and sort the live process table across CPU, memory,
  energy, and disk views.
- Inspect real aggregate history, per-core CPU usage, memory pressure, and
  system-wide network-interface counters.
- Choose visible process columns and retain that preference locally.
- Inspect a selected process, request Quit or Force Quit, review the exact PID
  and process name in a confirmation dialog, then confirm or cancel.

## Platform authorities

- `sysinfo` supplies process, CPU, memory, disk-I/O, users, and network data.
- Linux CPU user/system/idle proportions use cumulative kernel CPU ticks.
- Network data is system-wide per interface. No per-process network metric is
  shown because the current authority cannot provide one reliably.
- Energy impact is explicitly an app-local approximation derived from real CPU
  and interval disk-I/O signals, not a kernel or hardware energy measurement.
- Visible-column preferences use the typed local storage layer.

## Sampling and failure states

- One retained `System` instance refreshes on a two-second off-render-path
  cadence so delta-based metrics remain meaningful.
- The selected PID is retained separately from the visible row index so a
  refresh cannot silently retarget a process action.
- Column-preference load/save failures appear in the visible error banner.
- Process disappearance, unsupported signals, permissions, and signal failure
  must not be reported as confirmed process termination; richer result feedback
  remains a hardening requirement.

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
- All Activity Monitor labels, metrics, tables, inspector text, and banners
  follow rmac's bounded 100%, 115%, and 130% application text preference.
- Linux clipping evidence, full keyboard traversal, accessible table semantics,
  roles/names/states/actions, announcements, and Orca evidence remain required
  by roadmap items G4 and I3.
