# Linux shell status events

`rmac-shell-status-linux` turns Linux service activity into bounded refresh
hints for `rmac-shell-status`. It does not parse signal payloads into partial
state. Network, Bluetooth, audio, and power crates remain authoritative and are
re-read after a relevant event burst.

This boundary exists for the desktop experience, not merely architecture. A
MacBook user expects the menu bar to react immediately when Wi-Fi connects,
AirPods-like headphones appear, volume changes, or charging begins—without an
idle polling loop, icon flicker, stale state, or needless battery use.

## Sources

- NetworkManager changes are observed below
  `/org/freedesktop/NetworkManager`, covering manager, device, access-point,
  active-connection, and VPN objects.
- BlueZ changes are observed below `/org/bluez`, covering ObjectManager and
  property signals for adapters and devices.
- UPower and both supported power-profile service paths are observed on the
  system bus.
- `pw-mon --color=never` observes the PipeWire object graph. The child is
  configured to die with the watcher so a closed UI cannot leave a monitor
  process behind.

The implementation follows the platform contracts rather than scraping a
periodic command:

- [NetworkManager D-Bus API](https://networkmanager.dev/docs/api/latest/spec.html)
- [NetworkManager notes on D-Bus properties and ObjectManager](https://networkmanager.dev/blog/notes-on-dbus/)
- [UPower D-Bus API](https://upower.freedesktop.org/docs/ref-dbus.html)
- [PipeWire `pw-mon`](https://pipewire.pages.freedesktop.org/pipewire/page_man_pw-mon_1.html)

## Ordering and coalescing

The system-bus match rules and PipeWire monitor are established before the
first refresh event is published. Changes occurring while the consumer reads
its initial snapshots therefore remain queued instead of being lost.

D-Bus and PipeWire bursts use a 75 ms quiet period. Every source touched during
the burst is represented in one `Sources` set, so simultaneous network and
power changes cannot overwrite one another. This latency is short enough to
feel immediate while avoiding repeated commands and redraws during multi-signal
service transitions.

System-bus and audio watchers reconnect independently after one second. A
transport failure publishes `Event::Unavailable` once per distinct failure and
identifies exactly which snapshots may be stale. A successful subscription
clears that suppression, allowing a later regression to be reported again.

## Consumer contract

Consumers should use a bounded channel with room for multiple refresh and
failure events. For each `Event::Refresh`:

1. read only the selected authoritative service snapshots on a background
   executor;
2. publish complete typed events into `rmac-shell-status::State`;
3. request a GPUI frame only when `Change::visible` is true;
4. retain the last-known-good visible value while exposing transport health to
   diagnostics instead of flashing every icon to an empty state.

Compositor events and the shell-settings watcher already have their own live
streams. Focus now has its own timedate/logind hints and runtime authority;
notification unread state still needs its E3 connection. The D2 top-bar UI and
real niri/hardware behavior remain unproven until those streams are orchestrated
and exercised on the Linux reference PC.
