# VPN profile and import contract

System Settings treats NetworkManager as the sole Linux authority for VPN
profiles, imported configuration, secrets, and connection state. It lists and
connects profiles with bounded completion and live refresh, and can stage a
reviewed installed plugin's configuration for confirmation before anything is
written to disk. Existing profiles can be deleted through a separate exact,
confirmed transaction. General plugin-specific editing remains open F4 work.
It also offers a bounded typed editor for the common non-secret fields that can
be changed without reconstructing plugin-owned settings.

## Scope and private identity

The service recognizes plugin-backed `vpn` profiles and native `wireguard`
profiles. A displayed profile carries an opaque identity containing the exact
NetworkManager `Settings.Connection` object path and UUID. Those values redact
themselves from `Debug` and are never reconstructed from a display name. Every
mutation enumerates NetworkManager again and accepts only an exact path-and-UUID
match.

Profile enumeration uses `GetSettings`, not `GetSecrets`. The pane displays the
profile name, normalized service type, and connection state, but never reads or
logs passwords, private keys, certificates, or plugin data. NetworkManager and
the installed VPN plugin remain responsible for authentication prompts,
secret-agent integration, and persistent secret storage.

This division is required by the Secret Agent contract rather than being a UI
shortcut. VPN hints can name multiple plugin-specific values and carry an
`x-vpn-message:` prompt. An agent that collects an `AGENT_OWNED` value must
persist it itself; an ephemeral rmac prompt cannot honestly claim that role.
OpenVPN alone may request the account password, certificate passphrase, and
HTTP proxy password, while OpenConnect uses dynamic authentication forms. rmac
therefore delegates new credentials to the installed plugin/authentication
agent until the session has a reviewed credential-store and external-UI
protocol implementation.

## Installed import capabilities

Import does not guess Ubuntu package names or claim that every known VPN type is
installed. Away from the UI thread, rmac uses libnm's
`NMVpnPluginInfo` inventory, loads each installed editor plugin through libnm's
validated loader, and exposes it only when the plugin advertises the `IMPORT`
capability. Native WireGuard is included through NetworkManager's built-in
import path. The command identity is opaque to the UI and is re-discovered
immediately before use.

The reviewed set admits OpenVPN, OpenConnect, WireGuard, strongSwan, Libreswan,
L2TP/IPsec, Fortinet SSL VPN, and SSTP only when their actual authority exists.
PPTP and unknown third-party types are deliberately not presented. A missing
`nmcli`, libnm, or importer produces an honest capability limitation rather than
a nonfunctional button.

## Portal-selected temporary import

The desktop portal selects one local file without imposing an extension:
plugins own format recognition. Before invoking a plugin, rmac canonicalizes
the selection and requires a non-empty regular file no larger than 4 MiB. It
keeps a bounded copy solely to prove that the selected bytes did not change
while the importer ran; those buffers are zeroized on every exit path.

The official `nmcli connection import --temporary` frontend runs with separate
arguments, no shell, no interactive secret prompt, a fixed C locale, and a
30-second process timeout. Standard output and error are continuously drained
into bounded buffers and zeroized. Human-readable output never becomes profile
state; at most, a UUID-shaped token is used as a disambiguation hint and must
still match D-Bus authority.

The transaction compares NetworkManager's complete Settings object inventory
before and after the command. Exactly one new `Unsaved=true` object must match
the selected native type or exact plugin service. Its Settings path, UUID,
complete non-secret settings map, display name, and service become a private
preview identity. Concurrent additions that cannot be unambiguously separated
abort the preview rather than selecting or deleting by name.

The importer must explicitly produce `connection.autoconnect=false`. If it does
not, rmac deletes the exact unchanged temporary object and rejects the import.
rmac never rewrites the imported map from `GetSettings`, because doing so could
drop plugin secrets that `GetSettings` intentionally withholds. The confirmation
sheet displays only the bounded filename, resulting profile name, and service.

