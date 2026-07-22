# Notification authority

`rmac-notifications` is the framework-neutral authority for notification
validation, replacement, delivery policy, lifecycle, actions, and bounded
history. It deliberately has no GPUI, D-Bus, portal, filesystem, clock, or
sound dependency. E2 banners, E3 Notification Center, E4 Focus, shell status,
and both Linux protocol adapters must consume this same reducer.

The contract follows XDG Notification portal version 2 and the freedesktop.org
Desktop Notifications Specification 1.3. The portal adapter maps its
application-scoped string ID to `Source::Portal`; reusing that ID updates the
existing record without flicker unless `show-as-new` was requested. The
`org.freedesktop.Notifications` adapter maps the returned nonzero integer to
`NotificationId`, passes `replaces_id` back as `Request::replaces`, and converts
close reasons to the protocol signal values.

## Input and privacy boundary

Adapters authenticate the sender, validate transport-specific icons and sealed
file descriptors, strip unsupported markup, and then construct the bounded
domain types. Titles are limited to 512 bytes, bodies to 16 KiB, action targets
to 16 KiB, and a notification to eight visible buttons. Conflicting
`transient` and `tray` requests fail instead of being guessed.

User-visible content, app identity, portal replacement IDs, button labels, and
action targets are redacted from `Debug`. Domain errors contain only a field
and problem classification. Transport and persistence adapters must preserve
that property: never log request payloads, document paths, message text, or
serialized action targets.

`protocol::portal` maps portal-v2 priority, display hints, default activation,
buttons, targets, category, and sound policy. It advertises only the standardized
categories and button purposes it understands. Unknown display hints remain
forward-compatible; an unknown button purpose is treated as an ordinary button
when it has a label, or rejected when no accessible label could be shown.
`protocol::freedesktop` maps action pairs, default activation, urgency,
resident/transient/suppress-sound hints, replacement, and exact timeout values.
The visible legacy `app_name` is never used as authenticated ownership.

`rmac-notifications-linux` is the only `a{sv}` decoder. Known keys with the
wrong D-Bus type fail with redacted field-only errors; unknown extensible keys
are ignored. Portal targets are serialized into bounded canonical variant bytes
and targets containing file descriptors are rejected. Markup bodies become
inert text with bounded input and balanced-tag validation. Portal-v2 themed
icons accept at most 16 safe theme names. Sealed descriptor icons are frozen
before dispatch, capped at 4 MiB, and fully decoded as square PNG/JPEG images no
larger than 512 pixels or parsed as square SVGs capped at 4 KiB. SVGs use a
non-executable element profile and reject scripts, event attributes, embedded
media, styles, and non-local resources. Deprecated byte icons fail closed.

Sealed custom sounds are likewise frozen before dispatch and structurally
validated as Ogg Opus, Ogg Vorbis, or PCM WAV. Container sequence/checksum,
codec headers, PCM layout, a 2 MiB byte limit, and a 15-second duration limit
on PCM data or the Ogg-declared granule are enforced before presentation can
see the bytes. The eventual player must independently stop at the same
wall-clock deadline because a compressed stream can lie about its granule.
Media is carried only by
the bounded, backpressured runtime event and is removed when computed banner or
sound policy suppresses it; it never enters the reducer or durable Center
history. Portal admission is nonblocking and capped at four in-flight requests;
full media decoding runs off the D-Bus executor and is serialized, preventing a
concurrent maximum-image flood from multiplying decoder allocations. The
freedesktop server still advertises only `actions`, `body`, and
`persistence`—not markup, sound, hyperlinks, or image capabilities—because its
unimplemented optional media path must remain undiscoverable.

The same crate now serves `org.freedesktop.Notifications` at the standard
object path and `org.freedesktop.impl.portal.Notification` version 2 at the
portal backend path. Both interfaces share one locked reducer and a bounded,
backpressured runtime event stream. Legacy ownership is the authenticated
unique D-Bus sender—not the untrusted visible `app_name`. Portal methods verify
that the caller currently owns `org.freedesktop.portal.Desktop` before trusting
its forwarded app ID. `CloseNotification` checks ownership and emits protocol
reason 3. The session installer places the rmac portal descriptor, D-Bus
activation service, and `rmac-portals.conf` under the user's XDG data home.
Normal startup prepends `rmac` to `XDG_CURRENT_DESKTOP`, starts the backend, and
then restarts only an already-running portal frontend so it rereads selection.
Safe mode does none of those changes. Other portal interfaces continue through
the GNOME/GTK fallback order.

