# App Drawer product specification

## Purpose

App Drawer is the complete, searchable view of installed applications. It
should feel as immediate and spatially predictable as a carefully designed app
library while remaining truthful to Linux desktop-entry metadata. It is not a
software store, package manager, or imitation of Apple Launchpad.

## Authorities and boundaries

- `rmac-apps` owns discovery from XDG application directories, desktop-entry
  precedence, localization, visibility, `TryExec`, icon-theme inheritance,
  categories, launch specifications, declared actions, and filesystem watches.
- App Drawer's isolated catalog projection converts that authoritative catalog
  into category/search/render records while retaining the exact parsed launch
  specifications and desktop actions. It also owns the development-only macOS
  category and cached icon fallbacks; neither fallback is compiled on Linux.
- A separate view-render boundary owns grid/list cells, icon fallback
  presentation, category controls, empty/error/busy states, context-menu
  placement, toolbar, and semantic action dispatch. Catalog refresh, selection
  policy, launch/reveal work, and supervised lifetime remain outside it.
- A public framework-neutral accessibility boundary consumes the catalog's
  exact stable desktop identity, localized name/generic name, category, and
  ordered declared actions together with the controller's authoritative search
  and visible index projections. It defines a named grid/list, category result
  counts and selection, item position/selection/actions, Open-first context-menu
  focus, distinct empty states, and polite busy/assertive failure announcements.
  The private query, source path, icon, search metadata, and launch command
  never enter the snapshot.
- That semantic snapshot fails closed above 4,096 applications, 32 declared
  actions per application, or 2 MiB of retained text. It also rejects invalid
  or duplicate stable identities/actions, inconsistent search/filter indices,
  impossible selection/menu state, control-bearing labels, and oversized text.
  Shared labels keep the rendered and semantic Open, Show in Folder, busy, and
  empty-state text identical.
- Linux parsing follows the freedesktop Desktop Entry Specification 1.5.
  Desktop actions come only from identifiers named by `Actions=` and matching
  `[Desktop Action <id>]` groups. Labels use the active message locale.
- Launches use parsed program/argument boundaries. No desktop-entry value is
  passed through a shell. Declared action order is preserved and bounded.
- “Show in Folder” uses the file-manager portal through `rmac-apps::reveal`.
- `rmac-ui` owns live appearance, shared controls, text scaling, and window
  chrome. App Drawer does not maintain a private theme.
- `rmac-app-drawer.service` owns only the action-scoped `app-drawer` shortcut
  endpoint and one on-demand window. Its explicit `--service` mode reports ready
  only after binding; standalone launches remain available for development and
  performance measurement without competing for the shortcut.
- That supervised authority is implemented in an isolated service module which
  owns shortcut watching/readiness, key registration, active-window tokens,
  repeat-invocation dismissal, window creation, and release cleanup. The view
  can request token release but cannot mutate service-global state directly.
- The 39-line binary entrypoint owns only module composition, run-mode
  selection, semantic action registration, and standalone boot. A separate
  controller owns catalog subscriptions, stable selection, launch/reveal
  intents, and filter state, with the renderer nested beneath it.

## Primary journeys

1. Open App Drawer and see a stable grid of installed, visible applications
   with their resolved icons and localized names.
2. Type a localized name, generic name, keyword, category, or desktop-action
   label in Search and receive an immediate filtered result without moving
   focus away from the text field.
3. Filter by a category that actually has visible matches; switch between grid
   and list without changing the selected application.
4. Navigate with arrow keys and press Return to open the selected application.
5. Right-click an application and choose Open, any localized desktop action
   such as New Window, or Show in Folder.
6. Install, remove, or edit a desktop entry and see a coalesced catalog refresh
   that preserves selection by source path where possible.
7. Invoke the global shortcut to open one App Drawer, invoke it again to close
   that window, and invoke it a third time to open a fresh view without
   restarting the supervised endpoint.

## Interaction contract

- Arrow keys move the visible selection. Grid vertical movement uses the
  current viewport column count; list vertical movement advances one row.
- Return opens the selected application. Escape clears Search and restores
  focus to the drawer.
- Pointer activation first selects the exact tile/row it opens. Right-click
  selects that item before building its menu.
- Context-menu actions dispatch an immutable parsed `LaunchSpec`; a catalog
  change closes the menu so a stale positional action cannot target another
  application.
- Only one application spawn is in flight. The drawer displays “Opening
  application…” while the spawn is pending and keeps a failed launch visible
  until dismissal or retry.
- Catalog, launch, and reveal failures expose a dedicated shared focusable
  Dismiss button; the notice banner itself is not a pointer-only target.
- In the supported niri session, Open and declared actions use direct niri IPC
  spawn so the child receives an XDG activation token. A compositor rejection
  is never bypassed by direct spawning.
- Context menus close on activation, Escape, or outside press through the
  shared `rmac-ui` menu contract.

## Desktop-action contract

- Action identifiers contain only `A-Z`, `a-z`, `0-9`, and `-`, are at most
  255 bytes, and are unique within one projected application.
- At most 32 actions are exposed, in the author-declared order.
- An action requires a localized, non-empty name no larger than 512 bytes and
  an `Exec` value no larger than 32 KiB. Missing/malformed groups and unknown
  field codes are ignored rather than made clickable.
- `%c` expands to the localized application name; `%i` uses the action icon;
  file/URL codes are removed when no file or URL was supplied, as required for
  launching from an application list.
- The main entry's terminal and working-directory context applies to its
  additional actions. D-Bus-only activation is not advertised until the
  activation-token and application-bus adapter exists.

## Visual and state contract

- Loading must not replace navigation or cause layout jumps.
- An empty catalog and a query with no matches use distinct centered states;
  neither removes Search, filters, or the view control.
- A catalog read failure retains the last known-good applications. A watcher
  failure explains that live updates are unavailable without hiding the loaded
  catalog.
- A launch or reveal failure is user-visible and does not clear selection.
- Grid/list, category, query, and selection states use shared semantic tokens
  in light, dark, increased-contrast, and scaled-text modes.

## Release gates and remaining work

- Add stable persisted view preference if usability evidence supports it.
- Prove service readiness, first-dispatch delivery, repeated-invocation toggle,
  restart recovery, and zero catalog watcher activity while no window exists.
- Prove keyboard focus order, context-action activation, live cache
  invalidation, cold-cache performance, icon fallback, and install/removal on
  Ubuntu 26.04 with niri.
- Prove strict-focus activation for native Wayland, XWayland, terminal,
  working-directory, and declared-action launches from real hardware.
- Export the now-defined grid/list, category, item, menu, empty-state, and live-
  region semantics through the framework selected by A5/A6, then prove roles,
  names, selected state, menu actions, announcements, and 200% scaling with
  Orca.
- Native drag-out remains framework-gated; Show in Folder is the honest bridge.
