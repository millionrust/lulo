# Login Items authority

System Settings presents one macOS-like Login Items pane backed by two Linux
authorities. Applications opened at sign-in come from XDG autostart desktop
entries. Background services come from the per-user systemd manager. The pane
does not maintain a parallel preference database, start or stop running
services, delete applications, or edit system-owned unit files.

The implementation follows the freedesktop
[Application Autostart Specification](https://specifications.freedesktop.org/autostart/0.5/),
[Desktop Entry Specification](https://specifications.freedesktop.org/desktop-entry/latest-single/),
[XDG Base Directory Specification](https://specifications.freedesktop.org/basedir/),
[Trash Specification](https://specifications.freedesktop.org/trash/latest/),
and systemd's
[`org.freedesktop.systemd1`](https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.systemd1.html)
user-manager contract.

## Effective XDG inventory

Enumeration reads `$XDG_CONFIG_HOME/autostart` before each directory in
`$XDG_CONFIG_DIRS/autostart`. The first occurrence of a desktop-entry filename
is authoritative even when malformed or unreadable, so a broken user override
cannot accidentally reveal and run a lower system entry. Inventories are
bounded to 512 effective entries and 128 issues.

Desktop-entry reads accept only regular, non-symlink files. Metadata and the
opened file are checked, Linux opens with `O_NOFOLLOW`, and at most 256 KiB plus
one detection byte is read. Files must be UTF-8 `Type=Application` entries with
a bounded display name and either a bounded `Exec` value or D-Bus activation.
`Hidden`, `OnlyShowIn`, `NotShowIn`, and `TryExec` determine the effective and
current-session state. An unavailable `TryExec` is reported without exposing a
private executable path.

Toggling starts from a complete snapshot, captures the exact source bytes, and
requires a second identical snapshot and byte read before writing. A user entry
is rewritten atomically with only the managed `Hidden` keys changed. Disabling
a system entry creates a full user-owned hidden override; re-enabling removes
only an override explicitly marked as rmac-managed. Success requires a fresh
inventory plus exact destination-file readback.

## Reviewed add and remove transactions

Add begins with the desktop portal's local `.desktop` chooser. The preview
captures the validated filename, display name, exact sign-in command, source
bytes, and the exact optional destination bytes. Settings shows the command and
an explicit Add or Replace decision. Confirmation rereads both files and
refuses a changed source, a newly appeared target, a removed replacement, or a
changed replacement. The installed user copy is normalized to enabled state,
written atomically, and accepted only after exact authoritative readback.

Remove is available only for a user-owned application entry that is not an
rmac-managed override. Opening the confirmation performs a fresh read and
retains the exact identity and contents. Confirmation repeats that preparation
and refuses any changed file before moving it through the desktop Trash
contract. The application itself is untouched. If removal reveals an enabled
lower system entry with the same filename, rmac revalidates that entry and
creates an exact-readback managed hidden override so removal cannot make a new
program start at the next sign-in. A concurrent user entry is never replaced by
that recovery step.

## systemd user services

The adapter uses the session-bus user manager's `ListUnitFiles`,
`EnableUnitFiles`, `DisableUnitFiles`, `Reload`, `UnitPath`, and
`UnitFilesChanged` authority. The bounded list includes enabled or otherwise
active-looking services and user-installed disabled services. Runtime-only,
masked, static/generated, unknown, and `rmac-*` infrastructure states remain
read-only with an explanation.

Only unit files owned from the user's systemd configuration/data directories
can be toggled. System-provided enabled services remain visible but read-only;
otherwise disabling one could make it disappear from the bounded inventory and
leave the user without a way to restore it. A mutation performs a second exact
inventory check, changes persistent unit-file state without starting or
stopping the process, reloads the manager, and requires a fresh user-owned
readback. Raw D-Bus diagnostics are never displayed.

## Live state, privacy, and recovery

A capacity-one filesystem stream coalesces changes under the XDG autostart and
user-unit roots. A sender-filtered `UnitFilesChanged` stream and filtered
well-known-name owner changes cover systemd mutations and manager restart. The
watcher reconnects after failure and retains a separate live-update error while
the last known-good snapshot remains visible.

Settings assigns a generation to every refresh, preview, reveal, and mutation.
An older stream snapshot cannot replace a newer transaction or its readback;
one pending refresh survives a busy operation. All I/O runs off the UI thread.
Errors are bounded, control-character-free, and use public entry filenames or
generic descriptions instead of private directories, executable paths, raw
filesystem diagnostics, bus peers, or authorization details.

## Linux acceptance matrix

F14 remains unchecked until the Ubuntu/niri reference PC proves:

- empty, normal, duplicate-precedence, hidden, session-excluded, unavailable
  `TryExec`, malformed, non-UTF-8, oversized, symlink, unreadable, and truncated
  XDG inventories;
- user and system enable/disable, exact managed-override restoration,
  permission failure, changed-source conflict, changed-destination conflict,
  post-write mismatch, and external-edit recovery;
- add, replace, cancel, command review, invalid activation, source/target races,
  and authoritative installed-copy readback;
- remove cancellation, Trash success/failure, changed-file refusal, lower
  system-entry recovery, and concurrent user-entry preservation;
- user-owned enabled/disabled/linked systemd services; read-only system-owned,
  runtime, masked, static, unknown, and protected rmac services; manager
  denial/failure, reload failure, and readback mismatch;
- filesystem changes, user-manager stop/restart, session-bus loss/recovery,
  event bursts, suspend/resume, and no stale stream overwrite;
- keyboard-only operation, focus restoration, 100–200% scaling, contrast
  modes, Orca names/states/errors, bounded idle CPU/wakeups, and no private path
  exposure in every failure state.

Commit only privacy-safe results. Do not record usernames, home directories,
private commands from personal entries, raw D-Bus errors, or Trash locations.
