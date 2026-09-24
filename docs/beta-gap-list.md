# Beta gap list: the first hour on a new Mac, 2026-09-24

This list walks through what a Mac user does in their first hour on a new
machine. For each step it records what Lulo OS does today and ranks the gaps
that are left before the Beta ships.

It was built by reading the code on branch `gap-finder`, which starts from
`dev` at `5bea40ca`. Nothing was run on the laptop. The Mac was not measured
in this pass either: every behaviour marked **S** comes from knowing macOS 26
rather than from a capture. Out of scope: anything `todo.md` lists under
"Not before 1.0", and the areas other agents own right now (Settings visuals,
Files visuals, Notes and Terminal visuals, the security review, niri
packaging, the system audit). Gaps in those areas are listed but were not
fixed here.

Status key: **Works**, **Partial**, **Missing**, **Broken**, **Fixed** (fixed
on this branch). Effort: **S** is a day or less, **M** is a few days, **L** is
more than that.

## 1. Ranked gaps

### Beta-blocking

These can lose work, leave a Mac user unable to do something basic, or show
them something that is not true.

| # | Gap | Status | Effort | Files to change |
|---|---|---|---|---|
| B1 | **⌘C, ⌘V, ⌘Q and the other ⌘ shortcuts don't work in Firefox or GTK apps out of the box.** Translating them (ADR 0017, `keyd`) is opt-in, needs `pkexec`, and only takes effect after logging in again. It is buried in Settings › Keyboard. Copy and paste in the browser is the first thing a Mac user tries. | Partial | S to offer it in Setup Assistant; changing the default is a product decision | `crates/setup-assistant/src/flow.rs` (add a step or a row on the Keyboard step), `crates/setup-assistant/src/services.rs`, `crates/system-settings/src/controller/input/mac_keyboard.rs` (reuse its enable path), `docs/decisions/0017-mac-keyboard.md` |
| B2 | Log Out, Restart and Shut Down ended the session straight away (`niri msg action quit --skip-confirmation`, `systemctl reboot`), so apps were never asked to quit. | **Fixed** `0253d0c2` | — | `shell/bins/rmac-menubar/src/main.rs` (`quit_all_then`), `menu_model.rs` (`quit_all_progress`) |
| B3 | Text Editor lost unsaved edits when the Dock, the menu bar, ⌘Tab's Q or log out closed its window. Only ⌘W and the red button went through the Save prompt; a close request from the compositor skipped it. | **Fixed** `1bdefcc9` | — | `crates/text-editor/src/view/lifecycle.rs` |
| B4 | Terminal quits (from the Dock, the menu bar, ⌘Q, or log out) without asking, even while a command is running. Mac Terminal asks "Do you want to terminate running processes in this window?" | Missing | S | `crates/terminal/src/controller/lifecycle.rs`: add `window.on_window_should_close` with a running-process check (Terminal owner) |
| B5 | There was no low-battery warning. The only sign was the menu-bar battery turning red at 10%. | **Fixed** `d7290bb0` (wording S) | — | `shell/bins/rmac-menubar/src/menu_model.rs` (`LowBatteryWatch`) |
| B6 | ⌘Q, ⌘H, ⌥⌘H and ⌘M did nothing in most Lulo OS apps. The app menu showed these shortcuts, but only System Settings and System Monitor had them bound. | **Fixed** `406f79a6`, `04055a24` | — | `crates/rmac-ui/src/components.rs`, `chrome.rs`, `shortcuts.rs` |
| B7 | The Log Out item showed a ⇧⌘Q hint, but nothing was bound to it. | **Fixed** (hint removed) `70a00336`. The shortcut itself is still missing; see S6. | — | `shell/bins/rmac-menubar/src/main.rs` |
| B8 | Double-clicking a PDF, picture or text file opened whichever installed app claimed the type first. The packaged defaults covered only archives and media. | **Fixed** `11d3d5c3` | — | `packaging/rmac-apps/rmac-mimeapps.list`, `scripts/linux/verify-application-package.py` |
| B9 | Unsaved work is not protected when the session ends some other way (`systemctl poweroff` from a terminal, a logind-initiated shutdown, OOM). Apps have no SIGTERM handler, and none takes a logind `shutdown` delay inhibitor while a document is dirty. Text Editor's recovery file is written after a 2 s debounce, so up to 2 s of typing can be lost. | Missing | M | `crates/text-editor/src/view/document_state.rs` (flush on SIGTERM), a logind `Inhibit("shutdown", …, "delay")` held while dirty |

