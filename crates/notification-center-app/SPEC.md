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

## Authorities and privacy

- `org.rmac.NotificationCenter1` is the only history/read/policy/clear authority.
- The panel subscribes before its first snapshot, revalidates the bounded wire
  projection, and retains last-known-good content during a transient restart.
- `rmac-apps` supplies exact desktop-entry identity and inherited icons. Only the
  standard `.desktop` suffix alias is accepted.
- The panel never reads or writes the private history file and never logs app
  IDs, titles, bodies, paths, or action labels.
- The service projects only validated visible labels and original button
  positions for an exact live record. Action IDs, targets, and stale persisted
  actions never cross the Center snapshot boundary.
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
- Prove trailing placement, focus restoration, outside dismissal, hotplug,
  live action focus/activation, 100/125/150/200% scaling, keyboard order, and
  idle behavior on niri. GPUI 0.2.2 supplies no initiating Wayland seat/serial,
  so the Linux gate must prove focus without a fabricated activation token.
- Prove names, roles, unread/urgent state, button actions, announcements, and
  200% layout with Orca after the GPUI accessibility gate passes.
- Integrate the action-free lock-preview projection only with the reviewed
  secure lock provider; never reveal more than application and user policy allow.
