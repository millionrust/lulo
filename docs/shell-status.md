# Shell status projection

`rmac-shell-status` is the GPUI-free read model shared by the future top bar and
quick settings. It combines typed domain events without owning a compositor
socket, D-Bus connection, command process, timer, or persistence file.

## Inputs and ownership

- `rmac-compositor` remains authoritative for focused output, workspace, and
  window identities. The projection resolves their current labels, app ID, and
  title without inventing identity from display text.
- `rmac-network`, `rmac-bluetooth`, `rmac-audio`, and `rmac-power` own platform
  state and mutations. Their snapshots are normalized into compact indicators;
  the projection never writes hardware state.
- `rmac-shell-settings` owns indicator visibility and saved UI preferences.
  Hidden indicators are absent from the consumer snapshot rather than merely painted
  transparent.
- the notification and Focus services publish their runtime results into
  the typed boundary. The current notification input is deliberately only an
  unread count and urgent flag; it is not a substitute for the E1 server.

`rmac-shell-status-linux` publishes coalesced service refreshes from D-Bus and
PipeWire. A UI must not turn this reducer into a polling loop. Initial snapshots
establish coherent state; subsequent compositor, service, settings-watcher,
notification, and Focus events update only their owned input. Focus visibility
comes from settings, while active mode/expiry comes exclusively from the live
Focus authority and remains last-known-good across a service restart.

## Redraw contract

`State::apply` compares the complete consumer projection before and after an
event. `Change::visible` is true only when something the shell can currently
show changed. Duplicate snapshots, unknown compositor events, changes to a
hidden indicator, and internal compositor diagnostics therefore do not request
a frame.

The normalized snapshot contains:

- focused output/workspace/window identity plus workspace label, app ID, and
  title;
- overall network state, connection name, and connected Wi-Fi strength;
- sorted active VPN names and a connecting/disconnecting flag;
- Bluetooth power and connected-device count;
- output volume and mute state;
- battery percentage, charge state, and power-source state;
- Focus mode/expiry, unread/urgent notification state, and visibility policy.

Tests cover focused-context resolution, duplicate/unknown-event suppression,
visibility changes, connected Wi-Fi/VPN normalization, and the provisional
notification/Focus boundary. The Linux subscription contract is documented in
`shell-status-linux.md`; orchestration, notification/Focus authorities, and the
product UI remain separate D2 work. This crate does not mark D2 complete.
