# Session recovery

Use the same unprivileged account that owns the rmac session. Recovery never
requires root, deleting user data, resetting the user manager, or replacing the
stock Ubuntu/GNOME session.

## Collect a safe diagnostic report

From safe mode, GNOME, or a local TTY, run:

```sh
/usr/libexec/rmac/rmac-session-supervisor diagnostics
```

The versioned JSON report contains only allowlisted rmac unit names, typed
availability/health/restart facts, the safe-mode trigger, and whether shell
settings are current or recoverable. It deliberately excludes journal message
bodies, configuration values, paths, usernames, hostnames, PIDs, environment
values, tokens, notification/document content, and device identity.

`status` is an operator view and includes process state. Use `diagnostics`, not
`status` or raw journal output, for a report that may be shared.

## Restore broken shell settings

First inspect `shell_settings_recovery` in the diagnostic report. When it is
`last-good-available`, run:

```sh
/usr/libexec/rmac/rmac-session-supervisor restore-last-good-settings
/usr/libexec/rmac/rmac-session-supervisor clear-safe-mode
```

Restore refuses to replace an already-valid primary file. For an invalid
primary, it validates the last-good copy, preserves the rejected bytes in the
same settings directory as the owner-only
`shell.json.rejected-before-restore`, and only then atomically installs the
validated settings. The rejected copy can contain personal paths and
preferences; do not attach it to a report.

`defaults-only` means no user shell settings have been saved and defaults are
safe to load. `unavailable` means at least one settings file exists but no
validated recovery candidate is available; leave the files untouched and use
the stock GNOME session while investigating.

## Recover from a local TTY

1. Press `Ctrl`+`Alt`+`F3` and sign in as the affected user.
2. Run the `diagnostics` command above.
3. Restore settings only when the report says `last-good-available`.
4. Run `clear-safe-mode` only after the cause is repaired.
5. Sign out of the TTY and return to GDM with `Ctrl`+`Alt`+`F1` or the virtual
   terminal used by the distribution.

If the packaged binary or its dependencies are missing, do not copy executables
into `/usr` by hand. Select the stock Ubuntu/GNOME Wayland session in GDM and
repair the signed package with APT. The rmac package and its recovery commands
must never remove or rename that session.

Lock-screen recovery is a separate fail-closed security boundary. Follow
[Secure lock recovery](secure-lock-recovery.md) instead of clearing session
safe mode to bypass a lock.