Confirm revalidates the exact path, UUID, `Unsaved` state, and complete visible
map, then calls `Settings.Connection.Save`. Success requires `Unsaved=false`, an
unchanged authoritative map, and a fresh VPN snapshot containing the exact
profile within ten seconds. Cancel performs the same revalidation before
`Delete` and waits for the exact object to disappear. A changed temporary
profile is left untouched rather than destructively guessed. A process crash
can leave an unsaved profile in NetworkManager memory, but `--temporary`
guarantees that it is not present after NetworkManager restarts.

## Existing-profile deletion

Delete is offered only on Linux and starts with an off-thread preparation read;
it never operates on the row's display name. NetworkManager's global
`Settings.VersionId` must remain stable around the full `GetSettings` read, and
the exact Settings path, UUID, complete visible map, persistent `Unsaved=false`
state, display name, service, and current activation consequence become the
private confirmation identity. Temporary profiles owned by another editor are
rejected.

Confirmation warns explicitly when the selected profile must disconnect and
that its NetworkManager-managed secrets will be removed. Immediately before any
mutation, rmac requires another stable full-map read equal to the preview. If an
exact ActiveConnection exists, it is revalidated by profile path, UUID, and VPN
type, deactivated, and given ten seconds to disappear. The profile is then
enumerated again: a replacement activation aborts deletion, and the stable full
settings map must still equal the preview.

Only then does rmac call `Settings.Connection.Delete` on the exact object and
wait until both `ListConnections` and a fresh VPN snapshot prove that identity
absent. NetworkManager provides no conditional/versioned Delete call, so the
last stable read and exact object identity form the narrowest available race
boundary; names are never accepted as authority. Failure after disconnection
leaves the profile installed and triggers an independent recovery snapshot.
Failure after Delete never claims success without authoritative absence.

## Non-secret profile editing

Details opens off the UI thread and captures an opaque identity containing the
exact Settings path, UUID, stable `Settings.VersionId` read, and complete
non-secret `GetSettings` map. Temporary `Unsaved=true` objects are rejected.
Plugin-backed profiles expose name, optional account name, persistence policy,
and the unsigned connection timeout. Native WireGuard profiles expose name
only because their protocol fields are not the generic `vpn` setting. Names and
usernames are trimmed, bounded to 256 characters, and reject control
characters; timeout input must fit NetworkManager's `uint32` contract.

NetworkManager's full `Update`, `UpdateUnsaved`, and `Update2` methods replace
the prior settings while `GetSettings` intentionally omits secrets. rmac
therefore never rebuilds a VPN map from that incomplete read. After exact
preflight revalidation, it invokes the official `nmcli connection modify uuid`
frontend with separate arguments and only the four reviewed property names. No
shell is involved, standard streams are discarded, the process has a fixed C
locale and 20-second bound, and passwords, certificates, private keys, and
plugin-specific data never enter the command or rmac memory.

Command exit status is not authoritative. A stable D-Bus read must prove the
exact requested typed values, persistent `Unsaved=false` state, unchanged
connection type, and an unchanged projection of every other visible setting;
a fresh VPN snapshot must contain the same opaque profile with its new name.
If another editor changes the profile before Save, the original full map no
longer matches and rmac refuses to mutate it. If other fields change during the
unversioned partial NetworkManager transaction, rmac leaves the newer state
untouched and requires a refresh instead of attempting a lossy rollback.

## Forgetting saved authentication

Plugin-backed profiles expose a separate destructive “Forget Saved
Authentication” action. Preparation runs off the UI thread and captures the
exact Settings path, UUID, stable full non-secret map, persistent
`Unsaved=false` state, display name, service, and whether the tunnel is active.
The confirmation explains that the profile and current tunnel remain, while
the installed plugin may ask again after the next disconnect.

Confirm performs the same stable exact-map revalidation and calls
`Settings.Connection.ClearSecrets`; it never calls `GetSecrets`, enumerates
secret names, or copies a credential through UI state. NetworkManager owns
clearing both its persistent secrets and the appropriate registered agent's
stored values. The method's successful D-Bus reply is the secret authority;
because `GetSettings` intentionally cannot reveal secrets, verification instead
requires the exact profile to remain persistent with an unchanged visible map
and to appear in a fresh VPN snapshot. A concurrent visible edit is preserved
and reported rather than rolled back.

