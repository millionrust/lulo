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
- Linux parsing follows the freedesktop Desktop Entry Specification 1.5.
  Desktop actions come only from identifiers named by `Actions=` and matching
  `[Desktop Action <id>]` groups. Labels use the active message locale.
- Launches use parsed program/argument boundaries. No desktop-entry value is
  passed through a shell. Declared action order is preserved and bounded.
- “Show in Folder” uses the file-manager portal through `rmac-apps::reveal`.
- `rmac-ui` owns live appearance, shared controls, text scaling, and window
  chrome. App Drawer does not maintain a private theme.

## Primary journeys

1. Open App Drawer and see a stable grid of installed, visible applications
   with their resolved icons and localized names.
2. Type in Search and receive an immediate filtered result without moving
   focus away from the text field.
3. Filter by a category that actually has visible matches; switch between grid
   and list without changing the selected application.
4. Navigate with arrow keys and press Return to open the selected application.
5. Right-click an application and choose Open, any localized desktop action
   such as New Window, or Show in Folder.
6. Install, remove, or edit a desktop entry and see a coalesced catalog refresh
   that preserves selection by source path where possible.

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
- Empty search results occupy the content area and must not look like an empty
  catalog. This explicit state is still pending.
- A catalog read failure retains the last known-good applications. A watcher
  failure explains that live updates are unavailable without hiding the loaded
  catalog.
- A launch or reveal failure is user-visible and does not clear selection.
- Grid/list, category, query, and selection states use shared semantic tokens
  in light, dark, increased-contrast, and scaled-text modes.

## Release gates and remaining work

- Add compositor activation-token propagation for startup focus correctness.
- Add an explicit no-results state, localized search over keywords/actions,
  and stable persisted view preference if usability evidence supports it.
- Prove keyboard focus order, context-action activation, live cache
  invalidation, cold-cache performance, icon fallback, and install/removal on
  Ubuntu 26.04 with niri.
- Prove roles, names, selected state, menu actions, announcements, and 200%
  scaling with Orca after the GPUI accessibility gate passes.
- Native drag-out remains framework-gated; Show in Folder is the honest bridge.
