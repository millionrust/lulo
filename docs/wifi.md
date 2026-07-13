# Wi-Fi connection contract

System Settings treats NetworkManager as the sole Linux Wi-Fi authority. The
pane never stores a local connected value and never sends a display string back
to NetworkManager as an identifier.

## Identity and discovery

NetworkManager exposes SSIDs as byte arrays. `WifiNetworkId` therefore keeps the
exact bytes private and includes the access point's typed security class: open,
Enhanced Open, WPA Personal PSK, SAE, PSK/SAE transition, enterprise, legacy,
or otherwise protected. The UI receives a separate sanitized display label.
This prevents lossy UTF-8 conversion, control characters, or access points with
the same visible label but different security from redirecting an activation.

Each snapshot reads visible access points and the saved connection profiles the
current user may access. A network is `known` only when a saved
`802-11-wireless` profile has the exact SSID bytes and matching security class.
When more than one compatible profile exists, the most recently used profile
is selected deterministically. A separate Known Networks collection deduplicates
profiles by exact identity, keeps the most recent ordering, and remains
available for management when a network is out of range or the radio is off.

## Activation transaction

Selecting a row runs one bounded background transaction:

1. Re-read the NetworkManager radio, Wi-Fi device, access points, and saved
   profiles. A stale row cannot activate an access point that disappeared or
   changed security class.
2. For a known network, call `ActivateConnection` with the exact saved profile,
   device, and access-point object paths.
3. For a new open or Enhanced Open network, call `AddAndActivateConnection`
   with the exact device and access point. NetworkManager completes and stores
   the profile from those authoritative objects using its policy.
4. For a new WPA Personal PSK, SAE, or transition network, present a masked
   password sheet. Validate WPA-PSK as 8–63 printable ASCII bytes or 64 hex
   digits and SAE as 1–63 non-control UTF-8 bytes before starting any mutation.
5. Register a one-shot Secret Agent on its own system-bus connection, submit a
   partial profile with the exact SSID and key-management mode, and deliver the
   password only for the matching connection and security setting. The result
   uses system-owned secret flags, so NetworkManager—not rmac—owns persistence.
6. Follow the returned ActiveConnection for at most ten seconds. Success is
   published only after its state is activated and a fresh Wi-Fi snapshot marks
   the exact network connected.

Radio-off, missing-adapter, out-of-range, permission, rejection, disappearance,
and timeout failures leave the last known-good snapshot visible. A protected
connection reports authentication and secret failures inside the sheet so the
user can retry. While a transaction is active, power, scan, and all network rows
are disabled; the selected row reads `Connecting…`. Stop or Escape sets a
cancellation token, deactivates the in-flight ActiveConnection, and closes the
sheet only after the bounded worker acknowledges cancellation. A timeout also
deactivates the attempt before reporting failure.

## Forget transaction

Known Networks exposes a destructive `Forget…` action behind a macOS-style
confirmation alert. The background transaction re-reads accessible profiles,
selects every profile with the exact SSID bytes and compatible security, and
never uses the display label as an identifier. If one of those profiles is
active, NetworkManager is asked to deactivate that exact ActiveConnection
before each matching `Settings.Connection` object receives `Delete`.

The operation waits up to four seconds for the authoritative profile list to
contain no compatible profile and, when applicable, for the device to stop
using the old ActiveConnection. Only a fresh Wi-Fi snapshot is published on
success. If a failure might follow a partial deletion, System Settings performs
a separate authoritative recovery read before showing the error, so it does
not keep presenting a profile that NetworkManager already removed. Window close
is suppressed during this short irreversible transaction.

## Password lifetime

The sheet uses GPUI's masked editor. A submitted value moves immediately into a
non-cloneable `WifiPassword` whose `Debug` output is redacted and whose owned
bytes are zeroized on drop. The editor entity is replaced at the same boundary
to discard its text and undo history. The one-shot agent validates the exact
profile identity, consumes the password on its first successful `GetSecrets`,
and never logs or stores it. `SaveSecrets` and `DeleteSecrets` are no-ops because
the requested secret is system-owned by NetworkManager.

## Explicitly remaining

F1 stays open until service and property signals replace manual refresh,
NetworkManager restart recovery is proven, and permission, cancellation,
wrong-secret, active-forget, and partial-delete behavior has Linux interaction
evidence. Enterprise authentication needs a separate certificate and identity
design; legacy security remains intentionally unavailable.

The adapter follows NetworkManager's official
[`ActivateConnection` and `AddAndActivateConnection` contract](https://networkmanager.dev/docs/api/latest/gdbus-org.freedesktop.NetworkManager.html)
and the
[`Settings.Connection.GetSettings` secrecy contract](https://networkmanager.dev/docs/api/latest/gdbus-org.freedesktop.NetworkManager.Settings.Connection.html).
The protected path also follows the official
[`SecretAgent`](https://networkmanager.dev/docs/api/latest/gdbus-org.freedesktop.NetworkManager.SecretAgent.html)
and
[`AgentManager`](https://networkmanager.dev/docs/api/latest/gdbus-org.freedesktop.NetworkManager.AgentManager.html)
interfaces.