### Should fix before Beta

| # | Gap | Status | Effort | Files to change |
|---|---|---|---|---|
| S1 | ⌥⌘⎋ was not bound. Force Quit… opens all of System Monitor, not the Mac's small Force Quit window (a list of apps, "(Not Responding)" in red, and a Force Quit button). | ⌥⌘⎋ **Fixed** `20c998af`. The window is still Partial. | M | New mode in `shell/bins/rmac-app-switcher` (it already lists running apps and quits them) or a small app; `packaging/rmac-session/shell.kdl` |
| S2 | Choosing System Settings… or Force Quit… a second time opened another window instead of bringing the open one forward. | **Fixed** `acf66602`. The niri ⌥⌘⎋ bind still spawns directly. | S | `packaging/rmac-session/shell.kdl` (route through a single-instance path) |
| S3 | About This Lulo OS opened the General list rather than the About summary. | **Fixed** `beafa880` | — | `crates/system-settings/src/navigation.rs` (`subpage_route`) |
| S4 | Control Centre: the Wi-Fi and Bluetooth modules don't expand into a network or device list; they only open Settings. The sound module has no output picker. Neither Control Centre nor Notification Centre can be opened from the keyboard. | Partial | M | `crates/quick-settings-app/src/render/cards.rs`, `render/controls.rs`, `crates/rmac-quick-settings/src/layout.rs` |
| S5 | Files has no Empty Trash command of its own (Finder › Empty Trash, ⇧⌘⌫). You have to use the Dock, or select everything and choose Delete Immediately. | Missing | S | `crates/finder/src/view/chrome_presentation/menus_tabs.rs`, `startup/shortcuts.rs` (bind `FORCE_DELETE`); reuse the Dock's alert copy (Files owner) |
| S6 | ⇧⌘Q, and ⌃F2 to move focus to the menu bar. The logout confirmation lives in the menu-bar popover, and no key can open that popover. | Missing | M | `shell/bins/rmac-menubar/src/main.rs` (a command endpoint and the Dock's invisible focus-surface technique), `crates/rmac-shortcuts/src/model.rs` |
| S7 | Printing: ⌘P works in Text Editor through the portal, but Notes and Preview have no print path. | Partial | S each | `crates/notes/src/startup_controller.rs`, `crates/preview/src/main.rs`; follow `crates/text-editor/src/view/printing.rs` |
| S8 | Notes has no ⌘W. | Missing | S | `crates/notes/src/startup_controller.rs` (Notes owner) |
| S9 | Files has no ⌘N (New Window) and no ⇧⌘G (Go to Folder). Go to Folder exists only in the Open/Save panel. | Missing | S (⇧⌘G) / M (⌘N) | `crates/finder/src/view/startup/shortcuts.rs`; lift `crates/rmac-file-chooser/src/goto.rs` (Files owner) |
| S10 | Wi-Fi: there is no way to join a hidden network (Other… / Join Other Network). | Missing | M | `crates/system-settings/src/connectivity.rs`, `controller/wifi/credentials.rs`, `crates/rmac-network/src/linux.rs` (`AddAndActivateConnection`) |
| S11 | Keyboard-backlight keys (`XF86KbdBrightnessUp/Down`) are not bound, and the OSD has no row for them. | Missing | S–M | `crates/rmac-osd/src/lib.rs`, `linux.rs` (logind `SetBrightness` on `leds/*::kbd_backlight`), `packaging/rmac-session/shell.kdl` |
| S12 | Dock app menus have no Show All Windows, Hide, or (with ⌥) Hide Others. The comment explaining their absence says niri can't hide apps, but parking (ADR 0014) now does that, and App Exposé exists. | Missing | M | `crates/rmac-dock/src/menu.rs`, `crates/rmac-dock-system/src/dispatch.rs`, `shell/bins/rmac-dock/src/main.rs` |
| S13 | Title-bar double-click calls GPUI's `zoom_window()` (xdg maximize). The green button uses the compositor's Fill instead. Whether niri honours maximize for floating windows is unverified. There is also no "double-click a window's title bar to" setting. | Partial | S (route through `send_window_action`); M with the setting | `crates/rmac-ui/src/chrome.rs` (`client_bar`), `crates/system-settings/src/controller/chrome.rs`, `crates/weather/src/view.rs`, `crates/clock/src/view.rs` |
| S14 | The emoji and symbol picker (⌃⌘Space, fn E) isn't there. The binding exists, but GPUI's Linux `show_character_palette` does nothing. | Missing | L | New shell overlay; `crates/rmac-ui/src/text_keys.rs` |
| S15 | No app binds ⌘, (Settings…). Apps with real settings (Terminal profiles, Weather units, Clock) should open them. | Missing | S each | per app `main.rs` / `lifecycle.rs` |
| S16 | Default web browser and mail app can't be chosen anywhere. macOS offers this in Desktop & Dock. | Missing | M | `crates/system-settings/src/controller/…` (Desktop & Dock), `crates/rmac-apps/src/catalog.rs` (`xdg-mime default` for `x-scheme-handler/http(s)`, `mailto`) |

### Later (after Beta)

| # | Gap | Effort | Notes |
|---|---|---|---|
| L1 | Drag a window to a screen edge to tile it, and the full green-button menu (quarters, Arrange). | M–L | The hover menu has halves, Fill and Full Screen (`crates/rmac-ui/src/chrome.rs` `zoom_menu`) |
| L2 | A login window that looks like the Mac's (avatar, password pill, blurred wallpaper) | M | Only a GDM theme today (`packaging/rmac-session/greeter/`) |
| L3 | Setup Assistant creates the user account (macOS creates it on first boot) | L | It only renames the signed-in user (`crates/setup-assistant/src/services.rs`). Account creation needs an installer. |
| L4 | Screen Mirroring and an AirDrop equivalent in Control Centre | L | niri has no mirroring; AirDrop is out of scope (`docs/known-limitations.md`) |
| L5 | A countdown on the log-out confirmation, and "Reopen windows when logging back in" | M | The confirmation is still the inline menu panel |
| L6 | Kill-ring yank (⌃Y) and ⌃N/⌃P in text fields | S | The other Emacs keys were added in `275f87e1` |

## 2. The first-hour walk

Each row traces one step to the code. Rows marked Fixed were changed on this
branch.

### First login

| Step | Status | Evidence |
|---|---|---|
| Setup Assistant on first login | Works | `crates/rmac-session/units/rmac-setup-assistant.service` (runs until `%E/rmac/setup-assistant-complete` exists); steps: Welcome, Language & Region, Keyboard, Wi-Fi, Account, Appearance, Tips, Privacy, Done (`crates/setup-assistant/src/flow.rs:7-42`) |
| Setup Assistant offers Mac shortcuts for PC apps | Missing | See B1 |
| Setup Assistant creates the account | Partial | Only `SetRealName`/`SetIconFile` on the existing user (`services.rs:94-110`) |

### Menu bar: the Apple-menu equivalent

The rows, their order and the separators match macOS 26:
`shell/bins/rmac-menubar/src/main.rs` `system_menu`.

| Item | Status | Evidence |
|---|---|---|
| About This Lulo OS | **Fixed** | Opens Settings › General › About (`--pane about`, `navigation.rs` `subpage_route`). macOS 26 shows a small separate About window; this is the closest real surface. |
| System Settings… | **Fixed** | Focuses the open Settings window or launches one (`open_or_focus_app`) |
| Software Center | Works | `gtk-launch snap-store_snap-store`, the Ubuntu App Center (the App Store row's place) |
| Recent Items › | Works | Submenu, Clear Recent Items |
| Force Quit… ⌥⌘⎋ | Partial (**Fixed** binding and focus) | Opens System Monitor; see S1 |
| Sleep | Works | `systemctl suspend`. logind locks first (`crates/rmac-shortcuts/src/lock.rs:278-345`) |
| Restart… / Shut Down… | **Fixed** | Confirmation, then every app is asked to quit; an app still open after 30 s cancels the request and a notification names it |
| Lock Screen ⌃⌘Q | Works | GlobalShortcuts portal (`crates/rmac-shortcuts/src/model.rs:19`), fallback `shortcuts-fallback.kdl:5`; unlock through the PAM broker |
| Log Out Name… | **Fixed** | As Restart; the dead ⇧⌘Q hint was removed |
| App menu: Hide ⌘H, Hide Others ⌥⌘H, Show All, Quit ⌘Q | Works by click; keys **Fixed** | Menu: `app_menu`. Keys: `crates/rmac-ui/src/components.rs` |

### Control Centre, Notification Centre, Spotlight

| Step | Status | Evidence |
|---|---|---|
| Control Centre modules: Wi-Fi, Bluetooth, Focus, Display, Sound, Now Playing | Works | `crates/quick-settings-app/src/render/{cards,controls}.rs` |
| Wi-Fi and Bluetooth lists inside Control Centre, sound output picker | Missing | S4 |
| Wi-Fi menu-bar menu with network list and join | Works | `shell/bins/rmac-menubar/src/menu_model.rs` `wifi_menu_rows`; `docs/journey-7-trace.md` |
| Notification Centre (click the clock), grouping, Clear All, banners with actions | Works | `crates/notification-center-app`, `crates/rmac-notifications-linux/src/service.rs:763` (the `org.freedesktop.Notifications` server) |
| Spotlight ⌘Space | Works | Portal `launcher` shortcut; `crates/launcher-app` |

### Dock

| Step | Status | Evidence |
|---|---|---|
| App menu: windows, Options › Keep in Dock / Open at Login / Show in Files, Quit (⌥ → Force Quit) | Works | `crates/rmac-dock/src/menu.rs:238-385` |
| Show All Windows, Hide, Hide Others | Missing | S12 |
| Drag out to remove; kept apps persist; recent apps | Works | `crates/rmac-dock/src/reorder.rs`, `pins.rs`, `recents.rs` |
| Trash: Open, Empty Trash with the Mac's alert, drop files to trash | Works | `shell/bins/rmac-dock/src/main.rs:3064-3147`, `:807-819` |

### Files, Trash, Quick Look, opening files

| Step | Status | Evidence |
|---|---|---|
| Double-click opens with the default app | Works | Portal OpenURI (`crates/rmac-portal/src/open.rs:41-69`) |
| Packaged defaults for PDF, pictures and text | **Fixed** | `packaging/rmac-apps/rmac-mimeapps.list` |
| Open With, Always Open With | Works | `crates/finder/src/view/open_with_controller.rs` |
| Move to Trash ⌘⌫, Put Back, ⌘Z undo | Works | `crates/finder/src/view/undo_controller.rs`, `trash_recovery_controller.rs` |
| Empty Trash from Files | Missing | S5 |
| Quick Look (Space): images, PDF, audio/video poster, text, folders | Works | `crates/rmac-quick-look/src/content.rs` |
| ⌘N new window / ⇧⌘G Go to Folder | Missing | S9 |

### Windows and switching

| Step | Status | Evidence |
|---|---|---|
| Traffic lights: close through the app's guard, minimise, green = full screen, ⌥-click = Fill, hover menu with halves and Fill | Works | `crates/rmac-ui/src/chrome.rs` |
| Title-bar double-click | Partial | S13 |
| ⌘Tab per app, with Q/H while held; unhides hidden apps | Works | `shell/bins/rmac-app-switcher/src/main.rs:300-301`, `model.rs:287-326` |
| ⌘` cycles an app's windows | Works | `shell.kdl` `recent-windows` |
| Mission Control ⌃↑, App Exposé ⌃↓, Spaces ⌃←/⌃→, Show Desktop F11 | Works | `shell/bins/rmac-mission-control` (ADR 0014) |
| ⌘H / ⌥⌘H / ⌘M / ⌘Q from the keyboard in Lulo OS apps | **Fixed** | `crates/rmac-ui/src/components.rs` (context-free bindings an app's own binding still overrides) |
| The same keys in Firefox and GTK apps | Partial | B1 |
| Drag to an edge to tile | Missing | L1 |

### Keyboard shortcuts

| Shortcut | Status | Evidence |
|---|---|---|
| ⇧⌘3 / ⇧⌘4 (Space for a window) / ⇧⌘5, ⌃ to copy, floating thumbnail, saved to Desktop | Works | `shell/bins/rmac-screenshot` (ADR 0010) |
| ⌘Space | Works | Portal |
| ⌃⌘Q | Works | Portal and fallback |
| ⌥⌘⎋ | **Fixed** | `shell.kdl` |
| ⇧⌘Q | Missing | S6 |
| ⌃⌘Space emoji | Missing | S14 |
| ⌃F2 / ⌃F3 | Missing / Works | `docs/known-limitations.md` |

### Text editing in every text field

All first-party fields are gpui-component `Input`s, and `crates/rmac-ui/src/text_keys.rs`
installs the Mac keys on Linux.

| Keys | Status |
|---|---|
| ⌥←/→, ⌥⇧←/→, ⌘←/→, ⌘↑/↓, ⌥⌫, ⌘⌫, ⌘A/C/V/X/Z, ⇧⌘Z, ⌃A/⌃E | Works |
| ⌃K, ⌃D, ⌃H, ⌃F, ⌃B | **Fixed** `275f87e1` |
| ⌃Y, ⌃N, ⌃P | Missing (L6) |

### Hardware, power and system

| Step | Status | Evidence |
|---|---|---|
| Volume and brightness keys, OSD, volume feedback sound | Works | `packaging/rmac-session/shell.kdl`, `crates/rmac-osd/src/linux.rs` (logind `SetBrightness`, WirePlumber) |
| Keyboard-backlight keys | Missing | S11 |
| Sleep on lid close | Works (logind default) | Nothing overrides `HandleLidSwitch`; the lock is taken before sleep (`crates/rmac-shortcuts/src/lock.rs`) |
| Idle lock and suspend | Works | `rmac-idle-lock.service` (swayidle) |
| Low-battery warning | **Fixed** | B5 |
| External display hotplug: menu bar and Dock on each output, arrangement, Keep Changes | Works (arrangement is a popup, not drag) | `shell/crates/rmac-shell-layer/src/lib.rs`, `crates/system-settings/src/controller/displays` |
| Wi-Fi password and enterprise join; Bluetooth pairing with a passkey | Works | `controller/wifi/credentials.rs`, `crates/rmac-bluetooth/src/pairing_agent.rs` |
| Hidden Wi-Fi network | Missing | S10 |
| Printing | Partial | S7 |
| Log out, shut down or restart with unsaved work | **Fixed** (menu path); Missing (forced path) | B2, B3, B9 |

## 3. What changed on this branch

| Commit | Change | Test |
|---|---|---|
| `20c998af` | ⌥⌘⎋ opens Force Quit (System Monitor) | `crates/rmac-session/src/tests.rs`, `crates/rmac-keyboard/src/tests.rs` (keyd never translates ⌘⎋) |
| `beafa880` | About This Lulo OS opens General › About | `crates/system-settings/src/navigation.rs` `the_about_route_opens_general_about` |
| `1bdefcc9` | Text Editor's close guard also covers compositor close requests | none; needs a laptop check |
| `0253d0c2` | Log Out, Restart and Shut Down ask every app to quit first | `menu_model.rs` `quit_all_*` (run standalone with `rustc --test`) |
| `11d3d5c3` | PDF, picture and text defaults | `scripts/linux/verify-application-package.py` exact-match check |
| `406f79a6` | ⌘M, ⌘H, ⌥⌘H in every app | `crates/rmac-ui/src/chrome.rs` `hide_parks_…` |
| `d7290bb0` | Low-battery notifications at 10% and 5% | `menu_model.rs` `low_battery_*` (run standalone with `rustc --test`) |
| `04055a24` | ⌘Q in every app, through each window's close guard; Files never quits | `chrome.rs` `quit_closes_…` |
| `70a00336` | The dead ⇧⌘Q hint is removed | none |
| `275f87e1` | ⌃K/⌃D/⌃H/⌃F/⌃B in text fields | none |
| `acf66602` | System Settings… and Force Quit… focus the open window | `menu_model.rs` `system_apps_come_forward_…` |

To check on the laptop, one at a time:

1. Log out with an edited Text Editor document. The Save alert should come
   forward. Choosing Save or Don't Save should let the logout continue;
   Cancel should leave the session running and post "Log Out Cancelled"
   after 30 s.
2. Press ⌘Q in Text Editor with unsaved edits (it should prompt), then in
   Calculator, Clock and Terminal (each should quit), then in Files
   (nothing should happen).
3. Press ⌘H, then ⌘Tab back to the app; press ⌘M; press ⌥⌘H.
4. Press ⌥⌘⎋. Choose System Settings… twice.
5. Choose About This Lulo OS.
6. Double-click a PDF, a PNG and a `.txt` file in Files. After installing
   the new package, `xdg-mime query default application/pdf` should print
   `org.rmac.Preview.desktop`.
7. Low battery: unplug and drain to 10%, or watch
   `busctl --user monitor org.freedesktop.Notifications`.
