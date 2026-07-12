# Notification authority

`rmac-notifications` is the framework-neutral authority for notification
validation, replacement, delivery policy, lifecycle, actions, and bounded
history. It deliberately has no GPUI, D-Bus, portal, filesystem, clock, or
sound dependency. E2 banners, E3 Notification Center, E4 Focus, shell status,
and both Linux protocol adapters must consume this same reducer.

The contract follows XDG Notification portal version 2 and the freedesktop.org
Desktop Notifications Specification 1.3. A future portal adapter maps its
application-scoped string ID to `Source::Portal`; reusing that ID updates the
existing record without flicker unless `show-as-new` was requested. A future
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

`hide-on-lockscreen` and `hide-content-on-lockscreen` normalize to a typed lock
visibility policy. No lock UI may weaken that policy. The secure default is to
hide a notification on the lock screen when no explicit trusted policy has
been established.

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

## Next adapters and surfaces

1. E1 Linux service: own `org.freedesktop.Notifications`, implement capabilities,
   `Notify`, `CloseNotification`, signals, and the XDG portal backend mapping.
2. E2 banner runtime: subscribe to reducer outcomes, pause visual expiry while
   hovered or keyboard-focused, stack deterministically, and request frames
   only while motion is active.
3. E3 Notification Center: persist permitted history atomically, group by app,
   expose clear/read actions, and publish `Indicator` to shell status.
4. E4 Focus: calculate schedule and allow-list policy, then pass the resulting
   `DeliveryPolicy` into `Server::post`; it must not duplicate notification
   state.
