# macOS Tahoe parity specification

> Research baseline: macOS Tahoe 26.6, reviewed 2026-08-11
>
> Product baseline: the owner's current Mac and supplied screenshots
>
> Purpose: define what rmac must reproduce, what remains optional, and what it
> must never fake

This is the master macOS research and parity contract for rmac. It describes
the complete product experience rather than a theme. It covers the visible
shell, interaction model, system behavior, built-in applications, settings,
accessibility, security, lifecycle, performance, and Linux substitutions.

Apple documents macOS Tahoe as a system with a new Liquid Glass design,
expanded Spotlight actions, a customizable Control Center, rounder windows,
and updated first-party apps. The current stable security release at the time
of this audit is 26.6. The primary sources are Apple's
[Mac User Guide](https://support.apple.com/guide/mac-help/welcome-mh43558/mac),
[Tahoe feature inventory](https://www.apple.com/mideast/os/pdf/All_New_Features_macOS_Tahoe_Sept_2025.pdf),
[Human Interface Guidelines](https://developer.apple.com/design/human-interface-guidelines/),
and [Tahoe 26.6 security release](https://support.apple.com/en-us/128067).

No finite document can expose Apple's private source code, undocumented
algorithms, or every hardware-, language-, region-, account-, and app-specific
branch. “Complete” therefore means every public product domain is inventoried,
every shipped rmac surface has a reference and acceptance test, and omissions
are explicit instead of accidental. This document is versioned research and
must be refreshed when the reference Mac or stable macOS changes.

## 1. Non-negotiable product rule

Only implement behavior that exists in current macOS or is required to make
that behavior truthful on Linux.

- The owner's Mac configuration defines rmac's shipped default.
- Current macOS defines the available behavior and optional settings.
- An optional macOS feature stays off by default unless it is enabled on the
  reference Mac. Downloads in the Dock, Dock magnification, recent apps,
  Stage Manager, desktop widgets, and automatic hiding are examples.
- Linux plumbing may differ internally, but it must not add Linux-looking
  chrome, duplicate panels, invented widgets, or fake status.
- A Linux-only recovery or compatibility control belongs in a clearly named
  advanced/support area, not in the everyday macOS-shaped experience.
- Apple-only services are absent or honestly replaced. They are never shown as
  dead iCloud, AirDrop, FaceTime, Apple Pay, AppleCare, Find My, or Apple
  Intelligence controls.
- rmac uses original names, icons, sounds, fonts, wallpapers, and artwork where
  Apple owns the asset or mark. It may reproduce hierarchy and behavior, not
  redistribute Apple's intellectual property or imply Apple affiliation.

### Reference classes

Every planned control or feature receives one class before implementation.

| Class | Meaning | Shipping rule |
|---|---|---|
| `M0` | Observed on the owner's Mac | Match it in the default profile |
| `M1` | Standard current macOS behavior | Implement as core behavior |
| `M2` | Genuine configurable macOS option | Implement in Settings; default to the reference Mac's choice |
| `LE` | Linux equivalent required for a macOS outcome | Use a real Linux authority and macOS-shaped presentation |
| `AO` | Apple ecosystem or proprietary service | Hide until a truthful, separately named integration exists |
| `NA` | Hardware or platform capability unavailable on the machine | Hide or show a clear unavailable state only when useful |

No item may be implemented because it merely “looks Mac-like.” It needs an
Apple reference, an observed reference-Mac behavior, or a documented `LE`
reason.

## 2. Research and measurement protocol

Apple documentation is the behavior authority. The reference Mac is the
visual and default-settings authority. Open-source replicas are secondary
implementation references only and may not override either authority.

For each surface:

1. Record macOS version, display scale, resolution, appearance, accent,
   contrast, transparency, motion, and the relevant enabled settings.
2. Capture idle, hover, pressed, focused, selected, disabled, busy, error,
   empty, menu-open, and keyboard-focus states where they exist.
3. Measure bounds, padding, gaps, radii, shadow spread, separators, alignment,
   typography baselines, and animation duration from the captures. Do not
   infer geometry from memory.
4. Record the complete pointer, keyboard, gesture, drag-and-drop, focus,
   click-away, Escape, Return, modifier-key, and context-menu behavior.
5. Record multi-window, multi-space, multi-display, scale, light/dark, reduced
   transparency, reduced motion, and high-contrast variants.
6. Identify the Linux state authority, mutation authority, persistence format,
   authorization boundary, failure state, and recovery behavior.
7. Implement shared tokens before individual approximations.
8. Compare rmac and the reference Mac side by side at identical scale.
9. Accept only when the full journey works; a screenshot alone is not proof.

When Apple documentation and the owner's observed Mac differ, first check the
OS version and setting. Preserve the current behavior as an `M2` option and use
the owner's choice as `M0`.

## 3. The macOS visual system

### 3.1 Hierarchy, not universal blur

Tahoe's visual system has distinct layers. Apple's
[Materials guidance](https://developer.apple.com/design/human-interface-guidelines/materials)
places Liquid Glass in a functional layer for controls and navigation, while
ordinary content remains in the content layer. It explicitly warns against
putting glass everywhere.

rmac must preserve these layers:

1. **Wallpaper and desktop content.** The furthest background.
2. **Application content.** Documents, lists, media, editors, tables, and other
   reading/work surfaces. These are not globally translucent.
3. **Structural material.** Sidebars, inspectors, content separators, titlebar
   blending, and standard materials that organize a window.
4. **Functional glass.** Menu bar controls, Dock shelf, floating navigation,
   selected controls, tool groups, and transient navigation surfaces.
5. **Transient overlays.** Menus, popovers, sheets, alerts, Spotlight,
   Notification Center, Control Center, tooltips, and OSDs.

Global compositor blur over applications is forbidden. Blur is clipped to the
surface and region that owns it. Regular glass must prioritize legibility;
clear glass is reserved for sparse controls over visually rich content. Every
material has an opaque or simplified reduced-transparency fallback.

### 3.2 Tahoe appearance inventory

- Completely transparent menu bar with legible adaptive foreground.
- Rounder window corners whose child content and shadows share the same curve.
- Layered icons supporting light, dark, tinted, and clear appearances.
- Adaptive controls that respond to hover, press, selection, focus, disabled
  state, background luminance, contrast, and accessibility preferences.
- Floating sidebars and tool groups above edge-to-edge content where the app
  architecture calls for them.
- Expanded vertical edit/context menus only where macOS uses them.
- Folder color, tint, tag, emoji, and symbol personalization as optional
  Finder features.
- Lock Screen typeface choices as an optional personalization setting.

The [Tahoe feature inventory](https://www.apple.com/mideast/os/pdf/All_New_Features_macOS_Tahoe_Sept_2025.pdf)
is the source for these system-wide changes.

### 3.3 Shared component requirements

All rmac surfaces use one semantic token set for:

- window, sheet, alert, menu, popover, sidebar, toolbar, shelf, HUD, and OSD
  materials;
- outer and inner corner radii, including concentric nested curves;
- active/inactive window state and key/non-key control state;
- label, secondary label, tertiary label, separator, selection, keyboard focus,
  destructive, warning, success, and disabled colors;
- standard row heights, control sizes, content insets, sidebar widths, toolbar
  heights, menu padding, and separator thickness;
- elevation, ambient shadow, contact shadow, inner highlight, and border;
- spring, ease, duration, reduce-motion, and immediate-transition policies;
- text roles rather than copied Apple font files;
- pointer shapes, hover affordances, focus rings, and accessible hit targets.

Exact numerical tokens come from controlled reference captures. They are not
declared “macOS exact” merely because a round number looks plausible.

### 3.4 Application-window grammar

Apple's
[macOS design guidance](https://developer.apple.com/design/human-interface-guidelines/designing-for-macos/)
expects resizable, movable, multi-window applications; high-precision pointer
input; keyboard workflows; menu-bar commands; large-display information
density; full screen; and user-customizable windows and toolbars.

Every rmac app must therefore support, when applicable:

- red/yellow/green traffic-light placement and correct close, minimize, zoom,
  fill, full-screen, and modifier behavior;
- key-window versus inactive-window styling;
- drag by valid titlebar regions without blocking toolbar controls;
- resize from all supported edges/corners with minimum useful size;
- restoration of safe size, position, selected sidebar item, and open document;
- tabs for document-based apps where macOS offers them;
- sheets attached to their parent, alerts for app-modal decisions, and no
  arbitrary centered modal boxes;
- toolbar customization where the app has a macOS toolbar;
- sidebar collapse/expand and correct narrow-window adaptation;
- app-owned settings from the application menu;
- standard File, Edit, View, Window, and Help command placement where relevant;
- menus and shortcuts reflecting current selection and enabled state;
- close-window versus quit-app distinction;
- document autosave, modified state, close confirmation policy, reopen policy,
  and crash recovery where relevant.

Clicking outside a normal app window does **not** quit or close it on macOS. It
only changes focus. Clicking outside a transient menu, popover, Spotlight,
Control Center, Notification Center, or similar overlay dismisses that
transient surface. This distinction is mandatory.

## 4. Desktop shell parity

### 4.1 Login, unlock, lock, and session lifecycle

The login window and the in-session Lock Screen are related but not duplicate
authentication layers. A user must never enter the same password twice because
rmac stacked a cosmetic lock over the real lock provider.

Required behavior:

- one GDM/session selection starts one supervised rmac desktop;
- boot/login and in-session unlock have one authentication authority each;
- password characters remain concealed while typing is visibly acknowledged;
- keyboard layout, accessibility, power, restart, and shutdown affordances are
  available only according to policy;
- wrong-password feedback is clear without revealing private information;
- successful authentication transitions once to the ready desktop;
- suspend/resume returns to the same secure provider without flashing another
  lock UI;
- user switching and multiple accounts remain coherent;
- screen saver, display-off delay, password-delay, hint, lock message, and
  power-button visibility are settings, not hard-coded decoration;
- Ubuntu/GNOME remains a recovery session, but its controls do not leak into
  the rmac shell.

Apple documents immediate lock, password/Touch ID/Watch unlock, other-user
selection, hints, lock messages, display timeouts, and optional power controls
in [Lock Screen behavior](https://support.apple.com/en-euro/guide/mac-help/mchl8e8b6a34/mac)
and [Lock Screen settings](https://support.apple.com/en-lb/guide/mac-help/mh11784/mac).
On Linux, PAM/logind/GDM and the audited rmac lock provider remain the security
authorities. Cosmetic rendering must never replace or weaken them.

### 4.2 Desktop and wallpaper

- Per-display wallpaper with Fill, Fit, Stretch, Center, and Tile only when the
  matching setting exists.
- Light, dark, automatic, dynamic, and downloaded/local-image states where
  supported.
- Wallpaper must not blur ordinary applications.
- Optional desktop icons, stacks, widgets, and wallpaper-click “Show Desktop”
  follow Desktop & Dock settings.
- Wallpaper click changes focus or reveals the desktop according to the chosen
  setting; it does not close normal applications.
- Desktop files use Finder semantics, selection, context menus, drag/drop,
  rename, Quick Look, tags, Trash, and mounted-volume behavior.
- Hot Corners are optional user assignments, not default hidden actions.

### 4.3 Menu bar

Apple's [menu-bar guide](https://support.apple.com/guide/mac-help/whats-in-the-menu-bar-mchlp1446/mac)
defines the left-to-right hierarchy: system menu, active application name and
menus, status menus, Spotlight, Control Center, privacy indicators, then date
and time opening Notification Center.

rmac requirements:

- the bar spans the top edge and is visually transparent/adaptive in Tahoe;
- the leading system symbol opens the complete system menu;
- the active app name is bold, followed by that app's real menus;
- third-party apps that do not export a global menu retain menus in-window;
  rmac must not fabricate File/Edit/View labels;
- status items expose real state and menus, can be ordered/configured where the
  matching macOS capability exists, and have keyboard access;
- Spotlight and Control Center have their standard positions;
- privacy indicators represent microphone, camera, system audio capture, and
  location, with precedence and app attribution;
- date/time opens Notification Center;
- full-screen and auto-hide behavior follow settings;
- menus dismiss on selection, Escape, clicking elsewhere, app deactivation,
  or owning-surface loss according to macOS behavior;
- menu commands show shortcuts, checkmarks, mixed state, submenus, separators,
  ellipses for commands requiring more input, and disabled state correctly.

### 4.4 System menu

The system menu contains only working equivalents of the items Apple lists in
its [system-menu guide](https://support.apple.com/guide/mac-help/whats-in-the-apple-menu-mchlp1130/mac):

1. About This rmac/Linux PC.
2. System Settings.
3. Software Center, not a falsely branded App Store.
4. Recent Items.
5. Force Quit.
6. Sleep.
7. Restart.
8. Shut Down.
9. Lock Screen.
10. Log Out the current user.

The grouping, separators, shortcuts, confirmation rules, disabled state, and
submenu behavior follow the reference Mac. Destructive and session actions use
real logind/systemd authorities and preserve unsaved-work warnings.

### 4.5 Dock

The Dock is a persistent application and item shelf, not an app drawer. Apple's
[Dock guide](https://support.apple.com/guide/mac-help/open-apps-from-the-dock-mh35859/mac)
documents launch, activation, modifier clicks, drag-to-open, reveal in Finder,
context menus, Force Quit, keeping/removing/reordering items, folder stacks,
recent apps, Handoff, badges, and keyboard navigation. Desktop & Dock settings
control size, magnification, edge, hiding, indicators, launch animation,
minimize policy, titlebar double-click behavior, and recent apps.

Required grouping and behavior:

- kept/pinned applications form the first group;
- recent or unpinned-running apps form an optional distinct group;
- files, folders, minimized windows, and Trash form the item group;
- separator lines are semantic and appear only when their groups exist;
- the owner's default order and enabled groups are reproduced exactly;
- Downloads is a genuine optional folder stack but is not forced into the
  owner's default Dock;
- app icons never reorder when focus changes;
- one click launches, focuses, or reveals the correct app/window according to
  state; repeated clicks follow the selected policy;
- running indicators, progress, badges, urgent state, and unavailable-state
  question marks are truthful;
- hover labels appear with measured delay and placement;
- magnification, if enabled, grows the hovered icon and neighboring icons from
  stable centers without swapping, jumping, or changing semantic order;
- right-click/Control-click menus reflect app state: Open, windows, recents,
  Options/Keep in Dock, Show in Finder, Hide, Quit, and Force Quit when valid;
- drag reorders within legal groups, adds apps/items, removes aliases without
  deleting originals, opens files on accepting apps, and targets Trash;
- keyboard navigation reaches every item and menu;
- auto-hide, screen-edge reveal, full-screen policy, multiple displays, scale,
  and pointer barriers are verified on hardware;
- Trash reflects empty/full state and supports Open and Empty Trash with safe
  confirmation.

### 4.6 Windows, tiling, full screen, and Split View

- Windows can overlap freely; rmac is not visibly a tiling desktop by default.
- Focus changes active-window appearance without closing or moving windows.
- Standard traffic lights expose close/minimize/zoom/full-screen semantics.
- Windows can be moved, resized, filled, tiled to halves/quarters, maximized,
  minimized, restored, and sent to other workspaces/displays.
- Edge dragging, menu-bar dragging, Option-assisted tiling, and tiled-window
  margins follow Desktop & Dock options.
- Full screen creates the expected distraction-free space and menu-bar/Dock
  reveal behavior.
- Split View pairs compatible windows and exits without losing either window.
- Window shadows, corner clipping, content underlays, titlebars, and hit regions
  stay correct at 100%, fractional scales, and 200%.
- Dialogs, utility panels, popovers, sheets, picture-in-picture, and always-on-
  top media controls use their proper window types rather than normal windows.

### 4.7 Mission Control, Spaces, Stage Manager, and Show Desktop

Mission Control presents every open window in one navigable overview with the
Spaces bar above it. The official
[Mission Control guide](https://support.apple.com/en-gb/guide/mac-help/mh35798/mac)
documents trackpad, keyboard, mouse, and window-drag entry. Spaces support up
to 16 desktops and keyboard/gesture switching in Apple's
[Spaces guide](https://support.apple.com/en-mide/guide/mac-help/mh14112/mac).

Required capabilities:

- overview with spatially stable live window representations;
- app-window view, desktop reveal, and accessible keyboard selection;
- create, remove, reorder, name/identify, and switch Spaces;
- move windows between Spaces and displays;
- full-screen and Split View spaces;
- per-app Space assignment and “switch to a Space with app windows” policy;
- optional automatic Space reordering;
- separate-Spaces multi-display policy;
- Stage Manager only as an `M2` feature, never as invented default chrome;
- Show Desktop by shortcut, gesture, Hot Corner, or configured wallpaper click;
- correct behavior through hotplug, suspend, app crash, and session restart.

### 4.8 Spotlight

Tahoe Spotlight searches apps, files, settings, actions, web suggestions, and
clipboard history. It opens with Command-Space, supports keyboard-first
navigation, previews, calculations, conversions, file reveal, drag/copy, quick
actions, shortcuts, history, resizing, and moving. See Apple's
[Spotlight guide](https://support.apple.com/guide/mac-help/search-with-spotlight-mchlp1008/mac).

rmac requirements:

- immediate focused opening with the correct shortcut;
- app, file, folder, setting, recent item, calculator/conversion, command, and
  action providers backed by real data;
- stable ranked results and type-specific previews;
- Return opens/executes, Command reveals location, modifiers expose alternate
  actions, and Escape/click-away closes;
- explicit search-scope and privacy settings;
- bounded local indexing with no private content in logs;
- clipboard history off or consented according to the reference profile, with
  secret-sensitive exclusions and clearing;
- no fake web, Siri, Shortcuts, or Apple Intelligence results;
- failures isolate one provider instead of breaking all search;
- history, focus, selection, and accessibility announcements are coherent.

### 4.9 Control Center, status menus, and OSDs

Apple's [Control Center guide](https://support.apple.com/guide/mac-help/quickly-change-settings-mchl50f94f8f/mac)
defines toggle icons, sliders, expandable detail rows, drag/reorder/resize,
Controls Gallery, menu-bar copies, categories, third-party controls, and privacy
attribution.

The rmac equivalent must support only controls with real Linux authorities:

- Wi-Fi, Bluetooth, AirDrop-equivalent only under a non-Apple honest name,
  Focus, Stage Manager, Screen Mirroring/displays, Sound, Now Playing,
  brightness, battery, keyboard brightness where hardware exposes it,
  accessibility shortcuts, fast user switching, timer, window tiling,
  shortcuts, and supported third-party controls;
- compact toggle on the icon, detail on the row/arrow, live state, pending
  mutation, authorization, rollback, and precise failure;
- sliders with keyboard adjustment and coalesced mutations;
- customization matching Tahoe: add, remove, move, resize, category filter,
  custom pages, and copy to the menu bar where implemented;
- volume, brightness, keyboard backlight, media, and other OSDs that appear
  transiently, merge repeated adjustments, never steal focus, and dismiss on
  schedule/reduced-motion policy;
- no duplicate quick-settings panel or Linux tray beside Control Center.

### 4.10 Notifications, Notification Center, widgets, and Focus

Notifications arrive as transient banners or persistent alerts at the upper
right, support grouping, expansion, details, direct actions, Options, mute,
settings, clearing, time-sensitive/critical policy, sounds, and Focus. Apple's
[notification guide](https://support.apple.com/guide/mac-help/view-app-notifications-mh40609/mac)
and [Notification Center guide](https://support.apple.com/en-lamr/guide/mac-help/mchl2fb1258f/mac)
define these journeys.

Required behavior:

- one notification daemon and history authority;
- banner versus alert style, preview policy, grouping, badges, sounds, actions,
  and per-app settings;
- date/time click and two-finger edge gesture open Notification Center;
- click-away, repeat date/time click, edge gesture, and Escape close it;
- grouped history, clear one, clear group, clear all, and live actions;
- widgets only when backed by real providers, with supported sizes, edit mode,
  reorder, remove, and app deep links;
- Focus modes with people/app filters translated truthfully to Linux app/event
  policy, schedules, manual state, status, and urgent exceptions;
- privacy-safe lock-screen previews;
- notification content and action tokens never enter diagnostic logs.

### 4.11 Input, shortcuts, gestures, and shared interaction

- Command is the primary rmac accelerator concept; physical mapping must be
  configurable and printed consistently in menus/help.
- Standard shortcuts cover open, close, quit, hide, hide others, minimize,
  settings, undo/redo, cut/copy/paste, select all, find, save, print, tabs,
  windows, screenshots, Spotlight, app switching, Mission Control, Spaces,
  lock, and Force Quit.
- Command-Tab switches apps without reordering the Dock. Command-backtick
  switches windows in the active app.
- Tab/Shift-Tab and Full Keyboard Access reach every control.
- Escape dismisses transient UI or cancels the current reversible operation.
- Return performs the primary action; Space toggles/Quick Looks where macOS
  does; arrow keys preserve spatial/list semantics.
- Secondary click and Control-click open the same context menu.
- Trackpad gestures cover scrolling, secondary click, smart zoom, zoom/rotate,
  page navigation, app expose, Mission Control, Spaces, desktop reveal, and
  configured launcher behavior only where hardware supports them.
- Drag/drop negotiates MIME/type support, shows legal targets, provides spring-
  loading where implemented, supports modifier-copy/move/link semantics, and
  never silently destroys data.
- Clipboard, primary selection, IME, dead keys, compose, emoji/symbol picker,
  Dictation equivalent, key repeat, layouts, and accessibility input must work
  across rmac and third-party apps.

## 5. Finder and system document behavior

Finder is not just a Files window; it is the desktop's persistent file and
application identity. Closing all Finder windows leaves Finder active.

Required Finder domains:

- sidebar sections for favorites, iCloud-equivalent providers only when real,
  locations, mounted volumes, network, tags, and saved searches;
- Back/Forward, path/title, view controls, sort/group, Share, Tags, Search, and
  customizable toolbar;
- icon, list, column, and gallery views with per-folder retained view options;
- tabs, multiple windows, spring-loaded folders, status bar, path bar, preview
  pane, inspector, and Quick Look;
- selection, rubber-band selection, range/toggle selection, inline rename,
  contextual actions, keyboard navigation, and type-ahead;
- New Folder, open, open with, duplicate, alias/link, copy, move, rename,
  compress/extract, Get Info, tags, share, print, eject, Trash, restore, and
  Empty Trash;
- operation progress, pause/cancel where possible, name conflicts, replace,
  merge, keep both, permission/authentication, insufficient space, disconnect,
  partial failure, undo, and crash recovery;
- local disks, removable media, network mounts, portals, sandbox documents,
  symlinks, hidden files, packages/bundles, executable files, MIME/default apps,
  thumbnails, metadata, and search;
- Recent, Applications, Desktop, Documents, Downloads, home, mounted volumes,
  and Trash as real places, independent of whether any place is pinned in the
  Dock;
- folder color, tags, emoji/symbol, and default folder color as Tahoe `M2`
  personalization;
- desktop ownership, mounted-volume icons, file context menus, and default app
  launching shared with the shell.

Use Apple's [Finder overview](https://support.apple.com/guide/mac-help/organize-your-files-in-the-finder-mchlp2605/mac)
plus the Finder User Guide for per-feature review. Linux filesystem semantics
remain authoritative; rmac must never claim unsupported APFS, iCloud, or Time
Machine behavior.

### System-wide document services

- open/save/export panels through secure portals;
- recent documents and Open Recent;
- Quick Look from Finder, desktop, dialogs, and supported apps;
- Preview/Markup for images and PDFs;
- print panel, page setup, PDF workflow, print queue, and failure handling;
- Share equivalent only for installed real targets;
- Services/Quick Actions only for discoverable safe actions;
- spellcheck, substitutions, text replacements, emoji/symbols, Dictionary,
  data detectors, and input methods where available;
- autosave, versions/recovery, reopening, and unsaved-change policy for
  document apps;
- consistent tags, thumbnails, metadata, file promises, and drag/drop.

## 6. System Settings inventory

System Settings is a searchable sidebar/detail application. Hardware-, account-,
and region-specific panes appear only when relevant. App-specific settings stay
in the app menu rather than being moved into the system application.

### 6.1 Pane map

| Pane/domain | Required rmac behavior |
|---|---|
| Account header | Local user identity; online accounts only for configured real providers |
| Wi-Fi | radio, networks, security, join/forget, known networks, details, DNS/proxy/IP |
| Bluetooth | radio, discover, pair, trust, connect, disconnect, remove, device details |
| Network | Ethernet/Wi-Fi/VPN interfaces, state, addressing, DNS, routes, proxy, service order where supported |
| VPN | import, add, edit, connect, disconnect, secrets through a secure agent, delete |
| Notifications | per-app allow, style, previews, grouping, badges, sound, history, Focus exceptions |
| Sound | output/input devices, volume, mute, balance, alerts, routing, device profile |
| Focus | modes, filters, schedules, manual activation, urgent exceptions |
| Screen Time | usage and limits only after a truthful local authority exists; otherwise absent |
| General | About, Software Update, Storage, AirDrop/Handoff-equivalent only if honest, Login Items, Language & Region, Date & Time, Sharing, Time Machine-equivalent only if real, Transfer/Reset |
| Appearance | light/dark/automatic, accent, highlight, folder color, icon appearance, sidebar/control size where supported, contrast policy |
| Accessibility | VoiceOver/screen reader, Zoom, Display, Spoken Content, Descriptions, Audio, captions, Voice Control, Keyboard, Pointer Control, Switch Control, speech features, shortcuts |
| Control Center/Menu Bar | add/remove/reorder/resize controls and menu-bar visibility |
| Siri/Assistant and Spotlight | local search providers/scopes/privacy; assistant hidden until a real privacy-reviewed provider exists |
| Desktop & Dock | Dock, desktop items, Show Desktop, Stage Manager, widgets, default browser, windows, Mission Control, Hot Corners |
| Displays | identify, resolution, scale, arrangement, main display, rotation, refresh, mirroring, color/night controls where supported, Keep/Revert |
| Wallpaper/Screen Saver | per-display source and fit, appearance, shuffle/dynamic behavior, screen saver selection and timing |
| Battery/Power | charge/state/health/history, Low Power/performance profiles, screen/sleep behavior, optimized charging where hardware permits |
| Lock Screen | display/saver timing, password delay, hints, message, login-window mode, power buttons, accessibility |
| Users & Groups | local users, admin/standard role, groups, password changes, guest and automatic login only when secure/policy permits |
| Passwords/Secrets | secure credential store UI only after an audited keyring boundary; no plaintext imitation |
| Internet/Online Accounts | only real provider integrations, scopes, sync categories, revocation, errors |
| Keyboard | repeat, navigation, shortcuts, input sources, text replacements, Dictation equivalent and privacy |
| Trackpad | pointing, clicking, scrolling/zoom, gestures with demonstrations and hardware capabilities |
| Mouse | tracking, scrolling, buttons, gestures, natural direction, secondary click |
| Printers & Scanners | discover, add, defaults, queue, options/supplies where CUPS/SANE support them |
| Game Controller | only when real hardware/service support is present |
| Privacy & Security | permissions, privacy indicators, encryption status, firewall, app launch trust, updates, analytics consent, developer mode/policy where relevant |

### 6.2 Settings behavior

- Search returns panes and exact controls with deep links and highlights.
- Sidebar selection, Back/Forward history, scroll position, and window state
  restore safely.
- Every switch reflects authoritative readback, not optimistic demo state.
- Mutations show pending state, authorization, success, specific failure, and
  externally changed state.
- Risky display/network/power changes have Keep/Revert and timed rollback.
- Controls disappear when meaningless; capability loss updates live.
- Dependent controls disable with an explanation.
- No row exists only to make the app look fuller.
- Search, keyboard navigation, screen reader semantics, text scaling, reduced
  motion, reduced transparency, and contrast apply to every pane.

Apple's [Desktop & Dock settings](https://support.apple.com/guide/mac-help/change-desktop-dock-settings-mchlp1119/26/mac/26)
are the detailed authority for Dock, desktop, Stage Manager, widgets, windows,
Mission Control, and Hot Corners. Other panes must link their specific Apple
guide and Linux authority in their implementation specification.

## 7. Built-in application inventory

Apple's current
[apps-included list](https://support.apple.com/guide/mac-help/apps-on-your-mac-mchl110b00b7/mac)
contains the following applications. This table prevents accidental omission;
it does **not** require rmac 1.0 to clone every proprietary service.

| Apple app | Core capability | rmac disposition |
|---|---|---|
| Activity Monitor | process, CPU, memory, energy, disk, network inspection and process actions | `M1/LE`: first-party System Monitor |
| AirPort Utility | Apple base-station administration | `AO`: no fake clone; ordinary Wi-Fi lives in Settings |
| Apple Games | game library, discovery, friends, challenges | `AO`: later honest Linux game library only |
| App Store | discover, buy, install, update, review apps | `LE`: Software Center with truthful APT/Flatpak/Snap sources |
| Audio MIDI Setup | configure audio and MIDI devices | `LE`: advanced audio/MIDI settings when PipeWire/ALSA expose them |
| Automator | visual task automation | `M2`: later automation app; do not duplicate Shortcuts prematurely |
| Bluetooth File Exchange | Bluetooth file transfer | `LE`: only if BlueZ transfer support is secure and reliable |
| Books | ebooks, PDFs, audiobooks, library | later compatible reader; proprietary store absent |
| Boot Camp Assistant | install Windows on Intel Mac | `AO/NA`: absent |
| Calculator | basic, scientific, programmer math and conversions | `M1`: first-party utility and Spotlight provider |
| Calendar | accounts, calendars, events, invitations, alerts | later first-party or integrated compatible app |
| Chess | local/network chess | optional, not a desktop-parity blocker |
| Clock | world clock, alarms, stopwatch, timers | `M1`: first-party utility and Control Center timer |
| ColorSync Utility | color profiles and filters | `LE`: display/color management only when Linux authority is adequate |
| Console | system/application logs and diagnostics | `LE`: bounded privacy-safe Logs/diagnostics app |
| Contacts | people, organizations, groups, account sync | later app with real providers; no implied iCloud |
| Dictionary | dictionaries, thesaurus, sources | later local/provider-backed lookup plus system text service |
| Digital Color Meter | sample display colors | small first-party utility after shell/core apps |
| Directory Utility | directory services | advanced admin feature only if real Linux authority exists |
| Disk Utility | disks, images, erase, partition, repair, RAID | `LE`: high-risk later utility using audited storage authorities |
| DVD Player | DVD/video playback | use a compatible player; optional hardware-dependent app |
| FaceTime | Apple audio/video calling | `AO`: absent; generic calling must not use FaceTime identity |
| Find My | Apple people/device/item location | `AO`: absent |
| Font Book | install, preview, enable, validate, organize fonts | `LE`: later first-party font manager |
| Freeform | collaborative infinite canvas | later local canvas; collaboration only with a real provider |
| GarageBand | music creation studio | use compatible Linux software; not a shell requirement |
| Grapher | graph equations and data | optional utility |
| Home | Apple Home/Matter accessory control | later generic smart-home integration under its real name |
| Image Capture | camera/scanner import | `LE`: later device-import utility |
| Image Playground | Apple Intelligence image generation | `AO`: absent until a consented provider-neutral design exists |
| iMovie | video editing | compatible Linux app; not cloned for 1.0 |
| iPhone Mirroring | control a paired iPhone | `AO`: absent |
| Journal | journals, media, map, insights, sync | later local-first app; no implied iCloud sync |
| Keynote | presentation authoring | compatible office suite; not cloned for 1.0 |
| Magnifier | camera-based magnification and filters | accessibility roadmap with real camera/portal support |
| Mail | multi-account email | later app or compatible client with real account providers |
| Maps | maps, places, routes, transit | provider-dependent later app; no Apple Maps identity |
| Messages | Apple messaging/SMS integration | `AO`: absent; generic messaging separately named |
| Migration Assistant | transfer users/data/settings | `LE`: future audited migration/import tool |
| Music | music library, playback, store/subscription | compatible player; system media integration required |
| News | Apple News and subscriptions | `AO`: absent; generic news reader separately named |
| Notes | folders, lists, tags, attachments, tables, scan, lock, sharing | `M1`: first-party local-first Notes |
| Numbers | spreadsheet authoring | compatible office suite; not cloned for 1.0 |
| Pages | word processing and page layout | compatible office suite; not cloned for 1.0 |
| Passwords | passwords, passkeys, Wi-Fi codes, verification codes, sharing | later audited keyring/passkey application; never simulated |
| Phone | calls, recents, contacts, voicemail through iPhone | `AO`: absent |
| Photo Booth | camera photos/video/effects | optional utility with camera portal |
| Photos | import, organize, edit, search, albums, people/places, sync | later local photo manager or integrated compatible app |
| Podcasts | discover, subscribe, download, play podcasts | compatible player; media integration required |
| Preview | PDF/image view, annotate, sign, edit, export, print | `M1`: first-party viewer/Quick Look/Markup foundation |
| Print Center | print queue, pause/resume/cancel, printer status | `LE`: CUPS-backed system utility |
| QuickTime Player | audio/video playback, recording, trimming, export | `M1/LE`: coherent media player/recorder using Linux codecs/portals |
| Reminders | lists, sections, dates, locations, tags, recurring tasks | later local-first app and Spotlight quick action |
| Safari | browser, profiles, privacy, passwords, extensions, web apps | integrate default browser; do not clone Safari branding |
| Screen Sharing | view/control remote desktops | `LE`: later RDP/VNC/Wayland remote desktop utility |
| Screenshot | still capture and screen recording | `M1/LE`: portal-backed screenshot/recording overlay |
| Script Editor | edit/run AppleScript/JavaScript automation | `AO` for AppleScript; generic script tooling remains Terminal/editor |
| Shortcuts | action-based automation and triggers | `M2/LE`: later provider-neutral automation with real actions |
| Stickies | desktop sticky notes | optional utility after Notes and desktop window rules are correct |
| Stocks | watchlists, charts, business news | provider-dependent optional app |
| System Information | detailed hardware, software, network report | `M1/LE`: privacy-safe About/System Information |
| System Settings | operating-system configuration | `M1/LE`: first-party Settings backed by Linux services |
| Terminal | UNIX shell, profiles, tabs/windows, search | `M1`: first-party Terminal |
| TextEdit | plain/rich text, HTML, document workflows | `M1`: first-party Text Editor with truthful format support |
| Tips | discover system and app features | later rmac Help/Tips matching implemented behavior only |
| TV | media library, playback, store/subscription | compatible player; proprietary service absent |
| Voice Memos | record, edit, organize, transcribe, share audio | later first-party or integrated recorder |
| VoiceOver Utility | configure screen reader | `LE`: accessibility settings and Orca integration |
| Weather | current conditions, forecasts, maps, alerts | provider-dependent later app/widget |

Apps downloadable from Apple but not guaranteed installed, such as GarageBand,
iMovie, Pages, Numbers, and Keynote, remain part of the ecosystem inventory but
are not rmac shell blockers. Availability also varies by hardware, language,
region, and account.

### 7.1 First-party app priority

The order follows daily desktop dependency, not the size of Apple's catalog:

1. Finder and desktop file behavior.
2. System Settings and Control Center.
3. Terminal.
4. Notes.
5. Text Editor.
6. Activity/System Monitor.
7. Preview, Quick Look, Screenshot, and print foundation.
8. Calculator, Clock, and Calendar.
9. Passwords only after its security design passes review.
10. Remaining local utilities.
11. Provider-dependent communication/media/productivity apps.
12. Apple-only services remain absent.

### 7.2 App acceptance template

Every implemented app needs:

- a reference-Mac screenshot/interaction set for its main windows, menus,
  toolbars, sidebars, dialogs, settings, empty/loading/error states, and narrow
  and full-screen layouts;
- complete menu and shortcut inventory;
- launch, reopen, multi-window, tabs, focus, close, quit, restore, crash,
  document, drag/drop, print/share, and notification behavior as applicable;
- real data authorities, bounded work, cancellation, errors, external changes,
  conflict handling, atomic persistence, and recovery;
- keyboard-only and screen-reader journeys;
- light/dark, tint, contrast, transparency, motion, scale, and localization;
- measured startup, interaction, idle CPU, wakeups, memory, and long-session
  stability on the Ubuntu reference PC.

## 8. System integration and invisible features

A convincing desktop depends on behavior that screenshots do not show.

### 8.1 Application lifecycle and interoperability

- desktop-file/bundle discovery, launch, activation, single-instance and multi-
  instance behavior, default apps, MIME types, URI handling, startup feedback,
  progress, badges, recent items, and Dock/window identity;
- GTK, Qt, Electron, browser, game, Flatpak, Snap, AppImage, XWayland, and native
  Wayland compatibility;
- XDG file, URI, screenshot, screencast, wallpaper, notification, settings,
  secret, inhibit, print, and global-shortcut portals;
- app menus when honestly exportable and in-window menus otherwise;
- notifications, media keys, MPRIS, clipboard, drag/drop, IME, accessibility,
  color management, fractional scale, and multi-display support;
- no shell surface may steal focus from the app unless the user invoked it.

### 8.2 Hardware and service mapping

| macOS outcome | Linux authority |
|---|---|
| Wi-Fi, Ethernet, VPN | NetworkManager and secure secret agent |
| Bluetooth | BlueZ |
| Audio routing and volume | PipeWire/WirePlumber |
| Battery and power | UPower, power profiles, supported firmware/sysfs |
| Displays and brightness | niri/output protocols, DRM/backlight, color services where available |
| Input and gestures | libinput, compositor configuration, input methods |
| Mounts and removable media | kernel mount state, udisks/GVfs/portals as appropriate |
| Printing/scanning | CUPS and supported scanner service |
| Time, locale, hostname | systemd timedated/localed/hostnamed |
| Users, sessions, power actions | accounts/PAM, logind, systemd, polkit |
| Secrets | desktop keyring/Secret Service with least privilege |
| Notifications and Focus | one rmac notification authority and portal backend |
| Privacy indicators | PipeWire/portal/session authorities and explicit attribution |
| Updates/software | signed rmac repository plus truthful distribution package authorities |

Service loss degrades only its own controls. State changes made outside rmac
must appear without polling loops or restarts.

### 8.3 Security and privacy

- secure boot, disk encryption, login, lock, PAM, polkit, keyring, sandbox,
  portal, package-signing, and recovery boundaries remain real Linux security;
- privacy permissions cover location, contacts/calendars where applicable,
  microphone, camera, input monitoring, accessibility control, screen/system
  audio capture, files/folders, removable media, automation, and notifications;
- permissions are requested at the moment of need with purpose and denial
  recovery, not during an indiscriminate first launch;
- indicators show active capture and the responsible app;
- secrets, clipboard data, notification content, file paths, usernames,
  addresses, identifiers, and document content are redacted from diagnostics;
- opening untrusted software has a clear provenance/trust warning and a narrow
  deliberate override, analogous in outcome to Gatekeeper but not branded as
  it;
- updates are signed, rollback-safe, recoverable, and do not remove the GNOME
  recovery session;
- no telemetry, online account, cloud sync, AI provider, or sponsorship
  analytics is enabled without a separate consent/privacy design.

### 8.4 Accessibility

The Mac User Guide inventories visual, hearing, mobility, and speech domains,
including screen reader, Zoom/Magnifier, font/icon size, color and contrast,
Hover Text, spoken content, motion, pointer visibility, captions, hearing
devices, background sounds, Voice Control, Accessibility Keyboard, Full
Keyboard Access, Switch Control, head pointer, Dwell, Personal Voice, and Vocal
Shortcuts. Tahoe adds Magnifier, Name Recognition, Braille Access,
Accessibility Reader, and expanded accessibility metadata.

rmac's mandatory foundation is:

- semantic roles, names, values, states, relationships, actions, headings,
  tables, lists, live regions, and focus order;
- Orca operation and a functioning AT-SPI bridge;
- keyboard-only use and visible focus;
- text/interface scaling without clipped critical content;
- high contrast, reduced transparency, reduced motion, color-independent
  meaning, captions, visual alerts, and adjustable pointer/input behavior;
- accessibility at login/lock and through authorization dialogs;
- no custom-drawn control without equivalent semantics and input behavior.

Apple's [accessibility HIG](https://developer.apple.com/design/human-interface-guidelines/accessibility/)
is the design authority; Linux assistive technologies are the runtime
authority. Advanced features may be staged, but core journeys cannot ship
inaccessible.

### 8.5 Localization and regional behavior

- Unicode, grapheme-safe editing, bidirectional text, IME/preedit, dead keys,
  compose, emoji, symbols, and multiple keyboard layouts;
- locale-aware date, time, number, measurement, currency, collation, file size,
  paper, week-start, timezone, and calendar formatting;
- translated, expandable UI without fixed English widths;
- right-to-left mirroring where appropriate, without mirroring media or
  universally meaningful direction controls;
- language/region changes have truthful sign-out/restart requirements;
- features unavailable in a language/region are absent or clearly described.

## 9. Performance, energy, and optimization contract

The goal is the *feeling* of a Mac: immediate input response, smooth coherent
motion, quiet idle behavior, predictable memory use, and safe recovery. Visual
effects may never justify a desktop that is hot, laggy, or blurry.

Apple's archived but still relevant
[energy-efficiency guidance](https://developer.apple.com/library/archive/documentation/Performance/Conceptual/power_efficiency_guidelines_osx/BestPractices.html)
requires applications to return to idle, avoid polling, minimize timers, keep
heavy work off the main thread, batch work, and prioritize user-visible work.
Its [graphics guidance](https://developer.apple.com/library/archive/documentation/Performance/Conceptual/power_efficiency_guidelines_osx/UsingEfficientGraphics.html)
warns against drawing obscured or unchanged content.

rmac requirements:

- event-driven subscriptions instead of periodic command execution;
- zero continuous animation when nothing is changing;
- frame-clock rendering only for visible animation and immediate stop at rest;
- dirty-region and occlusion-aware drawing where the framework permits;
- blur/material passes clipped, cached, and disabled/replaced under reduced
  transparency or insufficient hardware capability;
- no blocking filesystem, D-Bus, network, package, thumbnail, search, or child-
  process work on the UI thread;
- bounded worker concurrency, cancellation, generation checks, and stale-result
  rejection;
- debounce/coalescing for sliders and bursty service events without delaying
  direct user feedback;
- lazy thumbnails, icons, previews, search providers, app panes, and large data;
- cache decoded images and stable layout, but bound memory and invalidate on
  real changes;
- batch and atomically commit disk writes; do not rewrite unchanged state;
- backpressure for notifications, logs, search, media, and subprocess output;
- user-visible work receives priority; maintenance waits for safe idle/power
  conditions;
- hidden/background first-party apps stop display refresh and nonessential
  sampling;
- shell component failure restarts only that component and preserves the
  session; repeated failure enters a usable safe mode;
- storage guardrails preserve at least 15 GiB and warn before low-space failure.

### Required measurements

- session-ready time from authentication;
- cold/warm app launch and first-interaction latency;
- Spotlight and Control Center invocation latency;
- pointer/key-to-frame latency for menus, Dock, sliders, and typing;
- frame pacing and dropped frames during representative animations;
- idle CPU, GPU, wakeups, D-Bus traffic, process count, and memory for the full
  shell;
- per-app idle and active memory, CPU, wakeups, I/O, and startup;
- thumbnail/search/file-operation throughput and cancellation latency;
- lock/unlock, suspend/resume, output hotplug, network/service restart, and
  crash-recovery times;
- four-hour and eight-hour memory/handle/process stability;
- performance at 100%, fractional scale, 200%, multiple displays, software
  fallback, and the supported Intel/AMD/NVIDIA matrix.

Budgets live in `PLAN_V2.md` and performance audit documents. They may be
revised only from measured reference-hardware evidence, never to hide a
regression.

## 10. Installation, update, recovery, and removal

The macOS-like experience begins before the desktop and includes confidence
that the machine remains usable.

- one signed installer installs packages, session entry, portals, services,
  schemas, assets, and defaults;
- selecting rmac at login starts the complete experience without terminal
  commands;
- GNOME stays selectable for recovery and package maintenance during early
  development;
- updates validate compatibility, signature, dependencies, disk space, and
  recovery path before changing the installed session;
- update while inside rmac must be deferred or performed transactionally so a
  live session never mixes old and new binaries/configuration;
- interrupted install/update recovers to a bootable desktop;
- same-version reinstall, upgrade, rollback, uninstall with data retention,
  and complete purge are tested;
- user documents/settings are preserved or exported according to an explicit
  migration contract;
- TTY and GNOME recovery instructions remain available even if rmac cannot
  start;
- release notes, known limitations, installed version, diagnostics, and update
  state all refer to the same candidate.

## 11. Whole-product acceptance journeys

The product is not complete until one exact packaged build passes these real-
hardware journeys:

1. Boot, select rmac, authenticate once, reach a complete desktop.
2. Open, focus, switch, minimize, restore, tile, full-screen, close windows,
   quit apps, and use multiple Spaces without Dock reordering or lost state.
3. Use the system menu, real app menus, context menus, status menus, shortcuts,
   Control Center, OSDs, and click-away/Escape behavior.
4. Search, calculate, reveal, open, and take an action through Spotlight.
5. Find, preview, create, rename, copy, move, conflict-resolve, undo, trash,
   restore, eject, and search files in Finder.
6. Connect and manage Wi-Fi, Ethernet/VPN, Bluetooth, audio, battery/power,
   brightness, and displays using real hardware/services.
7. Receive, act on, group, mute, clear, and focus-filter notifications.
8. Create/recover a note, edit/save a text document, use Terminal, inspect/end
   a process, preview/annotate a file, take a screenshot, and print/export PDF.
9. Lock, type a wrong password, recover visibly, unlock once, suspend/resume,
   switch user, log out, restart, shut down, and return to GNOME recovery.
10. Change appearance, Dock, wallpaper, input, accessibility, notification,
    privacy, and system settings; verify authoritative readback and persistence.
11. Repeat core journeys keyboard-only, with Orca, reduced motion, reduced
    transparency, high contrast, light/dark, 100%, fractional scale, and 200%.
12. Restart shell services, disconnect authorities, hotplug displays/devices,
    fill storage near the safe threshold, and recover without data loss.
13. Install, upgrade, roll back, remove, and recover from an interrupted update.

Each journey records commit, package version, OS, compositor, hardware, output
scale, settings profile, commands/log evidence, screenshots/recording, pass or
failure, and remaining defect.

## 12. Implementation order

Do not polish isolated apps while the login/session/window foundation makes the
whole desktop feel unlike macOS.

### P0 — Safe complete session

Single authentication, secure lock, supervised components, no duplicates,
GNOME recovery, package/update boundary, storage floor.

### P1 — Shared visual and interaction foundation

Measured tokens, materials, curves, typography roles, controls, menus, sheets,
focus, accessibility, motion, transparency fallbacks.

### P2 — Shell and window model

Wallpaper/desktop, menu bar/system menu/app menus, Dock, window behavior,
Mission Control/Spaces, Spotlight, Control Center, Notification Center, Focus,
OSDs, screenshots, shortcuts, gestures.

### P3 — Finder and system services

Finder/desktop files, Quick Look, portals, default apps, clipboard/drag-drop,
printing, screenshots, real network/Bluetooth/audio/power/display authorities,
Settings.

### P4 — Daily built-in apps

Terminal, Notes, Text Editor, System Monitor, Preview, Calculator, Clock,
Calendar, then the remaining local utilities.

### P5 — Compatibility, optimization, accessibility, delivery

Third-party app matrix, hardware matrix, performance/energy, resilience,
security review, install/update/remove, public evidence, sponsor demo.

Complete each priority as an end-to-end journey. Continue visual refinement as
shared evidence improves, but do not skip a broken critical interaction to add
another decorative surface.

## 13. Current repository alignment audit

This research document describes the target, not the current completion state.
Source files, unit tests, package builds, and real Ubuntu runtime evidence remain
the completion authority.

### Already represented in the repository

- native package and session machinery with a preserved GNOME recovery path;
- supervised wallpaper, top bar, Dock, search, notification, quick-settings,
  lock, shortcut, and first-party application processes;
- shared theme/UI crates and initial Tahoe-oriented shell rendering;
- real Linux domain crates for applications, compositor state, displays,
  network/VPN, Bluetooth, audio, power, mounts, time/locale, printing, privacy,
  updates, shortcuts, notifications, and portals;
- first-party Finder/Files, Terminal, Notes, Text Editor, System Monitor,
  Settings, and installed-app views;
- extensive behavioral specifications and focused tests.

These facts prove engineering coverage, not macOS parity. The reference-PC UI,
window behavior, authentication, application journeys, accessibility, and
performance still require end-to-end proof.

### Known contract mismatches to remove

1. `docs/user-guide.md`, `docs/places.md`, and parts of `docs/dock.md` describe
   a fixed Files/Downloads/Trash Dock tail. Tahoe supports folder stacks, but
   the visible Dock must use configured items and the reference profile must
   not force Downloads or Files.
2. Several documents and binaries use the product-facing name “App Drawer.”
   Tahoe 26 presents an Apps view from the Dock. Internal crate names may remain
   stable, but the visible name, icon, menus, animation, layout, and settings
   must follow the measured Tahoe Apps experience.
3. Shell rendering is now built from the separately locked
   `experiments/gpui-upstream-lab` graph into the native session package. Keep
   each host connected to its maintained runtime/model crate and prevent any
   preview-only implementation from diverging from the packaged path.
4. System Settings has substantial real Linux authority coverage, but visual
   hierarchy, pane ordering, capability-driven visibility, and interaction need
   controlled comparison against Tahoe.
5. Existing first-party apps have deep domain work but are not accepted until
   their packaged Ubuntu journeys and reference-Mac visual/interaction reviews
   pass.
6. The lock/login stack has previously shown duplicate or inconsistent
   presentation. It remains unaccepted until one-password unlock and every
   suspend/resume/wrong-password path pass on hardware.
7. Visual values described as approximate in older docs must be replaced by
   measured reference tokens before being called exact.
8. Any global blur, duplicate panel, debug border, placeholder icon, invented
   status value, or nonfunctional control is a release-blocking defect.

### Immediate execution queue

1. Finish the default-profile cleanup: configured Dock contents, Apps naming,
   and no forced optional item.
2. Capture a controlled reference set for menu bar, system menu, Dock, normal
   windows, transient overlays, lock/unlock, Control Center, Notification
   Center, Spotlight, Mission Control, Apps, Finder, and Settings.
3. Complete shared measured material/curve/type/motion tokens and remove local
   approximations from the shell.
4. Make ordinary window management, focus, close-versus-quit, app switching,
   minimize/restore, full screen, and click-away match before polishing apps.
5. Finish and prove every P2 shell journey on the packaged Ubuntu session.
6. Move to Finder and Settings, then the daily app order in section 7.1.
7. Run accessibility, performance, resilience, installation, and hardware
   gates at milestone boundaries rather than rebuilding after every pixel edit.

## 14. Explicit non-goals until a truthful design exists

- Apple logo, Finder face, SF fonts, Apple wallpapers, Apple sounds, Apple app
  icons, proprietary assets, or misleading Apple product names.
- iCloud, Apple Account, Apple Pay/Wallet, AppleCare, Find My, FaceTime,
  iMessage, Phone, iPhone Mirroring, AirDrop, Handoff, Universal Control,
  Sidecar, AirPlay, Apple Home, Apple News, Apple TV store, Apple Music store,
  Apple Arcade, or Apple Intelligence presented as if supplied by Apple.
- APFS snapshots, FileVault, Gatekeeper, Time Machine, Secure Enclave, Touch ID,
  or Apple Watch unlock claimed when the implementation is a different Linux
  technology.
- fake global menus, fake devices, fake battery history, fake storage
  categories, fake privacy indicators, fake updates, fake cloud sync, fake
  actions, or settings that mutate only the UI.
- mandatory Dock magnification, Downloads, recent apps, widgets, Stage Manager,
  clear icons, tinted icons, or other optional features not enabled on the
  reference Mac.
- an app drawer mislabeled Finder or a Linux panel merely themed to resemble a
  menu bar.

Equivalent Linux features may be built under accurate rmac/generic names after
their authority, privacy, security, failure, and UX designs are reviewed.

## 15. Source index

### Current platform and features

- [Mac User Guide for macOS Tahoe](https://support.apple.com/guide/mac-help/welcome-mh43558/mac)
- [New features available with macOS Tahoe](https://www.apple.com/mideast/os/pdf/All_New_Features_macOS_Tahoe_Sept_2025.pdf)
- [Apps included on your Mac](https://support.apple.com/guide/mac-help/apps-on-your-mac-mchl110b00b7/mac)
- [macOS Tahoe 26.6 security release](https://support.apple.com/en-us/128067)

### Design and interaction

- [Human Interface Guidelines](https://developer.apple.com/design/human-interface-guidelines/)
- [Designing for macOS](https://developer.apple.com/design/human-interface-guidelines/designing-for-macos/)
- [Materials and Liquid Glass](https://developer.apple.com/design/human-interface-guidelines/materials)
- [Accessibility](https://developer.apple.com/design/human-interface-guidelines/accessibility/)
- [Menus](https://developer.apple.com/design/human-interface-guidelines/menus)
- [Sidebars](https://developer.apple.com/design/human-interface-guidelines/sidebars)
- [Toolbars](https://developer.apple.com/design/human-interface-guidelines/toolbars)

### Shell behavior

- [Menu bar](https://support.apple.com/guide/mac-help/whats-in-the-menu-bar-mchlp1446/mac)
- [System menu](https://support.apple.com/guide/mac-help/whats-in-the-apple-menu-mchlp1130/mac)
- [Dock](https://support.apple.com/guide/mac-help/open-apps-from-the-dock-mh35859/mac)
- [Desktop & Dock settings](https://support.apple.com/guide/mac-help/change-desktop-dock-settings-mchlp1119/26/mac/26)
- [Spotlight](https://support.apple.com/guide/mac-help/search-with-spotlight-mchlp1008/mac)
- [Control Center](https://support.apple.com/guide/mac-help/quickly-change-settings-mchl50f94f8f/mac)
- [Notifications](https://support.apple.com/guide/mac-help/view-app-notifications-mh40609/mac)
- [Notification Center](https://support.apple.com/guide/mac-help/mchl2fb1258f/mac)
- [Mission Control](https://support.apple.com/guide/mac-help/mh35798/mac)
- [Spaces](https://support.apple.com/guide/mac-help/mh14112/mac)
- [Lock Screen](https://support.apple.com/guide/mac-help/mchl8e8b6a34/mac)
- [Lock Screen settings](https://support.apple.com/guide/mac-help/mh11784/mac)

### Efficiency

- [Energy Efficiency Guide](https://developer.apple.com/library/archive/documentation/Performance/Conceptual/power_efficiency_guidelines_osx/)
- [Energy best practices](https://developer.apple.com/library/archive/documentation/Performance/Conceptual/power_efficiency_guidelines_osx/BestPractices.html)
- [Minimize timers](https://developer.apple.com/library/archive/documentation/Performance/Conceptual/power_efficiency_guidelines_osx/Timers.html)
- [Minimize I/O](https://developer.apple.com/library/archive/documentation/Performance/Conceptual/power_efficiency_guidelines_osx/MinimizingIO.html)
- [Avoid extraneous drawing](https://developer.apple.com/library/archive/documentation/Performance/Conceptual/power_efficiency_guidelines_osx/UsingEfficientGraphics.html)

## 16. Maintenance rule

At every stable macOS release or material reference-Mac settings change:

1. update the baseline and source access date;
2. diff Apple's feature inventory, Mac User Guide table of contents, bundled-
   app list, Desktop & Dock settings, Control Center, accessibility, and
   security/update behavior;
3. capture the changed reference-Mac surfaces and interactions;
4. classify additions as `M0`, `M1`, `M2`, `LE`, `AO`, or `NA`;
5. update component specifications and acceptance journeys;
6. never silently change shipped defaults because Apple added an optional
   feature.

This file decides what rmac is trying to reproduce. Runtime evidence decides
what rmac has actually completed.
