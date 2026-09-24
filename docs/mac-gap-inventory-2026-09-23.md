# Mac gap inventory, 2026-09-23

This report lists what rmac still needs so that someone who has used a Mac
feels at home on a low-end Linux PC. It is based on a hands-on survey of a real
Mac (macOS 26.2 "Tahoe", Dark appearance, Liquid Glass set to "Clear") and a
read of the rmac tree at commit `2d9f0a5`.

- **Reference screen.** A 1470×956 logical display at 2x, which is a notched
  built-in panel. All numbers below are in **logical points**, measured from
  full-screen captures with PIL (pixels ÷ 2). Values marked "≈" are accurate
  to about ±1 pt; those marked "~" are estimates.
- **Privacy.** The screenshots stay local under `target/evidence/mac-2026-09-23/`,
  which is gitignored. They are not committed and not referenced by content.
  This document describes UI structure only.
- **Size key.** **S** is about one engineer-week or less, **M** is two to three
  weeks, and **L** is more than three weeks.

---

## 1. What the Mac measured

### 1.1 Global chrome

| Element | Measured on macOS 26.2 | Notes |
|---|---|---|
| Menu bar height | ≈33 pt (65 px, and windows start at y=66 px) | This panel has a notch. On a display without a notch (the low-end PC case) macOS uses the shorter classic bar. Keep the height tied to the output's safe area rather than fixed. |
| Menu bar font | 13 pt; the app name is bold, the other titles regular | cap height 9.5 pt |
| Menu bar spacing | Apple logo spans x = 21–33 pt. The gap between titles, text edge to text edge, is ≈22–23 pt, so each title has about 11 pt of padding per side | |
| Menu bar background | "Show menu bar background" is **off** by default in Tahoe, so the bar is transparent over the wallpaper | Settings › Menu Bar |
| Dock | Icon ≈52 pt at the owner's Size setting, with magnification off. The glass shelf is ~82 pt tall and floats ~5 pt above the screen bottom. A separator comes before the **recent apps** group and another before Trash. Running dots sit below the icons. | Recently launched but closed apps (Clock, Weather) stayed in the recent section **without** a dot |
| Screenshot HUD (⇧⌘5) | A floating dark capsule ~545 × 44 pt, centred above the Dock. Left to right: close ⊗, three capture modes (entire screen / window / selection), two record modes (entire screen / selection), an "Options ▾" menu and a blue **Capture** pill (~78 pt wide). A dashed selection rectangle with 8 grab handles is drawn on screen. | |
| Alert (permission prompt) | Compact, ~260 pt wide. App icon at top left, bold 13 pt title, 11 pt body, a "?" help button at top right, and two **equal-width pill buttons side by side** | Seen from Weather's location request. The app was quit without answering. |

### 1.2 Windows (Tahoe)

| Element | Toolbar window (Finder, Settings, Activity Monitor, Calculator, Preview) | Title-bar-only window (TextEdit) |
|---|---|---|
| Corner radius | **≈27 pt**. A squircle fit gives 54 px on both Calculator and Activity Monitor. | **16 pt** (32 px) |
| Traffic-light diameter | **14 pt** (28 px) | 14 pt |
| Traffic-light pitch (centre to centre) | **23 pt** (a 9 pt gap) | 23 pt |
| First light's centre from the outer corner | **(26, 26) pt** | **(16, 16) pt** |
| Title bar / toolbar height | ≈52 pt unified toolbar | ≈31 pt |
| Window edge | a 1 px near-black outer line plus a 1 pt light inner highlight (≈rgb 75–78 / 81–87 in dark mode) | same |
| Title | 15 pt bold, left-aligned after the traffic lights or sidebar. Preview and Activity Monitor use **two lines**: a bold 13 pt title over an 11 pt secondary subtitle ("1 page", "My Processes"). | 13 pt semibold, centred, with a document icon and a ▾ disclosure |
| Toolbar controls | Buttons are grouped into **glass capsules** (back/forward pair, view switcher, action group, search). Capsule height ≈34 pt. | none |

### 1.3 Sidebars and lists

