# Battery and energy authority

System Settings treats UPower as the Linux authority for battery presence,
charge, state, power source, time estimates, energy rate, health, cycle count,
and model. It treats power-profiles-daemon as a separate optional authority for
the active and supported energy modes. Missing properties remain absent rather
than being estimated from unrelated values.

## Authoritative snapshot

The blocking service runs off the UI thread. It reads UPower's display device
for the aggregate charge and time state, then uses the first present physical
power-supply battery for health, cycle count, and model when those details are
available. Percentages are rounded and clamped only at the service boundary;
zero or invalid time/rate/health values are not presented as measurements.

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

## Explicitly remaining

F5 remains open until the Ubuntu/niri reference PC proves:

- AC transitions, charge/discharge/full/pending states, time and rate changes,
  battery add/remove, suspend/resume, and UPower restart recovery;
- both supported power-profile endpoints, external changes, daemon restart,
  authorization/rejection, degradation changes, and readback;
- charge-threshold controls only on hardware exposing a reviewed writable
  authority, with explicit unsupported state everywhere else;
- bounded authoritative history where UPower provides it, without inventing
  usage categories or retaining private activity data;
- keyboard, 100–200% scaling, contrast, Orca, idle wakeup, and error behavior.

The implementation follows the official
[UPower D-Bus API](https://upower.freedesktop.org/docs/ref-dbus.html) and
[power-profiles-daemon D-Bus API](https://upower.pages.freedesktop.org/power-profiles-daemon/gdbus-org.freedesktop.UPower.PowerProfiles.html).