Native WireGuard is deliberately excluded. Its private key is itself a profile
secret, so a generic clear action could make the connection unusable. The UI
states that WireGuard keys are never cleared here.

## Activation and deactivation

Connecting calls NetworkManager's `ActivateConnection` with the exact saved
profile. The returned `ActiveConnection` path becomes the transaction identity.
Plugin VPNs use the more precise
`org.freedesktop.NetworkManager.VPN.Connection.VpnState`; native virtual
connections fall back to typed ActiveConnection state. Success requires fresh
exact-profile state. Plugin rejection, disappearing or replaced objects,
service failure, and a 60-second timeout fail visibly.

Disconnecting targets only the exact active object associated with the selected
profile, calls `DeactivateConnection`, and waits up to ten seconds for that
object to disappear or be replaced before returning a fresh snapshot. A profile
already in the requested stable state is a successful no-op with readback.

## Activation cancellation and recovery

While an activation started by rmac is pending, System Settings shows Stop and
also accepts Escape. Cancellation first proves that the active object still has
the exact saved profile path, UUID, and VPN/virtual-connection type. Only then
does it deactivate and wait. It never tears down a pre-existing activation that
rmac was merely observing or a replacement object after an ownership mismatch.

A timed-out or failed activation created by rmac follows the same exact cleanup
path. If cleanup cannot be proven or completed, the operation reports an error
instead of pretending cancellation succeeded. System Settings takes an
independent recovery snapshot after any error or cancellation and preserves the
last known good state if recovery is unavailable. Closing Settings is deferred
while an activation or import transaction is settling.

## Live state and service recovery

VPN shares the bounded, coalescing NetworkManager watcher used by Wi-Fi and
Network. Signals are refresh hints only; payloads never become UI state. Each
pane retains an independent mutation generation so a snapshot captured before a
mutation cannot overwrite authoritative readback. NetworkManager owner loss
keeps the last known good VPN list and shows a separate live-update error; owner
recovery forces a full resample. Stream refresh is paused while an unsaved import
preview exists, so that temporary object is never presented as an installed VPN.

## Explicitly remaining

F4 remains open. The next slices must:

- prove that the Ubuntu/niri session starts a compatible NetworkManager secret
  agent and each installed plugin's authentication UI, including agent-owned,
  system-owned, always-ask/OTP, multi-field, cancel, retry, and wrong-secret
  paths; rmac must not implement agent-owned persistence without a reviewed
  credential store and plugin external-UI protocol;
- keep plugin-specific configuration under the installed plugin's own editor or
  another typed authority rather than reconstructing its opaque data map;
- prove capability discovery, each supported import format, secret handling,
  authentication, connect, cancel, failure, save/cancel preview, active/inactive
  deletion and concurrent-edit preservation,
  daemon/plugin restart, suspend/resume, keyboard, scaling, and accessibility
  behavior on the Ubuntu/niri reference PC.

The implementation follows NetworkManager's official
[`NMVpnPluginInfo`](https://networkmanager.dev/docs/libnm/latest/NMVpnPluginInfo.html),
[`NMVpnEditorPlugin`](https://networkmanager.dev/docs/libnm/latest/NMVpnEditorPlugin.html),
[`nmcli import`](https://networkmanager.pages.freedesktop.org/NetworkManager/NetworkManager/nmcli.html),
[`Settings.Connection`](https://networkmanager.dev/docs/api/latest/gdbus-org.freedesktop.NetworkManager.Settings.Connection.html),
[`SecretAgent`](https://networkmanager.dev/docs/api/latest/gdbus-org.freedesktop.NetworkManager.SecretAgent.html),
[`ActivateConnection`/`DeactivateConnection`](https://networkmanager.dev/docs/api/latest/gdbus-org.freedesktop.NetworkManager.html),
and [`VPN.Connection`](https://networkmanager.dev/docs/api/latest/gdbus-org.freedesktop.NetworkManager.VPN.Connection.html)
contracts.
