# Product reference matrix

Every rmac surface follows the same fast implementation loop:

1. Use the current Apple guide as the behavior and hierarchy reference.
2. Inspect maintained open-source replicas for geometry, motion, and edge cases.
3. Inspect a strong Linux implementation for platform integration.
4. Implement with original rmac artwork and Linux-native authorities.
5. Compare on the Ubuntu reference PC against a current Mac screenshot and
   interaction recording.

Code and assets enter the MIT tree only when their license is compatible and
their provenance is recorded. GPL projects are behavior/visual references
only. Apple fonts, icons, logos, wallpapers, and sounds are never copied.

## Whole desktop and windows

- Apple: desktop, window controls, tiling, Mission Control, Spaces, menu bar,
  Dock, Control Center, Notification Center, privacy indicators, Spotlight,
  appearance, accessibility, reduced transparency, and reduced motion.
- Open source: `PuruVJ/macos-web` and `Renovamen/playground-macos` (MIT) for
  interaction/proportion comparisons; Noctalia (MIT) for a cohesive native
  Wayland shell; Starling (Apache-2.0) for whole-desktop packaging and client
  compatibility; WhiteSur GTK (MIT) for application-theme comparison.
- rmac target: a single package-owned Niri session with floating, borderless,
  resizable windows; original traffic lights; adaptive light/dark chrome; one
  menu bar and Dock per selected output; GNOME retained as recovery.

## Menu bar, Dock, and desktop services

- Apple: app name and menus lead; status/Spotlight/Control Center/privacy/time
  trail; date/time opens Notification Center. Dock owns kept applications,
  recent and unpinned-running applications, semantic separators, indicators,
  optional magnification, autohide, Downloads and other folder stacks,
  minimized windows, and Trash. Desktop & Dock separately controls size,
  position, launch animation, recent apps, running indicators, minimized-window
  placement/effect, autohide, and multi-display Spaces behavior.
- Open source: macos-web and playground-macos for layout/motion; Noctalia for
  hotplug, layer-shell, status services, lock, OSD, and live configuration.
- rmac target: typed live Linux state, original status icons, no placeholder
  controls, stable Dock order, real activation and places, accessibility, and
  event-driven rendering.

## Spotlight and App Drawer

- Apple: immediate Command-Space focus, apps/files/settings/actions/calculation,
  ranked results, keyboard operation, and explicit reveal/open behavior.
- Open source: playground-macos Spotlight (MIT), macos-web app catalog (MIT),
  and mature Linux launcher provider behavior as integration reference.
- rmac target: bounded private indexing, live desktop-entry catalog, settings
  deep links, calculator provider, recent documents, exact keyboard semantics,
  and no decorative search results.

## Finder

- Apple: sidebar, toolbar, tabs, icon/list/column/gallery views, tags, preview
  pane, status/path bars, Get Info, Quick Look, contextual actions, Trash, and
  transactional file operations.
- Open source: macOS web Finder clones with compatible licenses for visual
  comparison; Files/Nautilus for Linux mounts, portals, and filesystem edge
  cases without importing GPL source.
- rmac target: real asynchronous Linux file operations, exact recovery and
  conflict flows, removable media, thumbnails, Quick Look, drag/drop, tags,
  search, accessibility, and familiar three-pane hierarchy.

## System Settings and Control Center

- Apple: searchable sidebar/detail hierarchy, reversible display changes,
  appearance/accessibility policy, Desktop & Dock, network, Bluetooth, sound,
  battery, privacy, users, updates, storage, and About.
- Open source: macos-web Action Center (MIT) and Noctalia control center (MIT)
  for compact control layout; GNOME Settings for authoritative Linux service
  integration without importing GPL UI source.
- rmac target: every visible row backed by a real Linux authority, compact
  controls in Quick Settings, complete controls in Settings, no fake toggles,
  and Keep/Revert for risky changes.

## Notifications, Focus, lock, and OSD

- Apple: grouped history, actionable banners, Focus policy, widgets, privacy
  indicators, secure lock, media/brightness/volume feedback, and predictable
  keyboard dismissal.
- Open source: Noctalia for Wayland lifecycle and macos-web for visual grouping.
- rmac target: one notification authority and portal backend, bounded history,
  truthful actions, secure lock ownership, Focus enforcement, original OSDs,
  multi-output behavior, and screen-reader announcements.

## Notes

- Apple: folders/accounts sidebar, note list or gallery, editor, date grouping,
  pinning, tags, smart folders, search, attachments browser, tables/lists,
  locking, import/export/print, and recovery.
- Open source: maintained note applications are workflow references only; the
  Apple Notes guide remains the hierarchy source.
- rmac target: local-first transactional storage, three-column layout,
  autosave/recovery, attachments, tags/folders, search, export/print, and no
  implied cloud collaboration until a real provider exists.

## Text Editor

- Apple TextEdit: plain/rich text and HTML, document-centric open/save,
  formatting, wrapping, find/replace, spelling, lists/tables, print, and format
  conversion.
- Linux reference: portal-driven document access and established editor input,
  IME, clipboard, and recovery behavior.
- rmac target: complete plain-text daily use first, truthful rich-text support,
  atomic recovery, encodings/line endings, find/replace, print, accessibility,
  and recent-document integration.

## Terminal

- Apple Terminal: profiles, tabs/windows, titles derived from working directory
  and process, activity state, scrollback, selection/search, and settings.
- Open source: Alacritty (Apache-2.0) for terminal correctness/performance and
  compatible terminal libraries already used by rmac.
- rmac target: correct PTY lifecycle/resize, tabs, profiles, titles, search,
  selection, clipboard, links, shell integration, Unicode/IME, accessibility,
  and clean shutdown.

## Activity Monitor

- Apple: sortable/searchable process table; CPU, memory, energy, disk, network,
  and GPU views; bottom summary graphs; process information; safe quit/force
  quit; adjustable refresh interval.
- Linux reference: `/proc`, cgroups, UPower, and kernel counters are authority;
  mature monitors provide edge-case comparison without copied UI code.
- rmac target: truthful Linux metrics and terminology where semantics differ,
  macOS-familiar table/summary hierarchy, safe process actions, bounded refresh,
  and no invented energy/GPU figures.

## App Drawer and software management

- Apple: Launchpad/Apps presents an icon grid with search and folders; App
  Store is a separate browsing/install/update experience.
- Open source: macos-web App Store and Calendar/Calculator components (MIT) are
  visual references; Linux desktop entries and package systems are authority.
- rmac target: finish the app grid first. A future software center must use a
  real backend and clearly distinguish distribution, Flatpak, Snap, and update
  sources rather than imitate unavailable Apple services.

## Acceptance rule

A component is not complete because it resembles a screenshot. It must also
match expected keyboard/pointer behavior, expose a coherent accessibility tree,
survive scale 1/2 and hotplug, retain state safely, idle quietly, recover from
service failure, and remain removable through the signed installer lifecycle.
