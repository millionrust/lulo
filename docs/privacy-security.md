# Privacy & Security authority and Linux acceptance

System Settings presents a macOS-like Privacy & Security destination without
inventing one universal Linux permission database or treating package origin as
proof of trust. Portal decisions, available updates, Ubuntu lifecycle and
coverage, automatic-update status, and desktop application provenance remain
separate authorities with separate failure states.

The current contracts are the XDG
[PermissionStore interface](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.PermissionStore.html),
the [Ubuntu Pro Client API](https://documentation.ubuntu.com/pro-client/en/docs/references/api/),
[`ubuntu-distro-info`](https://manpages.ubuntu.com/manpages/questing/man1/ubuntu-distro-info.1.html),
and
[`unattended-upgrade`](https://manpages.ubuntu.com/manpages/noble/man8/unattended-upgrade.8.html).

## Portal decisions and reset boundary

The pane reads only the PermissionStore `devices` table's `camera` and
`microphone` resource IDs. The store defines application IDs and permission
arrays as free-form strings and does not interpret them, so rmac preserves the
tokens instead of translating `yes`, `no`, `ask`, or an unknown token into an
invented cross-desktop policy. At most 512 decisions, 32 tokens per decision,
512 rendered token bytes, and bounded control-free IDs/tokens are accepted. An
invalid or excessive payload fails the complete read and leaves the last
known-good snapshot visible rather than hiding selected entries.

Version 1 remains read-only. Version 2 exposes a confirmed Reset action backed
only by `DeletePermission(table, resource, app)`. The confirmation retains the
exact resource, application ID, and uninterpreted token array. Immediately
before deletion, the adapter rechecks the interface version and calls
`GetPermission` for that exact pair. A missing or changed value is refused. A
complete post-delete read must prove that the pair is absent. PermissionStore
does not offer compare-and-delete, so a mutation occurring after the preflight
cannot be made transactionally atomic; post-readback and the live stream expose
the resulting authority rather than attempting a lossy rollback.

Reset removes a stored portal decision so a future portal request may ask
again. It does not stop active capture, revoke access held by an unsandboxed
application, edit another portal table, or prove that a camera or microphone is
not in use.

## Live state, recovery, and privacy

The adapter subscribes to PermissionStore `Changed` and filtered
`NameOwnerChanged` signals before publishing an initial complete-refresh hint.
It reconnects after session-bus or signal-stream failure. Settings coalesces
events through a capacity-one channel, preserves one pending refresh across a
manual read or reset, and assigns a generation to every transaction. An older
stream snapshot cannot replace a newer reset readback. Last-known-good values
remain visible with a separate live-update error during service loss.

Application IDs and valid tokens are authority data intentionally shown to the
user. Raw D-Bus errors, unique bus peers, helper stderr, filesystem diagnostics,
and arbitrary Ubuntu Pro failure titles are not shown. Errors are bounded and
control-normalized; public failures use fixed capability descriptions.

## Security status authorities

PackageKit supplies only the current available security-update count and the
separate Software Update workflow. It does not prove lifecycle coverage or
repository trust.

On Ubuntu, `ubuntu-distro-info --days=eol` reports remaining standard release
support for the validated `/etc/os-release` series. Independent, offline
`pro api` endpoints report installed APT-origin counts, attachment/contract
state, enabled Pro services, and unattended-upgrades status. Each helper is
argument-separated, has a 15-second deadline and 1 MiB output cap, drains both
pipes, validates bounded machine-readable fields, and can fail without hiding
the other successful authorities. “Automatic security updates enabled”
requires the reported service, APT timer, periodic job, and non-zero upgrade
interval together.

These values still do not prove that a repository is trustworthy, that every
installed package is supported, that a particular vulnerability is fixed, or
that Flatpak, Snap, AppImage, locally built, and manually installed software is
covered by Ubuntu security maintenance. Automatic-update mutation remains
absent until a polkit-aware, rollback-safe APT policy design is accepted.

## Desktop application provenance

The live shared desktop-entry catalog classifies exact Flatpak and Snap export
locations, integrated AppImages, and remaining system/user/other desktop
entries. These counts cover launchable desktop applications only. A system
desktop path is not presented as proof of APT ownership, and source location is
not presented as a signature, sandbox, update, vulnerability, or safety result.

## Linux acceptance matrix

F17 remains unchecked until the Ubuntu/niri reference PC proves:

- exact camera/microphone inventories, missing resources, unknown tokens,
  bounded-payload refusal, version 1 read-only state, and version 2 capability;
- reset cancel/success, exact selected-pair deletion, changed/missing preflight
  refusal, post-delete mismatch, external changes during a reset, and no false
  active-capture or native-application revocation claim;
- initial subscription, external changes, signal bursts, PermissionStore and
  session-bus loss/reappearance, mutation-generation ordering, suspend/resume,
  last-known-good retention, and privacy-safe failures;
- PackageKit security count independence; Ubuntu and non-Ubuntu lifecycle;
  current, old, missing, malformed, oversized, timed-out, and partially failing
  Pro/distro-info authorities; accurate contract/services/origins/frequencies;
- live Flatpak, Snap, integrated AppImage, system, user, and other desktop-entry
  changes without APT ownership or trust overclaim; and
- keyboard-only focus and confirmation, visible busy/error states, 100–200%
  scaling, contrast, reduced motion, Orca roles/names/states/warnings, bounded
  idle work, and no private diagnostics in any failure state.

Use `scripts/linux/run-privacy-security-evidence.sh` and the procedure in
`docs/linux-reference-bringup.md`. Generated evidence stays under ignored
`target/linux-evidence`. Commit only reviewed summaries; never commit raw
PermissionStore payloads, Pro API responses, application IDs from a personal
machine, host/user identifiers, bus peers, private paths, tokens, or logs.