E2/E3 receive a `ServiceHandle`, never a raw connection. Its dismiss and expiry
methods emit freedesktop close reasons 2 and 1. Action invocation preserves the
declared target, emits the legacy activation-token signal before
`ActionInvoked`, and reports the resulting close. Non-exported portal actions
emit the backend `ActionInvoked` parameter array. `app.*` actions call
`org.freedesktop.Application.ActivateAction`, stripping the prefix, deriving
the standardized object path, and passing the target plus activation token.
Opaque target decoding is bounded and rejects trailing or malformed bytes.

`hide-on-lockscreen` and `hide-content-on-lockscreen` normalize to a typed lock
visibility policy. No lock UI may weaken that policy. An unspecified hint stays
`Policy`; `Center::lock_previews` resolves it through trusted per-app settings
and applies whichever rule reveals less. The projection is newest-unread-first,
capped at 16 records, and contains only notification ID, app identity, update
time, and optional title/body. Hidden records are omitted; redacted records have
no content. Actions, targets, categories, sounds, and transport replacement IDs
never cross the lock projection boundary, and its diagnostics redact app and
message data. The current swaylock provider consumes none of this projection,
so it remains stricter and displays no notifications.

## Delivery and lifecycle

Delivery is calculated once from the notification request and current per-app
and Focus policy:

- disabled apps receive no banner, history, or sound;
- Focus suppresses normal banners and sounds while still allowing policy-owned
  history; urgent bypass is an explicit policy switch;
- `transient` allows a banner but never history;
- `tray` allows history but no banner;
- history blocking does not silently block a permitted banner;
- sound is permitted only when a banner is actually delivered and the request
  is not silent.

The reducer uses caller-supplied monotonic milliseconds. Default low, normal,
and high banner lifetimes are 5, 7, and 10 seconds. Urgent notifications do not
expire automatically. Explicit freedesktop values retain their specified
meaning: `-1` uses policy, `0` never expires, and positive values are
milliseconds. Banner expiration keeps policy-allowed Notification Center
history and its actions; transient records are discarded.

Replacement retains the internal ID and creation time, resets update and
expiry state, and replaces the matching history record. Portal identity is the
pair `(authenticated app, external ID)`, so one app can never replace another
app's notification. Legacy numeric replacement also checks authenticated
ownership. Portal buttons are activated by declared position, allowing several
buttons to export the same action with distinct targets. Named activation is
retained for the freedesktop protocol.

History is bounded in memory and exposes an unread/urgent projection for the
top bar. The E3 store and live Center surface consume only records whose
computed delivery permits history.
Clearing, expiration, withdrawal, and action closure are distinct typed events
so adapters can emit truthful protocol results and UI can animate without
inventing state.

## Banner behavior

`rmac_notifications::banner::Stack` is the content-free E2 presentation state.
It resolves active, pointer, then primary output according to an explicit
policy and falls back only to a connected output. Hardware output identifiers
are redacted from diagnostics. Each output shows at most three banners by
default, newest first at the trailing top edge. A flood retires the oldest
visual banner while retaining the authoritative notification and Center
history; keyboard-focused banners are never displaced.

Atomic replacement keeps the existing output and phase without replaying
motion. Portal `show-as-new` deliberately moves to the newly resolved output
and replays entrance motion. Full motion uses bounded 220 ms entrance and 180
ms exit phases; reduced motion completes either phase immediately, including
when the preference changes mid-transition. The scheduler requests frames only
during those phases and otherwise returns one exact monotonic timeout.

Banner timeout begins when the banner is visibly presented. Pointer hover and
keyboard focus independently pause the exact remaining duration; removing one
pause does not resume while the other remains. Moving keyboard focus into the
stack emits one capture effect, moving among banners retains that capture, and
leaving or closing the focused banner emits one restoration effect. Output
disconnect moves visual banners to the connected fallback and reapplies the
bound without closing their notification records.

