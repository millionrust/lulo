# Secure lock recovery

This runbook is for a local Ubuntu rmac session whose lock provider is not
showing an authentication prompt or has entered a crash loop. It does not
bypass authentication. The supported recovery result is either a new
PAM-backed prompt on the still-locked compositor (the rmac lock screen, or its
swaylock fallback) or, as a destructive last resort, termination of the exact
graphical session back to GDM.

The installed lock is `rmac-lock.service`, which runs `rmac-lock-provider`
(ext-session-lock, PAM service `/etc/pam.d/rmac-lock`). It is a `Type=notify`
unit with a 10-second watchdog that kills a hung provider, restarts on failure,
and gives up after 5 starts in 30 seconds. When it fails for good (including a
missing PAM service, which `AssertPathExists=` turns into a failed start),
systemd runs `OnFailure=rmac-lock-fallback.service`: `rmac-locker` with the
distribution `/usr/bin/swaylock`, which restarts without a start limit. The
nested evidence units are separate; recover their compositor only through
`scripts/linux/run-lock-provider-recovery-gate.sh --execute`. Never mix the
nested evidence environment with the real graphical session.

## Prepare before enabling custom-provider evidence

Use a disposable local test account. While the normal graphical session is
unlocked:

1. Run `loginctl show-session "$XDG_SESSION_ID" --property=VTNr --value` and
   remember the graphical session's numeric VT.
2. Press `Ctrl+Alt+F3` and prove a local text login appears.
3. Sign in as the same unprivileged account that owns the graphical session.
4. Run `systemctl --user is-active rmac-lock-coordinator.service` and confirm it
   reports `active`.
5. Return to the remembered graphical VT; do not assume a fixed return function
   key.
6. Keep the account's password available without storing it in a script,
   command line, note, or evidence file.

Do not run custom-provider evidence through SSH, as root, or without this TTY
check. The nested recovery gate enforces the same prerequisites.

## First recovery: the locked-session shortcut

On niri's locked fallback screen, press the configured `Mod+Ctrl+Q` lock
shortcut once. That binding is the only generated shortcut marked
`allow-when-locked=true`; it starts the fixed readiness-gated
`rmac-lock.service`. Wait at least the unit's 15-second start timeout plus its
one-second restart delay before repeating it. Authenticate only when the rmac
lock screen or the swaylock fallback presents a real prompt.

Do not repeatedly invoke the shortcut, type a password into an unlabelled
surface, or interpret `LockedHint=true` as proof that a prompt is secure.

## TTY recovery: restart the accepted provider

If the shortcut cannot restore a prompt, switch to the already-proven local TTY
and sign in as the same unprivileged user. Do not use `sudo systemctl --user`:
that targets the wrong user manager and can make recovery state harder to
understand.

Inspect only bounded unit properties:

```sh
systemctl --user show rmac-lock.service \
  --property=ActiveState,SubState,Result,NRestarts --no-pager
systemctl --user show rmac-lock-fallback.service \
  --property=ActiveState,SubState,Result,NRestarts --no-pager
systemctl --user show rmac-lock-coordinator.service \
  --property=ActiveState,SubState,Result,NRestarts --no-pager
```

Then restart the accepted provider without changing the compositor, lock hint,
configuration, or imported Wayland environment:

```sh
systemctl --user reset-failed rmac-lock.service
systemctl --user start rmac-lock.service
```

A successful `start` means the readiness transaction completed and the
compositor is hiding session content. Return to the existing graphical VT and
authenticate on the lock screen. If `start` fails, the fallback unit should
already be running swaylock; if it is not, start it the same way:

```sh
systemctl --user reset-failed rmac-lock-fallback.service
systemctl --user start rmac-lock-fallback.service
```

If neither starts, record only the bounded properties above. Review any journal locally before sharing it because PAM modules and
system services may add machine or account metadata.

Never recover by running `swaylock` or `rmac-lock-provider` directly from the
TTY. The user manager
owns the reviewed graphical environment, readiness protocol, restart behavior,
and control group.

## Destructive last resort: terminate the exact session

If the accepted provider cannot become ready, preserve fail-closed state. Do
not clear `LockedHint`, kill niri, delete runtime files, rewrite the unit, or
disable PAM. Those actions are not unlock operations and can expose or corrupt
session state.

Terminating the graphical session loses all unsaved work. Use it only after the
user explicitly accepts that loss. First list sessions and inspect a candidate
without putting human-readable output into a script:

```sh
loginctl list-sessions
loginctl show-session SESSION_ID \
  --property=Name,User,Type,Class,State,Remote,Seat --no-pager
```

The candidate must name the same user, be local (`Remote=no`), use
`Type=wayland`, belong to a seat, and be the stuck graphical session—not the
current TTY session. Type the opaque session ID as one argument; never construct
it through `eval` or a shell-expanded command string. After explicit
confirmation of unsaved-work loss:

```sh
loginctl terminate-session SESSION_ID
```

This ends the session instead of bypassing its lock and returns control to the
display manager. A machine reboot is outside normal rmac recovery and remains
the final operator action if logind itself is unavailable.

## Evidence required before E5

On the Linux reference PC, record pass/fail for:

- locked niri shortcut recovery to a ready lock-screen prompt;
- same-user local-TTY restart of `rmac-lock.service`, and of
  `rmac-lock-fallback.service` after the provider's start limit is hit;
- refusal to use a root, SSH, or mismatched-user recovery path;
- preservation of the still-locked compositor while the provider restarts;
- successful PAM authentication after recovery;
- exact graphical-session identification and an explicitly accepted destructive
  termination using a disposable account; and
- a privacy review proving shared evidence contains no user name, session ID,
  PID, journal text, PAM prompt, or credential content.

Until this matrix passes on a station, `tty-recovery-proven` stays pending and
E5 remains incomplete.
