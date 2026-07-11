# Shell status runtime

`rmac-shell-runtime` is the process-facing coordinator for live shell chrome.
It gives the top bar one coherent stream instead of making a view own niri,
D-Bus, PipeWire, settings persistence, retries, and error policy itself.

The experience target is the quiet reliability a MacBook user expects from the
macOS menu bar: Wi-Fi, sound, battery, focused-app, and settings changes appear
quickly; restarting a Linux service does not blank or flash an icon; and idle
diagnostics do not consume frames or battery.

## Inputs

The runtime starts bounded streams for:

- focused output, workspace, and window identity from the niri adapter;
- NetworkManager, BlueZ, PipeWire, UPower, and power-profile refresh hints from
  `rmac-shell-status-linux`;
- the versioned `rmac-shell-settings` store and its filesystem watcher.

Service signals are hints, not state. Complete authoritative snapshots are read
on the blocking executor and then applied to `rmac-shell-status::State`. No
D-Bus call, command, or settings read runs on a GPUI render path.

## Publication contract

Each publication contains a complete status snapshot, per-source health, and a
`visible` flag:

- `visible = true` means projected shell content changed and a surface may
  request a frame;
- `visible = false` means only diagnostic health changed and the top bar must
  not redraw;
- duplicate snapshots are not published;
- a failed refresh marks only the affected source unavailable and retains its
  last known good visible value.

The first publication requests a frame so a newly created surface can render a
deterministic initial state. Channel closure ends the watchers, including the
PipeWire monitor child, rather than leaving background work behind.

## Failure behavior

The niri, settings, system-bus, and PipeWire watchers reconnect independently.
Their failures remain observable through `HealthSnapshot`; they do not erase
unrelated status or turn a temporary Linux service restart into visual churn.
Bounded channels provide backpressure instead of allowing an unbounded event
queue during a desktop-service burst.

Notification and Focus authorities are not part of this slice yet. The
isolated layer-shell top-bar candidate is the next consumer; reference-PC
validation remains required before this can satisfy the D2 product gate.
