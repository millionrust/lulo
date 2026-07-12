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

The next slice adds a timezone/timedate adapter, shell-status publication, and
the System Settings/Quick Settings controls.
