# Focus policy

`rmac-focus` is the framework-neutral E4 policy authority. Focus is not a
decorative shell toggle: its current mode is composed into every notification's
`DeliveryPolicy` before the notification server decides banner, sound, and
history delivery.

Each validated mode has an opaque ID, user-visible name, up to 256 allowed app
IDs, and an explicit urgent-notification bypass. Configuration supports up to
32 modes and 64 weekly schedules. Schedules have enabled state, a nonempty day
set, local start/end minutes, and priority. Overnight ranges attribute the
after-midnight portion to the previous start day. Overlaps choose the highest
priority and then a stable schedule-ID tie break. Duplicate IDs, missing mode
references, zero-length ranges, invalid local minutes, and unbounded input fail
closed.

Manual activation overrides schedules and may be indefinite, for a bounded
duration, or until an explicit Unix time. Durations are limited to seven days.
The manual record is separately exposed for private persistence; on restart the
first clock evaluation removes an expired record and immediately falls back to
the applicable schedule. Disabling manual Focus likewise reveals schedule
state instead of forcing Focus off globally.

The time adapter supplies both Unix and monotonic milliseconds, current local
weekday/minute, and the timezone-aware next local minute boundary. The engine
recomputes schedule state from each sample rather than advancing a cached
weekday. A difference greater than two seconds between wall and monotonic
deltas is reported as a clock jump, but recomputation occurs for every sample
regardless. This handles suspend, manual clock changes, timezone changes, and
DST without treating a stale timer as authority. While a manual mode is active,
no per-minute schedule wake is requested; a temporary mode requests only its
exact expiry.

Notification enforcement starts with the app's Notifications policy. Allowed
apps bypass Focus while retaining their own banner/history/sound settings.
Other apps have `focus_active` set; urgent bypass is allowed only when both the
app policy and active mode permit it. The status projection exposes active
state, redacted mode identity/name, activation source, and an exact end time
only for temporary manual activation. Scheduled end remains a local-time rule
and is intentionally not presented as a potentially false Unix timestamp.

Mode names, IDs, schedule IDs, and allow-listed apps are redacted from default
diagnostics.

`rmac-focus-store` persists configuration and the manual override under
`$XDG_CONFIG_HOME/rmac/focus.json`. Its directory is forced to `0700`; primary
and last-good files are created as `0600` before content is written and use
same-directory atomic replacement. Input is capped at 1 MiB and every restored
mode ID, name, app ID, schedule range/day/reference, duplicate, and manual mode
reference is reconstructed through the domain validators. Unsupported or
corrupt primary data uses last-good; double corruption returns four safe
original defaults (Do Not Disturb, Personal, Work, Sleep) with no active manual
override. Missing files are a clean first run. Permission/I/O failures remain
errors, and paths plus all user/app identifiers are redacted from diagnostics.

`rmac-focus-runtime` loads that store, immediately evaluates the restored
manual/schedule state against a timezone-aware `chrono::Local` sample, and
persists removal of an expired manual override. All mutations save first-class
mode state and return the new evaluation, projection, persistence health, and
exact wake. A save failure does not falsify the in-memory active policy; it is
reported as degraded persistence. Replacing configuration retains a manual
override only while its mode still exists.

`rmac-focus-linux` listens to systemd-timedated property signals,
systemd-logind's post-resume `PrepareForSleep(false)`, and timedate/logind
service-owner restarts. These are wake hints only: every event causes the
runtime to resample local and monotonic clocks. The watcher reconnects with a
bounded delay and uses backpressure rather than polling. This follows the
official logind contract that `PrepareForSleep` is emitted immediately before
and after sleep.

The same crate exports the single-writer `org.rmac.Focus1` user-session D-Bus
authority. It owns the runtime and private store, exposes state, activate,
disable, and Quick Settings enable/disable methods, and emits bounded typed
state changes. Calls require an authenticated session-bus sender. Enabling from
the compact tile selects Do Not Disturb; explicit mode and duration activation
is available for the full Focus pane. A compact disable never pretends to
override a running schedule: it returns an actionable instruction to change
that schedule in Focus settings. Persistence degradation is returned to the
caller even though the live in-memory policy remains truthful.

