# Troubleshooting and recovery

Start with the least invasive recovery. Do not delete configuration, reset a
user manager, fill the disk, or reinstall packages before capturing the
failure state and preserving user data.

## Return to a working session

If rmac does not start or the shell repeatedly fails, sign out or return to GDM
and select the stock Ubuntu/GNOME Wayland session. The rmac package must never
remove that recovery option.

The packaged session uses a safe-mode marker after a bounded crash loop. Safe
mode starts niri without rmac portal selection and optional shell surfaces,
while retaining the supervised security boundary. Use it to repair
configuration or export data. See
[Ubuntu session packaging](ubuntu-session-packaging.md#safe-mode-and-recovery).

## Inspect service health

From the affected user's session:

```sh
systemctl --user --failed
systemctl --user status rmac-session.target
systemctl --user status rmac-session-supervisor.service
```

Prefer a normal restart of the failed rmac component or target over restarting
the entire user manager. A service restart must preserve unrelated state and
force an authoritative refresh.

## Logs

View a bounded time range for one unit:

```sh
journalctl --user --unit rmac-session-supervisor.service \
  --since "15 minutes ago" --no-pager
```

Journals may contain private operating-system or application data. Do not post
raw logs. Remove usernames, hostnames, paths, addresses, document/notification
content, environment values, process/session IDs, D-Bus peers, credentials,
tokens, and device identity. Prefer the privacy-safe status/report action in
System Settings when available.

## Common failures

- **Low disk:** stop builds and evidence collection before the host reaches
  15 GiB free. Remove only understood build artifacts or move synthetic
  evidence; never delete user documents automatically.
- **Malformed configuration:** validate the candidate, retain the last-known-
  good file, and use the supported rollback. Do not overwrite the user's niri
  graph with a generated default.
- **Missing portal:** confirm the full niri session environment and portal
  services. Do not enable the shortcut fallback unless runtime status requires
  it.
- **Disconnected mount/device/display:** wait for authoritative recovery or
  move work to a known local location. Do not continue writing to a stale path.
- **Update interrupted:** boot the stock recovery session and let APT/dpkg and
  PackageKit report their real state. Never disable signature verification.

## Lock recovery

A dead cosmetic shell is not permission to bypass authentication. Use the
locked-session recovery shortcut or the same-user local TTY procedure in
[Secure lock recovery](secure-lock-recovery.md). Never use root, SSH, or a
different user's manager to fake a successful unlock.

## Reporting a problem

Include the exact revision/package version, public H8 station class, Ubuntu and
niri versions, whether GNOME recovery works, the failed journey, expected and
observed behavior, and a minimal reproduction with synthetic data. Attach only
redacted, bounded evidence.