The `rmac-notifications-runtime` coordinator joins this state with compositor
topology/focus and resolved appearance without copying content. A post uses the
notification's computed banner delivery and timeout duration, then resolves
the focused connected output. Temporary loss of the compositor keeps the
last-known-good topology; hotplug moves existing banners without closing
records. Appearance changes can finish active motion immediately.

Runtime effects collapse to one redraw plus typed focus and service commands.
Paused timeout completion emits `Expire(id)`, which calls the service's
single-banner expiry path instead of scanning other notifications whose
banners may still be paused. User dismissal emits `Dismiss(id)`. Successful
action dispatch closes only the visual banner because the service already owns
the notification transaction. This keeps D-Bus signals, Center history, and UI
animation in one order without polling or content duplication.

Service closure is also an explicit reconciliation input. Withdrawal,
history-only posts, and duplicate closes for banners that were never shown or
were already flood-retired are inert. A policy-suppressed replacement begins a
visual-only exit for any older banner with the same stable ID. If an
authoritative close arrives during a local expiry/dismiss animation, the
existing visual deadline is preserved but its pending service command is
disarmed, preventing a second close after the authority has already committed
the first one.

`rmac_notifications_runtime::presentation::Presenter` is the renderer-neutral
banner view contract. It atomically joins bounded stack geometry to the exact
application name, title, body, priority, default action, and visible buttons.
Explicit button text is preserved; known portal purposes receive their standard
verb, and unknown unlabeled purposes never become guessed controls. Stable card,
button-position, and dismiss identities preserve the service's exact action
target. Persistent notifications expose no dismiss control. All presentation
content, application names, control labels, and output identities remain
redacted from diagnostics.

Every card is an assistive live region: urgent notifications are assertive and
the rest polite. New banners are announced once, while replacement is announced
again only when `show-as-new` explicitly requests it. Cards and controls form a
stable keyboard order supporting Tab, Shift+Tab, arrows, Home, End, Enter,
Space, and Escape. One exact control may be busy at a time, and only its matching
completion releases the activation latch. If the notification authority closes
a banner before its exit animation completes, the presenter retains its last
validated content until the stack removes the terminal visual; focused content
then releases focus exactly once. A malformed, duplicate, missing, or oversized
snapshot fails atomically without replacing the last valid presentation.

`rmac_notifications_linux::banner::BannerSession` is the authority-bound E2
host state. It consumes the service's owned runtime events after the same event
has updated Center history, supplies exact validated content to the presenter,
and retains only icon media belonging to current or terminal cards. Posts wait
in a replacement-aware queue until a real compositor output exists; neither
their announcement nor sound cue fires early, and each fires only once when the
banner can actually be published. Policy-suppressed replacements animate the
previous validated card out and never substitute the newly suppressed content
or icon.

Each renderer card now carries its exact validated application ID while keeping
that identity redacted from diagnostics. `rmac_notifications_linux::icon`
turns a logical icon edge and finite output scale into one exact rounded-up
physical edge, then resolves a portal file or ordered themed name before the
exact application-catalog icon. Only the standard `.desktop` suffix alias is
accepted; missing or failed sources remain an explicit generic fallback rather
than guessing from the visible application name. The shared XDG resolver uses
theme directory size and scale metadata, and the shared `rmac-icon` boundary
content-detects bounded PNG, JPEG, or non-executable SVG into centered square
RGBA8 pixels.

Frozen portal pixels use a notification-, size-, declared-format-, and exact
source-identity cache. Theme and catalog files use a metadata-revalidated LRU;
both are byte bounded, serialize decoding, redact sources and pixels from
diagnostics, and can be refreshed when the theme or application catalog
changes. A replaced notification cannot reuse the old frozen image, a declared
format mismatch fails closed, and retiring cards release their portal cache
entries.

`rmac_notifications_linux::icon_worker` is the off-UI execution boundary for
that synchronous resolver. `BannerSession::icon_source` constructs each job
from the retained card's exact authenticated app ID and media; the host never
reconstructs identity from visible text. A batch contains at most 500 unique
notification/logical-size/physical-size keys, 16 MiB of frozen source data, and
32 MiB of possible output. Two-slot command and event queues make backpressure
explicit, while one serial worker owns both caches. Theme refresh,
application-catalog replacement, and live-notification cache pruning are typed
commands whose diagnostics expose no source identity.