Every successful D-Bus policy mutation also submits one nonblocking request to
the service evaluator. The one-slot channel coalesces bursts without delaying
the caller; the evaluator resamples the latest complete authority and replaces
its previous timer with the new temporary expiry or local-minute schedule
boundary. A no-wake policy waits only for a mutation, timedated/logind hint, or
service shutdown—there is no daily polling fallback. This prevents a temporary
activation or newly edited schedule from inheriting the deadline that existed
before its D-Bus transaction.

The authority now exposes bounded `Configuration` and
`ReplaceConfiguration` operations for the full Settings pane. One transaction
contains all modes and schedules: mode ID, user-visible name, exact allowed-app
set, urgent behavior, schedule ID/mode reference, weekday mask, local start/end
minutes, priority, and enabled state. The decoder enforces the domain's 32-mode,
64-schedule, and 256-app-per-mode limits before construction; duplicate app
IDs, duplicate mode/schedule IDs, empty or unknown weekday masks, invalid local
times, unknown mode references, and unbounded text fail closed. Encoding is
deterministically ordered.

Replacement reuses `rmac-focus-runtime`'s atomic whole-config mutation, retains
a manual activation only if its mode still exists, reevaluates immediately,
and reports degraded persistence without falsifying live state. A dedicated
configuration-change signal is emitted even when active top-bar projection is
unchanged. Settings clients subscribe before the initial read, reread after
every signal, reconnect with bounded delay, and never edit `focus.json`
directly.

Notification admission uses the same authority's typed delivery-policy method.
The notification service supplies its per-app enabled, banner, sound, history,
and urgent-through-Focus choices; Focus composes the active mode and exact app
allow list without exporting that private list to every shell process. The
wire decoder rejects unknown history modes and inconsistent state.

The shell status reducer now accepts live Focus projection separately from
shell preferences. Visibility remains a preference, but active mode and expiry
come only from the Focus runtime. On service loss, the top bar preserves its
last-known-good indicator, Quick Settings becomes non-writable, and health
becomes unavailable. A stale saved toggle can no longer impersonate live Focus.
Quick Settings now mutates and rereads only the D-Bus authority; connection
loss makes the tile read-only while preserving last-known-good display state.

System Settings now loads both live state and the complete configuration from
that authority off the UI thread. Its Focus pane activates any built-in mode
indefinitely or for one hour, turns off a manual mode, changes urgent delivery,
and manages application allow lists discovered from Notification Center. It
shows the authority's opaque active mode ID only for exact row selection; the
client's default diagnostics continue to redact that ID.

`rmac-focus-settings` performs every pane edit by rebuilding and validating the
whole configuration before it can reach D-Bus. A fresh installation can create
a real Monday-through-Friday 9:00 AM–5:00 PM schedule for any mode, then change
its days, start/end time in 15-minute increments, enabled state, or delete it.
Overnight ranges are presented honestly as continuing into the next day. The UI
updates optimistically while busy, rereads authority after every mutation, and
rolls back its optimistic configuration when both mutation and refresh fail.
Errors remain visible and no control writes `focus.json` directly.

The authority's `Settings` method encodes configuration and live state while
holding the same runtime lock. The Settings client installs both signal
subscriptions before its initial atomic read, then publishes that complete
pair. Every state or configuration signal repeats the atomic method, so the
pane never has to merge independently versioned streams. Service loss preserves
last-known-good content, reports connection health separately from mutation
failures, and reconnects with a bounded delay. This separation prevents a later
healthy signal from hiding an earlier persistence error.

Live state also carries an explicit absent/manual/scheduled source. Scheduled
state includes the exact bounded schedule ID, which is redacted from default
diagnostics. The client rejects inconsistent source combinations and, for the
atomic Settings projection, requires the schedule to exist and target the
reported mode. System Settings therefore offers `Turn Off` only for manual
Focus; scheduled Focus is labelled truthfully and routes `Edit Schedule` to the
exact authoritative rule instead of issuing a disable request that is defined
to fail.

The Focus pane has a scoped GPUI build but still requires live Linux/niri
interaction and accessibility evidence before E9 can be checked complete.
Installed portal application IDs are resolved through the live XDG
desktop-entry catalog for localized names and original theme icons. Resolution
is exact except for the standard `.desktop` suffix alias; unmatched or
transient D-Bus sender IDs retain an honest generic icon and their original
identifier rather than borrowing a plausible name.
