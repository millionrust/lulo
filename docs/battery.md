# Battery and energy authority

System Settings treats UPower as the Linux authority for battery presence,
charge, state, power source, time estimates, energy rate, health, cycle count,
and model. It treats power-profiles-daemon as a separate optional authority for
the active and supported energy modes. Missing properties remain absent rather
than being estimated from unrelated values.

## Authoritative snapshot

The blocking service runs off the UI thread. It reads UPower's display device
for the aggregate charge and time state, then enumerates present physical
power-supply batteries for health, cycle count, model, charge-threshold
capability, and an authoritative history fallback. The first physical battery
continues to supply health details; a writable threshold is exposed only when
exactly one physical system battery is present. Percentages are rounded and
clamped only at the service boundary; zero or invalid time/rate/health values
are not presented as measurements.

Power profiles support both the current
`org.freedesktop.UPower.PowerProfiles` endpoint and the legacy
`net.hadess.PowerProfiles` endpoint. Only advertised profile identifiers are
shown. A mutation writes one advertised typed value and System Settings then
performs a complete authority read; it never assumes the selected row became
active from the setter's return alone. Performance degradation text comes from
the service and is mapped to a bounded user-facing explanation.

## Live changes and recovery

The power watcher subscribes to all UPower signals, both power-profile service
signal streams, and D-Bus owner changes. A bounded one-event channel coalesces
bursts from percentage, time, device, and profile changes into complete
snapshots instead of treating signal payloads as state.

UPower owner loss publishes one unavailable event and retains the last known
good UI snapshot with a separate live-update error. Owner recovery publishes a
non-droppable changed event so a full resample is guaranteed. Profile-service
owner changes trigger a normal resample because profile selection is optional
and must not make otherwise valid battery state unavailable. Broken streams
reconnect after one second.

System Settings increments a power mutation generation for manual refresh and
profile changes. A signal snapshot is accepted only when its captured
generation is still current and no mutation/loading transaction is active. If
a signal or service recovery arrives during a transaction, a pending bit forces
one more authority read after that transaction completes; the recovery event is
not silently lost.

## Optimized charging

System Settings shows an Optimized Charging switch only when the one present
physical battery reports UPower's `ChargeThresholdSupported=true`. It displays
reported start/end percentages when valid, or explicitly says that firmware
chooses the limits when UPower advertises optimized firmware behavior. It does
not offer arbitrary percentage editing because UPower's reviewed API only
enables or disables limits already configured by the kernel, firmware, or
platform.

The snapshot carries a private identity containing the UPower unique bus owner,
device object path, native path, and battery serial. Before mutation, the
service requires the same UPower owner, exactly one present physical battery,
the same identity, and a still-writable capability. The method call targets the
captured unique owner rather than the replaceable well-known name. Owner state
is checked again around inventory reads, preventing a daemon restart from
redirecting a validated request to a replacement process.

`EnableChargeThreshold` runs off the UI thread. Success requires the same
battery to report the requested `ChargeThresholdEnabled` value within three
seconds and a complete fresh snapshot to preserve the same identity and state.
Failures preserve the previous UI state and take an independent recovery
snapshot because a timeout can occur after hardware applied the request.
Multiple-battery and unsupported systems receive an explanation, not a disabled
switch that implies hidden write support.

## Recent battery history

History is capability-gated by `HasHistory`. The service requests only UPower's
persistent `charge` series for the previous 24 hours, asks for approximately 96
points, rejects invalid timestamps and out-of-range or non-finite percentages,
orders and de-duplicates samples, and enforces a final 96-point bound. It first
uses the aggregate display device and falls back only to the single physical
battery, where the series still represents the displayed battery.

The pane downsamples to at most 48 bars while preserving the first and last
sample. It provides textual minimum, maximum, and latest values and separately
explains authoritative-but-empty history, unsupported history, and temporary
read failure. rmac does not persist or synthesize this history and does not
infer application usage or energy categories from it.

## Explicitly remaining

F5 remains open until the Ubuntu/niri reference PC proves:

- AC transitions, charge/discharge/full/pending states, time and rate changes,
  battery add/remove, suspend/resume, and UPower restart recovery;
- both supported power-profile endpoints, external changes, daemon restart,
  authorization/rejection, degradation changes, and readback;
- supported and unsupported charge-threshold hardware, polkit outcomes,
  daemon-restart races, readback timeout, and hardware/firmware limit behavior;
- UPower display/physical history availability, empty/error behavior, and the
  rendered 24-hour graph against authoritative samples;
- keyboard, 100–200% scaling, contrast, Orca, idle wakeup, and error behavior.

The implementation follows the official
[UPower device D-Bus API](https://upower.freedesktop.org/docs/Device.html) and
[power-profiles-daemon D-Bus API](https://upower.pages.freedesktop.org/power-profiles-daemon/gdbus-org.freedesktop.UPower.PowerProfiles.html).
