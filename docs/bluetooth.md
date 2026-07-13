# Bluetooth pairing and device-management contract

System Settings treats BlueZ as the sole Linux Bluetooth authority. The pane
never marks a device paired, trusted, connected, or forgotten from a local
toggle. Every mutation starts from a current ObjectManager identity and ends
with a complete authoritative snapshot.

## Device identity and stale-row defense

BlueZ device object paths are service identities, not display values. Before
connect, disconnect, cancellation, pairing, or removal, `rmac-bluetooth`
re-reads `GetManagedObjects` and requires the exact path to expose `Device1`.
Its `Adapter` property must resolve to a live object exposing `Adapter1`. A
device name, alias, icon, or Bluetooth address is never accepted as a mutation
target, so duplicate labels and stale UI rows cannot redirect an operation.

Snapshots expose `Paired`, `Trusted`, and `Connected` separately. The Settings
row labels untrusted paired devices honestly rather than treating pairing as
proof of trust.

## Pairing transaction

Selecting **Pair** creates one `PairingSession` and a 16-event,
transaction-lifetime channel, then performs the BlueZ work off the UI thread.
BlueZ permits only one unanswered agent prompt at a time and at most a short
keyboard-entry display sequence; a closed or unexpectedly full channel fails
the request instead of accumulating work. The transaction then:

1. Build a fresh system-bus connection that serves
   `/org/rmac/SystemSettings/BluetoothAgent` as `org.bluez.Agent1`.
2. Register that path with `AgentManager1.RegisterAgent` using
   `KeyboardDisplay`. The application does not request default-agent status;
   the agent exists only for the action initiated through this Settings sheet.
3. Revalidate the selected `Device1` on that same connection and call `Pair`.
   BlueZ therefore routes input for this transaction to the rmac agent.
4. Refuse every agent callback whose device path differs from the exact
   selection. Only one unanswered prompt may exist at a time, and every reply
   carries a monotonic prompt identifier so stale buttons cannot answer a later
   request.
5. Present numeric comparison as a zero-padded six-digit code; accept a legacy
   alphanumeric PIN of 1–16 ASCII characters; accept a numeric passkey from
   000000 through 999999; and explicitly ask before just-works pairing or a
   requested Bluetooth service. Display-only keyboard PIN/passkey callbacks
   show progress while the user types on the remote device.
6. Return an explicit BlueZ `Rejected` or `Canceled` agent error when the user
   declines, cancels, the daemon cancels, or a prompt remains unanswered for 60
   seconds. Cancel also invokes `Device1.CancelPairing` on a separate
   connection so a Pair call that is not waiting in the agent can terminate.
7. After `Pair` succeeds, set `Device1.Trusted=true`, read a fresh snapshot, and
   publish success only if the exact device reports both `Paired` and `Trusted`.

The sheet replaces its input entity immediately after a PIN or passkey is
submitted, dropping the old text and undo history. `PairingPinCode` is
non-cloneable, redacts `Debug`, and zeroizes its owned string on drop. No PIN,
passkey, device callback, or service authorization is logged or persisted by
rmac.

If pairing fails after BlueZ may have changed state—such as a successful bond
followed by a failed trust write—System Settings performs a separate recovery
snapshot before reporting the error. Rejection and user cancellation close
quietly; timeout and service failures remain visible and dismissible.

## Forget transaction

Paired devices expose **Forget…** behind a destructive confirmation that
explains the disconnect and need to pair again. The background transaction
revalidates the exact device and owning adapter, then calls
`Adapter1.RemoveDevice`. BlueZ owns the disconnect and removal of pairing
information. Success waits up to four seconds for a fresh snapshot in which the
object is absent. If removal or verification fails, Settings performs an
additional snapshot so the last-known-good view reflects any partial authority
change.

## Live state and BlueZ recovery

The Linux watcher subscribes to every signal sent by the `org.bluez` service,
including ObjectManager additions/removals from `/` and property changes below
`/org/bluez`. A 75 ms quiet period collapses discovery and RSSI bursts into one
full snapshot read. Signal bodies are never treated as state.

A separate `NameOwnerChanged` subscription tracks `org.bluez`. Both signal
subscriptions are installed before initial ownership is queried. Owner loss
publishes a separate stream error without discarding the last-known-good
snapshot; owner reappearance guarantees a change event and full read. Settings
suppresses stream snapshots during mutations and rejects a snapshot captured
before the current mutation generation, preventing late discovery work from
overwriting pair, trust, remove, or connection readback.

## Explicitly remaining

F2 remains open until the Ubuntu/niri reference PC proves:

- numeric-comparison, just-works, PIN/passkey-entry, keyboard display, and at
  least one audio-device pairing path with representative hardware;
- accepted, rejected, canceled, 60-second timeout, out-of-range, powered-off,
  missing-agent, and permission/service failure outcomes;
- `Paired` and `Trusted` readback, reconnect after Settings restart, removal of
  connected and disconnected devices, and recovery after a failed removal;
- device discovery plus property changes across `bluetooth.service` restart,
  adapter power cycles, suspend/resume, and device disappearance;
- keyboard-only operation, visible focus, 100/125/150/200% scale, reduced
  motion, contrast, and Orca behavior.

The adapter follows the official BlueZ
[`Agent1` and `AgentManager1`](https://bluez.readthedocs.io/en/latest/agent-api/),
[`Device1`](https://bluez.readthedocs.io/en/latest/device-api/), and
[`Adapter1`](https://bluez.readthedocs.io/en/latest/adapter-api/) contracts.