| Element | Measured |
|---|---|
| Finder sidebar | A **floating inset glass panel**, 8 pt in from the window's top, left and bottom edges. Inner width ≈147 pt, and the sidebar column is ≈155 pt overall. The traffic lights sit *inside* the panel. |
| Settings sidebar | Same floating panel, inner width ≈214 pt, inset 8 pt. Search field at the top, then an account card, then grouped sections with coloured rounded-square icons. |
| Sidebar row pitch / selection height | **32 pt** (with Appearance › Sidebar icon size = Medium). The selection is a rounded neutral-grey fill, not the accent colour. |
| Sidebar label / section header | 13 pt regular / 11 pt semibold secondary |
| Finder column-view row | 22 pt |
| Activity Monitor table row | 24 pt with alternating stripes. Header row ≈24 pt. |
| Finder status bar | 26 pt tall, 11 pt centred text |
| Settings grouped-form row | **38 pt**. There are 10 pt between groups. The group fill is a step lighter than the window (dark mode: rgb(39,40,50) on rgb(32,33,44)). |
| Finder window (new, `open ~`) | 947 × 833 pt |
| Settings window | 722 pt wide |

### 1.4 Apps

| App | Structure observed |
|---|---|
| Calculator | A 231 × 409 pt window with no title. The toolbar holds only a sidebar (history) toggle and a mode button. Results are right-aligned: the expression in secondary text sits above the large result. Buttons are **48 pt circles on a 54 pt pitch**, operators orange #FF9200, digits dark grey, function keys light grey. |
| Preview (PNG and PDF) | A two-line title, then capsule groups: sidebar ▾ · zoom − / fit / + · markup ▾ · rotate / crop / redact · annotate · info · share, with a search field on the right. The document runs edge to edge under the toolbar. |
| TextEdit (.txt) | A plain title bar (title, document icon and ▾) with no toolbar. Plain text uses a monospaced font. |
| Activity Monitor | Toolbar: stop ⊗ · info ⓘ · action ⋯▾, then a 5-tab segmented capsule (CPU / Memory / Energy / Disk / Network), then search. Below the table is a **bottom summary panel** (System/User/Idle, a CPU LOAD graph, Threads/Processes). |
| Disk Utility | Toolbar with labelled icon buttons (Volume +/−, First Aid, Partition, Erase, Restore, Unmount, Info). The sidebar has "Internal" and "Disk Images" sections with disclosure triangles. The main area has a volume header card, a large capacity pill, a **segmented usage bar** (Used / Other Volumes / Free) and a two-column key/value info grid. |
| Font Book | A sidebar with Fonts, Languages, Collections and Smart Collections sections. The main area is a grid of "Aa" specimen tiles showing the style count, a size slider and a grid/list toggle. |
| Clock | A 4-tab capsule in the toolbar (World Clock / Alarms / Stopwatch / Timers) and a + button. World Clock shows a dark world map with the day/night terminator and city pins, above cards with analogue clocks. |
| Weather | Asked for location access on first launch, so the main UI was not captured (see §6). |
| System Settings | Captured panes: General, Appearance, Desktop & Dock, Wallpaper, Displays. In Tahoe, **Control Center settings live in the Menu Bar pane**: the `ControlCenter-Settings` URL opens "Menu Bar". Appearance now has *Liquid Glass (Clear/Tinted)*, *Icon & widget style (Default/Dark/Clear/Tinted)*, *Folder colour*, *Sidebar icon size* and *Tint window background with wallpaper colour*. Desktop & Dock has Size and Magnification sliders, position, Genie/Scale minimise, a title-bar double-click action, "Show suggested and recent apps", Desktop & Stage Manager options and Widgets. |

---

## 2. Missing first-party apps

rmac has Files (Finder), System Settings, Terminal, Notes, TextEdit, Activity
Monitor, Apps (Launchpad), Spotlight, Control Center and Notification Center.
**No crate exists for any of the apps below.** The only related pieces are
`crates/rmac-media`, a 428-line play/pause key dispatcher, and
`crates/rmac-mounts`, which only unmounts.

