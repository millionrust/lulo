# System Settings product contract

## Purpose

System Settings is the macOS-like sidebar-and-detail home for configuring the
rmac session and the Linux host beneath it. It should feel calm, familiar, and
coherent while remaining truthful about the service that owns every setting.
It is not a second settings database, a collection of cosmetic mock controls,
or an excuse to imply that Linux exposes a capability it does not provide.

## Primary journeys

1. Open System Settings and immediately understand the local account, complete
   pane inventory, selected pane, and current detail heading.
2. Search the exact visible category names without moving focus away from
   Search; selecting a result opens that authoritative pane.
3. Open System Settings from Launcher, Quick Settings, Notification Center, or
   another first-party application and land on the exact stable pane identity.
4. Enter About, Software Update, Storage, notification-application, Focus-mode,
   or Focus-schedule detail and use one predictable Back action to unwind it.
5. Change a supported setting through its typed Linux authority, retain the
   last known-good state through service interruption, and see a durable error
   when a refresh or mutation fails.
6. Complete the shell journey with keyboard-only navigation, increased text
   size and contrast, reduced motion, and—after the framework gate—Orca.

## Authorities and boundaries

- The navigation module owns one exact 24-route inventory. It is the shared
  category-name-to-pane-ID authority for the sidebar, launcher routing, and
  accessibility projection; renderer-local aliases are forbidden.
- The controller owns selection, search, account identity, subpage depth, and
  one deterministic priority order for the visible global error. The shell
  projection reads those values and never creates a parallel state store.
- Every pane retains its existing typed authority. NetworkManager, BlueZ,
  PipeWire/WirePlumber, UPower, niri, timedate1, locale1, systemd/XDG, portals,
  and the rmac session services remain responsible for their respective facts
  and mutations. The shell does not flatten them into one generic preference
  document.
- Search uses the same case-insensitive category-name predicate in the renderer
  and semantic projection. A selected pane remains the truthful detail
  authority when its sidebar row is filtered out.
- Subpage titles and the global Settings error are shared controller helpers so
  visible and semantic output cannot silently diverge. Private account names,
  queries, subpage identities, and backend errors are redacted from custom
  diagnostics.
- The public framework-neutral accessibility boundary exposes the named Search
  field, noninteractive local-account identity, visible section/item positions,
  selected state, selected detail heading, Back state and depth, dismissible
  global failure, and polite/assertive announcements. Pane-owned controls append
  their own semantic order rather than being guessed by the shell.
- Shell keyboard order is Back when present, global-error dismissal when
  present, Search, then visible categories in reading order. Initial semantic
  focus is Back on a subpage and Search at the navigation root.
- The projection accepts at most eight sections, 64 categories, eight subpage
  levels, 4 KiB queries and labels, 16 KiB errors, and 256 KiB aggregate text.
  Invalid or duplicate routes, impossible selection/depth state, controls in
  labels, and oversized data fail closed.

## Visual and interaction contract

- Use one quiet window with macOS-like traffic lights, a fixed-width searchable
  sidebar, and a scrolling detail region. Shared live theme, text scaling,
  contrast, motion, focus, and control primitives are mandatory.
- The account row is identity context, not a fake clickable cloud-account
  control. Categories use one restrained selected treatment and preserve stable
  section order under filtering.
- Search filters immediately. Empty results leave Search and the account
  context present and announce the truthful match count.
- The Back control appears only while a typed subpage is active. Escape closes
  transient overlays first; it must not silently discard a reviewed mutation.
- Loading, unavailable, degraded, busy, success, and failure states remain
  visually distinct. An error never fabricates an empty authoritative state.
- The global error toast uses one stable dismissal target. Pane-local errors
  remain with the pane that can explain and recover them.

## Remaining release evidence

- Give every pane-specific child authority its complete semantic adapter rather
  than treating the shell snapshot as coverage of its controls.
- Export the now-defined shell and pane trees through the framework selected by
  A5/A6, then prove roles, names, selected state, actions, focus order, error
  announcements, and 200% layout with Orca.
- Prove launcher/deep-link routing, search, Back behavior, service restart,
  mutation recovery, keyboard-only operation, theme modes, and 100/125/150/200%
  scaling on the Ubuntu/niri reference PC.
- Keep unsupported Linux capabilities explicit and noninteractive until a
  reviewed authority and failure/recovery contract exists.
