# macOS UI reference policy

This document records the external implementations and primary platform
guidance used to review rmac's shell. It is a design and behavior reference,
not permission to copy Apple assets or incompatible source code.

## Primary behavior reference

- Apple Human Interface Guidelines, Materials:
  <https://developer.apple.com/design/human-interface-guidelines/materials>
- Apple Desktop & Dock behavior:
  <https://support.apple.com/guide/mac-help/change-desktop-dock-settings-mchlp1119/mac>

The current Apple guidance makes material adaptive rather than merely
transparent. Foreground content must remain legible over different wallpapers,
with reduced-transparency and reduced-motion behavior treated as first-class
states. Dock size, optional magnification, placement, autohide, launch
animation, running indicators, recent apps, window behavior, and per-display
Spaces are user policy rather than hard-coded decoration.

The Dock is not one flat application list. Its separators describe real
groups: kept applications lead; recent or running applications not already
kept occupy the next application group; files, folders, minimized windows, and
Trash follow the application boundary. Apple currently documents up to three
recent applications, a default Downloads folder, drag reordering, folder
stacks, and separate controls for recent apps, running indicators, minimize
behavior, launch animation, size, placement, magnification, and autohide.

## Open-source implementation references

### macos-web

- Repository: <https://github.com/PuruVJ/macos-web>
- License: MIT
- Useful reference: stable icon order, pointer-distance magnification, a
  bottom-centered shelf, active dots, notification badges, tooltips, and a
  translucent double-edge/shadow treatment.

Its Dock implementation uses a compact shelf around roughly 58-pixel resting
icons, expands neighboring icons from stable centers, and separates the shelf
background, blur, border, and shadow. rmac uses the behavior and proportions as
a comparison point while retaining its own Rust model, renderer, motion curve,
artwork, and accessibility tree.

### Noctalia

- Repository: <https://github.com/noctalia-dev/noctalia>
- License: MIT
- Useful reference: one cohesive native Wayland shell owning bars, Dock,
  launcher, control center, notifications, wallpaper, lock screen, OSDs,
  multi-monitor surfaces, live configuration, and compositor adapters.

rmac follows the same whole-shell product principle. It does not assemble a
permanent desktop from unrelated panels and scripts.

### Starling

- Repository: <https://github.com/starling-build/starling>
- License: Apache-2.0
- Useful reference: a complete Linux desktop package, explicit third-party
  Wayland/X11 application compatibility, shell-owned surfaces, portals,
  notifications, and a renderer-level glass experiment.

### macOS Tahoe Liquid KDE

- Repository:
  <https://github.com/lestercorderomurillo/macos-tahoe-liquid-kde>
- License: GPL-3.0; its README separately excludes bundled Apple wallpapers,
  sounds, and fonts from that license.
- Useful reference: visual comparison and completeness checklist only.

No source code or bundled asset from this project may enter rmac's MIT tree
without a deliberate license review. Apple-owned fonts, wallpapers, sounds,
logos, and application icons must not be imported.

### elementary Dock

- Repository: <https://github.com/elementary/dock>
- License: GPL-3.0
- Useful reference: Linux application launch/window-switch behavior only.

## rmac shell target

The visual target is the current compact macOS desktop hierarchy, expressed
with original rmac identity:

- a 29-logical-pixel adaptive menu bar on every selected output (measured
  2026-09-18 on macOS 27.0; fully transparent over the wallpaper);
- a bottom-centered glass shelf with 64-logical-pixel resting icons, 12-pixel
  gaps, 4-pixel shelf padding (72-pixel shelf, 18-pixel bottom margin),
  approximately 29-pixel corner radius (`tile × 0.46`), a subtle light inner
  edge, dark outer edge, and wallpaper-aware shadow;
- real desktop-entry artwork for installed applications and original embedded
  rmac artwork for first-party or fallback entries;
- stable icon order during focus changes, semantic pinned/running/place
  separators, running dots, urgent badges, and tooltip labels;
- the reference profile keeps magnification off, matching the reference Mac;
  Settings may enable stable-center magnification explicitly, while reduced
  motion always disables it;
- active-app identity and exported app menus on the leading side; compact live
  status menus, Spotlight, Control Center, privacy state, and date/time on the
  trailing side, with no centered clock;
- external Linux apps that do not export a global command model keep their
  commands inside the application window; rmac must not draw fake menu labels;
- no compositor debug borders, duplicate third-party bar, decorative fake
  controls, Apple logo, SF font redistribution, or Apple application artwork;
- one installable rmac login session that owns the complete shell, with the
  original GNOME session preserved as recovery.

The reference-PC gate compares screenshots and interaction recordings against
these rules at scale 1 and 2. A visual resemblance is not enough: launch,
focus, keyboard navigation, screen-reader semantics, hotplug, recovery, idle
cost, and installer rollback must also pass.
