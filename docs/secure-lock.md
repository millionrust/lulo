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
`wl_compositor` version 4, `wl_shm` version 1, at least one `wl_output`, and
`wl_seat` version 4, then drops the connection without binding or locking. A
prepared connection can bind those authorities, complete three output/seat/
keymap initialization roundtrips, and track live output, keyboard, and optional
pointer events.
Required-global removal is terminal. Neither public API exposes the lock
request, so ordinary callers cannot blank or strand a development session.

The platform-neutral lock-surface lifecycle now coalesces configure events,
requires the newest serial to be acknowledged before its exact-size commit,
invalidates paints made stale by resize or scale changes, and retains committed
buffer identities until compositor release. Integer scale is bounded to 8,
each ARGB buffer to 512 MiB, and each output to three in-flight buffers. Invalid
dimensions, arithmetic overflow, and more than 1 GiB reserved across all
outputs fail before allocation. Output removal abandons an unfinished paint and
requires the wire adapter to destroy the surface role while keeping any
released-later buffers accounted for. Failed painting can now abandon only its
exact reservation, so a wire error does not retain phantom memory budget.

The first renderer is an original rmac midnight composition painted on the CPU
as opaque ARGB8888. It uses a fixed 16 KiB working chunk rather than allocating
a second output-sized image. On Linux the destination is a CLOEXEC, no-exec
anonymous memfd sized from the validated layout and sealed against write,
resize, and seal changes after the complete frame is flushed. The owned file
and redacted buffer identity are ready to remain alive until `wl_buffer.release`.
Prompt shaping uses the distro's Inter and fallback fonts; no font is bundled.
No Apple wallpaper, color token, icon, font, or other proprietary asset is used.

The Linux-only internal lock typestate now issues the generated session-lock
request and immediately creates exactly one empty `wl_surface` and lock role per
current output; hotplug does the same. It waits for the first configure before
painting, coalesces later configures, acknowledges before attach/commit, sets
integer buffer scale, and damages the exact pixel extent. Each sealed frame is
passed through a temporary `wl_shm_pool`; the pool is destroyed immediately,
while the `wl_buffer` and frame remain owned until compositor release. Output
removal destroys its role/surface but preserves unreleased buffers. The wire
requires the core state machine's move-only authentication token before sending
`unlock_and_destroy`; every role is then destroyed and a display-sync roundtrip
must complete before exit. Client frame commits are reported only as commits;
the compositor `locked` event remains the sole readiness/presentation authority.

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

Pointer/touchpad capability is optional and tracked independently per seat. The
wire binds and releases `wl_pointer`, records only redacted focus plus bounded
surface-local coordinates, and clears a pending gesture on focus loss, output
removal, capability loss, or seat removal. Pure hit testing mirrors the visible
logical geometry: the 28-pixel submit target emits the same semantic Submit as
Return, and the two radio halves emit previous/next selection. Activation needs
the left-button press and release to resolve to the same target, so dragging
away cancels it. Binary prompts remain inert because the generic editor cannot
safely manufacture a module-specific binary reply. Raw coordinates and Linux
button identities never enter the credential editor or diagnostics.

Caps Lock state follows xkbcommon's locked modifier rather than inferring from
key presses. A platform-neutral aggregate shows the warning only while at least
one focused keyboard seat reports Caps Lock, keeps it active when another such
seat remains, and clears it on focus, capability, keymap, or seat loss. The
semantic boolean triggers a repaint but never enters the credential queue; the
original upward indicator is drawn only for text/password prompts. Seat and raw
modifier identities remain inside the Wayland adapter.

The wire also owns a per-seat client repeat scheduler. Compositor rates are
clamped to 100 events per second and delays to 10 seconds, a late event-loop
iteration emits at most one repeat instead of replaying a burst, and release,
focus loss, keymap replacement, or compositor-generated repeat cancels the
client schedule. Repeated keys are decoded again against the current modifier
state; raw key identity remains inside the Wayland adapter.

A platform-neutral prompt editor now consumes those semantic keys and owns one
broker prompt capability. It edits echo-off input in the existing fixed secret
allocation, edits echo-on input in a separately zeroized bounded allocation,
and handles Unicode fragments atomically so overflow cannot retain a prefix.
Backspace erases removed scalar bytes; submit moves secret/text responses to
PAM, notices acknowledge, radio prompts select, and Escape cancels. Generic
binary MFA is intentionally rejected for a module-specific UI. Editor, event,
and error diagnostics disclose neither input nor raw key identity.

A platform-neutral runtime coordinator now joins output/frame events, the
authoritative compositor lock decision, authentication attempt IDs, broker
prompts, semantic input, cancellation, worker completion, and the move-only
unlock token. It starts one initial attempt only after `locked`, bounds
pre-prompt input to 32 events, discards overflow instead of allocating or
crashing, drains a cancelled PAM worker before starting another, rejects stale
worker results, and treats worker panic as terminal fail-closed failure. Five
native tests execute success, failure/retry, cancellation/drain, compositor
finish, stale results, panic, bounded input, and diagnostic redaction.

On Linux a crate-internal pump now owns that coordinator, the lock wire, one PAM
worker, and its conversation endpoint. A `poll(2)` wait capped internally at 50
ms lets it service Wayland while checking worker/prompt completion instead of
blocking forever on either source. It converts wire input to coordinator events,
spawns only the attempt requested by the core, consumes unlock authority in the
synchronized wire method, and returns readiness, prompt-change, failure, and
an explicit authenticated/denied/failed-locked exit reason. Username validation
happens before connecting or locking. The pump remains crate-internal.

