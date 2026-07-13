# Wi-Fi connection contract

System Settings treats NetworkManager as the sole Linux Wi-Fi authority. The
pane never stores a local connected value and never sends a display string back
to NetworkManager as an identifier.

## Identity and discovery

NetworkManager exposes SSIDs as byte arrays. `WifiNetworkId` therefore keeps the
exact bytes private and includes the access point's open/protected class. The UI
receives a separate sanitized display label. This prevents lossy UTF-8
conversion, control characters, or an open and protected access point with the
same visible label from redirecting an activation.

Each snapshot reads visible access points and the saved connection profiles the
current user may access. A network is `known` only when a saved
`802-11-wireless` profile has the exact SSID bytes and matching security class.
When more than one compatible profile exists, the most recently used profile
is selected deterministically.

## Activation transaction

Selecting a row runs one bounded background transaction:

1. Re-read the NetworkManager radio, Wi-Fi device, access points, and saved
   profiles. A stale row cannot activate an access point that disappeared or
   changed security class.
2. For a known network, call `ActivateConnection` with the exact saved profile,
   device, and access-point object paths.
3. For a new open network, call `AddAndActivateConnection` with an empty
   template plus the exact device and access point. NetworkManager completes
   the profile from those authoritative objects and persists it using its
   default policy.
4. Refuse a new protected network before any mutation. Password collection and
   storage require the separate NetworkManager Secret Agent slice.
5. Follow the returned ActiveConnection for at most ten seconds. Success is
   published only after its state is activated and a fresh Wi-Fi snapshot marks
   the exact network connected.

Radio-off, missing-adapter, out-of-range, permission, rejection, disappearance,
and timeout failures leave the last known-good snapshot visible and produce a
dismissible Settings error. While a transaction is active, power, scan, and all
network rows are disabled; the selected row reads `Connecting…`.

## Explicitly remaining

F1 stays open until protected first-time connections have a reviewed Secret
Agent and password sheet, saved-network forgetting is implemented, service and
property signals replace manual refresh, NetworkManager restart recovery is
proven, and permission/wrong-secret behavior has Linux interaction evidence.

The adapter follows NetworkManager's official
[`ActivateConnection` and `AddAndActivateConnection` contract](https://networkmanager.dev/docs/api/latest/gdbus-org.freedesktop.NetworkManager.html)
and the
[`Settings.Connection.GetSettings` secrecy contract](https://networkmanager.dev/docs/api/latest/gdbus-org.freedesktop.NetworkManager.Settings.Connection.html).
