# VPN connection transaction contract

System Settings treats NetworkManager as the sole Linux authority for VPN
profiles and connection state. The current F4 foundation lists profiles and
connects or disconnects them with bounded completion, cancellation, live
refresh, and authoritative readback. It does not claim that profile import,
editing, or deletion is complete.

## Scope and private identity

The service recognizes both plugin-backed `vpn` profiles and native
`wireguard` profiles. A displayed profile carries an opaque identity containing
the exact NetworkManager `Settings.Connection` object path and UUID. Those
values are private, redact themselves from `Debug`, and are never reconstructed
from a display name. Every mutation enumerates NetworkManager again and accepts
only an exact path-and-UUID match.

Profile enumeration uses `GetSettings`, not `GetSecrets`. The pane displays the
profile name, normalized service type, and connection state, but never reads or
logs passwords, private keys, certificates, or plugin data. NetworkManager and
the installed VPN plugin remain responsible for authentication prompts and
secret-agent integration.

## Activation and deactivation

Connecting calls NetworkManager's `ActivateConnection` with the exact saved
profile. The returned `ActiveConnection` object path becomes the transaction
identity. Plugin VPNs use the more precise
`org.freedesktop.NetworkManager.VPN.Connection.VpnState`; native virtual
connections fall back to the typed ActiveConnection state. The transaction
polls fresh NetworkManager state away from the UI thread and succeeds only when
the exact profile reports connected. Plugin rejection, disappearing or replaced
objects, service failure, and a 60-second timeout fail visibly.

Disconnecting targets only the exact active object associated with the selected
profile, calls `DeactivateConnection`, and waits up to ten seconds for that
object to disappear or be replaced before returning a fresh snapshot. A profile
that is already in the requested stable state is a successful no-op followed by
readback.

## Cancellation and recovery

While a connection started by rmac is pending, System Settings shows a Stop
action and also accepts Escape. Cancellation first proves that the active object
still has the exact saved profile path, UUID, and VPN/virtual-connection type.
Only then does it deactivate and wait for completion. It never tears down a
pre-existing activation merely because rmac was observing it, and it never
deactivates a replacement object after an ownership mismatch.

A timed-out activation created by rmac follows the same exact cleanup path. If
cleanup cannot be proven or completed, the operation reports an error rather
than pretending cancellation succeeded. System Settings performs an independent
recovery snapshot after any error or cancellation and preserves the last known
good snapshot if even recovery is unavailable. Closing the Settings window is
deferred while a VPN transaction is settling.

## Live state and service recovery

VPN shares the bounded, coalescing NetworkManager watcher already used by Wi-Fi
and Network. Signals are refresh hints only; their payloads never become UI
state. Wi-Fi, Network, and VPN each retain independent mutation generations so a
snapshot captured before a mutation cannot overwrite its authoritative
readback. NetworkManager owner loss keeps the last known good VPN list and shows
a separate live-update error; owner recovery forces a complete resample.

## Explicitly remaining

F4 remains open. The next slices must:

- discover installed and import-capable VPN plugins without assuming package
  names or parsing localized command output;
- import only formats owned by a discovered plugin, validate bounded local
  portal-selected input, and preview the exact profile before persistence;
- define supported typed editors without lossy rewriting of plugin-owned data;
- delegate secrets to NetworkManager's appropriate secret storage or agent,
  never rmac's preferences;
- confirm deletion, revalidate exact identity, recover after partial failure,
  and handle an active profile safely;
- prove import, authentication, connect, cancel, failure, delete, daemon/plugin
  restart, suspend/resume, keyboard, scaling, and accessibility behavior on the
  Ubuntu/niri reference PC.

The implementation follows NetworkManager's official
[`ActivateConnection`/`DeactivateConnection`](https://networkmanager.dev/docs/api/latest/gdbus-org.freedesktop.NetworkManager.html),
[`ActiveConnection`](https://networkmanager.dev/docs/api/latest/gdbus-org.freedesktop.NetworkManager.Connection.Active.html),
[`VPN.Connection`](https://networkmanager.dev/docs/api/latest/gdbus-org.freedesktop.NetworkManager.VPN.Connection.html),
[`VPN setting`](https://networkmanager.dev/docs/api/latest/settings-vpn.html),
and [VPN plugin overview](https://networkmanager.dev/docs/vpn/) contracts.
