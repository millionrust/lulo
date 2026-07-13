# Quick Settings product contract

## Purpose

Quick Settings is the session-owned, on-demand control surface for the small
set of host controls used throughout the day: Wi-Fi, Bluetooth, output sound,
power mode, and Focus. It should feel immediate and composed like macOS Control
Center while remaining a truthful client of Linux authorities.

It is not a second settings store, a polling daemon, or a collection of local
demo toggles. System Settings remains the destination for deeper configuration.

## Primary journeys

1. Invoke Quick Settings and receive one trailing popover with immediate
   keyboard focus; invoking it again closes the same surface.
2. Read authoritative availability and current state for every control without
   transient service failure erasing the last useful value.
3. Toggle Wi-Fi, Bluetooth, mute, or Focus; select only a power profile the host
   advertises; and adjust output volume from 0–100.
4. Keep the previous authoritative value visible while a mutation is busy,
   reread that authority after success, and show a dismissible failure after a
   rejected mutation or refresh.
5. Close with Escape or focus loss without terminating the supervised shortcut
   endpoint. Reopening starts a fresh bounded live subscription.

## Authority and lifecycle

- `rmac-shell-runtime` supplies complete Wi-Fi, Bluetooth, audio, power, and
  Focus inputs plus explicit availability. The surface starts this subscription
  only while open and drops it on dismissal.
- `rmac-quick-settings::State` owns validation, one in-flight mutation per
  control, stale-completion rejection, and last-known-good values.
- `rmac-quick-settings-system` performs typed mutations off the render thread
  and rereads only the changed authority before completion.
- The supervised process owns the `quick-settings` shortcut socket and at most
  one popover. It reports service readiness only after that socket is bound. No
  private network name, device name, or backend error is logged.
- GPUI 0.2.2 does not expose a stable niri output identity or layer-shell
  placement. The current candidate uses the primary display and makes no final
  multi-output or focus-restoration claim.

## Visual and interaction contract

- A compact, trailing, blurred surface uses shared semantic theme tokens,
  restrained active color, a 16-pixel radius, and no decorative animation.
- Wi-Fi, Bluetooth, and Focus use clear label/summary/toggle rows. Sound owns a
  mute control and keyboard-operable shared slider. Power shows only supported
  profiles and never fabricates an unavailable mode.
- Busy, unavailable, stream-degraded, mutation-failed, and refreshed states are
  visibly distinct. Controls remain disabled whenever their authority is not
  writable.
- Tab order follows Wi-Fi, Bluetooth, Sound, Power Mode, Focus, error actions,
  and the System Settings route. Escape always dismisses.

## Remaining release evidence

- Integrate the surface with the real D1/D2 status-cluster invoker and map the
  invoker's niri output/seat once the framework exposes a proven boundary.
- Prove trailing placement, outside dismissal, focus restoration, hotplug,
  service restart, suspend/resume, and 100/125/150/200% scaling on Ubuntu/niri.
- Prove names, roles, values, disabled/busy/error announcements, slider
  operation, and keyboard order with Orca after the GPUI accessibility gate.
- Measure open latency, idle CPU/wakeups, mutation latency, and combined shell
  frame behavior on the reference PC.