Each request generation cancels its predecessor between icons. The renderer
session applies only the exact current generation, cardinality, key order, and
logical/physical pixel dimensions; stale, partial, reordered, or malformed
completion cannot replace current artwork. Matching last-accepted pixels stay
visible during refresh and return on cancellation. Worker shutdown cancels
active work and closes the result receiver before joining, including when the
bounded event queue was full. The Linux surface host therefore only submits a
current frame's jobs and uploads accepted renderer-ready RGBA pixels.

Pointer/touch and keyboard activation share the presenter's exact stable
control IDs. The session maps those IDs to default, original button position,
dismiss, or single-banner expiry requests against the same `ServiceHandle` as
both D-Bus protocols. Activation tokens are optional, bounded, control-free,
and redacted. The bounded event receiver must continue draining while a service
request executes; action/close events may lawfully arrive before the operation
future completes, and the session reconciles either ordering without restarting
terminal motion or unlocking a different control. The remaining Linux host
must render the frame, submit its exact icon jobs to the ready worker, upload
only accepted RGBA pixels, play each emitted sound cue through the
single-admission `SoundPlayer`, and surface operation failures. The player uses
an original bounded default cue or the validated custom bytes,
passes a sealed seekable `memfd` to `pw-play` without a temporary pathname, and
kills/reaps playback at the independent 15-second deadline.

## Notification Center storage

`rmac-notifications-store` owns the E3 history and per-app policy file under
`$XDG_STATE_HOME/rmac/notifications/`. The directory is forced to mode `0700`;
the primary and last-good JSON files are created as `0600` before any content
is written and replaced with same-directory atomic renames. Files are limited
to 8 MiB, 500 total records, 100 records per app, eight actions per record, and
512 app policies. Every restored record is reconstructed through the domain's
bounded content, identity, action, target, and category validators.

Malformed/unsupported primary data recovers the last-good file; if both are
invalid, the Center starts empty with an explicit recovery state. Missing files
mean a clean first run. Permission and other real I/O failures surface as
errors rather than pretending the Center is empty. Errors and `Debug` output
contain operation classifications and counts only—never paths, app IDs,
content, labels, replacement IDs, or targets.

The in-memory Center performs atomic replacement, newest-group-first
projection, per-app/all clear and mark-read controls, badge-aware unread/urgent
projection, and bounded eviction. Transient notifications and apps whose
history policy is disabled never enter storage; turning history or the app off
immediately removes its existing records. Per-app policy separately controls
enablement, banners, sounds, badges, urgent Focus bypass, history, and the
trusted lock-screen preview level.

The notification service now loads that Center before claiming either D-Bus
name. Each legacy or portal post resolves the app policy, asks the live Focus
authority to compose the active mode and exact allow list, and only then calls
`Server::post`. The service keeps one reusable session-bus connection for this
hot path. If Focus temporarily disappears, normal banners fail closed while
policy-allowed history remains available in Notification Center. Urgent bypass
still follows the app and active-mode policy. The per-app Sounds choice is part
of authoritative `DeliveryPolicy`; disabling it prevents sound without
silencing the banner or discarding history, while disabling banners alone does
not silently disable an otherwise allowed sound. Focus suppression still
suppresses both unless urgent bypass is allowed.

Posted and closed runtime events update the private Center off the D-Bus
dispatch path. A posted event carries its exact validated snapshot atomically,
so an immediate sender withdrawal cannot race a later mutable-state lookup.
Replacement upserts the full validated notification, expiry
keeps its history, and dismissal, withdrawal, or action closure removes it.
Each changed snapshot is atomically persisted; a save failure is reported
without terminating the live protocol service or freezing the live indicator.

The same service owns `org.rmac.NotificationCenter1`. Its state method and
change signal expose only unread count and urgent presence—never content or app
identity. The signal is emitted after each in-memory Center change even when a
disk save is degraded. Shell clients subscribe before their initial read,
reconnect with bounded delay, and retain last-known-good indicator state while
the authority restarts. A D-Bus activation entry routes the internal name to
the already supervised notification-center process.

