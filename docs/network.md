# Network connection editing contract

System Settings treats NetworkManager as the sole Linux authority for active
connection profiles. The Network pane presents live interface, address, route,
DNS, and hardware state separately from the saved profile values that can be
edited. It never reports success from form state.

## Scope and identity

The first editor slice intentionally targets the profile currently active on a
managed interface. A mutation identity contains the exact NetworkManager
device, ActiveConnection, Settings.Connection object paths, and profile UUID;
the object paths remain private and are redacted from `Debug`. Immediately
before mutation, `rmac-network` requires all four identities to still refer to
the same active relationship. Interface names and connection display names are
never accepted as mutation targets.

Inactive interfaces remain read-only until they have an active profile.
Profiles with inaccessible settings, external unsaved changes, unsupported IP
methods, unsupported proxy methods, inline PAC scripts, or deprecated address
arrays explain their limitation instead of offering a lossy editor. No code in
this path calls `GetSecrets`; unrelated settings and system-owned secrets remain
under NetworkManager's authority.

## Typed editor and validation

IPv4 supports Automatic, Manual, Link-Local Only, and Off. IPv6 additionally
supports DHCP Only. The editor accepts at most 16 addresses and 16 DNS servers
per family, deduplicates them without reordering, requires CIDR prefixes,
rejects mixed address families, and requires at least one address for Manual.
Off and Link-Local reject addresses, gateways, DNS values, and automatic-DNS
override state.

Automatic and Manual modes may retain additional static addresses and DNS
servers because NetworkManager permits those additions. “Use only these DNS
servers” maps to `ignore-auto-dns`; it is not presented as a global DNS policy.
Routes, search domains, metrics, DHCP options, and every unrelated property are
preserved byte-for-byte in the settings map.

NetworkManager's proxy setting provides Off or Automatic PAC configuration,
not a general manual HTTP proxy model. PAC URLs are bounded to 2,048 bytes and
must be HTTP, HTTPS, or absolute file URLs without embedded credentials or a
fragment. Inline PAC scripts remain read-only so the editor never silently
replaces script content it cannot present.

## Versioned apply and persistence transaction

Every Apply action runs away from the GPUI thread and follows this sequence:

1. Validate the typed IPv4, IPv6, and proxy model again at the service boundary.
2. Revalidate the exact device, active connection, profile object, and UUID;
   reject an inaccessible or already-unsaved profile.
3. Read NetworkManager's global `Settings.VersionId` before and after
   `GetSettings`. Retry a bounded number of times until the complete non-secret
   profile and version form one stable snapshot.
4. Clone that complete map, changing only modern `address-data`, `gateway`,
   `dns-data`, `ignore-auto-dns`, and the supported proxy properties. Deprecated
   address and DNS arrays are never migrated implicitly; profiles that still
   expose them remain read-only until another NetworkManager client migrates
   the profile.
5. If IP or DNS changed, call `Device.Reapply` with a map that differs from the
   original only in IPv4/IPv6 settings and the version returned by
   `GetAppliedConnection`. This stages the candidate only in the device's
   applied connection; the saved profile remains unchanged, so a Settings
   crash cannot strand an unsaved profile in NetworkManager. Proxy metadata is
   not presented as device state and does not require a device reapply.
6. Poll fresh applied settings for at most ten seconds and require the exact
   requested IPv4/IPv6 projection.
7. Require the saved profile and global version to still equal the stable
   original read. Persist the complete candidate atomically using
   version-checked `Update2` with the to-disk flag. A concurrent profile change
   makes NetworkManager reject the write instead of accepting a stale
   overwrite; plain `Save()` is not used because it cannot carry the token.
8. Require `Unsaved=false`, exact saved settings, and a final complete Network
   snapshot whose active configuration matches the request before the editor
   closes.

NetworkManager owns polkit authorization. Denial, cancellation, unavailable
service, unsupported reapply, device/profile replacement, concurrent change,
and timeout preserve the visible last-known-good state and produce a
dismissible error.

## Rollback and concurrent edits

If failure occurs after device staging, rmac first obtains another stable full
profile snapshot. If the saved map is still the original, it reapplies that
current authority without writing the profile. If the candidate was already
persisted, rmac restores the original map with the exact version token, checks
`Unsaved=false`, and reapplies it. Both paths verify the device's applied IP/DNS
configuration; PAC proxy state is verified from the saved profile authority.

If the saved map differs from both, another actor changed the profile. rmac
leaves that newer state untouched and reapplies the newer current authority
instead of performing a destructive “rollback.” If ownership cannot be proven,
it performs no write. System Settings then takes an independent recovery
snapshot and asks the person to reopen Connection Details.

## Live state and service recovery

The same bounded NetworkManager watcher used by Wi-Fi now refreshes both Wi-Fi
and Network snapshots. Every signal under NetworkManager's object namespace is
a refresh hint; signal bodies never become UI state. A quiet period coalesces
bursts, while owner loss preserves the last-known-good snapshot and reports a
separate stream error. Owner recovery forces a full read.

Network mutations increment their own generation. A stream snapshot captured
before a refresh or mutation cannot replace its authoritative readback. If an
external profile update arrives while the form is open, the form closes and
the pane requires it to be reopened with current values.

## Explicitly remaining

F3 remains open until the Ubuntu/niri reference PC proves:

- DHCP-to-manual, manual-to-DHCP, additional-address, IPv4/IPv6 DNS, PAC URL,
  Off, and Link-Local edits on representative Ethernet and Wi-Fi profiles;
- successful polkit authorization plus denial/cancellation, invalid values,
  reapply rejection, timeout, device disconnect, and profile disappearance;
- exact rollback after a post-stage failure and preservation of a deliberately
  concurrent external profile edit;
- NetworkManager restart, active-profile replacement, DHCP lease changes,
  suspend/resume, and Settings restart recovery;
- keyboard-only operation, visible focus, 100/125/150/200% scale, contrast,
  reduced motion, and Orca behavior.

The implementation follows NetworkManager's official
[`Settings.Connection.Update2`](https://networkmanager.dev/docs/api/latest/gdbus-org.freedesktop.NetworkManager.Settings.Connection.html),
[`Settings.VersionId`](https://networkmanager.dev/docs/api/latest/gdbus-org.freedesktop.NetworkManager.Settings.html),
[`Device.GetAppliedConnection` and `Device.Reapply`](https://networkmanager.dev/docs/api/latest/gdbus-org.freedesktop.NetworkManager.Device.html),
and [settings specification](https://networkmanager.dev/docs/api/latest/nm-settings-dbus.html).
