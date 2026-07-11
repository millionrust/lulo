# Quick Settings transaction model

`rmac-quick-settings` is the framework-neutral interaction model for the shell
popover opened from the top-bar status cluster. It covers the first coherent
controls a daily desktop user needs: Wi-Fi, Bluetooth, output sound, power
mode, and Focus.

The model deliberately does not paint a requested toggle as successful. A
MacBook-like experience feels immediate, but it also stays truthful when a
Linux service needs authorization, restarts, rejects a request, or reports a
slightly different normalized value.

## Transaction contract

1. The UI calls `State::begin` with a typed command.
2. The model rejects unavailable, unsupported, invalid, or already-busy
   controls before platform work begins.
3. The affected tile becomes busy while continuing to display its last
   authoritative value.
4. A platform executor performs the mutation off the render thread.
5. Success is completed with a fresh `Inputs` snapshot read from authority.
6. Failure clears busy state, retains the last-known-good value, and exposes
   actionable detail that the UI may dismiss.

Each operation has an internal monotonic identity. Late completion from an old
task is ignored, so a restart or retry cannot roll the popover back to stale
state. External service refreshes may update values while an operation remains
busy; only the matching completion or failure ends that transaction. A
completion adopts only its control's authoritative field, so simultaneous
Wi-Fi and sound work cannot roll one another back through an older aggregate
snapshot.

## Availability and capabilities

- Wi-Fi and Bluetooth mutations require their adapters to report available.
- Sound mutations require an authoritative audio service snapshot; volume is
  rejected outside 0–100.
- Power modes are limited to the exact profiles advertised by the host.
- Focus is writable only while the shell-settings authority is reachable.

The transaction model alone is not the D3 completion claim.

## System executor

`rmac-quick-settings-system` maps a validated operation to the existing typed
platform authorities. It changes Wi-Fi through NetworkManager, Bluetooth
through BlueZ, output sound through PipeWire/WirePlumber, power mode through
the power-profile service, and current Focus state through the versioned shell
settings store. The executor is deliberately blocking and must run on a
background executor.

Mutation and refresh failures remain distinct. A rejected mutation does not
issue a misleading read; a successful mutation always rereads its affected
authority. The returned aggregate contains only that owned field, matching the
model's per-control merge rule and preventing an older Wi-Fi task from rolling
back newer sound state.

The live runtime bridge, layer-shell popover, outside-click/Escape dismissal,
keyboard focus order, and real Orca/niri evidence remain pending. The existing
`rmac-shell-runtime` now supplies full inputs, disables mutation when a source
is unreachable, and emits a popover-specific redraw flag without waking the
compact bar for device-list-only changes. A future
Focus policy service must also serialize schedule/mode changes across shell
processes; this executor only updates the current C4 settings authority and
does not claim E4 complete.