The renderer receives a copyable redacted state snapshot plus at most one
bounded PAM presentation label. Valid UTF-8 prompt text is normalized to one
display string, whitespace is collapsed, bidi controls are removed, invalid or
empty text gets a style-specific fallback, and the result is truncated on a
scalar boundary to 256 bytes. The owned label is zeroized on replacement/drop;
diagnostics expose neither text, prompt identity, character count, selection,
nor raster pixels. Credential response bytes never enter this path.

Linux shapes the label with exact `cosmic-text` 0.14.2 against installed Inter
and Unicode fallback fonts before blending only its alpha mask into the sealed
frame. The exact logind account name is also presented so the person knows
which session PAM will authenticate. It must already be nonempty, free of
whitespace normalization, controls, and bidi directives, and at most 128 bytes;
unsafe or oversized names are rejected before Wayland can lock rather than
being transformed into a misleading identity. The admitted value is zeroized
on drop and redacted from diagnostics. This is an intentional physical-screen
disclosure of the exact login name, not the full name, session ID, UID, or other
account metadata.

Font discovery begins while the Wayland connection is prepared, before the
session-lock request. The account raster is cached independently while prompt
identity avoids reshaping on every password dot; one shared eight-entry cache
keeps every layout raster at or below 2 MiB. The account sits between the avatar
and input field while changing PAM guidance appears below the field. The glyph
cache resets for every new prompt. Failure to rasterize either required label
is a wire failure, not a silently unreadable authentication UI. The existing
capped indicators and generic shapes remain as secondary state cues. Every
configured output repaints when state or label identity changes, while rendered
text still has no authority to unlock or declare secure readiness.

When PAM is working without an outstanding prompt—before the first prompt,
after a submitted response, or between messages—the coordinator now projects a
static `Authenticating…` label and three-dot state cue. It is event-driven, does
not claim measurable progress, and causes no animation or periodic redraw.
Pointer activation is disabled while that state, a binary prompt, or no prompt
is visible. Bounded keyboard input may still wait for the next real prompt under
the existing 32-event queue; only that prompt can interpret it. PAM success,
failure, cancellation, and multi-message transitions replace the status from
authority rather than a UI timer.

An opt-in `development-provider` feature now supplies the uninstalled
`rmac-lock-provider` process used by the future recovery/evidence harness. It
resolves `XDG_SESSION_ID` through logind, reads the exact session's UID and PAM
user name, and refuses to continue unless that UID equals the process's
effective UID. A portable lifecycle orders `LockedHint=true` before systemd
`READY=1`; denial never reports ready, failure after `locked` never clears the
hint, and only authenticated unlock after the Wayland display-sync barrier
attempts `LockedHint=false`. Hint failure remains an advisory warning, while a
readiness notification failure terminates the provider so systemd can restart
it fail-closed. Status and errors contain no session ID or username.

The evidence provider also participates in systemd's process-scoped watchdog
contract. It validates the manager-provided watchdog environment before the
Wayland connection can lock, arms only after compositor-confirmed readiness,
and sends one fixed watchdog notification at half the configured interval.
Late event-loop checks never emit a catch-up burst. The evidence unit uses a
ten-second timeout and `SIGKILL`, so a stopped provider is replaced without
waiting for code inside the failed process; notification failure is fatal and
therefore enters the same bounded restart path.

The Linux adapter still requires Linux validation of its separate unit,
localized prompt shaping, font fallback, and recovery harness, plus emergency
recovery. Module-specific binary MFA UI, IME/accessibility support, and real
niri/PAM evidence including the compiled fault tests also remain. The
development feature is not built by the session installer and must not be
launched ad hoc. Until the full matrix passes, the installed unit continues to
run swaylock.

The first recovery harness is now repository-owned but remains opt-in. A
separate installer builds only the feature-gated provider after checking for 25
GiB of build headroom, requires the reviewed PAM policy and normal swaylock
supervisor to exist, and installs two non-enabled evidence units under a
distinct namespace. The custom unit has a five-start/30-second recovery budget;
the fallback unit runs the accepted `rmac-locker` against the same nested
display without a start limit. Neither unit replaces or aliases
`rmac-lock.service`.

The interactive gate creates a nested Sway compositor, passes its generated
Wayland display through a private bounded environment file, waits for
compositor-confirmed custom-provider readiness, sends `SIGSTOP` and proves the
watchdog starts a new ready instance, then sends `SIGKILL` and proves crash
restart independently. It then stops the custom unit
while the nested session remains locked and starts the swaylock fallback. The
test succeeds only after the user authenticates in that fallback. Execution is
refused for root, SSH sessions, nonstandard runtime directories, missing local
session identity, missing assets, or absent interactive acknowledgement. Its
exit trap stops both evidence units and the nested compositor, clears the
advisory test hint, and removes its private runtime state. Static tests enforce
these separation and recovery contracts; real Linux execution remains pending.

## Not complete yet

- a reviewed Linux Wayland/PAM adapter, rmac lock presentation, and wallpaper;
- PAM password, wrong-password, cancellation, and supported MFA evidence;
- output add/remove, scaling, rotation, suspend/resume, and GPU-reset evidence;
- real hung/killed-locker recovery evidence and the documented TTY/manual recovery
  path;
- delay-inhibitor timing and forced-suspend failure evidence;
- accessibility and keyboard-layout evidence on the Linux reference PC.

Until those are proven, E5 and E10 remain unchecked and System Settings must not
show controls that imply the missing behavior exists.