| # | App | Why it matters | Minimal Mac-faithful version | Linux authority | Size |
|---|---|---|---|---|---|
| 1 | **Screenshot** (⇧⌘3 / ⇧⌘4 / ⇧⌘5) | It is used every day. Only full-screen ⇧⌘3 exists today (`packaging/rmac-session/shell.kdl:186-187` → `niri msg action screenshot-screen`). | ⇧⌘3 captures the screen. ⇧⌘4 gives a crosshair selection with a live W×H readout, and Space switches it to window-pick mode with a highlight. ⇧⌘5 opens the HUD from §1.1, with Options for save location, timer and pointer. The **floating thumbnail** appears bottom-right for 5 s: click it to open markup, swipe to dismiss, drag it into apps. Files are named "Screenshot YYYY-MM-DD at HH.MM.SS.png" and saved to Desktop. Holding ⌃ copies to the clipboard. Plays the shutter sound (already in `rmac-sound`). | niri `screenshot`/`screenshot-window` actions or wlr-screencopy / ext-image-copy-capture, `wl-copy`, and the existing `rmac-sound screenshot` cue. Recording via PipeWire and GStreamer can come later. | M |
| 2 | **Preview** (images and PDF) | Double-clicking a PDF or JPEG currently opens a foreign app. | Opens PNG, JPEG, HEIC, WebP, GIF and PDF. Has a thumbnail sidebar, zoom (⌘+ / ⌘− / ⌘0 / fit), rotate (⌘R / ⌘L), a **multi-image window** with the sidebar, text selection and search in PDFs, a two-line title with page count, and export as PNG/JPEG/PDF. Markup can wait. | `poppler-glib` (or `mupdf`, which is lighter) for PDF, `image` plus `libheif` for images | M–L |
| 3 | **Calculator** | People expect it in Spotlight and in Apps. It is tiny and very visible. | Basic mode as measured in §1.4, plus Scientific (⌘2) and a history/paper tape. Supports keyboard entry, ⌘C/⌘V, the `C`/`AC` toggle and the orange-highlight state on the pending operator. | Pure Rust, reusing `rmac-launcher-providers/src/calculator.rs` | S |
| 4 | **Archive Utility** | Downloads are often zips. There is currently no Compress item in Finder's context menu and no double-click-to-extract. | It has no main window. Double-clicking an archive extracts it next to itself (adding " 2" on name clashes), then reveals the result. A small progress window appears only for long jobs. Finder gets **Compress "X"** / **Compress N items** → `Archive.zip`. | `libarchive` (or the `zip` + `tar` + `flate2` crates) | S |
| 5 | **Music/Video player** ("QuickTime-lite") | Double-clicking MP4 or MP3 files, and Now Playing in Control Center. | A single window with a floating auto-hiding control HUD, scrubbing, space to play/pause, full screen, and MPRIS publishing so the existing media keys and `rmac-osd` work. | `libmpv` (hardware decode is good on low-end GPUs) or GStreamer `playbin` | M |
| 6 | **Clock** | It shows up in the Tahoe Dock and on the Lock Screen, and alarms and timers are useful. | The 4 tabs from §1.4. Timers and alarms post through the existing notifications stack. The world map can be a static image with the terminator computed. | `chrono-tz`; logind inhibit or systemd timers for alarms | S |
| 7 | **Weather** | It is a desktop widget and menu-bar staple. | One-location current conditions, hourly and 10-day forecasts, with a gradient background by condition. It asks for location the first time, with an alert styled as in §1.1. | Open-Meteo (no key), GeoClue2 for location | S–M |
| 8 | **Calendar** | A core productivity app, and the menu-bar clock is expected to open it. | Day, week and month views, event create/edit popovers, multiple calendars with colours, and notifications. | Evolution Data Server (EDS) over D-Bus, or CalDAV via a local store (e.g. `vdirsyncer`-compatible ics) | L |
| 9 | **Reminders** | It pairs with Calendar, and its list UI is simple. | Lists in the sidebar, rows with round checkboxes, due dates, and Today / Scheduled / All smart lists. | EDS task lists (VTODO) | M |
| 10 | **Photos-lite / image browser** | Double-clicking a folder of pictures shouldn't mean opening Finder gallery view each time. | Can be deferred: Finder's gallery view plus Preview's multi-image mode cover roughly 80% of this. | Reuse Preview | — |
| 11 | **Disk Utility** | Needed rarely, but it is the Mac answer to "format my USB stick". | A sidebar of drives and volumes, the header card, usage bar and info grid (§1.4), and Erase (FAT32/exFAT/ext4), Mount/Unmount, Eject and First Aid (fsck). | `udisks2` over D-Bus (polkit is already required) | M |
| 12 | **Font Book** | Installing a downloaded .ttf by double-click. | A grid of specimen tiles (§1.4), a preview window with an **Install** button that copies to `~/.local/share/fonts` and runs `fc-cache`. | fontconfig | S |
| 13 | Contacts | Low value without an Apple account sync story. | Skip, or add later backed by EDS. | EDS | — |

