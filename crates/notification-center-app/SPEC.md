# Notification Center product contract

## Purpose

Notification Center is the on-demand history surface for the rmac session. It
should feel like the compact trailing macOS panel while remaining a truthful
client of the rmac notification authority. It is not a second notification
store, daemon, banner stack, or lock-screen renderer.

## Primary journeys

1. Open the panel from shell chrome and immediately see the current date/time
   and policy-allowed history grouped newest application first.
2. Recognize each group by its localized desktop-entry name and inherited icon;
   unresolved authenticated IDs remain visibly honest instead of being guessed.
3. Opening the panel acknowledges unread records without deleting their content.
4. Clear one application's history or all history and observe the service-owned
   result. A failed mutation remains visible and retryable.
5. Turn off an application's notifications or open its complete Notifications
   pane in System Settings.
6. Invoke a visible default or button action while its exact notification is
   still live; a stale retained record exposes no control and cannot target a
   newly allocated notification after a service restart.
7. Dismiss with Escape or an outside focus change. Keyboard focus must traverse
   every visible action in reading order.
8. Repeating the Notification Center shortcut closes the current panel; a third
   activation opens a fresh panel without restarting the supervised endpoint or
   notification authority.

## Authorities and privacy

- `org.rmac.NotificationCenter1` is the only history/read/policy/clear authority.
- `rmac-notification-center-panel.service` owns only the action-scoped shortcut
  endpoint and GPUI window lifecycle. It must report ready after binding and
  must remain alive with no hidden window after dismissal.
- The panel subscribes before its first snapshot, revalidates the bounded wire
  projection, and retains last-known-good content during a transient restart.
- `rmac-apps` supplies exact desktop-entry identity and inherited icons. Only the
  standard `.desktop` suffix alias is accepted.
- The panel never reads or writes the private history file and never logs app
  IDs, titles, bodies, paths, or action labels.
- The service projects only validated visible labels and original button
  positions for an exact live record. Action IDs, targets, and stale persisted
  actions never cross the Center snapshot boundary.
- A public renderer-neutral accessibility boundary consumes only that validated
  service snapshot, the panel's exact catalog/fallback application-name
  resolver, rendered clock/date text, and explicit stream/operation/busy/read
  status. It exposes grouped application and record semantics, truthful counts,
  unread/urgent state, empty/loading/degraded states, and only the default or
  button actions still present on the exact live record. Empty notification
  titles receive the generic semantic name “Notification” without inventing
  visible content.
- Enabled keyboard order exactly follows the visual tree: Clear All, each
  group's Turn Off and Clear controls, its record actions, Refresh, then
  Notification Settings. The first enabled action is initial focus. A mutation
  disables all history actions and marks its exact action busy while that action
  remains live; a newer authoritative snapshot can retire it before local
  completion. Refresh and Settings remain available. Loading, empty,
  marking-read, and mutation progress are polite announcements, while service
  and operation failures are assertive.
- The accessibility projection accepts at most 500 records, 1,012 application
  policies, nine actions per record, 4 KiB application/action names, 16 KiB
  individual content/status text, and 16 MiB aggregate semantic text.
  Duplicate application, notification, or action identities and malformed or
  oversized text fail closed. Custom diagnostics redact application names,
  titles, bodies, action labels, and error text. Shared constants keep every
  fixed rendered and semantic label equal.
- Lock-screen previews remain a separate, stricter action-free projection. This
  ordinary unlocked-session surface must never be reused as a cosmetic locker.

## Visual and interaction contract

- A 420-logical-pixel translucent panel sits 12 pixels from the trailing edge
  and below the top-bar region, with an 18-pixel outer radius.
- The header uses a large local clock, secondary full date, and a compact
  “Notification Center” toolbar with Clear All only when records exist.
- Groups use 14-pixel cards, localized app identity, a truthful record count,
  Clear, and Turn Off. Records show an unread accent dot, two-line title,
  three-line plain-text body, separators, an explicit urgent badge, and compact
  default/button actions only while the service reports them live.
- Loading, empty, stream-degraded, mutation-failed, and busy states are visually
  distinct. A service failure never replaces a last-known-good snapshot with a
  fabricated empty state.
- Shared rmac semantic colors, text scaling, focus rings, buttons, reduced
  motion, and live appearance are mandatory.

## Remaining release evidence

- Wire the real top-bar indicator to launch or focus the panel once D1/D2 lands.
- Prove service readiness, first-dispatch delivery, repeated-invocation toggle,
  independent panel/authority restart behavior, and no idle content watchers.
- Prove trailing placement, focus restoration, outside dismissal, hotplug,
  live action focus/activation, 100/125/150/200% scaling, keyboard order, and
  idle behavior on niri. GPUI 0.2.2 supplies no initiating Wayland seat/serial,
  so the Linux gate must prove focus without a fabricated activation token.
- Export the now-defined names, grouped roles, unread/urgent state, button
  actions, focus order, and announcements through the A5/A6 framework boundary,
  then prove them and the 200% layout with Orca.
- Integrate the action-free lock-preview projection only with the reviewed
  secure lock provider; never reveal more than application and user policy allow.
