# Secure session lock

The accepted boundary is documented in
`docs/decisions/0004-secure-lock-boundary.md`. This file records the concrete
integration and its remaining proof rather than treating visual similarity as
security evidence.

## Current path

The global `lock` shortcut bypasses the general shell-event socket and runs the
fixed `systemctl --user start rmac-lock.service` request without a shell. The
unit starts `rmac-locker`, which launches `/usr/bin/swaylock` with the installed
absolute config path and captures its dedicated readiness descriptor on stdout.
Only the exact newline handshake advances the unit to ready. `systemd-notify`
then completes the `Type=notify` start transaction, so a successful shortcut
return means niri has already hidden security-sensitive content.

The locker remains a foreground child in the same systemd control group. A
normal zero exit clears logind's locked hint. A signal or nonzero exit fails the
unit; systemd retries after one second without a start-limit ceiling. niri's
protocol behavior keeps the session locked during that gap. The generated niri
fallback adds `allow-when-locked=true` only to the lock shortcut for manual
recovery.

The development installer requires `/usr/bin/swaylock`, installs the supervisor
and unit, and writes the default rmac swaylock config only when the user has no
existing config. Production packaging must depend on the Ubuntu swaylock build
with PAM support rather than installing a setuid or vendored binary.

The supervisor accepts only an absolute, regular UTF-8 config of at most 64 KiB.
It rejects `daemonize`, `ready-fd`, and nested `config` keys so appearance
customization cannot detach swaylock from its control group or counterfeit the
readiness channel.

`rmac-lock-coordinator.service` runs in normal and diagnostic safe-mode
sessions. The session bootstrap imports `XDG_SESSION_ID` through its narrow
routing allow-list, and the coordinator asks logind to resolve that concrete
session path. This is necessary because user-manager services do not reliably
belong to the graphical session and logind does not emit session signals on the
`/session/auto` convenience object. Before reporting itself ready it acquires a
`sleep:delay` inhibitor and subscribes to the session `Lock()` plus manager
`PrepareForSleep` signals. A lock request starts the same readiness-gated unit.
Before sleep, the inhibitor is released only after that unit is ready; after
resume, a new inhibitor is acquired. External logind `Unlock()` signals cannot
bypass swaylock/PAM.

The delay inhibitor is bounded by logind's configured maximum. If the locker
cannot become ready before that system deadline, logind may force the suspend;
the coordinator keeps the inhibitor open and reports the failure, but does not
claim it can override logind. Reference-PC failure injection must prove the
supported configuration locks within the deadline and characterize forced
suspend behavior before E5 is complete.

Idle locking is owned by `rmac-idle-lock.service`, which runs `/usr/bin/swayidle`
through a small validated wrapper. Its private versioned policy accepts only a
lock timeout from 60 seconds through 24 hours, or `null` for Never. The wrapper
constructs swayidle's arguments itself and exposes no configurable command;
the timeout always starts the same readiness-gated lock unit. It does not use
swayidle's logind hooks; exact-session logind work remains in the coordinator.
The installer creates a five-minute default only when no user policy exists.
Both normal and diagnostic sessions start this authority.

The required coordinator also owns `org.rmac.LockScreen1` on the user session
bus. Settings subscribes before its initial read and receives complete policy
snapshots. A timeout mutation validates the same versioned domain value,
atomically replaces the private policy, and restarts the idle authority. If the
new runtime cannot start, the service restores the previous file and runtime
before returning an error. The UI exposes only five supported timeout choices,
including Never, and reports loading, mutation, rollback, and reconnect states.

Lid-close and explicit suspend remain logind operations rather than duplicate
swayidle commands. The existing delay-inhibitor coordinator locks before those
operations. A future Settings control may request a supported logind action,
but must respect logind capability, authorization, docked-display policy, and
active inhibitors.

## Not complete yet

- supported suspend choices and capability/authorization reporting;
- notification preview filtering and lock wallpaper authority;
- PAM password, wrong-password, cancellation, and supported MFA evidence;
- output add/remove, scaling, rotation, suspend/resume, and GPU-reset evidence;
- killed-locker automatic recovery and the documented TTY/manual recovery path;
- delay-inhibitor timing and forced-suspend failure evidence;
- accessibility and keyboard-layout evidence on the Linux reference PC.

Until those are proven, E5 and E10 remain unchecked and System Settings must not
show controls that imply the missing behavior exists.