The private interface also provides authenticated `Snapshot`, `Applications`,
`Invoke`, `MarkRead`, `Clear`, and `SetPolicy` methods. `Snapshot` atomically
supplies the on-demand Center with at most 500 already-validated records in
newest-group-first order plus their per-app policies. For an exact record that
also remains in the live reducer, it may include a validated visible default
label and visible button labels paired with their original positions. It never
projects action IDs or targets. Persisted-only records and records whose source,
update time, or complete action set no longer match expose no actions. Retained
history IDs are reserved when the reducer starts so a new sender cannot inherit
an old ID.
The client revalidates every ID, app ID, title, body, and priority; diagnostics
expose only counts and classifications. Empty mutation scope means “all”;
non-empty IDs are validated by the notification domain. Application projection
is bounded and combines explicit policies with apps that currently have
history, applying truthful defaults where no override exists. Policy wire
values cover enabled, banners, sounds, badges, history, urgent Focus bypass,
and lock-screen preview; unknown preview values fail closed. Mutations execute
off the D-Bus dispatch thread, atomically save the refreshed Center, emit
indicator/policy signals, and return refreshed state. If saving fails, live
state and signals remain truthful while the caller receives an actionable
persistence error.

`Invoke` accepts only the projected default or bounded button selection plus an
optional bounded opaque activation token, then uses the same `ServiceHandle`
transaction as banners. Unknown/stale records and actions fail visibly. The
Center client revalidates unique button positions and labels before rendering;
its diagnostics redact every label. GPUI 0.2.2 cannot recover the initiating
Wayland seat/serial, so the current panel passes no invented token and leaves
strict focus behavior to the Linux evidence gate.

`rmac-notification-center-panel` is the supervised on-demand E3 presentation.
Its idle process owns only the action-scoped `notification-center` shortcut
socket and reports systemd readiness after that socket is bound. A validated
activation creates at most one trailing translucent panel with date/time,
localized application identity and icons from the live XDG catalog, grouped
cards, unread and urgent state,
Clear/Clear All, per-app Turn Off, authoritative empty/loading/error states,
and a direct route to Notification Settings. Opening an unread snapshot marks
it read through the service, while content remains visible until the user or
policy clears it. Live records also show the service-projected default/button
actions with one bounded busy state and explicit delivery failure; stale
retained records show none. The panel subscribes before its first read and keeps
the last received snapshot across service loss. Escape, focus loss, and a
repeated activation remove only the current window and its bounded watchers;
the idle shortcut endpoint remains alive. The distinct notification daemon
remains the only history/action authority. The panel never reads or writes the
private history file.

## Next adapters and surfaces

1. E1 Linux evidence completion: prove sealed icon/custom-sound descriptors and
   both protocol interfaces against the real portal frontend on the Linux
   reference PC, including malformed, unsealed, oversize, policy-suppressed,
   replacement, flood/backpressure, and daemon-restart cases.
2. E2 layer-surface renderer: render the runtime snapshot with real hover,
   keyboard, action, activation-token, and multi-output evidence on Linux.
3. E3 Notification Center evidence and completion: prove trailing placement,
   outside/Escape dismissal, live and restart-stale action behavior, keyboard
   focus order, scaling, and Orca semantics on Linux. System Settings exposes
   only already-observable allow/block,
   badge, and history controls; banner, sound, Focus-bypass, and lock-preview
   rows remain hidden until their presentation/security adapters are active.
   Its application rows
   use the live XDG catalog for localized names and original theme icons when
   the authenticated application ID exactly matches a desktop-entry ID (with
   only the standard `.desktop` suffix alias). Unresolved IDs keep a generic
   icon and the real identifier; the UI never guesses by display name. The
   Settings client subscribes to both policy changes and Center state changes
   before its initial application read, because first/last history records can
   also change the application set. Each wake rereads a bounded, validated,
   uniquely keyed list; service loss preserves last-known-good rows and
   reconnects after a bounded delay. Stream-health errors remain separate from
   mutation/persistence failures so a reconnect cannot hide an unsuccessful
   user action.
4. E4 Focus UI: expose mode, schedule, duration, and allow-list editing through
   the single-writer authority and prove real delivery behavior on Linux.
