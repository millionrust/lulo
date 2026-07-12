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
inert text with bounded input and balanced-tag validation. Custom icon and sound
file descriptors are not retained by the reducer. Until the media validator and
player exist, the freedesktop server advertises only `actions`, `body`, and
`persistence`—not markup, sound, hyperlinks, or image capabilities.

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
top bar. E3 will add the crash-safe on-disk store and its retention settings;
only records whose computed delivery permits history may enter that store.
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

`rmac-notifications-runtime` joins this state with the authoritative
notification snapshot, compositor topology/focus, and resolved appearance. It
copies no content. A post uses the notification's computed banner delivery and
timeout duration, then resolves the focused connected output. Temporary loss
of the compositor keeps the last-known-good topology; hotplug moves existing
banners without closing records. Appearance changes can finish active motion
immediately.

Runtime effects collapse to one redraw plus typed focus and service commands.
Paused timeout completion emits `Expire(id)`, which calls the service's
single-banner expiry path instead of scanning other notifications whose
banners may still be paused. User dismissal emits `Dismiss(id)`. Successful
action dispatch closes only the visual banner because the service already owns
the notification transaction. This keeps D-Bus signals, Center history, and UI
animation in one order without polling or content duplication.

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

The private interface also provides authenticated `Applications`, `MarkRead`,
`Clear`, and `SetPolicy` methods. Empty app scope means “all”; non-empty IDs are
validated by the notification domain. Application projection is bounded and
combines explicit policies with apps that currently have history, applying
truthful defaults where no override exists. Policy wire values cover enabled,
banners, sounds, badges, history, urgent Focus bypass, and lock-screen preview;
unknown preview values fail closed. Mutations execute off the D-Bus dispatch
thread, atomically save the refreshed Center, emit indicator/policy signals,
and return refreshed state. If saving fails, live state and signals remain
truthful while the caller receives an actionable persistence error.

## Next adapters and surfaces

1. E1 media/evidence completion: validate icon and custom-sound descriptors and
   prove both interfaces on the Linux reference PC.
2. E2 layer-surface renderer: render the runtime snapshot with real hover,
   keyboard, action, activation-token, and multi-output evidence on Linux.
3. E3 Notification Center UI: render persisted groups and connect its existing
   clear/read authority with keyboard, focus, and Linux evidence. System
   Settings now exposes only already-observable allow/block, badge, and history
   controls; banner, sound, Focus-bypass, and lock-preview rows remain hidden
   until their presentation/security adapters are active. Its application rows
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
