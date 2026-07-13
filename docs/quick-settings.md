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

## Popover interaction contract

The domain permits one open popover for the session. Invoking the same output's
status cluster toggles it closed; invoking another output transfers ownership
there. The UI receives the owning output with every dismissal so focus can
return to the correct status-cluster invoker.

Keyboard focus follows Wi-Fi, Bluetooth, Sound, Power Mode, then Focus. It
skips controls whose authorities are unavailable, wraps in both directions,
and reconciles immediately if a live service loss disables the focused tile.
Escape, outside press, invoker toggle, and owner-window loss have distinct
dismissal reasons. Busy controls ignore repeated activation.

Activation toggles Wi-Fi, Bluetooth, output mute, and Focus from their current
authoritative values. Sound increment/decrement uses bounded five-percent
steps, while Power Mode traverses only the profiles advertised by the host.
The layer-shell view will translate Tab/Shift-Tab, arrows, Space/Return, and
Escape into these framework-neutral intents.

## Availability and capabilities

- Wi-Fi and Bluetooth mutations require their adapters to report available.
- Sound mutations require an authoritative audio service snapshot; volume is
  rejected outside 0–100.
- Power modes are limited to the exact profiles advertised by the host.
- Focus is writable only while the live Focus authority is reachable; shell
  settings control indicator visibility but never impersonate active policy.

The transaction model alone is not the D3 completion claim.

## System executor

`rmac-quick-settings-system` maps a validated operation to the existing typed
platform authorities. It changes Wi-Fi through NetworkManager, Bluetooth
through BlueZ, output sound through PipeWire/WirePlumber, and power mode
through the power-profile service. Focus uses the session-owned
`org.rmac.Focus1` authority and never writes legacy shell preferences. The
executor is deliberately blocking and must run on a background executor.

Mutation and refresh failures remain distinct. A rejected mutation does not
issue a misleading read; a successful mutation always rereads its affected
authority. The returned aggregate contains only that owned field, matching the
model's per-control merge rule and preventing an older Wi-Fi task from rolling
back newer sound state.

`rmac-quick-settings-app` is the supervised on-demand presentation. It owns the
action-scoped `quick-settings` shortcut socket, keeps at most one trailing GPUI
popover, and starts the live shell-runtime subscription only while that popover
is open. Shared toggles, buttons, and the output-volume slider render the model;
all operations run on a blocking executor and return through `complete` or
`fail`. Escape, focus loss, and a repeated shortcut close the surface without
terminating its shortcut endpoint. A direct System Settings route remains
available for deeper controls.

Its systemd unit is `Type=notify`: readiness is sent only after the shortcut
socket is bound, and the broker is ordered after that handshake. A process that
cannot establish its endpoint therefore fails before it can silently drop the
first activation.

The Focus service evaluates persisted policy at startup and exact wake
boundaries and resamples local time after time-zone, clock, and resume changes.
`rmac-shell-runtime` subscribes to that authority, supplies full inputs,
disables mutation during reconnect, and emits a popover-specific redraw flag
without waking the compact bar for device-list-only changes. Scheduled Focus
cannot be deceptively switched off by a compact toggle; the service rejection
remains visible and points the user toward Settings.

D3 is still gated on the real D1/D2 layer-shell invoker, niri output/seat
placement and focus restoration, scaling, Orca semantics, service-restart
interaction, and performance evidence. GPUI 0.2.2 supplies no proven stable
niri output identity to this popup, so the candidate uses the primary display
and does not fabricate multi-output correctness.
