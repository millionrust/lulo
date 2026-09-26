# Troubleshooting and recovery

Start with the least invasive recovery. Do not delete configuration, reset a
user manager, fill the disk, or reinstall packages before capturing the
failure state and preserving user data.

## Return to a working session

If Lulo OS does not start or the shell repeatedly fails, sign out or return to
GDM and select the stock Ubuntu/GNOME Wayland session. The Lulo OS package
must never remove that recovery option.

The packaged session uses a safe-mode marker after a bounded crash loop. Safe
mode starts niri without Lulo OS portal selection and optional shell surfaces,
while retaining the supervised security boundary. It lasts one login: a notice
names the component that kept quitting, and the next login starts normally.
Run `/usr/libexec/rmac/rmac-session-start --clear-safe-mode` to leave it
without logging out. Use it to repair configuration or export data. See
[Ubuntu session packaging](ubuntu-session-packaging.md#safe-mode-and-recovery)
and [Session recovery](session-recovery.md).

## Inspect service health

From the affected user's session:

```sh
systemctl --user --failed
systemctl --user status rmac-session.target
systemctl --user status rmac-session-supervisor.service
```

Prefer a normal restart of the failed Lulo OS component or target over restarting
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
tokens, and device identity. Prefer `rmac-session-supervisor diagnostics`,
which excludes journal bodies, paths, PIDs, configuration values, and other
private content.

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
- **Touchpad dead after resume:** on a Synaptics RMI4 touchpad wired over
  SMBus with a PS/2 (`psmouse`) serio passthrough, a suspend/resume can leave
  the kernel logging `psmouse serioN: Failed to deactivate mouse … : -5` and
  only a non-gesture "PS/2 Generic Mouse" (or no pointer device at all)
  reappearing — everywhere, including the GDM greeter. The rmac-session
  package installs `/usr/lib/systemd/system-sleep/rmac-input-resume`, which
  always reloads the `rmi_smbus`/`psmouse` stack after resume when that driver
  stack is present. A stale touchpad entry in the kernel's input-device list
  does not prevent recovery; each module command is timeout-bounded and
  logged. The hook is a no-op without that stack; see [Hardware support](hardware-support.md#touchpad-recovery-after-resume).
  If a touchpad is still missing after that, check
  `journalctl -k -b | grep -i psmouse` and
  `journalctl -t rmac-input-resume -b` before filing a report.
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