---

## 3. System features and behaviours

Status key: **P** present · **p** partial · **A** absent. The evidence paths
are relative to the repo root.

| Feature | Status | Evidence / gap |
|---|---|---|
| Quick Look (Space) | p | `crates/finder/src/quick_look.rs` handles images and text (64 KB cap). It lacks PDF, video, audio, multi-select arrows, full-screen (⌥Space) and "Open with Preview". It should share Preview's renderer. |
| ⌘Tab app switcher | p | `shell.kdl:202-207` maps `Mod+Tab` to niri's **per-window** switcher. The Mac switcher is **per app**: a centred glass strip of 64 pt icons in a 16-radius HUD. Holding ⌘ keeps it open, Q quits and H hides the highlighted app, ⌘` cycles windows within the app, and it does not appear until about 150 ms. This needs an rmac overlay. |
| Force Quit (⌥⌘⎋) | p | The Apple-menu item launches the whole System Monitor (`shell/bins/rmac-menubar/src/main.rs:1315,1669`), and ⌥⌘⎋ is not bound. The Mac uses a small dedicated window: a list of apps with "(Not Responding)" in red, and a Force Quit button. |
| Mission Control / App Exposé | p | `shell/bins/rmac-mission-control` (ADR 0014): ⌃↑ Mission Control with the measured Spaces bar (Desktop pills, thumbnails, +, remove), ⌃↓ App Exposé, F11 Show Desktop, click wallpaper to show desktop (windows slide to the edges; 75 px slivers, niri's minimum) and ⌃←/⌃→ that skip parking. Missing: live thumbnails (pictures are taken once when it opens), drag between Spaces, the Dock staying visible, and three-finger Space swipes (niri owns gestures). |
| Stage Manager | A | none. Low priority (off by default on the Mac). |
| Hot corners | p | ADR 0014: Desktop & Dock › Hot Corners offers Mission Control, Application Windows, Desktop, Notification Centre, Apps and Lock Screen per corner; the Mission Control service maps a 1 × 1 surface in each corner in use. The Tahoe default, **bottom-right → Quick Note**, has no rmac backend, so every corner starts off. |
| Spotlight | p | `crates/launcher-app` draws query results as Tahoe does (one panel under the bar, answer card, 56 pt rows, grey top hit turning blue on the arrow keys, "Search in Files" last; `design-lab/spotlight.html`). Providers: apps, Settings, files (size · date · folder), calculator with locale grouping, unit conversions, currency (ECB rates, off until allowed in Settings), time in a city, definitions when a dictd dictionary is installed. Keys: ⌘↓/⌘↑ sections, ⌘↩ Show in Files, ⌘C copy, ⌘Y Quick Look (closes Spotlight rather than hiding it). Choices are learned locally. Tahoe has **no preview pane** (captured 2026-09-23). Missing: the per-app chips row ("Calculator", "Photos"), Siri Suggestions, web results and Quick Keys. Actions (⌘3) and Clipboard (⌘4, `rmac-clipboard-service`) exist. |
| Hide / Hide Others (⌘H / ⌥⌘H) | P | `crates/rmac-compositor/src/actions.rs:304-330` implements these through the parking workspace. Check that ⌘Tab back to the app **un-hides** it. |
| App-level activation | p | `rmac-shell-activation-runtime`, `rmac-focus-linux`. Clicking a Dock icon should bring *all* of that app's windows forward. The menu bar must follow the focused app. |
| Window tiling (drag to edge, ⌃🌐 arrows) | A | No snap code. niri's scrolling tiling is a different model. Tahoe has drag-to-top-edge fill, half/quarter snapping, and Window › Move & Resize. Needs a floating-by-default policy plus a snap overlay. |
| Full screen (green button) | P | `rmac-compositor/src/actions.rs:18`, `rmac-ui/src/chrome.rs:160-180`. Missing: the green-button **hover menu** (Full Screen / Move & Resize / Fill / Tile Left-Right), and full screen as its own Space. |
| Title-bar double-click | p | Tahoe setting "Window title bar double-click action: Zoom / Minimise / Fill / Do Nothing". Should be wired to Desktop & Dock. |
| Minimise animation | p | Minimise is a 350 ms fade to parking (`rmac-design/src/motion.rs:117`). Both **Genie** and **Scale** effects are missing. The Scale effect is cheap and enough for low-end hardware. |
| Drag and drop | p | Works inside Finder (`finder/src/view/list_presentation.rs`) and for Dock reordering (`rmac-dock/src/drag.rs`). Dragging out to other apps is blocked by GPUI (PARITY.md). Missing spring-loaded folders and drag images. |
| Open/Save dialogs | A (backend) | rmac is a portal *client* only. `rmac-portals.conf` → `default=gnome;gtk;*`, so the **GNOME file chooser appears**, which breaks the illusion in every app. Needs an rmac `org.freedesktop.impl.portal.FileChooser` that presents a Finder-style sheet (sidebar, column/list view, a "Where:" popup in its collapsed form, New Folder, tags). |
| Share menu | A | Finder's Share button is decorative. The minimal version offers Copy, Mail (`xdg-email`), Messages-less and "Save to…". |
| Services menu | A | Can be deferred. |
| AirDrop-like | A | Out of scope (`docs/known-limitations.md`). Possible later with LocalSend protocol compatibility. |
| Login window | p | Only a GDM theme (`packaging/rmac-session/greeter/`). The Mac login is a centred avatar and name with a password pill over a blurred wallpaper, and a large clock. |
| Lock screen | P | `crates/rmac-lock-provider-linux`. The shortcut should be **⌃⌘Q** (the Mac binding). `shortcuts-fallback.kdl:5` uses Mod+Ctrl+Q, which matches. |
| Notifications and grouping | P | `rmac-notifications-store/src/model.rs:246`. Check for stack-by-app with "Show less" and hover "Clear All". |
| Desktop widgets | p | `crates/rmac-desktop-widgets`: Clock, Calendar (month), Weather (the Weather app's cache, refreshed every 15 min) and Batteries, small size only, on the desktop (drag to move, right-click Remove) and in Notification Centre (two per row). Edit Widgets opens the measured gallery (`shell/bins/rmac-wallpaper/src/linux_wayland/gallery.rs`) from the desktop menu or Notification Centre; drag a tile out to place it or into the Notification Centre column. Missing: Medium/Large sizes, Reminders, Photos, Notes and Calendar Up Next (no rmac data source), gallery search (no shell text field), iPhone widgets, icons reflowing around widgets. |
| Desktop icons / Stacks | p | `crates/rmac-desktop` (grid, stacks, settings) drawn by `shell/bins/rmac-wallpaper`: the measured grid (64 icons 34 from the right, 41 from the top, 112 pitch), saved free positions, marquee and ⌘/⇧ selection, drag to move or into a folder, Snap to Grid, Sort By, Clean Up / Clean Up By, Use Stacks (click to expand), Get Info, Show View Options (icon size, grid spacing, text size), Duplicate and ⌘A/⌘⌫/⌘O/⌘I/⌘D/⇧⌘N. Missing: rename, Quick Look, video posters, dragging onto the Dock/Trash/Files (GPUI has no drag-out between surfaces), label text shadow (GPUI has none), Import from iPhone. |
| Trash | P | `finder/src/trash_store.rs`, `rmac-places-system/src/trash.rs:78`, full and empty Dock icons. Check dropping onto Dock Trash and the Empty Trash confirmation alert. |
| Spell check | A | No hunspell or enchant. TextEdit and Notes need red squiggles and suggestions in the context menu. |
| Emoji picker (⌃⌘Space, fn/🌐 E) | A | Only an IME note (`platform-lab/src/capabilities.rs:46`). Needs a popover with search, categories and a recents row that inserts text through the IME or the clipboard. |
| Text system details | p | Missing ⌥-arrow word jumps and Emacs keys (⌃A / ⌃E / ⌃K) in every text field, and the ⌘⌃D dictionary. |
| Fonts | p | Inter / JetBrains Mono substitute for SF (`rmac-design/src/lib.rs:28,30`, `packaging/rmac-session/fontconfig/99-rmac.conf`). That is acceptable. The Mac uses SF Mono in TextEdit and Terminal: tune Inter's tracking to match SF metrics (see §4). |
| System sounds | P | `crates/rmac-sound`, 14 cues |
| Accessibility | p | Contrast, motion and transparency are handled (`rmac-theme/src/model.rs`, `rmac-design/src/material.rs`), with Orca as the screen reader. There is **no Zoom**: niri lacks it (`system-settings/.../screen_reader.rs:124`). Ctrl-scroll zoom is a common Mac habit. |
| Volume/brightness OSD | P | `shell/bins/rmac-osd`. Recent macOS versions show volume and brightness as a small **top-right** pill near the menu-bar status items rather than a centred square. This was not captured here (it needs a key press), so verify it before changing anything. |
| Night Shift | A | Can be done with `wlsunset` or the niri gamma API, plus a toggle in Display settings and Control Center. |
| Clipboard history | p | `rmac-clipboard-service` (ADR 0010) records text, images and files after the user allows it; Spotlight ⌘4 lists them. The populated Mac list has not been captured, so its rows are estimated. |
| Recent apps in Dock | p | The rmac Dock shows running apps that aren't pinned. The Mac keeps up to 3 **recently quit** apps without a dot (observed). |
| Menu Bar settings | p | Tahoe merges the Control Center settings into Settings › Menu Bar, with per-control "Show When Active" and "Menu bar background" off by default. |

---

## 4. UI fidelity gaps: rmac tokens compared with measured Tahoe

The rmac values come from `crates/rmac-design/src/geometry.rs`,
`typography.rs` and `design-lab/tokens.css`. The measured values are from §1.

| Token | rmac today | Measured Tahoe | Action |
|---|---|---|---|
| `radius.window` (toolbar windows) | 16 (both geometry.rs and tokens.css) | **≈27** | Split into `window_toolbar = 26` and `window_titlebar = 16`. The code has one value for both today. |
| Traffic-light diameter | 12 | **14** | Change it. The rmac lights read as too small next to Mac screenshots. |
| Traffic-light pitch | 20 (code) / gap 8 (tokens.css) | **23** (gap 9) | Unify on pitch 23. |
| Traffic-light inset | 20 with a toolbar / 8 with a plain title bar | first centre at **(26, 26)** with a toolbar; **(16, 16)** with a title bar | Express the inset as a centre offset, not a leading edge. |
| Title bar height (no toolbar) | 38 in code / 28 in tokens.css | **≈31** | Resolve the mismatch; 31 is the measured value. |
| Unified toolbar | 52 | ≈52 | ✓ |
| Toolbar controls | flat icon buttons | **grouped glass capsules** ≈34 pt tall | Add a `ToolbarGroup` capsule component. |
| Window title | title3 15 semibold | 15 **bold**; optional 11 pt subtitle line | Add a subtitle slot (document page count, "My Processes"). |
| Sidebar | opaque column, 220 wide (180 in tokens.css) | **floating inset panel**, 8 pt margin, inner ≈147 (Finder) / ≈214 (Settings) | The biggest visual gap in Finder and Settings. The traffic lights move inside the panel. |
| `sidebar_row_height` | 28 | **32** (Medium icon size) | Change it, or tie it to a "Sidebar icon size" setting (S/M/L). |
| Sidebar selection | accent (`selection_focused` = accent) | neutral grey rounded fill (≈rgb 42,43,52 in dark mode) | Tahoe sidebars no longer use accent-blue selection. |
| Settings sidebar width | 248 | ≈230 overall (214 inner + 2×8) | Reduce it. |
| Settings grouped row | 36 (tokens.css) | **38**; 10 pt group gap | Change it. |
| List row, regular | 30 in code / 24 in tokens.css | 24 table (Activity Monitor), 22 column view (Finder) | Use 24 for tables and 22 for browser columns. Drop 30. |
| Menu bar height | 29 | ≈33 on a notched panel; the classic bar on displays without a notch | Keep 29 for displays without a notch (the likely PC case), but document the difference. |
| Menu bar title padding | x 8 (a 16 pt gap); tokens.css gap 18 | gap ≈22–23 | Increase the padding to 11. |
| Menu bar font | 13 / 600 in tokens.css | 13, with **bold only for the app name** and regular for the rest | Check that other titles are not semibold. |
| Menu bar background | material | transparent by default in Tahoe | Default to no background, with an option to show it. |
| Dock tile | 64 | 52 at the owner's size (the Mac default is about 48–64) | Keep this user-configurable. The rmac default of 64 is on the large side. |
| Dock indicator | 4 pt dot, 6 below | small dot just under the icon, inside the shelf | ✓ roughly |
| Switch | 38 × 22 | ~35 × 16 visual pill (an estimate) | Re-measure with a zoomed capture before changing it. |
| Alert | 300 wide card | ~260 wide, icon top-left, **side-by-side equal pill buttons** | Update `rmac_ui::alert` (`crates/rmac-ui/src/components.rs:98`). |
| Accent | #1372F9 | the system blue in the capture (toggles and checkboxes) measured rgb(57,124,247) ≈ #397CF7 in dark mode | Minor. Re-sample from a larger flat area to confirm. |
| Calculator (new) | — | 48 pt keys on a 54 pt pitch, operator #FF9200, window 231 × 409 | New tokens. |

Recommendation: fix `design-lab/tokens.css` first and make `geometry.rs` read
from the same numbers. The code and tokens.css currently disagree on seven
values (title bar, traffic spacing, sidebar width, Dock radius and padding, CC
module radius, list row).

---

## 5. Recommended build order: next 10

The order is ranked by Mac feel gained per engineering week on a low-end PC.
Each item avoids heavy runtimes such as WebKit or Electron.

| # | Item | Size | Why this order |
|---|---|---|---|
| 1 | **Tahoe token correction pass** (§4): traffic lights 14/23 and (26,26), window radius 27/16, floating inset sidebar, 32 pt sidebar rows, grey sidebar selection, capsule toolbar groups, 38 pt grouped rows, alert layout | S–M | It changes every window at once, costs almost no CPU, and is the first thing a Mac user sees. |
| 2 | **Screenshot suite** (⇧⌘3/4/5, window pick, floating thumbnail, ⌃ to clipboard) | M | Used daily. The shortcuts are muscle memory. Sounds and niri actions already exist. |
| 3 | **⌘Tab per-app switcher plus the ⌥⌘⎋ Force Quit window** | M | Muscle memory again. niri's per-window switcher feels wrong on first use. |
| 4 | **File chooser portal backend** (Finder-style open/save sheet) | M–L | Removes the most visible "this is GNOME" moment, which appears in every app, browsers included. |
| 5 | **Preview** (images plus PDF via poppler/mupdf), sharing its renderer with **Quick Look** (PDF, video poster, Space-arrow navigation) | M–L | Double-clicking a PDF or photo is the most common file action after opening folders. |
| 6 | **Calculator** plus **Spotlight conversions** (units, currency, time zones) | S | Cheap and visible. Reuses the launcher calculator. |
| 7 | **Archive Utility** (double-click extract, Finder "Compress") | S | Downloads are zips. One small crate on libarchive. |
| 8 | **Green-button menu and tiling** (Fill, halves, quarters, drag to top edge) plus Scale minimise and hot corners | M | Tahoe window management with niri primitives, cheaper than Genie. |
| 9 | **Media player** (libmpv) with MPRIS so media keys, the OSD and Control Center Now Playing work | M | The first time a user plays a video or song should not open a foreign app. |
| 10 | **Clock and Weather** apps, then **desktop widgets** that use them | S + S–M | Fills the Tahoe Dock and desktop at low cost. Calendar and Reminders (EDS) follow as the next L item. |

Next after these: Calendar and Reminders (EDS), the emoji picker, spell check
(hunspell), Disk Utility (udisks2), Font Book, Mission Control choreography and
App Exposé, desktop Stacks, and Night Shift.

---

## 6. Survey coverage and gaps

Captured reference screens (local only, `target/evidence/mac-2026-09-23/`):

- Calculator
- Finder (home, column view)
- System Settings: General, Appearance, Desktop & Dock, Wallpaper, Menu Bar (via the Control Center URL), Displays
- Preview (PNG and PDF)
- TextEdit
- Activity Monitor
- Font Book
- Disk Utility
- Clock
- Weather (permission alert only)
- Screenshot HUD

Not captured:

- **Notes, Mail, Messages, Photos, Contacts, Calendar, Reminders.** Skipped on
  purpose because they hold private data. Their structure is described from
  platform knowledge only.
- **Weather main UI.** First launch showed a location-permission alert. It was
  left unanswered and the app was quit.
- **Archive Utility.** Extracting a 200 MB zip finished without showing a
  window, which confirms the "no UI unless slow" behaviour.
- **Terminal.** It was already running and in use by the owner, so it was not
  foregrounded.
- **Menus, Control Center, ⌘Tab, Mission Control and hover states.** These
  need UI interaction, and there was no Accessibility permission.
- **Activity Monitor's process list.** It was captured before it populated.
  Row geometry was measured from the empty striped rows.
