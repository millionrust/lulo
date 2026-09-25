# Mac parity gaps

Purpose: the single tracker for where Lulo OS still differs from the Mac, so agents read less. Reference: **macOS 26.2, Dark** (en-GB locale unless noted). Severity: **P0** broken/missing core · **P1** obvious to a Mac user · **P2** polish. Size: **S** ≤1 day · **M** a few days · **L** more. Status: **Missing** · **Partial** · **Broken** · **Fixed `<short sha>`**.

Rules for agents: read only your surface's `##` section; when you fix a gap, set its Status to `Fixed <short sha>` in the same commit; add new gaps as rows here, never as new documents; Mac captures are never committed — rows describe the Mac only in words. Geometry that's reference data, not a gap, belongs in FEEL_SPEC.md / docs/macos-parity-spec.md — link to those instead of repeating their numbers.

---

## Shell

### Dock

Sizes/positions/colours: see FEEL_SPEC.md §4.7 and docs/macos-parity-spec.md
§4.5.

| ID | Sev | Size | Status | Gap | Where |
|---|---|---|---|---|---|
| DOCK-01 | P1 | S | Missing | Mac: Desktop & Dock › Size slider (Small–Large) sets the tile size; the owner's is 52 pt on a 68 pt pitch. / Lulo: no size setting at all, and the tile size is fixed. | `crates/rmac-shell-settings/src/model.rs:47`, `crates/system-settings/src/controller/desktop_dock.rs:40` |
| DOCK-02 | P1 | S | Missing | Mac: right-clicking the Dock separator shows Turn Hiding On, Turn Magnification On, Position on Screen ▸, Minimise Using ▸ (Genie/Scale), Dock Settings…. / Lulo: the separator has no menu; right-click only works on items and the Trash. | `shell/bins/rmac-dock/src/main.rs:2373`, `crates/rmac-dock/src/menu.rs` |
| DOCK-03 | P1 | S (needs DOCK-01) | Missing | Mac: dragging the separator up or down resizes the Dock. / Lulo: nothing happens. | `shell/bins/rmac-dock/src/main.rs:1774` |
| DOCK-04 | P1 | S | Missing | Mac: ⌥⌘D turns Dock hiding on and off. / Lulo: no binding. | `packaging/rmac-session/shell.kdl:283` |
| DOCK-05 | P1 | M | Partial | Mac: the Finder tile's menu is windows (ticked), New Finder Window, New Smart Folder, Find…, Go to Folder…, Connect to Server…, ~10 recent folders, Show All Windows, Hide, Hide Others — no Options, no Quit. / Lulo: Files gets the generic app menu (Options ▸ Keep in Dock, Quit when running) — no New Window, Go to Folder… or recent folders. | `crates/rmac-dock/src/dock.rs:198` |
| DOCK-06 | P2 | S | Partial | Mac: the Apps tile's menu is Remove from Dock, Show Apps. / Lulo: gets the generic app menu (Options ▸, Open or Quit). | `crates/rmac-dock/src/dock.rs:198` |
| DOCK-07 | P2 | S | Partial | Mac: Magnification slider runs Off · Small … Large, with the size shown live. / Lulo: an on/off switch plus a "Maximum size" stepper (1.5×). | `crates/system-settings/src/controller/desktop_dock.rs:78` |
| DOCK-08 | P2 | S | Partial (indicators + recents, 4d4eb780) | Mac: Desktop & Dock has switches for Show suggested/recent apps, Show indicators for open applications, Animate opening applications, Minimise windows into application icon. / Lulo: none of the four exist — recents, dots and the bounce are always on. | `crates/rmac-shell-settings/src/model.rs:47` |
| DOCK-09 | P2 | S | Missing | Mac: a Minimised-window-animation picker (Genie/Scale) in Desktop & Dock and the separator menu. / Lulo: no setting (the animations themselves are DOCK-11). | `crates/system-settings/src/controller/desktop_dock.rs` |
| DOCK-10 | P2 | S | Broken | Mac: a Settings pane shows its rows at once. / Lulo: Desktop & Dock opens with a "Loading system information…" progress bar above the Dock section. | `crates/system-settings/src/controller/detail.rs:74` |
| DOCK-11 | P2 | M | Missing | Mac: minimising uses Genie or Scale (DOCK-09's setting). / Lulo: a plain 350 ms fade to parking; neither effect exists. | `rmac-design/src/motion.rs:117` |
| DOCK-12 | P2 | S | Missing | Mac: recently-quit apps (up to 3) sit in the Dock without a running dot. / Lulo: no recent-but-closed tiles. | `crates/rmac-dock` |
| DOCK-13 | P2 | S | Missing | Mac: shelf ≈72 px tall, ≈18 px above the screen bottom (FEEL_SPEC.md). / Lulo: shelf rendered 92 px (≈20 px too tall), bottom inset 22 px (≈4 px too high) as of the 2026-09-19 capture — not re-measured since. | `shell/bins/rmac-dock` |
| DOCK-14 | P2 | S | Missing | Mac: an item's right-click menu is ≈165 px wide, 24 px item pitch, with a triangular pointer to the tile. / Lulo: menu 309 px wide (≈144 px too wide), ≈45 px item pitch, no pointer. | `crates/rmac-dock/src/menu.rs` |

### Menu bar

Bar height (29 px), spacing and transparency: see FEEL_SPEC.md §4.2/§4.7 and
docs/macos-parity-spec.md §4.3. The rows below are the shared menu **content**
contract every app menu is built from (`crates/rmac-app-menu`,
`shell/bins/rmac-menubar`), plus the bar's own chrome.

| ID | Sev | Size | Status | Gap | Where |
|---|---|---|---|---|---|
| MENUBAR-01 | P2 | S | Missing | Mac: menu bar renders 29 px tall (FEEL_SPEC.md). / Lulo: rendered 36 px (≈7 px too tall) as of the 2026-09-19 capture; not re-confirmed since. | `shell/bins/rmac-menubar` |
| BAR-01 | P1 | S | Fixed 44f223ee | Mac: on the desktop the bar shows Finder with File/Edit/View/Go/Window/Help, because Finder always runs. / Lulo: with no window focused the bar reads "Files" with no menus unless the Files process is running. | `shell/bins/rmac-menubar/src/main.rs:304`, `:339` |
| BAR-02 | P1 | M | Fixed 44f223ee | Mac: every app has a system Window menu (Minimise ⌘M, Zoom, Fill 🌐⌃F, Centre 🌐⌃C, Move & Resize ▸, the window list) and a Help menu with a search field. / Lulo: the shell adds neither — Settings shows only "Settings View", Calculator only "Calculator Edit View" (see MENU-01/MENU-06 for the general contract). | `crates/rmac-app-menu/src/lib.rs:263`, `shell/bins/rmac-menubar/src/main.rs:2583` |
| BAR-03 | P1 | M | Partial | Mac: Bluetooth, Sound, Focus and VPN menu-bar items each open their own dropdown (switch, device/output list, "… Settings…"). / Lulo: only Wi-Fi and Battery have menus; the rest just open Control Centre. | `shell/bins/rmac-menubar/src/main.rs:2152` |
| BAR-04 | P2 | S | Partial | Mac: holding ⌥ in the Apple menu turns Force Quit… into Force Quit "App" and drops the "…"/confirmation from Restart/Shut Down/Log Out; System Information follows About This Mac. / Lulo: no Option alternates, no System Information. | `shell/bins/rmac-menubar/src/main.rs:2494` |
| BAR-05 | P2 | S | Missing | Mac: the Battery menu has a "Using Significant Energy" section above Battery Settings…. / Lulo: Battery, Power Source, Energy Mode, Battery Settings… only. | `shell/bins/rmac-menubar/src/menu_model.rs:609` |
| BAR-06 | P2 | S | Partial | Mac: the App Store… row shows the pending count ("App Store…, 6 updates"). / Lulo: "Software Center" with no count and no "…". | `shell/bins/rmac-menubar/src/main.rs:2518` |
| BAR-07 | P2 | S | Partial | Mac: the app menu's About "App" is enabled and opens an About window. / Lulo: always disabled. | `shell/bins/rmac-menubar/src/main.rs:2601` |
| BAR-08 | P2 | M | Missing | Mac: ⌘-dragging a status item reorders it; dragging it out removes it. / Lulo: the order is fixed. | `shell/bins/rmac-menubar/src/main.rs:2140` |
| MENU-01 | P1 | M | Fixed 44f223ee | Mac: every app has a Window menu (Minimise ⌘M, Zoom, Fill fn⌃F, Centre fn⌃C, Move & Resize ›, the open-window list). / Lulo: only Files exports one, with just two tab items. | `shell/bins/rmac-menubar/src/main.rs` `dispatch_app_menu_action` |
| MENU-02 | P1 | M | Fixed 96d75f4b (not Terminal/Calculator) | Mac: every app has a standard Edit menu (Undo/Redo/Cut/Copy/Paste/Select All, plus Find/Spelling for text apps). / Lulo: Text Editor's Edit has only Find items; Notes only Find…; Settings/System Monitor/Clock have none. | `crates/rmac-app-menu/src/lib.rs`; `crates/rmac-ui/src/text_keys.rs` |
| MENU-03 | P1 | M | Fixed 1b265871 | Mac: menu items grey out when they don't apply. / Lulo: `definition_for_vocabulary` sets `enabled: true` on everything; there's no republish signal on the D-Bus interface. | `crates/rmac-app-menu/src/lib.rs` |
| MENU-04 | P1 | M | Fixed 1b265871 | Mac: menus have submenus (Open Recent, Sort By, Font, …) and checkmarks on the current choice. / Lulo: the wire format has no submenu and no checked state; radio groups are flattened into separate items; 8-menu/32-item cap. | `WireItem` wire format, `shell/bins/rmac-menubar` |
| MENU-05 | P1 | S (About), S each (Settings…) | Fixed 96d75f4b | Mac: app menu has About App, Settings… ⌘, when the app has settings, Services ›, and Quit and Keep Windows ⌥⌘Q. / Lulo: About is always disabled, no app shows Settings…, Services is disabled, no Quit and Keep Windows. Terminal binds ⌘, to its profile picker but shows no row for it. | `app_menu` in `shell/bins/rmac-menubar/src/main.rs`; `crates/rmac-apps` |
| MENU-06 | P2 | S | Missing | Mac: every app has a Help menu (App Help ⌘? plus search). / Lulo: only Files has one, with no shortcut. | `rmac-app-menu` |
| MENU-07 | P2 | S | Missing | Mac: menu hints always match the real binding. / Lulo: several mismatches, e.g. System Monitor's Quit Process shows "Delete" though ⌘⌫ is bound (Mac: ⌥⌘Q); Files' Rename has no ↩ hint; Quick Look shows "Space" where Finder shows ⌘Y. | `crates/rmac-app-menu/src/lib.rs` |
| MENU-08 | P1 | M | Fixed 96d75f4b | Mac: an app is single-instance; a second document opens as a new window of the running app. / Lulo: Text Editor, Notes and Preview start a second process for a second file, and that process can't own the menu bus name, so its window has no menu bar at all. | `boot_unified_app_instance_with_assets` (`crates/rmac-ui/src/window.rs:518`); `crates/text-editor`, `crates/preview`, `crates/notes` |
| MENU-09 | P2 | S | Missing | Mac: Enter Full Screen fn F is in every View menu. / Lulo: no app exports it. | standard View-menu tail item |
| MENU-10 | P1 | S | Fixed 9e650646 | Mac: ⌘F in a document's text opens Find. / Lulo: probable bug — `text_keys.rs:60` binds ⌘F to gpui-component's Search inside text fields, which returns early on a non-searchable field, so ⌘F while typing in Text Editor's body or a Notes field likely does nothing. Not yet confirmed live. | `crates/rmac-ui/src/text_keys.rs` |
| KB-05 | P1 | S | Missing | Mac: keyboard shortcuts always show the Mac's glyphs and match the live binding (general contract, see MENU-07 for concrete instances). | `crates/rmac-app-menu/src/lib.rs` |

### Control Center

Panel position/width (287 px): see FEEL_SPEC.md §4.9 and
docs/macos-parity-spec.md §4.9.

| ID | Sev | Size | Status | Gap | Where |
|---|---|---|---|---|---|
| CC-01 | P1 | S | Fixed 918aaa38 (niri blur; verify live) | Mac: modules are frosted glass, so nothing behind them can be read. / Lulo: modules are translucent without blur — the desktop folder icon and its "untitled folder" label show sharply through the Bluetooth module. | `crates/quick-settings-app/src/main.rs:108` (Blurred background), `render/cards.rs` |
| CC-02 | P2 | S | Partial | Mac: Now Playing is always shown as a 2×2 tile, reading "Not Playing" with dimmed transport controls. / Lulo: the tile is hidden when no MPRIS player exists, so the grid is shorter (4 rows vs the Mac's 5). | `crates/quick-settings-app/src/view.rs:33`, `render.rs:179` |
| CC-03 | P2 | M | Missing | Mac: an "Edit Controls" pill under the modules adds, removes and rearranges controls. / Lulo: the set is fixed. | `crates/quick-settings-app/src/render.rs` |
| CC-04 | P2 | S | Missing | Mac: panel 287 px wide (spec). / Lulo: panel rendered 380 px wide (≈93 px too wide), top ≈43 px too low, as of the 2026-09-19 capture — not re-measured since. | `crates/quick-settings-app` |
| CC-05 | P1 | M | Missing | Mac: date/time click and a keyboard path both open Control Centre / Notification Centre. / Lulo: only the click path works; no key opens either. | `shell/bins/rmac-menubar`, `crates/rmac-quick-settings` |
| CC-06 | P2 | L | Missing | Mac: Control Centre has Screen Mirroring. / Lulo: absent — niri has no mirroring protocol yet. | `crates/quick-settings-app` |

### Notification Center

Panel position/width (346 px): see FEEL_SPEC.md §4.10 and
docs/macos-parity-spec.md §4.10.

| ID | Sev | Size | Status | Gap | Where |
|---|---|---|---|---|---|
| NC-01 | P1 | M | Fixed d3a8f776 | Mac: every card and banner shows the sending app's icon. / Lulo: every card shows a grey "A" monogram, even with `notify-send -a "Text Editor" -i org.rmac.TextEditor -h string:desktop-entry:org.rmac.TextEditor` set correctly; Ubuntu's update notices look the same. | `crates/notification-center-app/src/model.rs:136`, `:194`, `:238` |
| NC-02 | P1 | S | Fixed d3a8f776 | Mac: a card shows its time top right ("Yesterday, 1:45 PM") and a banner shows "now". / Lulo: no card or banner shows a time — `posted_unix_ms` lives only in an in-memory origin map, so it's lost and never set for these senders. | `crates/rmac-notifications-linux/src/origin.rs:30`, `crates/notification-center-app/src/render/history.rs:162` |
| NC-03 | P1 | S (depends on NC-01) | Fixed d3a8f776 | Mac: an app's notifications stack into one group with a count and Show less. / Lulo: 4 "Build finished" cards and 2 "Updates Available" cards listed one by one, because unresolved senders (NC-01) each form their own group — the beta list's "grouping Works" claim is wrong in practice. | `crates/notification-center-app/src/model.rs:270` |
| NC-04 | P2 | S | Partial | Mac: a long list scrolls inside the column and ends with Edit Widgets. / Lulo: the column is cut off at the bottom edge in mid-card, with no fade. | `crates/notification-center-app/src/model.rs:306` |
| NC-05 | P2 | S | Partial | Mac: banner glass blurs the desktop. / Lulo: the desktop folder icon shows through the 344 pt banner card; width and position match. | `crates/notification-center-app/src/daemon/surface.rs:115` |
| NC-06 | P2 | S | Missing | Mac: panel 346 × 418 px, floating card stack. / Lulo: panel rendered 450 × 877 px (≈104 px too wide, ≈459 px too tall) with an opaque rail rather than floating cards, as of the 2026-09-19 capture — not re-measured since. | `crates/notification-center-app` |
| NC-07 | P1 | M | Missing | Mac: a key opens Notification Centre. / Lulo: no key does (see CC-05). | `crates/notification-center-app` |

### Spotlight

Field/panel geometry (642 × 59 pill): see FEEL_SPEC.md §4.7/§D and
docs/macos-parity-spec.md §4.8. Spotlight must stay labelled "Search" with no
AI/assistant row — that's a design rule, not a gap.

| ID | Sev | Size | Status | Gap | Where |
|---|---|---|---|---|---|
| SPOT-01 | P2 | S | Partial | Mac: the empty bar (637 pt wide, top at 20% of screen height) matches; typing "cal" completes inline to "calculator — Open" and the top hit shows "Search Calculator ⇥". / Lulo: geometry matches (640 pt, 20%); the ⇥ search-in-app hint wasn't found in code, and typed-query behaviour couldn't be driven live to confirm inline completion. | `crates/launcher-app/src/view/completion.rs`, `view/render/results.rs` |
| SPOT-02 | P2 | M | Missing | Mac: a per-app chips row under the field ("Calculator", "Photos") and Quick Keys. / Lulo: neither exists. (Siri Suggestions and web results are intentionally never added — see design rule above.) | `crates/launcher-app` |

### Launchpad

The Mac's Launchpad-equivalent panel (App Drawer) is 842 × 576 pt; see
FEEL_SPEC.md and docs/macos-parity-spec.md §4 for the general overlay
contract.

| ID | Sev | Size | Status | Gap | Where |
|---|---|---|---|---|---|
| APPS-01 | P1 | S | Fixed b8179d99 | Mac: Apps opens with a row of 7 recently used/suggested apps above a divider, then A–Z. / Lulo: A–Z only. | `crates/app-drawer/src/view.rs:52` |
| APPS-02 | P2 | M | Partial | Mac: panel 842 × 576 pt, top at 20% of screen height, 60 pt icons on a 115 pt pitch, placeholder "Applications" with a ⋯ menu. / Lulo: 760 × 520 pt, top at 16.5%, 54 pt icons on a 96 pt pitch, placeholder "Search Apps", and a non-Mac grid/list segmented control. | `crates/app-drawer/src/view.rs:19-22`, `view/lifecycle.rs:11` |
| APPS-03 | P2 | S | Partial | Mac: category chips with no "All" chip; the unfiltered view is the default. / Lulo: the first chip is an "All" pill, and Ubuntu tools (IBus Preferences, Firmware Updater, Sysprof, Logs, Advanced Network…) sit in the main grid. | `crates/app-drawer/src/view.rs:52` |

### Mission Control/App Switcher

⌘Tab per-app switching, ⌃↑/⌃↓/⌃←/⌃→ are implemented and working
(`shell/bins/rmac-app-switcher`, `shell/bins/rmac-mission-control`, ADR 0014).

| ID | Sev | Size | Status | Gap | Where |
|---|---|---|---|---|---|
| MC-01 | P1 | M | Fixed 0fad56ed (cached last picture) | Mac: every window in Mission Control is a live thumbnail. / Lulo: Text Editor's window was drawn as a blank dark rectangle with the app icon and a pill "Untitled — Text Edit…" in the middle, while Calculator's picture was real. | `shell/bins/rmac-mission-control/src/capture.rs:135` |
| MC-02 | P1 | S | Fixed 0fad56ed | Mac: Show Desktop (F11) slides every window to the screen edges, leaving slivers. / Lulo: F11 switches to an empty workspace and the windows vanish — the older inventory's "75 px slivers" claim doesn't apply to F11. | `shell/bins/rmac-mission-control/src/main.rs:1471` |
| WIN-04 | P2 | S–M | Missing | Mac's Mission Control section has Automatically rearrange Spaces, Switch to a Space with the app's windows, Group windows by application, Displays have separate Spaces, Drag windows to top to enter Mission Control, plus Shortcuts…. / Lulo: only Hot Corners is exposed. | `crates/system-settings/src/controller/desktop_dock.rs:229` |
| MC-03 | P2 | M | Missing | Mac: the overview preview is ≈1033 px wide with a titled Spaces pill (75 × 24 px) per Space. / Lulo: preview rendered 945 × 450 px (≈507 px too short, ≈88 px too narrow), top ≈256 px too low, no Spaces title pill, as of the 2026-09-19 capture — not re-measured since. | `shell/bins/rmac-mission-control` |
| MC-04 | P2 | M | Missing | Mac: windows can be dragged between Spaces, the Dock stays visible in Mission Control, and a three-finger swipe opens it. / Lulo: no cross-Space drag; three-finger gestures are owned by niri and unwired (thumbnail accuracy itself is MC-01). | `shell/bins/rmac-mission-control` |

### Desktop

Icon grid geometry (64 px icons, 112 px pitch) and window chrome (corner
radius, traffic lights, shadows) are in FEEL_SPEC.md and
docs/macos-parity-spec.md §4; those pass their measured comparisons as of the
2026-09-19/24 captures except where noted below.

| ID | Sev | Size | Status | Gap | Where |
|---|---|---|---|---|---|
| DESK-01 | P1 | M | Fixed (Share/Tags skipped) | Mac: a desktop item's menu has Open, Open With ▸, Move to Bin, Get Info, Rename, Compress "X", Duplicate, Make Alias, Quick Look, Copy, Share…, tag colours and Tags…. / Lulo: Open, Move to Trash, Get Info, Rename and Duplicate only. | `shell/bins/rmac-wallpaper/src/linux_wayland/menu.rs:156` |
| DESK-02 | P2 | S | Partial | Mac: Sort By offers None, Snap to Grid, Name, Kind, Date Last Opened, Date Added, Date Modified, Date Created, Size, Tags. / Lulo: Date Last Opened, Date Added, Date Created and Tags are missing. | `shell/bins/rmac-wallpaper/src/linux_wayland/menu.rs:178` |
| WIN-02 | P1 | M | Fixed b4a81749 | Mac: new windows are clamped between the menu bar and the Dock. / Lulo: System Settings opens 832 pt tall on an 864 pt output, so its bottom 70 pt sits under the Dock even with Reserve screen space on. | `crates/system-settings/src/controller/settings_style.rs:24` |
| WIN-03 | P2 | M | Missing | Mac's Desktop & Dock › Windows has Prefer tabs when opening documents, Ask to keep changes when closing documents, Close windows when quitting an application, Drag windows to left/right edge to tile, Drag windows to menu bar to fill screen, Hold ⌥ while dragging to tile, Tiled windows have margins. / Lulo: none of these. | `crates/system-settings/src/controller/desktop_dock.rs:201` |
| DESK-03 | P2 | M–L | Missing | Mac: dragging a window to a screen edge tiles it; the green button's hover menu offers quarters and Arrange. / Lulo: the hover menu has only halves, Fill and Full Screen; no edge-drag tiling. | `crates/rmac-ui/src/chrome.rs` `zoom_menu` |
| DESK-04 | P2 | M | Missing | Mac: desktop icons support rename, Quick Look, video posters, dragging onto the Dock/Trash/Files, a label text shadow. / Lulo: none of these — dragging out to other surfaces is blocked by a GPUI limitation (no drag-out between windows), and GPUI text has no shadow support. | `crates/rmac-desktop`, `shell/bins/rmac-wallpaper` |
| DESK-05 | P2 | S–M | Missing | Mac: desktop/Notification-Centre widgets come in Small/Medium/Large, include Reminders/Photos/Notes/Calendar Up Next, and the gallery has a search field. / Lulo: small size only, Clock/Calendar/Weather/Batteries only (no data source for the rest), and the widget gallery has no search field (no shell text field there yet). | `crates/rmac-desktop-widgets` |
| DESK-06 | P2 | M | Missing | Mac: toolbar windows (Finder, Settings, Activity Monitor, Calculator, Preview) use a ≈27 pt corner radius; title-bar-only windows (TextEdit) use 16 pt. / Lulo: one radius (16) for every window type, as of the 2026-09-23 measurement. | `crates/rmac-design` |

### Lock/Login

Lock-screen require-password delay is tracked as SET-08 in Settings.

| ID | Sev | Size | Status | Gap | Where |
|---|---|---|---|---|---|
| LOCK-01 | P1 | M | Fixed 126baaf8 | Mac: the lock screen shows the user's own wallpaper and account picture. / Lulo: paints the built-in Aurora gradient under a 20% veil and a grey monogram disc, whatever the wallpaper or AccountsService icon actually is. | `crates/rmac-lock-provider-linux/src/paint.rs:97`, `:476`, `:484` |
| LOCK-02 | P2 | S | Missing | Mac: the lock screen keeps battery, Wi-Fi and input-source items at top right. / Lulo: only the date, clock, avatar, name and password field. | `crates/rmac-lock-provider-linux/src/paint.rs:23` |
| PWR-01 | P1 | S | Fixed 77ae35cd + d3d4e833 (fail-safe) | Mac: a short press of the power key sleeps or locks, and never powers off. / Lulo: logind's `HandlePowerKey=poweroff` (the Ubuntu default) shuts down at once; nothing in rmac overrides it. | `docs/decisions/0020-unsaved-work-at-session-end.md:117`, `packaging/rmac-session` |
| PWR-02 | P2 | S | Partial | Mac: "Are you sure you want to shut down your computer now?" / "…restart…" / "…quit all applications and log out now?". / Lulo: "Shut down this computer?" / "Restart this computer?" / "Log out now?" — plus no countdown and no reopen-windows checkbox (LOGIN-03). | `shell/bins/rmac-menubar/src/main.rs:2653` |
| LOGIN-01 | P2 | M | Missing | Mac: login screen is a centred avatar and name, a password pill over a blurred wallpaper, a large clock. / Lulo: a GDM theme only, not Mac-styled. | `packaging/rmac-session/greeter/` |
| LOGIN-02 | P1 | L | Partial | Mac: first boot creates the user account. / Lulo Setup Assistant only renames the already-signed-in user (`SetRealName`/`SetIconFile`); it can't create one. | `crates/setup-assistant/src/services.rs:94-110` |
| LOGIN-03 | P2 | M | Missing | Mac: the log-out confirmation counts down, and login offers "Reopen windows when logging back in". / Lulo: the confirmation is a static inline menu panel (see PWR-02); no reopen-windows option. | `shell/bins/rmac-menubar/src/main.rs` |

### Keyboard shortcuts

Text-field Mac keys (⌥←/→, ⌘←/→, ⌘A/C/V/X/Z, ⌃A/⌃E, ⌃K/⌃D/⌃H/⌃F/⌃B) are bound
and working (`crates/rmac-ui/src/text_keys.rs`).

| ID | Sev | Size | Status | Gap | Where |
|---|---|---|---|---|---|
| WIN-01 | P1 | S | Fixed 0fad56ed (Ctrl+Super) | Mac: 🌐⌃F Fill, 🌐⌃C Centre, 🌐⌃←→↑↓ halves, 🌐⌃R Return to Previous Size. / Lulo: no keyboard tiling bindings at all. | `packaging/rmac-session/shell.kdl:237` |
| KB-01 | P2 | M | Missing | Mac: ⇧⌘Q opens the log-out confirmation from the keyboard. / Lulo: not bound; the confirmation lives in the menu-bar popover with no key to open it. | `shell/bins/rmac-menubar/src/main.rs`, `crates/rmac-shortcuts/src/model.rs` |
| KB-02 | P2 | L | Missing | Mac: ⌃⌘Space / fn E opens the emoji & symbol picker. / Lulo: the binding exists but GPUI's Linux `show_character_palette` does nothing — no overlay exists. | `crates/rmac-ui/src/text_keys.rs` |
| KB-03 | P2 | S | Missing | Mac: ⌃Y yanks the kill ring; ⌃N/⌃P move by line, in every text field. / Lulo: absent (the other Emacs keys were added already). | text field key handling |
| KB-04 | P2 | S–M | Missing | Mac: keyboard-backlight keys (`XF86KbdBrightnessUp/Down`) are bound and show an OSD row. / Lulo: not bound; no OSD row. | `crates/rmac-osd/src/lib.rs`, `linux.rs` |

### Accessibility

Screen-reader identity/role/state, focus rings and reduced-motion plumbing
are tracked in `docs/beta-checklist.md` §3, which stays a release checklist
(see the note at the end of this document); the rows below are the
concrete, Mac-relevant surfaces that are still unusable with Orca/keyboard
only.

| ID | Sev | Size | Status | Gap | Where |
|---|---|---|---|---|---|
| ACC-01 | P0 | L | Broken | Mac Terminal exposes its text buffer, caret and selection to accessibility APIs. / Lulo Terminal publishes **no** AT-SPI text, caret or selection at all — the accessibility module exists but isn't compiled into the running binary, so typing a command over AT-SPI is impossible. | `crates/terminal` |
| ACC-02 | P0 | L | Broken | Mac Notes exposes its note list, sidebar and every text field. / Lulo Notes has no text-entry AT-SPI surface (search/title/tags/body expose no Text/EditableText) and the note list/sidebar is entirely absent from the AT-SPI tree. | `crates/notes` |
| ACC-03 | P0 | M | Broken | Mac Finder exposes each file/folder row and supports accessible rename. / Lulo Files exposes only toolbar controls over AT-SPI — no file/folder/sidebar row — and rename needs a keyboard injector that isn't installed. | `crates/finder` |
| ACC-04 | P1 | — | Broken | Mac Spotlight's query field accepts synthetic typing over accessibility APIs. / Lulo's Spotlight field can't be typed into over AT-SPI: the pinned `accesskit_unix` bridge has no `EditableText` implementation. Upstream gap, not fixable in this repo. | `accesskit_unix` (pinned dependency) |
| ACC-05 | P1 | M | Missing | Mac: Control-F2 moves keyboard focus to the menu bar. / Lulo: explicitly not implemented (`docs/known-limitations.md`). | `shell/bins/rmac-menubar` |
| ACC-06 | P2 | L | Missing | Mac: Zoom (⌃-scroll) magnifies the screen. / Lulo: no screen zoom — niri has no zoom API yet. | `crates/system-settings/.../screen_reader.rs:124` |
| A11Y-01 | P2 | S | Partial | Mac: VoiceOver announces the app as "Calculator". / Lulo: AT-SPI `Application.Name` is the executable file name ("rmac-calculator"), because `accesskit_unix`'s private `app_name()` uses `current_exe()`; toolkit identity is now fixed (gpui_linux 0.1.0). Fix: a minimal `[patch]` fork of `accesskit_unix` that reads the name from an env var set by `rmac_ui` before the first window. See the ADR 0013 amendment for details. | `shell/compat/gpui_linux`, `accesskit_unix 0.22.1 context.rs` |

---

## Apps

### Files

Toolbar/sidebar/window geometry for the four views largely matches the Mac
as of the 2026-09-24 Files-pass capture; the rows below are functional gaps.

| ID | Sev | Size | Status | Gap | Where |
|---|---|---|---|---|---|
| FILES-01 | P2 | M | Missing | Mac: toolbar has Share and Tags buttons. / Lulo: absent by design pending a share-sheet portal and FILES-20. | `crates/finder/src/view/chrome_presentation/toolbar.rs:158` |
| FILES-02 | P2 | M | Missing | Mac: group-by menu has Use Groups ⌃⌘0 then Sort By › (None, Name, Kind, Date Last Opened/Added/Modified/Created, Size, Tags). / Lulo: Sort By Name/Date Modified/Size/Kind only, no grouping, and reselecting the current key silently reverses order. | `menus_tabs.rs:4-21`, `selection_controller.rs:324` |
| FILES-03 | P2 | S | Missing | Mac: search collapses when the query clears or on Esc. / Lulo: `search_open` is only ever set true — it never collapses. | `toolbar.rs:219`, `startup.rs:290` |
| FILES-04 | P1 | M | Fixed 90c50f25 (built-ins not removable) | Mac: Add to Sidebar ⌃⌘T, drag-to-reorder Favourites, Remove from Sidebar. / Lulo: Favourites are hard-coded; dropping a folder on the sidebar moves it into that row instead; no sidebar right-click menu. | `crates/finder/src/places.rs:45-61`, `view/sidebar.rs` |
| FILES-05 | P1 | M (with FILES-20) | Missing | Mac: the sidebar always has a Tags section. / Lulo: built only under `cfg(target_os = "macos")`, so it's absent. | `crates/finder/src/view/startup.rs:107-119`, `crates/rmac-search/src/platform.rs:106` |
| FILES-06 | P2 | S | Missing | Mac: the selected sidebar row follows the current view (Recents lights up while Recents shows). / Lulo: rows compare only `cwd == path`; Recents and tag views never highlight and the old folder stays lit. | `view/sidebar.rs:7-11` |
| FILES-07 | P2 | L | Missing | Mac: sidebar lists Network and connected servers. / Lulo: neither exists, nor Connect to Server (FILES-31). | new |
| FILES-08 | P1 | M | Missing | Mac: list view's disclosure triangles expand folders inline, with ⌘→/⌘←. / Lulo: the chevron is drawn but has no click handler. | `crates/finder/src/view/list_presentation.rs:316-334` |
| FILES-09 | P2 | M | Missing | Mac: list-view columns are resizable, reorderable and choosable (right-click the header); Calculate all sizes exists. / Lulo: four fixed columns (Name, Date Modified, Size, Kind), fixed widths, no chooser. | `list_presentation.rs:130-173` |
| FILES-10 | P0 | S | Fixed 39b8633d | Mac: in column view ↑↓ move within a column, ←→ move between columns. / Lulo: arrow keys and type-to-select act on `entries` while column view reads `column_selection`, so arrows clear the selection; ←/→ aren't handled at all. | `list_presentation.rs:856-901`, `selection_controller.rs:37-68`, `horizontal_navigation` (`:450`) |
| FILES-11 | P2 | S | Missing | Mac: column-view preview shows Information (Created/Modified/Last opened), Tags, Show More. / Lulo: Information holds only Modified. | `content_presentation.rs:7-83` |
| FILES-12 | P2 | S | Missing | Mac: gallery inspector has Information, a Tags field, Markup and More… quick actions. / Lulo: shows Kind/Size/Modified only, no Tags, no quick actions. | `gallery_presentation.rs` |
| FILES-13 | P1 | M | Missing | Mac: Show View Options ⌘J opens a floating panel with per-view controls (icon size, columns, grouping, etc.), remembered per folder. / Lulo: no panel and no ⌘J. | new overlay in `crates/finder/src/view/`; `presentation_persistence.rs` |
| FILES-14 | P2 | S (menu items) / M (preview pane) | Missing | Mac: Path Bar ⌥⌘P, Status Bar ⌘/, Sidebar ⌃⌘S, Preview ⇧⌘P and Toolbar ⌥⌘T all toggle from the View menu and are remembered. / Lulo: path bar and sidebar toggle only from the keyboard, not the menu; status bar can't be toggled; no preview pane; nothing is remembered. | `FILES_MENUS`; `presentation_persistence.rs:29-44` |
| FILES-15 | P1 | M | Missing | Mac: Get Info is a separate, non-modal window, one per item for a multi-selection; Show Inspector ⌥⌘I and Get Summary Info ⌃⌘I exist. / Lulo: a modal card, one at a time, first item only. | `crates/finder/src/view/search_info_controller.rs:187-411` |
| FILES-16 | P1 | S | Missing | Mac: folder size is calculated with item count. / Lulo: no size for folders. | `filesystem_helpers.rs:200-226` |
| FILES-17 | P2 | S | Missing | Mac: Where is a breadcrumb path. / Lulo: a raw parent path. | same |
| FILES-18 | P1 (Open with, permissions) / P2 (rest) | M | Missing | Mac Get Info has Tags, Hide extension, Comments, Locked, Open with (+ Change All…), editable permissions. / Lulo has none of these; permissions are a read-only `rwxr-xr-x` string. | same |
| FILES-19 | P2 | S | Missing | Mac Get Info sections collapse. / Lulo's are flat. | same |
| FILES-20 | P1 | L | Missing | Mac: colour tags with names, from the right-click row, toolbar, Get Info and gallery; sidebar filter; list-view column; sort/group by tag. / Lulo: files can't be tagged on Linux at all — nothing writes xattrs and there's no tag UI. | new: `user.xdg.tags` xattr in `crates/rmac-search` or `crates/finder` |
| FILES-21 | P2 | M (needs MENU-04) | Missing | Mac: Open With is a submenu (default app first, then others, then Other…). / Lulo: a dialog. | `crates/finder/src/view/dialog_presentation/open_with.rs` |
| FILES-22 | P2 | S | Missing | Mac: Make Alias ⌃⌘A, Show Original ⌘R. / Lulo: absent (symlinks are the natural backing). | `menus_tabs.rs`, `startup/shortcuts.rs` |
| FILES-23 | P2 | M–L | Missing | Mac context menu has Share…, tag dots, Tags…, Quick Actions ›, Services ›. / Lulo has none. | as above |
| FILES-24 | P2 | S | Missing | Mac: a folder's menu adds Open in New Tab; a multi-selection adds New Folder with Selection. / Lulo: absent. | `menus_tabs.rs` |
| FILES-25 | P2 | S | Missing | Mac background menu has Get Info, Use Groups, a checked Sort By submenu, Show View Options. / Lulo has flat sort items with no check and non-Mac "View as …" rows, and no Get Info for the folder itself. | `menus_tabs.rs` |
| FILES-26 | P2 | S (hide) / M (batch rename) | Missing | Mac disables Rename for a multi-selection (it's a batch "Rename N Items…" instead). / Lulo lists Rename and shows a notice when chosen. | `rename_controller.rs:7-19` |
| FILES-27 | P1 | S | Fixed fb6bc85a | Mac: with one file selected, ↑/↓ (or ←/→ in icon view) keep moving the selection and Quick Look follows. / Lulo: arrows only step through the already-selected items, so a single selection can't browse the folder. | `crates/rmac-quick-look/src/lib.rs:7`, `crates/finder/src/view/quick_look_controller/controller.rs` |
| FILES-28 | P2 | S (⌘Y) | Missing | Mac: ⌘Y opens Quick Look; the panel has Share, Markup, Rotate. / Lulo: ⌘Y isn't bound; no Share/Markup/Rotate. | `startup/shortcuts.rs`; `rmac-quick-look/src/panel.rs` |
| FILES-29 | P0 | S–M | Fixed 7d843483 | Mac: ⌘N opens an independent new window on the "New Finder windows show" folder; ⌘W closes the tab or the window. / Lulo: a new window restores the *other* window's tabs (one shared state file, last write wins); ⌘W with one tab does nothing; launch always restores the last folder, never Recents. | `crates/finder/src/view/startup.rs:175-187`, `view.rs:379`, `navigation.rs:45-47`, `presentation_persistence.rs:161-234` |
| FILES-30 | P2 | S | Missing | Mac: ⌘-double-click opens a folder in a new tab; ⌃⌘O, ⇧⌘T, ⇧⌘\\, ⇧⌘[/⇧⌘] all work; tabs are titled with folder names. / Lulo: ⌘-click only toggles selection; none of those shortcuts exist; the tab name falls back to a hard-coded "Macintosh HD". | `selection_controller.rs:13-17`, `menus_tabs.rs:169` |
| FILES-31 | P1 (Open, Duplicate, Find, Close Window, Eject) / P2 (rest) | S (bound actions) / M (rest) | Partial 93926def | Mac File menu has Open ⌘O, Open in New Tab, Close Window ⌘W/Close All ⌥⌘W, Show Inspector, Get Summary Info, Duplicate ⌘D, Make Alias, Quick Look ⌘Y, Print ⌘P, Share…, Add to Sidebar/Dock, Delete Immediately ⌥⌘⌫, Eject ⌘E, Find ⌘F, New Folder with Selection, New Smart Folder. / Lulo's File menu is missing all of these (some are bound but not shown in the menu). | `FILES_MENUS` |
| FILES-32 | P1 (Redo, ⌥⌘C, ⌥⌘V) | M (redo) / S (rest) | Partial 93926def | Mac Edit menu has Redo ⇧⌘Z, Copy as Pathname ⌥⌘C, Move Item Here ⌥⌘V, Deselect All ⌥⌘A, Show Clipboard. / Lulo has none — no redo at all. | `FILES_MENUS` |
| FILES-33 | P2 (P1 for ⌘J, see FILES-13) | S | Missing | Mac View menu has Use Groups, a checked Sort By submenu, Clean Up/Clean Up By, Show Tab Bar, Show All Tabs, Hide Sidebar ⌃⌘S, Show Preview, Hide Toolbar, Show Path Bar, Hide Status Bar, Customise Toolbar…, Show View Options ⌘J, Enter Full Screen. / Lulo's View menu has none of these rows (some bound but hidden). | `FILES_MENUS` |
| FILES-34 | P1 (⇧⌘D, ⇧⌘O, ⇧⌘F) | S | Partial 93926def | Mac Go menu has Enclosing Folder in New Window, Recents ⇧⌘F, Documents ⇧⌘O, Desktop ⇧⌘D, Computer ⇧⌘C, Network ⇧⌘K, Utilities ⇧⌘U, Library, Recent Folders ›, Connect to Server… ⌘K. / Lulo's Go menu is missing all of these. | `FILES_MENUS` |
| FILES-35 | P1 | M | Missing | Mac Finder app menu has Settings… ⌘, (New windows show, Open folders in tabs, Tags, Sidebar checkboxes, show extensions, Bin warnings, keep folders on top). / Lulo has none. | new pane in `crates/finder` |
| FILES-36 | P2 | S | Missing | Mac Finder app menu has Empty Bin (no confirmation) ⌥⇧⌘⌫. / Lulo: absent. | `FILES_MENUS` |
| FILES-37 | P1 (⌘F, scope) | S (⌘F) / M (scope) | Missing | Mac: ⌘F opens search with a "This Mac / '<folder>'" scope bar, a + filter button, and Smart Folders. / Lulo: ⌘F isn't bound (search opens only from the toolbar circle); no scope bar (current folder only); no filters. | `crates/finder/src/view/startup/shortcuts.rs`, `search_info_controller.rs:23-94` |

### Settings

Window/sidebar geometry matches the Mac (215 pt sidebar in a 223 pt column,
32 pt rows, 52 pt toolbar). Trackpad/Mouse-pane geometry is from
`docs/parity-audit-2026-09-24-apps.md §2.3` knowledge captures, marked (K)
there.

| ID | Sev | Size | Status | Gap | Where |
|---|---|---|---|---|---|
| SET-01 | P1 (⌘F) / P2 | S | Fixed c212e16e | Mac View menu has Forward ⌘], Search ⌘F, and every pane by name. / Lulo: Forward is bound in the window but not the menu; ⌘F isn't bound at all; no pane list. | `SETTINGS_MENUS`; `crates/system-settings/src/controller.rs:282-304` |
| SET-02 | P2 | S | Missing | Mac Back/Forward move through pane history (Wi-Fi → Sound → Back returns to Wi-Fi). / Lulo: Back/Forward only work inside a pane's own subpages; changing pane clears both stacks. | `controller/navigation_state.rs:143-243` |
| SET-03 | P2 | M | Missing | Mac sidebar search finds settings *inside* panes and highlights the matching control. / Lulo filters pane names/keywords only, and some keywords ("Dock size", "password required") promise controls that don't exist yet. | `crates/system-settings/src/settings_search.rs` |
| SET-04 | P2 | S (point at SET-06) | Missing | Mac's account card opens Apple Account / Users & Groups. / Lulo's card isn't clickable. | `controller/chrome.rs:213-253` |
| SET-05 | P1 | M | Missing | Mac has a Login Password pane (Touch ID & Password): change password, require password after sleep. / Lulo: no such pane; the first password change after setup has no home. | new pane; AccountsService + `passwd` over PAM |
| SET-06 | P1 | L | Missing | Mac has Users & Groups: list/add/remove users, change picture, automatic login, guest user. / Lulo: absent. | new pane; AccountsService D-Bus |
| SET-07 | P1 | M | Missing | Mac has Printers & Scanners: list/add printers, default printer, paper size, print queue. / Lulo: printing works through the portal but printers can't be managed. | new pane; CUPS D-Bus / `lpadmin` via polkit, `crates/rmac-print-linux` |
| SET-08 | P1 (require-password delay) / P2 (rest) | M | Missing | Mac Lock Screen pane has a require-password delay (Immediately…8h), large clock toggle, login-window mode, Sleep/Restart/Shut Down button toggle. / Lulo has two popups (lock/sleep-when-inactive) and a Lock Now button only. | `controller/lock_screen/render.rs` |
| SET-09 | P2 | M | Missing | Mac has a Screen Saver pane under Wallpaper. / Lulo: none. | new |
| SET-10 | P2 (Time Machine P1 but L) | L | Missing | Mac has Startup Disk, Transfer or Reset, Time Machine. / Lulo: none — a Déjà-Dup-style backup is expected by Mac users. | new |
| SET-12 | P1 | S (route to existing NetworkService subpage) | Fixed 0655b747 | Mac's Wi-Fi Details… sheet has TCP/IP, DNS, Proxies, Low Data Mode, Auto-Join, Forget This Network. / Lulo's current-network row is display-only; the Network › service page has the editor but Wi-Fi doesn't link to it. | `controller/wifi/render.rs:71-86`, `controller/network/render.rs` |
| SET-13 | P2 | S | Missing | Mac's known-network ⋯ menu has Auto-Join and Forget, plus Ask-to-join popups and an Advanced… sheet (auto-join list, admin-auth requirement, MAC address). / Lulo has Forget only. | `controller/wifi/render.rs` |
| SET-14 | P1 | S | Partial (scan on open, 11e21d04) | Mac scans for Wi-Fi networks continuously with a spinner. / Lulo needs a manual Refresh button; first open showed "No other networks found". | `controller/wifi` (subscribe NM `AccessPointAdded` / periodic `RequestScan`) |
| S10 | P2 | M | Missing | Mac: join a hidden network via Other…/Join Other Network. / Lulo: no such path at all. | `crates/system-settings/src/connectivity.rs`, `controller/wifi/credentials.rs`, `crates/rmac-network/src/linux.rs` (`AddAndActivateConnection`) |
| SET-15a | P2 | S | Missing | Mac Bluetooth is discoverable automatically while the pane is open, with no toggle. / Lulo has a manual Discoverable toggle. | `controller/bluetooth/render.rs:37` |
| SET-15b | P2 | S–M | Missing | Mac Bluetooth scans continuously, shows battery level per device, and has a per-device ⓘ sheet. / Lulo has a Refresh/Scanning button, no battery level, no ⓘ sheet. | `view_helpers/bluetooth.rs`; BlueZ `Battery1` |
| SET-16 | P1 | S | Fixed 11e21d04 | Mac Displays has a brightness slider for the built-in panel. / Lulo: absent — brightness keys work through the OSD (logind `SetBrightness`) but Settings has no slider. | `controller/displays/render/output.rs`; `crates/rmac-osd/src/linux.rs` |
| SET-17 | P1 | M | Missing | Mac has Night Shift… (schedule, warmth slider). / Lulo: absent. | wlsunset or gamma |
| SET-18 | P2 | M | Missing | Mac shows resolution as Larger Text ↔ More Space thumbnails (4–5 choices); "Show all resolutions" is in a ⋯ menu. / Lulo shows raw Resolution/Scale popups with the refresh rate baked into the label. | `output.rs:72-192` |
| SET-19 | P2 | M | Missing | Mac lets you drag display arrangement (Arrange…) and has refresh-rate/HDR/True Tone/Colour-profile controls. / Lulo's arrangement is a popup; no refresh-rate popup. | same |
| SET-20s | P2 | S (needs sound assets) | Missing | Mac has ≈14 named alert sounds. / Lulo has 3 (Alert, Error, Notification). | `controller/sound/render.rs:14-94`, `crates/rmac-sound` |
| SET-21s | P2 | M | Missing | Mac Output/Input are tabs over one device table, with a live input-level meter. / Lulo has separate stacked lists and no input meter. | `controller/sound/render.rs:166-304` |
| SET-22 | P2 | M (GPUI scrollbar policy) | Missing | Mac Appearance has "Show scroll bars" (3 choices) and "Click in the scroll bar to" (2 choices). / Lulo: absent. | `controller/appearance/render.rs` |
| SET-23 | P2 | M | Missing | Mac Appearance has Text highlight colour, Sidebar icon size, Folder colour, Icon & widget style, Liquid Glass Clear/Tinted. / Lulo: absent (Contrast/Motion sit here instead of in Accessibility). | same |
| SET-33 | P1 | S (route through `send_window_action`) / M (with the setting) | Partial | Mac has a "Window title bar double-click action" setting (Zoom/Minimise/Fill/Do Nothing). / Lulo's double-click calls GPUI's `zoom_window()`, the green button uses Fill instead, and there's no setting. niri's maximize handling for floating windows is unverified. | `crates/rmac-ui/src/chrome.rs` `client_bar` |
| SET-36 | P2 | S | Missing | Mac has a Widgets section (Show on Desktop, Dim widgets). / Lulo: absent, although desktop widgets exist. | `desktop_dock.rs` |
| SET-37 | P2 | S | Missing | Mac's Hot Corners… is a sheet with four popups around a screen picture, each taking a modifier key. / Lulo has four inline popups instead. | `desktop_dock.rs` |
| S16 | P2 | M | Missing | Mac: default web browser and mail app are chosen in Desktop & Dock. / Lulo: nowhere to choose either. | `crates/system-settings/src/controller/…`, `crates/rmac-apps/src/catalog.rs` (`xdg-mime default`) |
| SET-40 | P1 | M | Missing | Mac Keyboard Shortcuts… is a full sheet (13 categories, in-place editing, Modifier Keys… remapping). / Lulo has 2 read-only categories (Spotlight, Lock Screen); rebinding is only through the portal's own UI; screenshot/Mission Control/Dock keys aren't listed. | `crates/system-settings/src/controller/input/shortcuts.rs:11-26`, `crates/rmac-shortcuts` |
| SET-41 | P1 (accessibility) | M (focus rings in rmac-ui) | Missing | Mac has Keyboard navigation (Tab reaches every control). / Lulo: absent. | `controller/input/render.rs` |
| SET-42 | P2 | S (with S11) | Missing | Mac has a keyboard-brightness slider and backlight timeout. / Lulo: absent (the keys aren't bound either — see KB-04). | `render.rs` |
| SET-43 | P2 | S | Missing | Mac has a "Press 🌐/fn key to" popup. / Lulo: absent. | `render.rs` |
| SET-44 | P2 | M | Missing | Mac has Text Replacements… and Spelling/auto-capitalise/auto-full-stop/smart-quotes toggles under Input Sources › Edit…. / Lulo's Edit… just jumps to Language & Region. | `render.rs:162-183` |
| SET-46 | P2 | S | Missing | Mac key-repeat has 8 stops, delay has 6. / Lulo has 5 and 5. | `controller/input.rs:64-80` |
| SET-50 | P1 | M | Missing | Mac has a Trackpad "More Gestures" tab documenting every three/four-finger swipe with on/off and finger-count popups. / Lulo: absent — niri owns the gestures and nothing configures them from Settings. | `controller/input/render.rs:253-358`; niri `gestures` config |
| SET-51 | P1 | S | Fixed 3f75bc04 (2 libinput methods) | Mac Secondary click offers two-finger click/tap or a corner. / Lulo only offers "Primary click on right" (left-handed). libinput has `click-method`. | same; niri `click-method` |
| SET-52 | P2 | M | Missing | Mac Scroll & Zoom has Zoom in/out, Smart zoom, Rotate. / Lulo has Natural scrolling only. | same |
| SET-53 | P2 | S | Missing | Mac Mouse pane has Double-click speed and Scrolling speed. / Lulo: absent (niri has `scroll-factor`; double-click interval is per-toolkit). | `render.rs:188-251` |
| SET-54 | P2 | S | Missing | Mac orders Trackpad before Mouse and groups Click pressure/Force Click/Silent clicking under an implicit "point & click" section. / Lulo orders Mouse before Trackpad and has no Advanced… grouping for its Linux-only extras (Ignore-while-typing, Drag lock, Pointer acceleration, Middle-click emulation). | `navigation.rs:232-252`; `render.rs` |
| SES-01 | P2 | S | Fixed ca0728f4 | Mac: n/a. / Lulo: each System Settings launch leaves two `gsettings monitor` children orphaned after exit — 20 were found in one session, holding other tools' lock files. | `crates/rmac-gtk-settings/src/api.rs:95` |

### Text Editor

Window shape (656×422, monospaced plain text) matches the Mac.

| ID | Sev | Size | Status | Gap | Where |
|---|---|---|---|---|---|
| TE-01 | P1 | S | Missing | Mac: Save As… is ⌥⇧⌘S, and ⇧⌘S is Duplicate. / Lulo: ⇧⌘S is bound to Save As — a Mac user pressing ⇧⌘S expects a duplicate window. | `TEXT_EDITOR_MENUS`, `crates/text-editor/src/view/lifecycle.rs:48-93` |
| TE-02 | P1 | S (with MENU-04) | Missing | Mac has Open Recent › with Clear Menu. / Lulo: absent, though opened files are recorded. | `rmac-app-menu`, `crates/text-editor` |
| TE-03 | P0 (open) / P1 (edit) | S (open as plain text or strip with a pure-Rust parser) / L (rich editing) | Broken | Mac: rich text is the default; Bold/Italic/Underline/fonts/colours/alignment/lists/ruler. / Lulo is plain-text only, and opening an `.rtf` file fails outright with "could not be decoded safely" — RTF parsing is AppKit-only. | `crates/text-editor/src/view/render/rtf.rs`, `document_io.rs:43-45` |
| TE-04 | P1 | S (verify the installed package includes `ExportPdf`) | Missing | Mac's File menu has Export as PDF…. / Lulo has it implemented but it's missing from the live installed menu. | `crates/text-editor/src/main.rs:12-38`, `rmac-app-menu` |
| TE-05 | P2 | M | Missing | Mac has Revert To › Last Saved/Browse All Versions…, Rename…, Move To…, Duplicate. / Lulo only has a recovery draft; the title ▾ menu shows encodings only. | `view/render/chrome.rs:63-106` |
| TE-06 | P1 (case sensitivity) / P2 (rest) | S | Missing | Mac Find has Use Selection for Find ⌘E, Jump to Selection ⌘J, Select Line… ⌘L, and Ignore Case/Contains/Wrap Around in the find bar. / Lulo's find is case-sensitive substring only. | `crates/text-editor/src/view/editing.rs:18-23` |
| TE-07 | P1 | M | Missing | Mac has Spelling and Grammar (red underlines, right-click suggestions). / Lulo: absent. | — |
| TE-08 | P2 | S | Missing | Mac's Wrap to Page ⇧⌘W toggles window-width vs page-width wrap. / Lulo always wraps to the window. | `crates/rmac-editor/src/lib.rs:44-56` |
| TE-09 | P2 | S | Missing | Mac separates view zoom (⌘0/⇧⌘./⇧⌘,) from font size (⌘+/⌘−). / Lulo's ⌘+/⌘− change font size directly and don't persist; there's no ⌘0. | `editing.rs:164-172` |
| TE-10 | P2 | S | Missing | Mac has Page Setup… ⇧⌘P and Show Properties ⌥⌘P. / Lulo: absent. | — |
| TE-11 | P2 | S | Missing | Mac has Transformations (case changes), Substitutions (smart quotes/dashes), Complete ⌥⎋. / Lulo: absent. | `crates/text-editor/src/view/editing.rs` |
| TE-12 | P2 | M | Missing | Mac supports tabs (Show Tab Bar, Merge All Windows). / Lulo is one document per window. | — |

### Notes

The Mac's own note content wasn't captured (privacy); the rows below come
from the menu dump and toolbar accessibility tree.

| ID | Sev | Size | Status | Gap | Where |
|---|---|---|---|---|---|
| NOTES-01 | P1 (the single biggest felt difference) | L (WYSIWYG) / M (Markdown-marker shortcuts + live-styled view) | Missing | Mac: rich, styled notes with paragraph-style shortcuts (Title/Heading/Subheading/Body/Monostyled) and an "Aa" toolbar menu. / Lulo: a Markdown-source editor with separate Title/Body/Tags fields and a read-only preview toggle; no style commands. | `crates/notes/src/editor_presentation.rs:179-263`, `rmac-notes-storage/src/markdown_preview.rs` |
| NOTES-02 | P1 | M | Missing | Mac checklists are clickable to tick; ⇧⌘U marks ticked; More › has Tick All etc. / Lulo's ⇧⌘L just inserts "- [ ] " text and the preview checkboxes can't be clicked. | `edit_recovery_controller.rs:24-48`, `markdown_preview.rs` |
| NOTES-03 | P1 (⌘B/⌘I/⌘U, lists) / P2 (rest) | M (as Markdown-insertion commands) | Missing | Mac binds Lists, Block Quote, Table, Bold/Italic/Underline, Strikethrough, Highlight, Add Link, Indent, Move Item. / Lulo binds none of them. | `crates/notes/src/startup_controller.rs:14-33`, `NOTES_MENUS` |
| NOTES-04 | P2 | S (title) / M (rest) | Missing | Mac toolbar has Format "Aa", Table, Media, Share, Lock; window title is "<folder> – N notes". / Lulo has none of those buttons and the attachments are photos-only; title is always "Notes". | `crates/notes/src/toolbar.rs:89-213`, `main.rs:331-338` |
| NOTES-05 | P2 | M | Missing | Mac has Gallery view ⌘2 and Show Attachments Browser ⌘3. / Lulo has List only. | `note_navigation.rs` |
| NOTES-06 | P1 (⌘D) / P2 (Lock) | S / M | Missing | Mac File menu has Pin Note, Duplicate Note ⌘D, Lock Note. / Lulo's Pin exists only in ⋯ with no menu item/shortcut; no Duplicate; no Lock. | `NOTES_MENUS`, `library_actions.rs` |
| NOTES-07 | P1 | S | Missing | Mac's ⌫ removes the selected note from the list. / Lulo uses ⌘⌫ (a Lulo invention); plain ⌫ does nothing and the list can't be arrow-navigated. | `startup_controller.rs:14-33` |
| NOTES-08 | P2 | S (with MENU-04) | Missing | Mac has a checked Sort By submenu, Newest/Oldest First, Group By Date, Hide Folders ⌃⌘S. / Lulo has three flat sort rows and no folder toggle. | `NOTES_MENUS` |
| NOTES-09 | P1 | M | Missing | Mac separates Note List Search ⌥⌘F from in-note Find ⌘F, plus Find and Replace ⇧⌘F. / Lulo's ⌘F only focuses the list search — no in-note find or replace. | `crates/notes` |
| NOTES-10 | P1 | S | Missing | Mac's File menu has Export as › PDF. / Lulo has it implemented but missing from the live menu (as TE-04); the PDF save panel also starts in the process's working directory, not Documents. | `print_controller.rs:102-155`, `:114` |
| NOTES-11 | P2 | M | Missing | Mac tags are typed inline as #tag with a sidebar Tags browser and Smart Folders. / Lulo has a comma-separated tags field only, no browser, no smart folders. | `note_navigation.rs` |
| NOTES-12 | P2 | S | Missing | Mac has view zoom ⇧⌘./⇧⌘,/⇧⌘0. / Lulo: absent. | — |
| NOTES-13 | P2 | S | Missing | Mac Settings… ⌘, covers default account, sort, new-note starting style, group-by-date, default text size. / Lulo: absent. | — |

### Calculator

Window shape (230×408, sidebar/mode toolbar buttons) matches the Mac.

| ID | Sev | Size | Status | Gap | Where |
|---|---|---|---|---|---|
| CALC-01 | P1 | S (hide) / M (implement) | Broken | Mac's toolbar sidebar (history) and Mode buttons both work. / Lulo's have no `on_click` at all — they're dead controls. | `crates/calculator/src/view.rs:110-141` |
| CALC-02 | P1 (Scientific) / P2 (Programmer) | M | Missing | Mac has Scientific ⌘2 (2nd, x², sin/cos/tan, π, memory) and Programmer ⌘3. / Lulo is Basic only; ShowBasic is a no-op. | `crates/calculator/src/engine.rs`, `keypad.rs` |
| CALC-03 | P2 | S | Missing | Mac has a History ⌃⌘S sidebar of past calculations, clickable to reuse. / Lulo: absent. | — |
| CALC-05 | P2 | S | Missing | Mac's thousands separator follows Region and can be hidden; Decimal Places is adjustable. / Lulo hard-codes "," with no options. | `engine.rs:482-491` |
| CALC-06 | P2 | M | Missing | Mac has Convert ⌥⌘C (units/currency) and RPN ⌘R. / Lulo: absent (Spotlight already has conversions to reuse). | — |
| CALC-07 | P2 | S | Missing | Mac's light appearance is measured. / Lulo's light palette is unmeasured/unverified. | `keypad.rs:224` |
| CALC-08 | P2 | S | Missing | Mac Window menu has Always on Top. / Lulo: absent (see MENU-01). | — |

### Preview

Toolbar shape matches when a document is open; two-line title matches.

| ID | Sev | Size | Status | Gap | Where |
|---|---|---|---|---|---|
| PREV-01 | P0 | M (pdftotext `-bbox` word boxes already exist; needs hit-test + draw) | Fixed 6bc1c39a | Mac supports text selection and ⌘C in PDFs. / Lulo: absent — ⌘C does nothing for PDFs. | `crates/preview/src/view.rs:536-563` |
| PREV-02 | P1 | S | Partial 6bc1c39a (PDF print only) | Mac has Print ⌘P. / Lulo: absent. | follow `crates/text-editor/src/view/printing.rs` |
| PREV-03 | P1 | L | Missing | Mac has Markup (Show Markup Toolbar ⇧⌘A, highlight/annotate/Signature/Text) — filling and signing PDF forms is a common reason to open Preview. / Lulo: absent. | new; needs a PDF writer (e.g. `lopdf`) |
| PREV-04 | P1 | M | Fixed 6bc1c39a | Mac supports Save/Export As (PNG/JPEG/HEIC/PDF/TIFF), Export as PDF, Duplicate, and persisted Rotate/Crop. / Lulo is view-only — rotation is never saved. | `crates/preview` |
| PREV-05 | P1 (Go to Page, links) | S / M | Partial 6bc1c39a (external URLs only) | Mac has Go to Page… ⌥⌘G, ⇞/⇟ paging, Back/Forward ⌘[/⌘] link history, clickable PDF links. / Lulo's paging keys just scroll; none of the rest exist. | `view.rs:372-416`, `poppler.rs` |
| PREV-06 | P2 (TOC is P1 for long PDFs) | M (`pdfinfo`/poppler outline) | Fixed 6bc1c39a | Mac sidebar has Table of Contents, Highlights and Notes, Bookmarks, Contact Sheet. / Lulo has Thumbnails only. | `view.rs:1192-1282` |
| PREV-07 | P2 | M | Missing | Mac has Single Page ⌘2 and Two Pages ⌘3. / Lulo has Continuous only. | `view.rs:1388-1413` |
| PREV-08 | P1 | S | Missing | Mac records opened documents to Recents (File menu and Files' Recents). / Lulo records nothing. | `crates/preview/src/main.rs`; `crates/rmac-recent-documents` |
| PREV-09 | P1 (HEIC) / P2 (rest) | M (`libheif`) | Missing | Mac opens HEIC, SVG, PSD, RAW, EPS and more. / Lulo opens PDF/PNG/JPEG/GIF/WebP/BMP/TIFF — no HEIC. | `crates/preview/src/document.rs:54-70` |
| PREV-10 | P2 | M | Missing | Mac has Adjust Size…, Adjust Colour…, Flip, Crop, Remove Background, Magnifier. / Lulo: absent. | — |
| PREV-11 | P2 | S | Missing | Mac has Slideshow ⇧⌘F, Zoom All variants, Zoom to Selection ⌘*. / Lulo: absent. | — |
| PREV-12 | P2 | L | Missing | Mac supports Insert › Page from File/Blank Page, deleting pages, dragging thumbnails to reorder or merge PDFs. / Lulo: absent. | — |
| PREV-13 | P2 | S | Missing | Mac remembers window bounds and accepts drag-in. / Lulo doesn't restore bounds and has no drop target. | `main.rs:94-104` |

### Terminal

Window shape (580×385 at 80×24) matches the Mac.

| ID | Sev | Size | Status | Gap | Where |
|---|---|---|---|---|---|
| TERM-01 | P0 | M (single-instance + multi-window, like Files) | Fixed ccda7f50 | Mac: ⌘N opens a new window; several Terminal windows is the default workflow. / Lulo: the app has exactly one window (`boot_app`). | `crates/terminal/src/main.rs`, `crates/rmac-ui/src/window.rs` |
| TERM-02 | P1 (Reset) / P2 (rest) | S | Fixed dbc08e5e | Mac has Clear Scrollback ⌥⌘K, Clear Screen ⌃⌘L, Reset ⌥⌘R, Hard Reset ⌃⌥⌘R in addition to ⌘K. / Lulo has ⌘K only — a wedged terminal can't be recovered without closing the tab. | `crates/terminal/src/controller/view_state.rs:43-56`, `emulator.rs` |
| TERM-03 | P1 | M | Partial dbc08e5e | Mac's Settings… ⌘, opens a full Profiles/General settings window (font, size, cursor, default login shell). / Lulo's ⌘,/⇧⌘P open a dropdown profile picker only — nothing is customisable. | `crates/terminal/src/profiles.rs`, `controller/lifecycle.rs` |
| TERM-04 | P2 | S | Missing | Mac's window title is "<user> — <process> — cols×rows". / Lulo shows "<OSC title or job> — cols×rows", e.g. the shell's own bashrc title on Ubuntu. | `controller/renderer/chrome.rs:46-63`, `session.rs:714-722` |
| TERM-05 | P2 | M | Fixed dbc08e5e | Mac has Split Pane ⌘D/⇧⌘D. / Lulo: absent. | — |
| TERM-06 | P1 | S (mark lines at Return, or ship a bash snippet) | Fixed dbc08e5e | Mac marks every Return-pressed line regardless of shell, so ⌘↑/⌘↓ always work. / Lulo's marks need the shell to emit OSC 133 — stock Ubuntu bash doesn't, so ⌘↑/⌘↓ do nothing out of the box. | `crates/terminal/src/shell_integration.rs:1` |
| TERM-07 | P2 | S | Fixed dbc08e5e | Mac bell is audible or a visual flash, with a Dock bounce/badge when backgrounded. / Lulo ignores bell events entirely. | `crates/terminal/src/session.rs:63-94` |
| TERM-08 | P1 | S | Missing | Mac: ⌘-click/⌘-double-click any URL in the text opens it. / Lulo: only explicit OSC 8 hyperlinks work; plain URLs aren't detected. | `controller/pointer.rs:48-62` |
| TERM-09 | P1 | S | Missing | Mac: dragging a file onto the window types its escaped path. / Lulo: no drop handler. | `crates/terminal/src/controller` |
| TERM-10 | P1 (UK/European keyboards) | S | Missing | Mac's "Use Option as Meta Key" defaults off, so ⌥ types special characters (€, #). / Lulo: Alt always sends an ESC prefix, so ⌥3 can't type "#" on a UK layout. | `crates/terminal/src/keyboard.rs:148-165` |
| TERM-11 | P2 | S each | Missing | Mac has Export Text As… ⌘S, Print ⌘P, Show Inspector ⌘I, Edit Title ⇧⌘I, Paste Escaped Text ⌃⌘V, Open man Page for Selection, Scroll to Top/Bottom, Page Up/Down. / Lulo: absent. | — |
| TERM-12 | P2 | S | Missing | Mac profile list is Basic, Clear Dark, Clear Light, Grass, Homebrew, Man Page, Novel, Ocean, Pro, Red Sands, Silver Aerogel, Solid Colors. / Lulo has 9 profiles including a non-Mac "Lulo OS Dark"; Clear Dark/Clear Light/Silver Aerogel/Solid Colors are missing. | `crates/terminal/src/profiles.rs` |
| TERM-13 | P2 | S | Missing | Marks, ⇧⌘A, ⇧⌘P and ⌘, are bound but not shown in the exported menu. | `TERMINAL_MENUS` |
| TERM-14 | P2 | — | Partial | Mac asks "Do you want to terminate running processes in this window?" on any close path (Dock, menu bar, ⌘Q, log out). / Lulo now prompts "Terminate / Cancel" per tab on tab/window close, but this hasn't been re-checked for ⌘Q, the Dock, or log-out. | `crates/terminal/src/controller/lifecycle.rs`, `tab_lifecycle.rs:88-168` |

### System Monitor

Window shape (960×640, 5-tab toolbar) matches the Mac.

| ID | Sev | Size | Status | Gap | Where |
|---|---|---|---|---|---|
| MON-01 | P0 | S (filter with `Process::thread_kind()`, or turn off task enumeration) | Broken | Mac's process list shows processes, with Threads as a column. / Lulo lists every Linux **thread** as its own process via sysinfo 0.33 — the summary reads "Threads 667 / Processes 667" on a real run, and energy/disk totals likely double-count. | `crates/activity-monitor/src/process_table.rs:259-304`, `metrics_panes.rs:319-323`, `sampling.rs:76-89` |
| MON-02 | P1 | M | Missing | Mac gives each tab its own column set (CPU/Memory/Energy/Disk/Network each have different columns, Process Name first). / Lulo shows the same six columns on every tab, PID first, no CPU Time or Threads column. | `crates/activity-monitor/src/columns.rs:28-101`, `view.rs:133-149` |
| MON-03 | P1 | S–M | Missing | Mac has a View filter (All/My/System/Other Users'/Active/Windowed/Hierarchically), and the window subtitle follows it. / Lulo: absent — subtitle is a fixed "All Processes". | `view/render/chrome.rs:286-293` |
| MON-04 | P2 | S | Missing | Mac's Energy column is a real Energy Impact score. / Lulo computes `cpu + disk_MiB × 0.5`, roughly equal to %CPU, and shows a value before %CPU has one on first refresh. | `process_table.rs:295` |
| MON-07 | P2 | M | Missing | Mac's Inspect Process is a separate window with Memory/Statistics/Open Files/Ports tabs. / Lulo shows a modal card with 10 facts and no tabs (Open Files is easy via `/proc/<pid>/fd`). | `view/render/overlays.rs:59-171` |
| MON-08 | P2 | S | Missing | Mac's Memory tab shows a Memory Pressure graph (green/yellow/red) plus Wired/Compressed/Cached/Swap breakdown. / Lulo shows a plain "MEMORY USED" percentage (Linux PSI at `/proc/pressure/memory` could supply pressure). | `metrics_panes.rs:343-382` |
| MON-09 | P2 | S | Missing | Mac's Disk/Network tabs show I/O counts and packet counts. / Lulo shows neither. | `metrics_panes.rs:397-451` |
| MON-10 | P2 | S–M | Missing | Mac has Update Frequency (1/2/5s), a menu Columns › chooser, Dock-icon CPU modes, Sample Process, Send Signal…, Clear CPU History, and floating CPU Usage/History windows. / Lulo's refresh is fixed at 2s; the column chooser exists only in the ⋯ button, not the menu; the rest is absent. | — |
| MON-11 | P2 | S | Missing | Mac's table shows every process. / Lulo caps the table at 300 rows. | `process_table.rs:322` |

### Clock

Window shape (1024×768, 4-tab toolbar) matches the Mac.

| ID | Sev | Size | Status | Gap | Where |
|---|---|---|---|---|---|
| CLOCK-01 | P1 (scroll) / P2 (analogue) | S / M | Broken | Mac's Stopwatch has digital and analogue views with a scrolling lap list. / Lulo is digital only, and the lap list can't scroll (`overflow_hidden`) — laps beyond the visible area are lost from view. | `crates/clock/src/view.rs:1045-1169`, `:1158` |
| CLOCK-02 | P2 | S | Missing | Mac Timers have presets, Recent timers, a label, a sound choice. / Lulo has none (the model has an unused `label` field). | `view.rs:1173-1394`, `countdown.rs:94` |
| CLOCK-03 | P2 | S | Missing | Mac Alarms have a sound picker. / Lulo always uses the Alert cue. | `ring.rs:189` |
| CLOCK-04 | P2 | S | Missing | Mac World Clock cities can be reordered and switched to a list. / Lulo only adds/removes. | `view.rs:525-652` |
| CLOCK-05 | P2 | S | Missing | Mac Clock follows Light mode. / Lulo is Dark-only (fixed 0x1E1E1E fill). | `crates/clock/src/metrics.rs:13` |
| CLOCK-06 | P2 | S | Missing | An idle Mac clock uses no CPU. / Lulo's ticker wakes every 250 ms on every tab even when idle. | `view.rs:35-36,134-157` |

### Player

| ID | Sev | Size | Status | Gap | Where |
|---|---|---|---|---|---|
| PLAYER-01 | P2 | M | Missing | Mac's QuickTime records screen/audio/movie (⌃⌘N variants). / Lulo Player plays local media and publishes MPRIS already, but has no recording — screen recording could live in the Screenshot tool instead. | `crates/rmac-media`; `shell/bins/rmac-screenshot` |

### Missing apps

Lulo has Files, Settings, Text Editor, Notes, Calculator, Preview, Terminal,
System Monitor, Clock, Player, Apps (Launchpad), Spotlight, Mission Control,
a screenshot tool and Force Quit. The rows below are the bundled Mac apps
with no Lulo counterpart yet, minus the ones the source audit marked
**Skip** (Apple-only services: Messages, FaceTime, iCloud-tied apps, Siri,
Maps, Journal, Books, Freeform, Games, Passwords, Migration Assistant, and
similar — no plan, not tracked as rows here).

| ID | Sev | Size | Status | Gap | Where |
|---|---|---|---|---|---|
| APP-01 | P1 | L | Missing | **Calendar**: core productivity app; the menu-bar clock is expected to open it, and the desktop Calendar widget has no app behind it yet. | build on Evolution Data Server |
| APP-02 | P2 | M | Missing | **Reminders**: pairs with Calendar; build after it, on EDS VTODO. | EDS |
| APP-03 | P1 | M | Missing | **Disk Utility**: the Mac answer to "format a USB stick" (Erase, Eject, First Aid). | build on udisks2 |
| APP-04 | P2 | S | Missing | **System Information**: reached from About This Mac › More Info…; Lulo's About covers the summary only. | new |
| APP-05 | P2 | M | Missing | **Console**: a `journalctl` viewer with search; GNOME Logs could substitute but breaks the look. | new |
| APP-06 | P2 | S | Missing | **Font Book**: install a downloaded `.ttf` by double-click. | fontconfig |
| APP-07 | P2 | L | Missing | **Photos**: substitute for now — Preview's multi-image mode plus Files' gallery view cover most of it. | reuse Preview/Files |
| APP-09 | P2 | S | Missing | **Stickies**: floating yellow notes; cheap and loved by long-time Mac users. | could back onto `rmac-notes-store` |
| APP-10 | P2 | S | Missing | **Dictionary**: Spotlight already has definitions when dictd is installed; a window app plus ⌃⌘D lookup would reuse that. | new |
| APP-11 | P1 (data safety) | L | Missing | **Time Machine**: a Déjà Dup engine behind a Lulo pane (see SET-10). | new |
| APP-12 | P2 | S | Missing | **Digital Color Meter**: cheap tool designers expect; worth doing alongside a Print Center queue view (SET-07). | new |
| APP-13 | P2 | S | Missing | **Mail**: substitute is Thunderbird as the chosen default mail handler (see S16 in Settings) — not yet wired as the actual default. | `crates/rmac-apps/src/catalog.rs` |

---

## Other

| ID | Sev | Size | Status | Gap | Where |
|---|---|---|---|---|---|
| OTHER-01 | P1 | L | Missing | Mac's Open/Save panel is a Finder-style sheet (sidebar, view switcher, New Folder, tags). / Lulo is a portal *client* only; `rmac-portals.conf` resolves to `default=gnome;gtk;*`, so the GNOME file chooser appears in every app, which is the single most visible "this is GNOME" moment in the product. | needs an rmac `org.freedesktop.impl.portal.FileChooser` implementation |

---

**On `docs/beta-checklist.md`:** that document stays separate — it's a
release-blocking checklist (journeys, performance budgets, accessibility
gates, packaging, CI, versioning) for shipping a specific build, not a
Mac-parity gap list. Its accessibility findings that are genuine, concrete
Mac-parity gaps (Terminal/Notes/Files/Spotlight having no accessible
surface, ⌃F2 not implemented) are folded into Shell → Accessibility above.
