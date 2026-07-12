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

`rmac-lock-provider-linux` begins the platform boundary with fixed-capacity,
UTF-8 credential input. It rejects responses beyond Linux-PAM's 512-byte
maximum, never clones or logs the value, erases removed bytes immediately, and
zeroizes the complete allocation on clear and drop. This is a memory-lifetime
primitive, not authentication. The accepted Wayland and secret-erasure
dependency lines—and the reasons no PAM crate is accepted yet—are recorded in
`docs/rmac-lock-provider-dependency-review.md`.

On Linux the same crate can perform a non-mutating Wayland registry preflight.
It accepts only a compositor advertising `ext-session-lock-v1` version 1,
`wl_compositor` version 4, `wl_shm` version 1, and at least one `wl_output`, then
drops the connection without binding or locking. A prepared connection can bind
those authorities, complete output initialization, and track live output
add/remove/integer-scale events. Required-global removal is terminal. Neither
API exposes the lock request, so this foundation cannot blank or strand a
development session.

The platform-neutral lock-surface lifecycle now coalesces configure events,
requires the newest serial to be acknowledged before its exact-size commit,
invalidates paints made stale by resize or scale changes, and retains committed
buffer identities until compositor release. Integer scale is bounded to 8,
each ARGB buffer to 512 MiB, and each output to three in-flight buffers. Invalid
dimensions, arithmetic overflow, and more than 1 GiB reserved across all
outputs fail before allocation. Output removal abandons an unfinished paint and
requires the wire adapter to destroy the surface role while keeping any
released-later buffers accounted for. This is executable protocol ordering, not
a renderer or Wayland object implementation.

The first renderer is an original rmac midnight composition painted on the CPU
as opaque ARGB8888. It uses a fixed 16 KiB working chunk rather than allocating
a second output-sized image. On Linux the destination is a CLOEXEC, no-exec
anonymous memfd sized from the validated layout and sealed against write,
resize, and seal changes after the complete frame is flushed. The owned file
and redacted buffer identity are ready to remain alive until `wl_buffer.release`.
No Apple wallpaper, color token, icon, font, or other proprietary asset is used.

The Linux PAM boundary now uses only raw, pre-generated `pam-sys2` declarations
beneath rmac-owned code. Its typed worker conversation supports echo-on,
echo-off, info, error, radio, and bounded binary batches. The callback validates
outer pointers/counts/styles and bounded message termination, catches Rust
panics, checks every C allocation, and overwrites partial responses on failure.
The thread-bound transaction always runs authentication followed by account
policy and pairs successful start with one end; injected APIs test failure
ordering and preserve a secondary end error. The Ubuntu policy source includes
both `common-auth` and `common-account`, preserving pam-auth-update/site choices.

The worker/UI conversation now has a dependency-free bounded broker. PAM may
publish only one owned prompt while it waits, every prompt is re-bounded and
redacted, and textual or binary prompt storage is overwritten on drop. The UI
receives a unique single-use response capability, so a stale reply cannot be
replayed into a later prompt. Typed replies are checked on both sides, secrets
move to the PAM worker without cloning, and explicit cancellation, prompt drop,
or UI loss wakes the worker and fails closed. The UI endpoint is deliberately
pollable so the eventual lock renderer never blocks its Wayland event loop.

The safe Wayland preparation path now also requires `wl_seat` version 4 and a
keyboard capability, completes a third setup roundtrip, and refuses readiness
until an `xkb_v1` keymap has compiled. It tracks every supported seat plus
keyboard add/remove, focus, modifiers, group, and repeat metadata. Exact
`xkbcommon` 0.8.0 decodes the compositor keymap with the mandatory Wayland
keycode offset, locale compose sequences, and serialized modifier state. Only
bounded, drop-zeroized UTF-8 or redacted semantic actions leave that adapter;
unsupported formats, invalid sizes/states, missing keymaps, and decoder panics
fail preparation.

A platform-neutral prompt editor now consumes those semantic keys and owns one
broker prompt capability. It edits echo-off input in the existing fixed secret
allocation, edits echo-on input in a separately zeroized bounded allocation,
and handles Unicode fragments atomically so overflow cannot retain a prefix.
Backspace erases removed scalar bytes; submit moves secret/text responses to
PAM, notices acknowledge, radio prompts select, and Escape cancels. Generic
binary MFA is intentionally rejected for a module-specific UI. Editor, event,
and error diagnostics disclose neither input nor raw key identity.

The Linux adapter still requires session-lock acquisition and wire lock-surface
objects, `wl_shm_pool`/buffer release wiring, event-loop routing and client-side
repeat scheduling, module-specific binary MFA UI, IME/accessibility support,
and real niri/PAM evidence including the compiled fault tests. Until then, the
installed unit continues to run swaylock and the preview projection remains
unrendered.

## Not complete yet

- a reviewed Linux Wayland/PAM adapter, rmac lock presentation, and wallpaper;
- PAM password, wrong-password, cancellation, and supported MFA evidence;
- output add/remove, scaling, rotation, suspend/resume, and GPU-reset evidence;
- killed-locker automatic recovery and the documented TTY/manual recovery path;
- delay-inhibitor timing and forced-suspend failure evidence;
- accessibility and keyboard-layout evidence on the Linux reference PC.

Until those are proven, E5 and E10 remain unchecked and System Settings must not
show controls that imply the missing behavior exists.
