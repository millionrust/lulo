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
lock timeout from 60 seconds through 24 hours and an optional automatic-suspend
timeout from five minutes through 24 hours; either may be `null` for Never. The
wrapper constructs swayidle's arguments itself and exposes no configurable
command. Lock timeout always starts the readiness-gated lock unit. Suspend
timeout calls the fixed `RequestSuspend` method through `/usr/bin/busctl`. It
does not use swayidle's logind hooks; exact-session logind work remains in the
coordinator. The installer creates a five-minute lock and Never-suspend default
only when no user policy exists. Both normal and diagnostic sessions start this
authority.

The required coordinator also owns `org.rmac.LockScreen1` on the user session
bus. Settings subscribes before its initial read and receives complete policy
snapshots. A timeout mutation validates the same versioned domain value,
atomically replaces the private policy, and restarts the idle authority. If the
new runtime cannot start, the service restores the previous file and runtime
before returning an error. The UI exposes five lock and five automatic-suspend
choices, each including Never, and reports loading, mutation, rollback, and
reconnect states.

Automatic suspend is exposed only when logind `CanSuspend()` returns `yes`.
`challenge` is not accepted for an unattended action because no user is present
to answer PolicyKit; `no`, `na`, malformed, and unavailable responses also hide
the enabling choices. The request rechecks capability, then calls
`Suspend(false)`, so logind remains the authorization and inhibitor authority.
The existing delay-inhibitor coordinator locks before the machine sleeps.
Lid-close policy remains the system-wide logind configuration and continues to
respect its docked-display behavior rather than being overridden per session.

Notification history now has a bounded, action-free lock projection. It applies
the stricter of the authenticated app hint and the user's per-app policy,
omits hidden and read records, redacts content when required, returns newest
first, and never returns more than 16 records. The projection does not grant
rendering authority. The current PAM-enabled swaylock provider has no content
interface and therefore enforces the stricter global result: Notification
Previews is truthfully shown as Hidden and no ineffective disclosure control is
offered. A future rmac lock client must own the ext-session-lock surfaces and
PAM handoff before it may consume this projection.

`rmac-lock-provider` now defines that future client's dependency-free security
state machine. It separates compositor readiness from output presentation,
tracks hotplugged output frames, issues unique asynchronous authentication
tokens, rejects stale outcomes, and emits unlock only for the matching success.
Compositor denial before readiness and compositor failure after readiness are
different terminal states, but neither unlocks. Diagnostics redact output and
attempt identity. See ADR 0005.

This is not yet an authentication provider. The Linux adapter still requires a
reviewed PAM binding, bounded multi-message conversation handling, secret
erasure, `pam_start`/`pam_end` lifetime correctness, renderer integration, and
real niri/PAM evidence. Until then, the installed unit continues to run
swaylock and the preview projection remains unrendered.

## Not complete yet

- a reviewed Linux Wayland/PAM adapter, rmac lock presentation, and wallpaper;
- PAM password, wrong-password, cancellation, and supported MFA evidence;
- output add/remove, scaling, rotation, suspend/resume, and GPU-reset evidence;
- killed-locker automatic recovery and the documented TTY/manual recovery path;
- delay-inhibitor timing and forced-suspend failure evidence;
- accessibility and keyboard-layout evidence on the Linux reference PC.

Until those are proven, E5 and E10 remain unchecked and System Settings must not
show controls that imply the missing behavior exists.
