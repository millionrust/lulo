# Parity audit: built-in apps, 2026-09-24

This audit compares the Mac's bundled apps with their Lulo OS counterparts,
side by side:

- **Mac:** macOS 26.2, Dark mode, en-GB locale, so the Mac says "Bin",
  "Minimise" and "Centre".
- **Lulo:** the laptop running `rmac-apps 0.9.0~beta.1-38`, installed
  2026-09-24 16:44. Its display is 1920×1080 at scale 1.25, so 1536×864
  logical.

## How the evidence was gathered

**Mac.** Accessibility access worked, so every Mac menu below comes from the
live accessibility tree (`AXMenuItemCmdChar/Modifiers/Glyph`). None of them
is recalled from memory.

- Finder, Settings panes and app toolbars were read from the same tree.
- Finder was also captured in all four views, plus Get Info, Show View
  Options and Quick Look, using a neutral scratch folder.
- The Sound, Trackpad and Mouse panes could not be read: the accessibility
  dump failed with a type error. Rows marked **(K)** come from knowledge of
  macOS 26, not from a capture.
- Finder's right-click menus could not be opened synthetically (neither
  AXShowMenu nor ⌃-click brought them up). Those rows are also marked **(K)**.

**Lulo.** Each app was launched on the laptop and captured with `grim`. Its
live exported menu was read with
`gdbus call … org.rmac.AppMenu1.Menus`. Window geometry came from
`niri msg -j focused-window`. Behaviour was read from the code at `dev`
(`2851720e`).

**Privacy.** Captures stay in the session scratchpad, and nothing here
describes their private content.

**Already tracked, not repeated.** Items already in `docs/beta-gap-list.md`
or `docs/mac-gap-inventory-2026-09-23.md` are skipped unless their status is
now wrong. Those corrections are in §12.

**Key.**
- Severity: **P0** means broken, or a core feature is missing. **P1** means
  a Mac user notices at once. **P2** is polish.
- Size: **S** is a day or less, **M** is a few days, **L** is more.

---

## 0. Shared across apps: the menu bar contract

These are the gaps with the widest reach. Every app's menus come from one
static table, `crates/rmac-app-menu/src/lib.rs`, and the menu bar adds only
the bold app menu (`shell/bins/rmac-menubar/src/main.rs` `app_menu`). The
Mac, by contrast, gives every app the standard Edit, View, Window and Help
menus.

| id | Mac | Lulo | Sev | Size | Where |
|---|---|---|---|---|---|
| MENU-01 | Every app has a **Window** menu: Minimise ⌘M, Minimise All ⌥⌘M, Zoom, Zoom All, Fill fn⌃F, Centre fn⌃C, Move & Resize ›, Full-Screen Tile ›, Remove Window from Set, Bring All to Front, Arrange in Front, the tab items, then a list of open windows. | Only Files exports a Window menu, and it holds just two tab items. Every other app has **no Window menu**, even though ⌘M is bound everywhere. | P1 | M | Add a standard Window menu in the menu bar next to `app_menu`, dispatching through `rmac_compositor` actions. Follow `dispatch_app_menu_action` in `shell/bins/rmac-menubar/src/main.rs`. |
| MENU-02 | Every app has an **Edit** menu: Undo ⌘Z, Redo ⇧⌘Z, Cut ⌘X, Copy ⌘C, Paste ⌘V, Delete, Select All ⌘A, then AutoFill ›, Start Dictation…, Emoji & Symbols. Text apps add Find ›, Spelling and Grammar ›, Substitutions ›, Transformations › and Speech ›. | Text Editor's Edit menu holds only Find, Find Next, Find Previous and Replace. It has no Undo, Redo, Cut, Copy, Paste or Select All, even though the keys work in the text field. Notes' Edit menu holds only Find…. Settings, System Monitor and Clock have no Edit menu. | P1 | M | Standard Edit items for any app with a text field, wired to the gpui-component input actions (`crates/rmac-ui/src/text_keys.rs`); `TEXT_EDITOR_MENUS`, `NOTES_MENUS` |
| MENU-03 | Every item is **greyed out when it doesn't apply**: nothing selected, no document, nothing to undo. | `definition_for_vocabulary` sets `enabled: true` on every exported item, so every item is always live. | P1 | M | `crates/rmac-app-menu/src/lib.rs`. Needs a way for an app to republish its state; there is no update signal on the D-Bus interface today. |
| MENU-04 | Menus have **submenus**: Open Recent, Open With, Sort By, Find, Font, Move & Resize and more. They also have **checkmarks** on the current choice (Sort By, as Icons/List, Basic/Scientific, Celsius/Fahrenheit). | The wire format `(label, action, shortcut, enabled, separator)` has no submenu and no checked state. Radio groups are flattened: "Sort by Name / Sort by Date…", "Celsius / Fahrenheit". Limits are 8 menus and 32 items. | P1 | M | Extend `WireItem` with a `checked` flag and child items, and teach `shell/bins/rmac-menubar` to draw the submenu arrow and the ✓. |
| MENU-05 | The app menu holds **About App** (a small About window), **Settings… ⌘,** in apps that have settings, **Services ›**, the Hide items, **Quit ⌘Q**, and with ⌥, **Quit and Keep Windows ⌥⌘Q**. | "About <App>" is always disabled. There is no Settings… row, even for Terminal, which binds ⌘, to its profile picker. Services is disabled. There is no Quit and Keep Windows. | P1 | S (About), S each (Settings…) | `app_menu` in `shell/bins/rmac-menubar/src/main.rs`; per-app metadata in `crates/rmac-apps` |
| MENU-06 | Every app has a **Help** menu: "<App> Help ⌘?" plus a search field. | Only Files has one ("Files Help", with no shortcut). The other apps have no Help menu. | P2 | S | `rmac-app-menu` |
| MENU-07 | Shortcuts are shown with the Mac's glyphs, and the menu hint always matches the binding. | System Monitor's menu shows "Delete" for Quit Process, but ⌘⌫ is bound; the Mac uses ⌥⌘Q. Files shows Rename with no ↩. Text Editor's Monospaced has no hint although ⇧⌘M is bound. Files lists Quick Look as "Space" where Finder shows ⌘Y. | P2 | S | `crates/rmac-app-menu/src/lib.rs` |
| MENU-08 | Each app is **single instance**: a second document opens as a new window of the running app. | Text Editor, Notes and Preview start a second process when a second file is opened from Files. That process can't own the menu bus name, so its window has **no menu bar menus at all**. | P1 | M | Use `boot_unified_app_instance_with_assets` (`crates/rmac-ui/src/window.rs:518`) in `crates/text-editor`, `crates/preview`, `crates/notes` |
| MENU-09 | Full screen is the green button or **Enter Full Screen fn F** in the View menu. | No app exports it. | P2 | S | Standard View-menu tail item |
| MENU-10 | ⌘F in a document's text opens Find. | Probable bug, not yet checked on the laptop: `text_keys.rs:60` binds ⌘F to gpui-component's `Search` inside text fields. That handler returns early when the field isn't searchable, so ⌘F while typing in Text Editor's body or a Notes field probably does nothing. | P1 | S | `crates/rmac-ui/src/text_keys.rs` (do not bind ⌘F on non-searchable inputs) |

---

## 1. Finder vs Files

### 1.1 Window and toolbar

**Measured.**

| | Mac (new window) | Lulo |
|---|---|---|
| Window | 920×464 in the test | 923×706 |
| Sidebar | Floating inset panel, about 147 pt inner width, traffic lights inside it | Floating inset panel, traffic lights inside it |
| Title | 15 pt bold, after the back/forward capsule | Same |

**Toolbar, left to right.**
- **Mac:** a back/forward capsule, the title, a 4-segment view capsule, a
  group-by capsule (grid glyph ▾), a **Share · Tags · ⋯** capsule, and a
  search circle.
- **Lulo:** a back/forward capsule, the title, a 4-segment view capsule, a
  sort capsule, a **⋯-only** capsule, and a search circle.

**Status bar.**
- **Mac:** a 26 pt bar reading "N items, X GB available". An icon-size
  slider appears in icon view only. In gallery view the bar reads "1 of 5
  selected".
- **Lulo:** the same wording, with an icon-size slider (48–88).

| id | Mac | Lulo | Sev | Size | Where |
|---|---|---|---|---|---|
| FILES-01 | The toolbar has **Share** (Copy, AirDrop, Mail and similar) and **Tags** buttons. | Absent by design (`toolbar.rs:158`). | P2 | M | `crates/finder/src/view/chrome_presentation/toolbar.rs`. A share sheet needs a portal backend; the Tags button needs FILES-20. |
| FILES-02 | The group-by button menu starts with **Use Groups ⌃⌘0**, then **Sort By ›**: None, Name, Kind, Date Last Opened, Date Added, Date Modified, Date Created, Size, Tags. | It offers only Sort By Name, Date Modified, Size and Kind. There is no grouping, and picking the current key again silently reverses the order. | P2 | M | `menus_tabs.rs:4-21`, `selection_controller.rs:324` |
| FILES-03 | Search stays open while a query is active and collapses when the query is cleared or on Esc. | Once opened, search never collapses: `search_open` is only ever set to true. | P2 | S | `toolbar.rs:219`, `startup.rs:290` |

### 1.2 Sidebar

**Mac rows, top to bottom:**
- Untitled group: Recents, Shared.
- **Favourites:** Applications, Downloads, Documents, Desktop, and folders
  the user has added.
- **Locations:** iCloud Drive, the home folder, AirDrop, Bin, volumes and
  Network.
- **Tags:** Red, Green, Orange, Yellow, Blue, Purple, Grey, user tags, and
  "All Tags…".

**Lulo on the laptop:**
- Recents and Shared.
- Favourites: Applications, Downloads, Documents, Desktop.
- Locations: the home folder, Trash.

| id | Mac | Lulo | Sev | Size | Where |
|---|---|---|---|---|---|
| FILES-04 | **Add to Sidebar ⌃⌘T**, dragging a folder into Favourites, reordering, and **Remove from Sidebar** from a right-click on a row. | Favourites are hard-coded. A folder dropped on the sidebar is *moved into* the row it lands on. The sidebar has no right-click menu. | P1 | M | `crates/finder/src/places.rs:45-61`, `view/sidebar.rs`; persist a favourites list |
| FILES-05 | The **Tags** section is always present. | Built only under `cfg(target_os = "macos")`, so it is absent on Lulo. | P1 | M, with FILES-20 | `crates/finder/src/view/startup.rs:107-119`, `crates/rmac-search/src/platform.rs:106` |
| FILES-06 | The selected row follows the view: Recents is highlighted while Recents is showing. | Rows compare only `cwd == path`. Recents and tag views never highlight, and the previous folder's row stays lit. | P2 | S | `view/sidebar.rs:7-11` |
| FILES-07 | The sidebar lists **Network** and connected servers. | Neither exists; there is no Connect to Server either (FILES-31). | P2 | L | new |

### 1.3 Views

| id | Mac | Lulo | Sev | Size | Where |
|---|---|---|---|---|---|
| FILES-08 | **List view:** the disclosure triangles expand folders inline, with ⌘→ and ⌘← on the keyboard. | The chevron is drawn but has no click handler, so folders can't be expanded. | P1 | M | `crates/finder/src/view/list_presentation.rs:316-334` |
| FILES-09 | **List view:** columns can be resized, reordered and chosen (Date Created, Date Last Opened, Date Added, Version, Comments, Tags). Right-click the header to pick columns. "Calculate all sizes" is an option. | Four fixed columns (Name, Date Modified, Size, Kind), fixed widths, no chooser. | P2 | M | `list_presentation.rs:130-173` |
| FILES-10 | **Column view:** ↑↓ move within the current column and ←→ move between columns. | **Bug:** arrow keys and type-to-select act on `entries` while Column view reads `column_selection`, so the arrows clear the selection. ←/→ are not handled. | **P0** | S | `list_presentation.rs:856-901`, `selection_controller.rs:37-68`, `horizontal_navigation` (`:450`) |
| FILES-11 | **Column view:** the preview column shows a large preview, name, kind and size, **Information** (Created, Modified, Last opened), **Tags**, and **Show More**. | The preview column's Information holds only Modified. | P2 | S | `content_presentation.rs:7-83` |
| FILES-12 | **Gallery view:** the right-hand preview pane has Information (Created, Modified, Last opened), a Tags field, and **Markup** and **More…** quick-action buttons. | The inspector shows Kind, Size and Modified. It has no Tags field and no quick actions. | P2 | S | `gallery_presentation.rs` |
| FILES-13 | **Show View Options ⌘J** opens a floating panel. <br>For list view it has: Always open in list view, Browse in list view, Group By, Sort By, Icon size (2 choices), Text size, Show Columns (12 checkboxes), Use relative dates, Calculate all sizes, Show icon preview, and Use as Defaults. <br>Icon view adds icon size, grid spacing, label position, and background. | No View Options panel and no ⌘J. | P1 | M | new overlay in `crates/finder/src/view/`; per-folder persistence in `presentation_persistence.rs` |
| FILES-14 | Show/Hide **Path Bar ⌥⌘P**, Show/Hide **Status Bar ⌘/**, Show/Hide **Sidebar ⌃⌘S**, **Show Preview ⇧⌘P**, Hide **Toolbar ⌥⌘T**: all in the View menu, all remembered. | Path bar (⌥⌘P) and sidebar (⌃⌘S) work from the keyboard only; they are not in the menu. The status bar can't be toggled. There is no preview pane. The path bar, sort order, hidden-files setting and icon size are not remembered. | P2 | S (menu items) / M (preview pane) | `FILES_MENUS`; `presentation_persistence.rs:29-44` |

### 1.4 Get Info (⌘I)

**Mac** (a 265×684 separate window, title "<name> Info"; several can be open at once):
1. Header: icon, name, size, "Modified:".
2. **Add Tags…** field.
3. **General:** Kind, Size (bytes, size on disk, item count), Where (a path
   of breadcrumbs), Created, Modified, **Shared folder**, **Locked**.
4. **More Info:** Last opened.
5. **Name & Extension:** a field and **Hide extension**.
6. **Comments.**
7. **Open with** (files only).
8. **Preview.**
9. **Sharing & Permissions:** an editable table, a lock, and a ⋯ menu.

Every section header has a disclosure triangle.

**Lulo:** a modal card over the window. It shows the first selected item only.

| id | Mac | Lulo | Sev | Size | Where |
|---|---|---|---|---|---|
| FILES-15 | Get Info is a **separate, non-modal window**, with one per item for a multi-selection. **Show Inspector ⌥⌘I** gives one live panel, and **Get Summary Info ⌃⌘I** covers a multi-selection. | A modal card, one at a time, first item only. | P1 | M | `crates/finder/src/view/search_info_controller.rs:187-411` |
| FILES-16 | Folder **size** is calculated, with the item count. | No size for folders. | P1 | S | `filesystem_helpers.rs:200-226` |
| FILES-17 | Where is a breadcrumb path; the Mac shows "Macintosh HD ▸ …". | A raw parent path. | P2 | S | same |
| FILES-18 | Tags, Hide extension, Comments, Locked, **Open with** (with **Change All…**), and **editable** permissions. | None of these. Permissions are a read-only `rwxr-xr-x` string. | P1 (Open with, permissions) / P2 (the rest) | M | same |
| FILES-19 | Collapsible section headers. | Sections are flat. | P2 | S | same |

### 1.5 Tags

| id | Mac | Lulo | Sev | Size | Where |
|---|---|---|---|---|---|
| FILES-20 | Tag files with colour tags and names from the right-click row, the toolbar, Get Info and the gallery pane. Filter by tag in the sidebar. A Tags column in list view. Sort and group by tags. | Files **cannot be tagged** on Linux: nothing writes xattrs, and there is no tag UI. | P1 | L | new: `user.xdg.tags` xattr (the freedesktop convention) in `crates/rmac-search` or `crates/finder` |

### 1.6 Context menus

These are the Mac menus from knowledge of macOS 26 **(K)**. The Lulo menus
come from `menus_tabs.rs:26-148`.

**Mac, file (K):**
1. Open
2. Open With ›
3. separator
4. Move to Bin
5. separator
6. Get Info
7. Rename
8. Compress "x"
9. Duplicate
10. Make Alias
11. Quick Look
12. separator
13. Copy
14. Share…
15. separator
16. the seven colour dots
17. Tags…
18. separator
19. Quick Actions › (Rotate, Markup, Create PDF, Convert Image, Customise…)
20. Services › (and similar)

A folder adds Open in New Tab and Folder Actions Setup… under Services. A
multi-selection gives "New Folder with Selection (N Items)" and
"Compress N Items".

**Mac, background (K):**
1. New Folder
2. separator
3. Get Info
4. separator
5. Use Groups
6. Sort By ›
7. Show View Options
8. separator
9. Paste Item when the clipboard has files

**Lulo, file:**
1. Undo X (when there is something to undo)
2. separator
3. Open ⌘↓
4. Open With… (opens a dialog, not a submenu)
5. separator
6. Move to Trash
7. Get Info
8. Rename
9. Compress "x"
10. Duplicate
11. Quick Look
12. separator
13. Copy

**Lulo, background:**
1. New Folder
2. Paste Item
3. separator
4. View as Icons / List / Columns / Gallery
5. separator
6. Sort by Name / Date Modified / Size / Kind, as flat items
7. separator
8. Select All

| id | Mac | Lulo | Sev | Size | Where |
|---|---|---|---|---|---|
| FILES-21 | Open With is a **submenu** listing the default app first ("(default)"), then the other apps, then "Other…". | A dialog. | P2 | M, needs MENU-04's submenu support in the in-window menu | `crates/finder/src/view/dialog_presentation/open_with.rs` |
| FILES-22 | **Make Alias ⌃⌘A**, **Show Original ⌘R**. | Absent. Symlinks are the natural backing. | P2 | S | `menus_tabs.rs`, `startup/shortcuts.rs` |
| FILES-23 | **Share…**, the **tag dots**, **Tags…**, **Quick Actions ›** and **Services ›**. | Absent. | P2 | M–L | as above |
| FILES-24 | A **folder** adds Open in New Tab. A **multi-selection** adds New Folder with Selection. | Absent. | P2 | S | `menus_tabs.rs` |
| FILES-25 | The background menu has **Get Info** (for the folder itself), **Use Groups**, a **Sort By** submenu with ✓, and **Show View Options**. | Lulo has flat sort items with no check and "View as …" rows, which Finder doesn't put in this menu. There is no Get Info for the current folder. | P2 | S | `menus_tabs.rs` |
| FILES-26 | Rename is disabled for a multi-selection. That is a batch rename on the Mac ("Rename N Items…"). | Rename is listed, and choosing it shows a notice. | P2 | S (hide) / M (batch rename) | `rename_controller.rs:7-19` |

### 1.7 Quick Look

**Measured on the Mac.** Quick Look is a floating panel. Its title strip
holds:
- a close ⊗ button;
- a full-screen button;
- the file name;
- a Share button;
- an **"Open with <App>"** button.

Space toggles the panel. With several files selected, ←→ step through them;
with one file selected, ↑↓ keep moving through the folder list behind it.

| id | Mac | Lulo | Sev | Size | Where |
|---|---|---|---|---|---|
| FILES-27 | With one file selected, **↑/↓ (or ←/→ in icon view) keep moving the selection in the window**, and Quick Look follows it. | Arrows step through the *selected* items only, so with one item selected you can't browse the folder. | P1 | S | `crates/rmac-quick-look/src/lib.rs:7`, `crates/finder/src/view/quick_look_controller/controller.rs` |
| FILES-28 | **⌘Y** opens Quick Look (it is the File-menu shortcut). The panel also has Share, Markup and Rotate. | ⌘Y is not bound. There is no Share, Markup or Rotate. | P2 | S (⌘Y) | `startup/shortcuts.rs`; `rmac-quick-look/src/panel.rs` |

### 1.8 Windows and tabs

| id | Mac | Lulo | Sev | Size | Where |
|---|---|---|---|---|---|
| FILES-29 | **⌘N** opens a new window showing the "New Finder windows show" folder (Recents by default). **⌘W** closes the tab, or the window when it has one tab. A new window is independent. | **Bug:** a new window restores the *other* window's saved tabs and then moves its active tab to Home. All windows write one shared state file, so the last write wins. **⌘W with a single tab does nothing**; only the red button closes the window. Launch restores the last folder: the laptop opened on "untitled folder" on the Desktop, never on Recents. | **P0** (⌘W), P1 (⌘N state) | S / M | `crates/finder/src/view/startup.rs:175-187`, `view.rs:379`, `navigation.rs:45-47`, `presentation_persistence.rs:161-234` |
| FILES-30 | **⌘-double-click** a folder opens it in a new tab (in a new window when "Open folders in tabs" is off). **Open in New Tab ⌃⌘O.** The tab bar is shown with ⇧⌘T; **Show All Tabs ⇧⌘\\**; **⇧⌘[ / ⇧⌘]** switch tabs. Tabs are titled with folder names. | ⌘-click only toggles selection. There is no ⌃⌘O, ⇧⌘[ / ⇧⌘], or ⇧⌘T. The tab name falls back to a hard-coded "Macintosh HD" instead of `root_volume_name()`. | P2 | S | `selection_controller.rs:13-17`, `menus_tabs.rs:169` |

### 1.9 Menus: Finder compared with Files

**The live Lulo menus**, as exported:

- **Files (app menu):** Empty Trash… ⇧⌘⌫
- **File:**
  - New Finder Window ⌘N
  - New Folder ⇧⌘N
  - New Tab ⌘T
  - Close Tab ⌘W
  - Move to Trash ⌘⌫
  - Get Info ⌘I
  - Rename
  - Compress
- **Edit:**
  - Undo ⌘Z
  - Cut ⌘X
  - Copy ⌘C
  - Paste ⌘V
  - Select All ⌘A
- **View:**
  - as Icons ⌘1, as List ⌘2, as Columns ⌘3, as Gallery ⌘4
  - four "Sort by" items
  - Show Hidden Files ⇧⌘.
  - Quick Look (Space)
- **Go:**
  - Back ⌘[, Forward ⌘], Enclosing Folder ⌘↑
  - Home ⇧⌘H, Applications ⇧⌘A, Downloads ⌥⌘L, Trash
  - Go to Folder… ⇧⌘G
- **Window:** Show Previous Tab ⌃⇧⇥, Show Next Tab ⌃⇥
- **Help:** Files Help

**Items on the Mac with no Lulo equivalent**, from the measured Finder menus:

| id | Menu | Missing items (Mac shortcut) | Sev | Size |
|---|---|---|---|---|
| FILES-31 | File | Open ⌘O; Open in New Tab ⌃⌘O; Open With › and Always Open With ›; Close Window ⌘W and Close All ⌥⌘W; Show Inspector ⌥⌘I; Get Summary Info ⌃⌘I; Duplicate ⌘D (bound, not in the menu); Make Alias ⌃⌘A; Quick Look ⌘Y; Print ⌘P; Share…; Show Original ⌘R; Add to Sidebar ⌃⌘T; Add to Dock ⌃⇧⌘T; Delete Immediately… ⌥⌘⌫ (bound, not in the menu); Eject ⌘E; Find ⌘F; New Folder with Selection ⌃⌘N; New Smart Folder | P1 (Open, Duplicate, Find, Close Window, Eject) / P2 (the rest) | S for bound actions, M otherwise |
| FILES-32 | Edit | Redo ⇧⌘Z (no redo at all); Copy "x" as Pathname ⌥⌘C; Move Item Here ⌥⌘V (Finder moves with ⌘C then ⌥⌘V); Deselect All ⌥⌘A; Show Clipboard | P1 (Redo, ⌥⌘C, ⌥⌘V) | M (redo journal), S (the others) |
| FILES-33 | View | Use Groups ⌃⌘0; a Sort By › submenu with ⌃⌥⌘0–7; Clean Up and Clean Up By › (⌥⌘1–7); Show Tab Bar ⇧⌘T; Show All Tabs ⇧⌘\\; Hide Sidebar ⌃⌘S (bound, not in the menu); Show Preview ⇧⌘P; Hide Toolbar ⌥⌘T; Show Path Bar ⌥⌘P (bound, not in the menu); Hide Status Bar ⌘/; Customise Toolbar…; Show View Options ⌘J; Enter Full Screen | P2 (P1 for ⌘J, see FILES-13) | S (menu rows) |
| FILES-34 | Go | Enclosing Folder in New Window ⌃⌘↑; Recents ⇧⌘F; Documents ⇧⌘O; Desktop ⇧⌘D; Computer ⇧⌘C (bound, not in the menu); Network ⇧⌘K; Utilities ⇧⌘U; Library; Recent Folders ›; Connect to Server… ⌘K | P1 (⇧⌘D, ⇧⌘O, ⇧⌘F are muscle memory) | S |
| FILES-35 | Finder app menu | Settings… ⌘, (General: "New windows show", "Open folders in tabs"; Tags; Sidebar checkboxes; Advanced: show extensions, warn before emptying the Bin, remove items from the Bin after 30 days, keep folders on top) | P1 | M |
| FILES-36 | Finder app menu | Empty Bin (no confirmation) ⌥⇧⌘⌫ | P2 | S |

The other Finder rows are either still missing and already tracked (S9 in the
beta list, now mostly fixed) or fine.

### 1.10 Search

| id | Mac | Lulo | Sev | Size | Where |
|---|---|---|---|---|---|
| FILES-37 | **⌘F** opens search. A scope bar offers "Search: This Mac / '<folder>'". The + button adds kind and date criteria. Searches can be saved as Smart Folders. | ⌘F is not bound; search opens only from the toolbar circle. There is no scope bar: searches cover the current folder only. There are no filters. | P1 (⌘F, scope) | S (⌘F) / M (scope via `rmac-search`) | `crates/finder/src/view/startup/shortcuts.rs`, `search_info_controller.rs:23-94` |

---

## 2. System Settings vs Settings

### 2.1 Window

| | Mac | Lulo |
|---|---|---|
| Window width | 722 pt | 723×832 in code (699×706 on the 864 pt-tall laptop screen) |
| Sidebar | Floating panel 215 pt wide inside a 223 pt column, 32 pt rows | 215 in 223 ✓ |
| Toolbar | 52 pt, with a Back / Forward pair and the title | ✓ |

**Mac menus:**
- **Edit:** the standard Edit items.
- **View:**
  - Back ⌘[, Forward ⌘], Search ⌘F
  - an alphabetical list of **all 41 panes**
  - Enter Full Screen
- **Help:** System Settings Help ⌘? and others.

**Lulo menus:** View › Back ⌘[ only.

| id | Mac | Lulo | Sev | Size | Where |
|---|---|---|---|---|---|
| SET-01 | The View menu has **Forward ⌘]** and **Search ⌘F**, plus every pane by name. | Forward is bound in the window but not in the menu. **⌘F is not bound at all.** There is no pane list. | P1 (⌘F) / P2 | S | `crates/rmac-app-menu/src/lib.rs` `SETTINGS_MENUS`; `crates/system-settings/src/controller.rs:282-304` |
| SET-02 | Back and Forward move through **pane history** (Wi-Fi → Sound → Back returns to Wi-Fi). | Back and Forward work only inside a pane's subpages. Changing pane clears both stacks. | P2 | S | `controller/navigation_state.rs:143-243` |
| SET-03 | Sidebar search also finds settings **inside** panes and highlights the matching control ("Dock size" shows Desktop & Dock with the slider lit). | It filters panes by name and keywords only. Some keywords promise controls that don't exist: "Dock size", "password required", "notification previews", "test lock screen". | P2 | M | `crates/system-settings/src/settings_search.rs` |
| SET-04 | The account card at the top opens **Apple Account**. On a local account, the equivalent is Users & Groups. | The card is not clickable. | P2 | S (point it at SET-06) | `controller/chrome.rs:213-253` |

### 2.2 Sidebar pane list

These are the Mac's panes, read from the sidebar in order. Blank lines in the
Mac sidebar mark the groups.

| Mac group | Mac pane | Lulo |
|---|---|---|
| Account | Apple Account | **Lacks** (out of scope). The account card shows "Local Account". |
| | Family | Lacks (out of scope) |
| Network | Wi-Fi | **Has** |
| | Bluetooth | **Has** |
| | Network | **Has**, plus a separate **VPN** row (the Mac puts VPN inside Network) |
| | Battery | **Has** |
| System | General | **Partial**. The Mac's General has About, Software Update, Storage, AppleCare & Warranty, AirDrop & Handoff, AutoFill & Passwords, Date & Time, Language & Region, Login Items & Extensions, Sharing, Startup Disk, Time Machine, Device Management, Transfer or Reset. Lulo has About, Software Update, Storage, Date & Time, Language & Region, Login Items and Sharing. |
| | Accessibility | **Partial**: Screen Reader, Display, Motion, Pointer Control. Zoom, Hover Text, Spoken Content, Sticky/Slow/Mouse Keys and more are absent. |
| | Appearance | **Partial** (see SET-20) |
| | Apple Intelligence & Siri | Lacks (skip) |
| | Desktop & Dock | **Partial** (see SET-30) |
| | Displays | **Partial** (see SET-15) |
| | Menu Bar | **Has** (it includes the Control Centre controls) |
| | Spotlight | **Has** |
| | Wallpaper | **Has**. The Mac's Screen Saver is inside Wallpaper; Lulo has no screen saver. |
| Alerts | Notifications | **Has** |
| | Sound | **Has** |
| | Focus | **Has** |
| | Screen Time | Lacks (skip) |
| Security | Lock Screen | **Partial** (SET-08) |
| | Privacy & Security | **Partial**: only Camera and Microphone, plus security updates |
| | Touch ID & Password | **Lacks** (SET-05) |
| | Users & Groups | **Lacks** (SET-06) |
| Accounts | Internet Accounts | Lacks (P2; GNOME Online Accounts could substitute) |
| | Game Center | Lacks (skip) |
| | iCloud | Lacks (skip) |
| | Wallet & Apple Pay | Lacks (skip) |
| Input | Keyboard | **Partial** (SET-40) |
| | Trackpad | **Partial** (SET-50) |
| | Mouse (when a mouse is attached) | **Has**, but always listed |
| | Printers & Scanners | **Lacks** (SET-07) |

Order: the Mac puts Keyboard, Trackpad and Printers after the Accounts group.
Lulo puts Keyboard, Mouse and Trackpad last, which matches with the account
groups left out. **The Mac orders Trackpad before Mouse**; Lulo orders Mouse
before Trackpad (P2, `navigation.rs:232-252`).

| id | Gap | Sev | Size | Where |
|---|---|---|---|---|
| SET-05 | **Login Password** pane (the Mac calls it "Touch ID & Password"): change password, and require a password after sleep. The first password change after setup has no home. | P1 | M | new pane; AccountsService + `passwd` over PAM |
| SET-06 | **Users & Groups**: list users, add or remove a user (admin), change the picture, automatic login, guest user. | P1 | L | new pane; AccountsService D-Bus |
| SET-07 | **Printers & Scanners**: list printers, add a printer (+), default printer, paper size, open the print queue. Printing works through the portal (S7) but printers can't be managed. | P1 | M | new pane; CUPS D-Bus / `lpadmin` via polkit, `crates/rmac-print-linux` |
| SET-08 | **Lock Screen** has: start Screen Saver when inactive; turn display off on battery / on power adapter when inactive; **require password after screen saver begins or display is turned off** (Immediately … 8 hours); show a large clock; show the user name and photo; show password hints; show a message when locked; login window shows (List of users / Name and password); Show the Sleep, Restart and Shut Down buttons. Lulo has two popups (lock and sleep when inactive) and a Lock Now button. | P1 (require-password delay), P2 (the rest) | M | `controller/lock_screen/render.rs` |
| SET-09 | **Screen Saver** (under Wallpaper). | P2 | M | new |
| SET-10 | **Startup Disk**, **Transfer or Reset**, **Time Machine**. A backup equivalent (Déjà Dup-like) is expected by Mac users. | P2 (Time Machine P1 for data safety, but L) | L | new |
| SET-11 | **AirDrop & Handoff**. | skip (tracked as out of scope) | — | — |

### 2.3 The eight panes a new user touches

#### Wi-Fi

Measured on the Mac, top to bottom:
1. Header card: icon, "Wi-Fi", a description and Learn More…, and a switch.
2. The current network row: lock and signal glyphs, "Connected", and a
   **Details…** button.
3. **Personal Hotspot** section.
4. **Known Networks**: rows with lock and signal glyphs and a **⋯** menu
   (Auto-Join, Forget This Network…, Network Settings…).
5. **Other Networks**: a scanning list, then an **Other…** button.
6. **Ask to join networks** popup (Off / Notify / Ask).
7. **Ask to join hotspots** popup (Never / Ask to Join / Automatic).
8. **Advanced…** button.
9. **?** Help.

Lulo has the header switch, a current-network row with no Details…, Known
Networks with a **Forget…** button on each row, Other Networks with click to
join, **Refresh** and a footnote.

| id | Mac | Lulo | Sev | Size | Where |
|---|---|---|---|---|---|
| SET-12 | **Details…** on the current network opens a sheet with: TCP/IP (Configure IPv4, IP address, Router), DNS, Proxies, **Low Data Mode**, **Auto-Join** and **Forget This Network…**. | The row is display-only. Network › service has an IPv4, DNS and proxy editor, but Wi-Fi doesn't link to it. | P1 | S (route to the existing NetworkService subpage) | `controller/wifi/render.rs:71-86`, `controller/network/render.rs` |
| SET-13 | The known-network **⋯** menu offers Auto-Join ✓ and Forget. **Ask to join networks** and **Ask to join hotspots** popups. **Advanced…** has the known-network list with auto-join checkboxes, "Require administrator authorisation to change networks", and the Wi-Fi MAC address. | Forget only. None of the others. | P2 | S (auto-join via the NM `autoconnect` property) | `controller/wifi/render.rs` |
| SET-14 | The Mac scans for other networks continuously and shows a spinner. | Scanning needs a manual **Refresh** button; there is no live scan. The laptop showed "No other networks found" on first open. | P1 | S | `controller/wifi` (subscribe to NM `AccessPointAdded` / periodic `RequestScan` while the pane is open) |

The hidden network row ("Other…") is already tracked as S10.

#### Bluetooth

Measured on the Mac:
1. Header with a switch.
2. "This Mac is discoverable as '<name>' while Bluetooth Settings is open"
   (text, not a toggle).
3. **My Devices**: each row has a name, status and an **ⓘ** button (Connect,
   Disconnect, Forget, device type, battery).
4. **Nearby Devices** with a spinner.
5. **Advanced…** and Help.

Lulo has a power switch, a **Discoverable** toggle, My Devices rows with
Connect/Disconnect and **Forget…** buttons, and Nearby Devices with **Pair**,
**Refresh** and a footnote.

| id | Mac | Lulo | Sev | Size | Where |
|---|---|---|---|---|---|
| SET-15a | The Mac is discoverable automatically while the pane is open. There is no toggle. | A manual toggle. | P2 | S | `controller/bluetooth/render.rs:37` |
| SET-15b | Scanning runs continuously while the pane is open. Rows show a battery level for devices that report one. There is a per-device ⓘ sheet. | A Refresh/Scanning button, no battery level, no ⓘ. | P2 | S–M | `view_helpers/bluetooth.rs`; BlueZ `Battery1` |

#### Displays

Measured on the Mac:
1. A display picker with the built-in display.
2. **Use as** (Main display / Mirror).
3. A resolution picker. The Mac shows **"Larger Text … Default … More Space"
   thumbnails** for scaled sizes.
4. **Brightness** slider.
5. **Automatically adjust brightness**.
6. **True Tone**.
7. **Colour profile**.
8. "When connected to TV".
9. **Advanced…** and **Night Shift…** buttons.
10. Help.

Lulo has an arrangement preview; Resolution, Scale (100–200 %) and Rotation
popups; facts; Use as Main Display; and Refresh.

| id | Mac | Lulo | Sev | Size | Where |
|---|---|---|---|---|---|
| SET-16 | A **Brightness slider** for the built-in display. | **Absent.** The brightness keys work through the OSD (logind `SetBrightness`), but Settings has no slider. | **P1** | S | `controller/displays/render/output.rs`; reuse `crates/rmac-osd/src/linux.rs` |
| SET-17 | **Night Shift…** (a schedule, and warmer ↔ less warm). | Absent (already in the gap inventory as absent, still missing). | P1 | M | tracked as inventory "Night Shift"; wlsunset or gamma |
| SET-18 | Resolution is shown as **Larger Text ↔ More Space** thumbnails (4–5 choices). "Show all resolutions" is in the ⋯ menu. | Raw Resolution and Scale popups with a refresh rate in the label ("1920 × 1080 at 60.049 Hz"). | P2 | M | `output.rs:72-192` |
| SET-19 | The arrangement can be dragged (Arrange… when several displays are connected). Refresh rate, HDR, Automatically adjust brightness, True Tone and Colour profile are also here. | Arrangement is a popup (already tracked). No refresh-rate popup; the others are hardware-dependent. | P2 | M | same |

#### Sound

**(K)** The Mac has two sections:
- **Sound Effects:** Alert sound (a list of about 14 named sounds), Play
  sound effects through, Alert volume, Play sound on startup, Play user
  interface sound effects, Play feedback when volume is changed.
- **Output & Input:** a tab control; a device table with Name and Type
  columns; Output volume, Balance and Mute; Input volume and an **input
  level meter**.

Lulo is close in structure.

| id | Mac | Lulo | Sev | Size | Where |
|---|---|---|---|---|---|
| SET-20s | About 14 alert sounds (Boop, Breeze, Bubble, Crystal, Funky, Heroine, Jump, Mezzo, Pebble, Pluck, Pong, Sonar, Sonumi, Submerge). Choosing one plays it. | Three sounds (Alert, Error, Notification) and a ▶ button. | P2 | S (needs original sound assets) | `controller/sound/render.rs:14-94`, `crates/rmac-sound` |
| SET-21s | Output and Input are **tabs** over one device table. The Input tab has a live **input level** meter. | Separate stacked lists and no input level meter. | P2 | M | `controller/sound/render.rs:166-304` |

#### Appearance

Measured on the Mac:
1. Appearance: Auto / Light / Dark thumbnails.
2. **Liquid Glass**: Clear / Tinted.
3. **Colour**: accent swatches and Multicolour.
4. **Text highlight colour** popup.
5. **Icon & widget style**: Default / Dark / Clear / Tinted.
6. **Folder colour** popup.
7. **Sidebar icon size** popup.
8. Tint window background with wallpaper colour.
9. **Show scroll bars**: Automatically based on mouse or trackpad / When
   scrolling / Always.
10. **Click in the scroll bar to**: Jump to the next page / Jump to the spot
    that's clicked.

Lulo has Auto/Light/Dark, Colour swatches, the tint toggle, and the Contrast
and Motion popups.

| id | Mac | Lulo | Sev | Size | Where |
|---|---|---|---|---|---|
| SET-22 | **Show scroll bars** (3 choices) and **Click in the scroll bar to** (2 choices). | Absent. | P2 | M, needs GPUI scrollbar policy | `controller/appearance/render.rs` |
| SET-23 | **Text highlight colour**, **Sidebar icon size** (Small/Medium/Large), **Folder colour**, **Icon & widget style**, **Liquid Glass Clear/Tinted**. | Absent. Contrast and Motion sit here in Lulo; they live in Accessibility on the Mac. | P2 | M | same |

#### Desktop & Dock

Measured on the Mac:

- **Dock:**
  - **Size** slider (Small ↔ Large).
  - **Magnification** slider (Off, Small ↔ Large).
  - Dock position on screen (Left / Bottom / Right).
  - **Minimised window animation** (Genie / Scale).
  - **Window title bar double-click action** (Zoom / Minimise / Fill / Do
    Nothing).
  - **Minimise windows into application icon**.
  - Automatically hide and show the Dock.
  - **Animate opening applications**.
  - **Show indicators for open applications**.
  - **Show suggested and recent apps in Dock**.
- **Desktop & Stage Manager:**
  - Show items: On Desktop ☑ / In Stage Manager ☑.
  - Click wallpaper to show desktop (Always / Only in Stage Manager).
  - Stage Manager.
  - Show recent apps in Stage Manager.
  - Show windows from an application.
- **Widgets:**
  - Show Widgets: On Desktop / In Stage Manager.
  - Dim widgets on desktop.
  - iPhone Widgets.
- **Default web browser** popup.
- **Windows:**
  - **Prefer tabs when opening documents** (Never / Always / In Full
    Screen).
  - **Ask to keep changes when closing documents**.
  - **Close windows when quitting an application**.
  - **Drag windows to left or right edge of screen to tile**.
  - **Drag windows to menu bar to fill screen**.
  - **Hold ⌥ key while dragging windows to tile**.
  - **Tiled windows have margins**.
- **Mission Control:**
  - Automatically rearrange Spaces based on most recent use.
  - When switching to an application, switch to a Space with open windows.
  - Group windows by application.
  - Displays have separate Spaces.
  - Drag windows to top of screen to enter Mission Control.
- **Shortcuts…** and **Hot Corners…** buttons.

**Lulo** has:
- **Dock:** Position; Auto-hide; Reserve screen space (not a Mac control);
  Magnification as an on/off toggle with a 1.25× / 1.5× / 2× popup; Click a
  focused app again.
- **Displays:** Show the Dock on.
- **Desktop:** Click wallpaper to show desktop.
- **Hot Corners:** four inline popups.

| id | Mac | Lulo | Sev | Size | Where |
|---|---|---|---|---|---|
| SET-30 | A **Dock Size slider**. | **Absent**, even though "Dock size" is a search keyword. | **P1** | S | `crates/system-settings/src/controller/desktop_dock.rs`, `crates/rmac-shell-settings` (the Dock tile size is 64 in code) |
| SET-31 | **Magnification is a slider** (Off → Large). | A toggle plus a 3-step popup. | P2 | S | same |
| SET-32 | **Minimised window animation** (Genie / Scale), **Minimise windows into application icon**, **Animate opening applications**, **Show indicators for open applications**, **Show suggested and recent apps in Dock**. | Absent. The Dock does show running indicators and recent apps; they just can't be switched off. | P1 (indicators and recents, because they change the Dock's look), P2 (the rest) | S each | `desktop_dock.rs`; `crates/rmac-dock` |
| SET-33 | **Window title bar double-click action** (Zoom / Minimise / Fill / Do Nothing). | Absent (tracked as S13; still missing). | P1 | S | as S13 |
| SET-34 | **Windows** section: Prefer tabs, **Ask to keep changes when closing documents**, **Close windows when quitting an application**, **Drag windows to left or right edge to tile**, **Drag windows to menu bar to fill screen**, **Hold ⌥ while dragging to tile**, **Tiled windows have margins**. | The whole section is absent. The tiling rows depend on L1 (drag to tile). "Close windows when quitting" (window restoration) has no backend. | P2 (P1 once L1 lands) | M | new rows in `desktop_dock.rs` |
| SET-35 | The **Mission Control** section (5 toggles) and a **Shortcuts…** button (keys for Mission Control, App windows, Show Desktop). | Absent. Mission Control exists (ADR 0014), but none of its options are exposed. | P2 | S–M | `desktop_dock.rs`, `shell/bins/rmac-mission-control` |
| SET-36 | **Widgets** section (Show Widgets on Desktop, Dim widgets). | Absent, although desktop widgets exist (`crates/rmac-desktop-widgets`). | P2 | S | `desktop_dock.rs` |
| SET-37 | **Hot Corners…** is a **sheet** with four popups around a screen picture. Each can take a modifier key. | Four inline popups. | P2 | S | `desktop_dock.rs` |

The default web browser popup is already tracked as S16.

#### Keyboard

Measured on the Mac:
1. Key repeat rate, a **slider with 8 stops**.
2. Delay until repeat, **6 stops**.
3. Adjust keyboard brightness in low light.
4. Keyboard brightness slider.
5. Turn keyboard backlight off after inactivity.
6. **Press 🌐 key to** (Change Input Source / Show Emoji & Symbols / Start
   Dictation / Do Nothing).
7. **Keyboard navigation** switch.
8. **Keyboard Shortcuts…** button.
9. **Text Input**: Input Sources (Edit…) and **Text Replacements…**.
10. **Dictation** section.

Lulo has:
- Key repeat rate and Delay until repeat, **5 steps each**.
- Use Num Lock on startup.
- Keyboard Shortcuts…
- PC Keyboard: Mac shortcuts, ⌘ next to space, Caps Lock key, ⌥ special
  characters.
- Text Input: Input Sources, Edit…
- Refresh.

| id | Mac | Lulo | Sev | Size | Where |
|---|---|---|---|---|---|
| SET-40 | **Keyboard Shortcuts…** is a sheet whose sidebar lists Launchpad & Dock, Display, Mission Control, Keyboard, Input Sources, Screenshots, Presenter Overlay, Services, Spotlight, Accessibility, App Shortcuts, Modifier Keys and Function Keys. Every shortcut has a checkbox and can be edited **in place**. **Modifier Keys…** remaps Caps Lock, ⌃, ⌥, ⌘ and 🌐 for each keyboard. | Two categories (Spotlight, Lock Screen), read-only. Rebinding happens only through the portal's configuration UI. Screenshot, Mission Control and Dock keys exist but aren't listed. | **P1** | M | `crates/system-settings/src/controller/input/shortcuts.rs:11-26`, `crates/rmac-shortcuts` |
| SET-41 | **Keyboard navigation** (Tab moves focus to every control). | Absent. | P1 (accessibility) | M (needs focus rings in rmac-ui) | `controller/input/render.rs` |
| SET-42 | **Keyboard brightness** slider and backlight timeout. | Absent. The keys aren't bound either (S11). | P2 | S | with S11 |
| SET-43 | **Press 🌐 / fn key to** popup. | Absent. | P2 | S | `render.rs` |
| SET-44 | **Text Replacements…** (the "omw" → "On my way!" table, used in all text fields) and the **Spelling / Correct spelling automatically / Capitalise words automatically / Add full stop with double-space / Use smart quotes and dashes** toggles under Input Sources › Edit…. | Absent. The Input Sources Edit… button just jumps to Language & Region. | P2 | M | `render.rs:162-183` |
| SET-45 | **Dictation**. | Absent. | skip | — | — |
| SET-46 | Key repeat has 8 stops and delay has 6. | 5 and 5. | P2 | S | `controller/input.rs:64-80` |

#### Trackpad and Mouse

**(K)** The Mac Trackpad pane has three tabs.

- **Point & Click:**
  - Tracking speed.
  - **Click** (Light / Medium / Firm).
  - **Force Click and haptic feedback**.
  - **Look up & data detectors** (Force Click / Tap with three fingers).
  - **Secondary click** (Click or tap with two fingers / Bottom-right corner
    / Bottom-left corner / Off).
  - **Tap to click**.
  - **Silent clicking**.
- **Scroll & Zoom:**
  - Natural scrolling.
  - **Zoom in or out** (pinch).
  - **Smart zoom** (double-tap with two fingers).
  - **Rotate**.
- **More Gestures:**
  - Swipe between pages.
  - Swipe between full-screen applications.
  - Notification Centre (swipe left from the right edge).
  - Mission Control (swipe up).
  - App Exposé (swipe down).
  - Launchpad/Apps (pinch).
  - Show Desktop (spread).

The Mac Mouse pane has: Tracking speed, Natural scrolling, **Secondary
click** (Right side / Left side), **Double-click speed**, and **Scrolling
speed**.

**Lulo Trackpad** has two tabs.
- **Point & Click:** Tracking speed, Pointer acceleration, Tap to click,
  Ignore trackpad while typing, Drag lock, Primary click on right,
  Middle-click emulation.
- **Scroll & Zoom:** Natural scrolling.

**Lulo Mouse** has: Tracking speed, Natural scrolling, Pointer acceleration,
Primary button on right, Middle-click emulation.

| id | Mac | Lulo | Sev | Size | Where |
|---|---|---|---|---|---|
| SET-50 | A **More Gestures** tab, showing which three- and four-finger swipes do what, with on/off and finger-count popups. | Absent. niri owns the gestures, and nothing documents or configures them in Settings. | **P1** | M | `controller/input/render.rs:253-358`; niri `gestures` config via the managed include (`crates/rmac-input`) |
| SET-51 | **Secondary click** choices: two-finger click or tap, or a corner. | Absent. Only "Primary click on right" (left-handed) exists. libinput has `click-method` (clickfinger / button-areas). | **P1** | S | same; niri `click-method` |
| SET-52 | Scroll & Zoom has **Zoom in or out**, **Smart zoom** and **Rotate**. | Natural scrolling only. | P2 | M | same |
| SET-53 | The Mouse pane has **Double-click speed** and **Scrolling speed**. | Absent. niri has `scroll-factor`; the double-click interval is per toolkit, and rmac-ui's threshold could read it. | P2 | S | `render.rs:188-251` |
| SET-54 | Mac-only controls: Click pressure, Force Click, Silent clicking. Linux-only additions: Ignore while typing, Drag lock, Pointer acceleration, Middle-click emulation. | Keep the extras, but group them under an **Advanced…** button as the Mac does, so the first screen looks like the Mac's. | P2 | S | same |

---

## 3. TextEdit vs Text Editor

**Window.**
- **Mac:** a plain 31 pt title bar with the document icon, the title and a
  **▾ title menu**. The window is 656×422. Plain text uses a monospaced
  font.
- **Lulo:** the same shape, 656×422 in code (640×398 on the laptop). There
  is a 32 pt bar, a drawn proxy icon and the title "Untitled". The ▾ menu
  holds only encoding and line-ending choices. ✓ overall.

**Mac menus** (measured):
- **File:**
  - New ⌘N, Open… ⌘O, **Open Recent ›**
  - Close ⌘W, **Close All ⌥⌘W**
  - Save ⌘S, **Save As… ⌥⇧⌘S**, **Duplicate ⇧⌘S**
  - **Rename…**, **Move To…**, **Revert To ›**
  - Export as PDF…, **Share ›**
  - **Show Properties ⌥⌘P**
  - **Page Setup… ⇧⌘P**, Print… ⌘P
- **Edit:**
  - Undo, Redo
  - Cut, Copy, Paste, Paste and Match Style ⌥⇧⌘V, Delete, **Complete ⌥⎋**,
    Select All
  - **Insert ›** (Line Break, Paragraph Break, Page Break)
  - Attach Files… ⇧⌘A, **Link… ⌘K**
  - **Find ›** (Find… ⌘F, Find and Replace… ⌥⌘F, Find Next ⌘G, Find
    Previous ⇧⌘G, Use Selection for Find ⌘E, Jump to Selection ⌘J, **Select
    Line… ⌘L**)
  - **Spelling and Grammar ›** (⌘:, ⌘;, and three toggles)
  - **Substitutions ›**
  - **Transformations ›** (Make Uppercase / Lowercase / Capitalise)
  - **Speech ›**
  - AutoFill ›, Start Dictation…, Emoji & Symbols
- **Format:**
  - **Font ›**: Show Fonts ⌘T, Bold ⌘B, Italic ⌘I, Underline ⌘U, Outline,
    Highlight ›, Styles…, Bigger ⌘+, Smaller ⌘−, Kern ›, Ligatures ›,
    Baseline ›, Character Shape ›, Show Colours ⇧⌘C, Copy Style ⌥⌘C, Paste
    Style ⌥⌘V
  - **Text ›**: Align Left ⌘{, Centre ⌘|, Justify, Align Right ⌘},
    Writing Direction ›, Show Ruler ⌘R, Copy Ruler ⌃⌘C, Paste Ruler ⌃⌘V,
    Spacing…
  - **Make Rich Text / Make Plain Text ⇧⌘T**
  - Prevent Editing
  - **Wrap to Page ⇧⌘W**
  - Allow Hyphenation
  - Make Vertical Layout
  - List…, Table…
- **View:**
  - Show Tab Bar, Show All Tabs
  - **Use Dark Background for Windows**
  - **Actual Size ⌘0**, **Zoom In ⇧⌘.**, **Zoom Out ⇧⌘,**
  - Enter Full Screen
- **Window:** standard.
- **Help:** TextEdit Help.

**Lulo live menus:**
- **File:** New ⌘N, Open… ⌘O, Save ⌘S, Save As… ⇧⌘S, Print… ⌘P, Close
  Window ⌘W. "Export as PDF…" is in the table but was absent from the live
  menu on the laptop.
- **Edit:** Find… ⌘F, Find Next ⌘G, Find Previous ⇧⌘G, Replace… ⌥⌘F.
- **Format:** Bigger ⌘+, Smaller ⌘−, Monospaced.

| id | Mac | Lulo | Sev | Size | Where |
|---|---|---|---|---|---|
| TE-01 | **Save As… is ⌥⇧⌘S**, and **⇧⌘S is Duplicate**. | ⇧⌘S is bound to Save As. A Mac user pressing ⇧⌘S expects a duplicate window. | P1 | S | `TEXT_EDITOR_MENUS`, `crates/text-editor/src/view/lifecycle.rs:48-93` |
| TE-02 | **Open Recent ›** with Clear Menu. | Absent, although opened files are recorded (`document_state.rs:270-296`). | P1 | S, with MENU-04 | `rmac-app-menu`, `crates/text-editor` |
| TE-03 | **Rich text** is the default for new documents (Format › Make Plain Text ⇧⌘T switches). Bold ⌘B, Italic ⌘I, Underline ⌘U, fonts, colours, alignment ⌘{ ⌘| ⌘}, lists and a ruler ⌘R. | Plain text only. On Linux, opening an **.rtf** file fails with "could not be decoded safely" because RTF parsing is AppKit-only (`view/render/rtf.rs:30-33`). | **P0** (RTF can't be opened), P1 (rich editing) | S (open RTF as plain text or strip it with a pure-Rust RTF parser) / L (rich editing) | `crates/text-editor/src/view/render/rtf.rs`, `document_io.rs:43-45` |
| TE-04 | **Export as PDF…** is in the File menu. | Implemented (`printing.rs:96-166`), but **missing from the live menu** on the installed build. | P1 | S (check that the installed package includes `ExportPdf`) | `crates/text-editor/src/main.rs:12-38`, `rmac-app-menu` |
| TE-05 | Autosave with **Revert To › Last Saved / Browse All Versions…**. **Rename…**, **Move To…** and **Duplicate** in the File menu and the ▾ title menu. | Only a recovery draft. The title ▾ menu shows encodings only. | P2 | M | `view/render/chrome.rs:63-106` |
| TE-06 | Find › **Use Selection for Find ⌘E**, **Jump to Selection ⌘J**, **Select Line… ⌘L**. The find bar has **Ignore Case**, Contains / Starts With / Full Word, and **Wrap Around**. | Case-sensitive substring only. None of the extra commands. | P1 (case sensitivity: Mac find ignores case by default), P2 (the rest) | S | `crates/text-editor/src/view/editing.rs:18-23` |
| TE-07 | **Spelling and Grammar**: red underlines while typing, and suggestions in the right-click menu. | Absent (tracked in the inventory as "Spell check A"; still absent). | P1 | M | — |
| TE-08 | **Wrap to Page ⇧⌘W** toggles between window width and page width. | Always wraps to the window. | P2 | S | `crates/rmac-editor/src/lib.rs:44-56` |
| TE-09 | **View zoom** (⌘0, ⇧⌘., ⇧⌘,) is separate from **font size** (⌘+ / ⌘−, which changes the font). | ⌘+ / ⌘− change the font size and are not persisted. There is no ⌘0. | P2 | S | `editing.rs:164-172` |
| TE-10 | **Page Setup… ⇧⌘P**, **Show Properties ⌥⌘P** (author, title, keywords). | Absent. | P2 | S | — |
| TE-11 | **Transformations** (Make Uppercase / Lowercase / Capitalise), **Substitutions** (smart quotes and dashes), **Complete ⌥⎋**. | Absent. Transformations are cheap. | P2 | S | `crates/text-editor/src/view/editing.rs` |
| TE-12 | **Tabs**: Show Tab Bar, Merge All Windows, with ⌘T under the "Prefer tabs" setting. | One document per window. | P2 | M | — |
| TE-13 | Launching TextEdit with no document shows the **Open panel**, with a New Document button in iCloud-enabled setups. | Opens a new Untitled window. This matches TextEdit with iCloud off, so it is **acceptable**. | — | — | — |

---

## 4. Notes

The Mac Notes window content was not captured, for privacy. These facts come
from the menu dump and the toolbar's accessibility tree.

**Mac toolbar** (window 1470×832):
- Folders toggle
- **View Options** menu (List / Gallery, sort, group)
- New Note
- **Format "Aa"**
- **Checklist**
- **Table**
- **Media** (attachments)
- **Share**
- **More ⋯** (Lock, Pin and similar)
- search field

The window title is "<folder> – N notes".

**Lulo toolbar:** folder name and count, ⋯, Compose, a Checklist and Add
Photo capsule, a Move Note and ⋯ capsule, and search. The window title is
always "Notes".

**Mac menus** (measured):
- **Notes:** Settings… ⌘,, Accounts…, Close All Locked Notes.
- **File:**
  - New Note ⌘N, New Folder ⇧⌘N, New Smart Folder, More ›
  - Share
  - Close ⌘W, Close All ⌥⌘W
  - Import to Notes…, Import Markdown…
  - **Export as › PDF / Markdown**, Open in Pages
  - **Pin Note**, **Lock Note**, **Duplicate Note ⌘D**
  - Print… ⌘P
- **Edit:**
  - Undo, Redo
  - Cut, Copy, Paste, Paste and Match Style ⌥⇧⌘V, Paste and Retain Style,
    Delete ⌫, Rename, Select All
  - **Attach File… ⇧⌘A**, **Add Link… ⌘K**, Record Audio…, Rename
    Attachment…
  - **Find ›** (Note List Search… ⌥⌘F, Find… ⌘F, Find and Replace… ⇧⌘F,
    Find Next ⌘G, Find Previous ⇧⌘G, Use Selection for Find ⌘E, Jump to
    Selection ⌘J)
  - Spelling and Grammar ›, Substitutions ›, Transformations ›, Speech ›
- **Format:**
  - **Title ⇧⌘T**, **Heading ⇧⌘H**, **Subheading ⇧⌘J**, **Body ⇧⌘B**,
    **Monostyled ⇧⌘M**
  - **Bulleted List ⇧⌘7**, **Dashed List ⇧⌘8**, **Numbered List ⇧⌘9**,
    **Block Quote ⌘'**
  - Checklist ⇧⌘L, **Mark as Ticked ⇧⌘U**, More › (Tick All, Untick All,
    Move Ticked to Bottom, Delete Ticked)
  - **Move Item › Up ⌃⌘↑ / Down ⌃⌘↓**
  - **Table ⌥⌘T**, Convert to Text
  - Show Note As Light Background
  - **Font ›** (Bold ⌘B, Italic ⌘I, Underline ⌘U, Strikethrough, Highlight
    ⇧⌘E, Bigger, Smaller, Baseline ›, Show Colours ⇧⌘C, Copy Style, Paste
    Style, Remove Style)
  - **Text ›** (alignment)
  - **Indentation › Increase ⌘] / Decrease ⌘[**
  - Maths Results ›
- **View:**
  - **as List ⌘1**, **as Gallery ⌘2**
  - Recent Notes
  - **Sort By ›** (Default, Date Edited, Date Created, Title; Newest First,
    Oldest First)
  - **Group By Date ›**
  - **Hide Folders ⌃⌘S**, Show Note Count
  - Attachment View ›, Show Attachments Browser ⌘3
  - Show Highlights ⌃⌘I, Show Note Activity ⌃⌘K
  - **Zoom In ⇧⌘., Zoom Out ⇧⌘,, Actual Size ⇧⌘0**
  - Hide Toolbar, Customise Toolbar…, Enter Full Screen
- **Help:** Notes Help, Using Tags, Using Smart Folders.

**Lulo live menus:**
- **File:** New Note ⌘N, New Folder ⇧⌘N, Export Notes… ⇧⌘E, Print… ⌘P.
  "Export as PDF…" was absent from the live menu.
- **Edit:** Find… ⌘F.
- **Format:** Checklist ⇧⌘L.
- **View:** Sort by Date Edited, Sort by Date Created, Sort by Title.

| id | Mac | Lulo | Sev | Size | Where |
|---|---|---|---|---|---|
| NOTES-01 | **Rich, styled notes.** The first line is the Title style. Paragraph styles have keys: Title ⇧⌘T, Heading ⇧⌘H, Subheading ⇧⌘J, Body ⇧⌘B, Monostyled ⇧⌘M. The "Aa" toolbar menu. | The editor is a **Markdown source** editor with separate Title, Body and Tags fields. Formatting is typed as Markdown, and a read-only preview toggle shows it rendered. No style commands. | **P1** (this is the biggest single difference a Notes user feels) | L (WYSIWYG) / M (make the shortcuts insert Markdown markers and show a live-styled view) | `crates/notes/src/editor_presentation.rs:179-263`, `rmac-notes-storage/src/markdown_preview.rs` |
| NOTES-02 | Checklists can be **clicked** to tick them, **⇧⌘U** marks as ticked, and More › offers Tick All and similar. | ⇧⌘L inserts the text "- [ ] ". The preview checkboxes can't be clicked. | P1 | M | `edit_recovery_controller.rs:24-48`, `markdown_preview.rs` |
| NOTES-03 | Lists ⇧⌘7 / 8 / 9, Block Quote ⌘', **Table ⌥⌘T**, Bold / Italic / Underline ⌘B / ⌘I / ⌘U, Strikethrough, Highlight ⇧⌘E, **Add Link ⌘K**, Indent ⌘] / ⌘[, Move Item ⌃⌘↑ / ↓. | None are bound. | P1 (⌘B / ⌘I / ⌘U, lists), P2 (the rest) | M (as Markdown-insertion commands) | `crates/notes/src/startup_controller.rs:14-33`, `NOTES_MENUS` |
| NOTES-04 | Toolbar: **Format "Aa"**, **Table**, **Media** (Attach File, Photos, Scan, Audio), **Share**, and **Lock** in ⋯. The window title is "<folder> – N notes". | No Aa, Table, Share or Lock. Attachments are photos only. The title is always "Notes". | P2 | S (title) / M | `crates/notes/src/toolbar.rs:89-213`, `main.rs:331-338` |
| NOTES-05 | **Gallery view ⌘2**, List ⌘1, Show Attachments Browser ⌘3. | List only. | P2 | M | `note_navigation.rs` |
| NOTES-06 | **Pin Note** (File menu and swipe), **Duplicate Note ⌘D**, **Lock Note**. | Pin exists in ⋯ only, with no menu item or shortcut. No Duplicate, no Lock. | P1 (⌘D), P2 (Lock) | S / M | `NOTES_MENUS`, `library_actions.rs` |
| NOTES-07 | **Delete ⌫** removes the selected note from the list. | ⌘⌫ (a Lulo invention). Plain ⌫ in the list does nothing. The note list can't be navigated with ↑↓. | P1 | S | `startup_controller.rs:14-33` |
| NOTES-08 | **Sort By ›** with ✓, Newest/Oldest First, **Group By Date ›**, **Hide Folders ⌃⌘S**. | Three flat sort rows and no folder toggle. | P2 | S, with MENU-04 | `NOTES_MENUS` |
| NOTES-09 | **Find › Note List Search ⌥⌘F** versus **in-note Find ⌘F**, and **Find and Replace ⇧⌘F**. | ⌘F focuses the list search only. There is no in-note find or replace. | P1 | M | `crates/notes` |
| NOTES-10 | **Export as › PDF** is in the File menu. | Implemented, but absent from the live menu (as TE-04). The PDF save panel starts in the process's working directory, not Documents. | P1 | S | `print_controller.rs:102-155`, `:114` |
| NOTES-11 | **Tags**: typed inline as #tag; the sidebar shows a Tags browser; **Smart Folders**. | A comma-separated tags field. No tag browser and no smart folders. | P2 | M | `note_navigation.rs` |
| NOTES-12 | **View zoom** ⇧⌘. / ⇧⌘, / ⇧⌘0. | Absent. | P2 | S | — |
| NOTES-13 | **Settings… ⌘,**: default account, sort notes by, new notes start with (Title / Heading / Body), group notes by date, default text size. | Absent. | P2 | S | — |

The Notes print path (S7) and ⌘W (S8) are now **present**; see §12.

---

## 5. Calculator

**Measured.**
- **Mac:** window 230×408, 52 pt toolbar with **Show Sidebar** (history) and
  **Mode** (a calculator glyph ▾) buttons. On first launch a tip popover
  appears.
- **Lulo:** window 230×406, the same two toolbar buttons, and a keypad that
  matches.

**Mac menus:**
- **Edit:** the standard items.
- **View:**
  - **Basic ⌘1**, **Scientific ⌘2**, **Programmer ⌘3**
  - **Convert ⌥⌘C**, **RPN Mode ⌘R**
  - **Maths Notes… ⌥⌘M**
  - **Hide Thousands Separator**, **Decimal Places › 0–15**
  - **Show History ⌃⌘S**
  - Enter Full Screen
- **Window:** Close ⌘W, Close All, the minimise, zoom and fill items, **Always
  on Top**, and similar.
- **Help:** Calculator Help ⌘?.

**Lulo menus:**
- **Edit:** Copy ⌘C, Paste ⌘V.
- **View:** Basic ⌘1.

| id | Mac | Lulo | Sev | Size | Where |
|---|---|---|---|---|---|
| CALC-01 | The two toolbar buttons work: **sidebar** shows the history tape; **Mode** lists Basic / Scientific / Programmer / Convert. | **Both buttons do nothing** (no `on_click`). | **P1** (dead controls) | S (hide them) / M (implement) | `crates/calculator/src/view.rs:110-141` |
| CALC-02 | **Scientific ⌘2** (a wide keypad with 2nd, x², xʸ, eˣ, sin/cos/tan, π, Rand, parentheses, memory keys) and **Programmer ⌘3**. | Basic only. ShowBasic is a no-op. | P1 (Scientific), P2 (Programmer) | M | `crates/calculator/src/engine.rs`, `keypad.rs` |
| CALC-03 | **History ⌃⌘S**: a sidebar of past calculations; clicking one reuses it. | Absent. | P2 | S | — |
| CALC-04 | **⌘W closes the window** (Calculator quits when its last window closes). | ⌘W calls `cx.quit()`. The end result is the same; there is no gap. | — | — | — |
| CALC-05 | The thousands separator follows the Region setting and can be hidden; Decimal Places can be set. | "," is hard-coded; no options. | P2 | S | `engine.rs:482-491` |
| CALC-06 | **Convert ⌥⌘C** (units and currency), **RPN ⌘R**. | Absent. Spotlight has conversions (`rmac-launcher-providers`) that could be reused. | P2 | M | — |
| CALC-07 | The light appearance is measured and supported. | The light palette is unmeasured (`keypad.rs:224`). | P2 | S | — |
| CALC-08 | **Always on Top** in the Window menu. | Absent (see MENU-01). | P2 | S | — |

---

## 6. Preview

**Mac window.**
- A **two-line title**: the name over "Page X of N" or "1 page".
- Toolbar capsules: sidebar ▾ · zoom − / + · **Share** · **Markup** ·
  rotate · **Highlight** · search. This comes from the previous inventory;
  there was no document open this time.
- Launching with no file shows the Open panel.

**Lulo window.** Launching with no file shows the Open panel too (the rmac
file chooser, 856×424). The toolbar has sidebar ▾, a zoom capsule, rotate,
info, and search (PDF only). There is a two-line title.

**Mac menus** (measured, with no document open):
- **File:**
  - **New from Clipboard ⌘N**, Open… ⌘O, **Open Recent ›**
  - Close Window ⌘W, Close All ⌥⌘W, **Close Selected ⇧⌘W**
  - **Save ⌘S**, **Save As… ⌥⇧⌘S**, **Duplicate ⇧⌘S**
  - Rename…, Move To…, Revert To ›
  - Enter Password…, Edit Permissions…
  - Import from Camera / Scanner…, **Take Screenshot ›**
  - **Export As…**, **Export as PDF…**, **Share ›**
  - **Print… ⌘P**
- **Edit:**
  - Undo, Redo
  - Cut, Copy, Paste, Delete, Select All, **Invert Selection ⇧⌘I**
  - **Insert › Page from File… / Blank Page**, **Move to Bin ⌘⌫**
  - Find › (Find… ⌘F, Find Next ⌘G, Find Previous ⇧⌘G, ⌘E, ⌘J)
  - Spelling ›, Speech ›, and the others
- **View:**
  - Show Tab Bar, **Show All Tabs ⇧⌘\\**
  - Hide Sidebar ⌥⌘1, Thumbnails ⌥⌘2, **Table of Contents ⌥⌘3**,
    **Highlights and Notes ⌥⌘4**, **Bookmarks ⌥⌘5**, **Contact Sheet ⌥⌘6**
  - **Continuous Scroll ⌘1**, **Single Page ⌘2**, **Two Pages ⌘3**
  - Soft Proof ›, Show Image Background ⌥⌘B, Use Dark Appearance for PDF
  - Actual Size ⌘0, **Actual Size on All ⌥⌘0**, Zoom to Fit ⌘9, **Zoom All
    to Fit ⌥⌘9**, Zoom In ⌘+, **Zoom All In ⌥⌘+**, Zoom Out ⌘−, **Zoom All
    Out ⌥⌘−**, **Zoom to Selection ⌘\***
  - **Show Markup Toolbar ⇧⌘A**, Hide Toolbar ⌥⌘T, Customise Toolbar…
  - **Slideshow ⇧⌘F**, Enter Full Screen
- **Go:**
  - **Up ⇞**, **Previous Document ⌥⇞**, **Down ⇟**, **Next Document ⌥⇟**
  - Previous Item ⌥↑, Next Item ⌥↓
  - **Go to Page… ⌥⌘G**
  - **Back ⌘[**, **Forward ⌘]**
- **Tools:**
  - Show Inspector ⌘I, **Show Magnifier `**
  - **Adjust Colour… ⌥⌘C**, **Adjust Size…**
  - **Automatic / Rectangular / Text Selection**, **Redact**
  - **Annotate ›** (Highlight ⌃⌘H, Underline ⌃⌘U, Strike Through ⌃⌘S,
    Rectangle ⌃⌘R, Oval ⌃⌘O, Line ⌃⌘I, Arrow ⌃⌘A, Polygon, Star, Text ⌃⌘T,
    Speech Bubble, Mask, Loupe ⌃⌘L, Note ⌃⌘N, Signature ›)
  - **Add Bookmark ⌘D**
  - Rotate Left ⌘L, Rotate Right ⌘R, **Flip Horizontal**, **Flip
    Vertical**, **Crop ⌘K**, **Remove Background ⇧⌘K**
  - Assign Profile…, Show Location Info
- **Window:** standard.
- **Help:** Preview Help.

**Lulo live menus:**
- **File:** Open… ⌘O, Close Window ⌘W.
- **Edit:** Copy ⌘C, Find ⌘F, Find Next ⌘G, Find Previous ⇧⌘G.
- **View:** Hide Sidebar ⌥⌘1, Thumbnails ⌥⌘2, Actual Size ⌘0, Zoom to Fit
  ⌘9, Zoom In ⌘+, Zoom Out ⌘−.
- **Go:** Previous Item ⌥↑, Next Item ⌥↓.
- **Tools:** Show Inspector ⌘I, Rotate Left ⌘L, Rotate Right ⌘R.

| id | Mac | Lulo | Sev | Size | Where |
|---|---|---|---|---|---|
| PREV-01 | **Text selection and ⌘C** in PDFs (Text Selection tool, drag to select). | Absent. ⌘C does nothing for PDFs. | **P0** (copying text out of a PDF is a core workflow) | M (pdftotext `-bbox` already provides word boxes; hit-test and draw the selection) | `crates/preview/src/view.rs:536-563` |
| PREV-02 | **Print ⌘P**. | Absent (tracked as S7 for Preview; still missing). | P1 | S | follow `crates/text-editor/src/view/printing.rs` |
| PREV-03 | **Markup**: Show Markup Toolbar ⇧⌘A, highlight and annotate, **Signature**, Text. Filling in and signing PDF forms is a very common reason to open Preview. | Absent. | **P1** | L | new; needs a PDF writer (e.g. `lopdf`) |
| PREV-04 | **Save / Export As…** (PNG, JPEG, HEIC, PDF, TIFF; quality slider), **Export as PDF…**, **Duplicate**. **Rotate and Crop ⌘K persist** when saved. | View-only. Rotation is never saved and nothing is written. | P1 | M | `crates/preview` |
| PREV-05 | **Go to Page… ⌥⌘G**, Page Up/Down with ⇞ / ⇟, **Back / Forward ⌘[ / ⌘]** for link history, **clickable links** inside PDFs. | Paging keys scroll. There is no Go to Page, no link following, and no Back/Forward. | P1 (Go to Page, links) | S / M | `view.rs:372-416`, `poppler.rs` |
| PREV-06 | Sidebar **Table of Contents ⌥⌘3**, **Highlights and Notes**, **Bookmarks**, **Contact Sheet ⌥⌘6**. | Thumbnails only. | P2 (TOC is P1 for long PDFs) | M (`pdfinfo` / poppler outline) | `view.rs:1192-1282` |
| PREV-07 | **Single Page ⌘2**, **Two Pages ⌘3**, Continuous ⌘1. | Continuous only. | P2 | M | `view.rs:1388-1413` |
| PREV-08 | **Open Recent ›**; opening documents adds them to the Recents list and to Files' Recents. | Absent: nothing is recorded to recents. | P1 | S | `crates/preview/src/main.rs`; `crates/rmac-recent-documents` |
| PREV-09 | Formats: **HEIC**, **SVG**, PSD, RAW, EPS, and more. | PDF, PNG, JPEG, GIF, WebP, BMP, TIFF. HEIC (iPhone photos) is missing. | P1 (HEIC), P2 | M (`libheif`) | `crates/preview/src/document.rs:54-70` |
| PREV-10 | **Adjust Size…**, **Adjust Colour… ⌥⌘C**, **Flip**, **Crop**, **Remove Background**, **Magnifier**. | Absent. | P2 | M | — |
| PREV-11 | **Slideshow ⇧⌘F**; **Zoom All** variants; **Zoom to Selection ⌘\***. | Absent. | P2 | S | — |
| PREV-12 | **Insert › Page from File…**, **Blank Page**, deleting pages, **dragging thumbnails to reorder**, dragging a thumbnail between documents (merging PDFs). | Absent. | P2 | L | — |
| PREV-13 | Bounds are remembered; a window can be dragged in. | Bounds are not restored and there is no drop target. | P2 | S | `main.rs:94-104` |

---

## 7. Terminal

**Mac window.**
- Title format: "**<user> — <process> — <cols>×<rows>**", for example
  "jake — -zsh — 80×24".
- The tab bar appears once there are two tabs.
- The default profile is Basic.

The owner's Terminal window was not foregrounded, so its size was not
re-measured.

**Lulo window.**
- 580×385 (80×24).
- Title: "jacob@Jake: ~ — 80×24", taken from the shell's OSC title.
- A folder proxy icon.

**Mac menus** (measured):
- **Terminal:** Settings… ⌘,, **Secure Keyboard Entry**.
- **Shell:**
  - **New Window › (with Profile ⌘N, Same Command ⌃⌘N, and each profile)**
  - **New Tab › (with Profile ⌘T, Same Command ⌃⌘T, and each profile)**
  - **New Command… ⇧⌘N**, **New Remote Connection… ⇧⌘K**, Open… ⌘O
  - Close Window ⌘W, Close All ⌥⌘W
  - Use Settings as Default, Export Settings…
  - **Export Text As… ⌘S**, Export Selected Text As… ⇧⌘S
  - **Show Inspector ⌘I**, **Edit Title ⇧⌘I**, Edit Background Colour ⌥⌘I
  - **Reset ⌥⌘R**, **Hard Reset ⌃⌥⌘R**
  - Print Selection… ⌥⌘P, Print… ⌘P
- **Edit:**
  - Undo, Redo
  - Cut, Copy, **Copy Special ›**
  - Paste, **Paste Escaped Text ⌃⌘V**, **Paste Selection ⇧⌘V**, Paste
    Escaped Selection ⌃⇧⌘V
  - Select All, **Select Between Marks ⇧⌘A**
  - **Marks ›** (Mark ⌘U, Mark as Bookmark ⌥⌘U, Unmark ⇧⌘U, Mark Line and
    Send Return ⌘↩, Send Return Without Marking ⇧⌘↩, Automatically Mark
    Prompt Lines), **Bookmarks ›**
  - **Navigate ›** (Jump to Previous / Next Mark ⌘↑ / ⌘↓, Select to Mark
    ⇧⌘↑ / ↓, Bookmarks ⌥⌘↑ / ↓)
  - **Clear to Previous Mark ⌘L**, Clear to Previous Bookmark ⌥⌘L, **Clear to
    Start ⌘K**
  - **Clear Scrollback ⌥⌘K**, **Clear Screen ⌃⌘L**, Fill Screen ⌃⌥⌘L
  - **Find ›** (Find… ⌘F, Find Next ⌘G, Find Previous ⇧⌘G, Hide Find Bar
    ⇧⌘F, Use Selection for Find ⌘E, Jump to Selection ⌘J)
  - Show Colours ⇧⌘C, **Use Option as Meta Key ⌥⌘O**
  - Start Dictation…, Emoji & Symbols
- **View:**
  - Show All Tabs ⇧⌘\\, Show Tab Bar ⇧⌘T, Hide Marks
  - Show / Hide Alternative Screen ⇧⌘⇟ / ⇧⌘⇞
  - **Allow Mouse Reporting ⌘R**
  - **Split Pane ⌘D**, **Close Split Pane ⇧⌘D**
  - **Default Font Size ⌘0**, Bigger ⌘+, Smaller ⌘−
  - **Scroll to Top ⌘↖**, **Scroll to Bottom ⌘↘**, **Page Up ⌘⇞**, **Page
    Down ⌘⇟**, **Line Up ⌥⌘⇞**, **Line Down ⌥⌘⇟**
  - Enter Full Screen
- **Window:** standard, plus Cycle Through Windows ⌘\`, **Open Window Group /
  Save Windows as Group…**, Show Previous / Next Tab ⌃⇧⇥ / ⌃⇥, Move Tab to New
  Window, Merge All Windows, Return to Default Size.
- **Help:** Terminal Help, **Open man Page for Selection ⌃⌘?**, Search man
  Page Index ⌃⌥⌘/.

**Lulo live menus:**
- **Shell:** New Tab ⌘T, Close Tab ⌘W, Next Tab ⇧⌘], Previous Tab ⇧⌘[.
- **Edit:** Copy ⌘C, Paste ⌘V, Select All ⌘A.
- **View:** Find… ⌘F, Find Next ⌘G, Find Previous ⇧⌘G, Clear ⌘K, Bigger ⌘+,
  Smaller ⌘−, Actual Size ⌘0.

| id | Mac | Lulo | Sev | Size | Where |
|---|---|---|---|---|---|
| TERM-01 | **⌘N New Window**. Several Terminal windows is the default way of working. | Absent: the app has exactly one window (`boot_app`). | **P0** | M (single-instance and multi-window, like Files) | `crates/terminal/src/main.rs`, `crates/rmac-ui/src/window.rs` |
| TERM-02 | **⌘K is "Clear to Start"**: it clears the screen and scrollback, as the Mac does, so it matches. There is also **Clear Scrollback ⌥⌘K**, **Clear Screen ⌃⌘L**, **Reset ⌥⌘R** and **Hard Reset ⌃⌥⌘R**. | ⌘K only. No ⌥⌘K, ⌃⌘L or Reset. A wedged terminal (binary output) can't be recovered without closing the tab. | P1 (Reset), P2 | S | `crates/terminal/src/controller/view_state.rs:43-56`, `emulator.rs` |
| TERM-03 | **Settings… ⌘,** opens a Settings window: Profiles (Text, Window, Tab, Shell, Keyboard, Advanced), General ("On startup, open", "New windows open with", "New tabs open with", **Default login shell**). | ⌘, and ⇧⌘P open a dropdown profile picker. There is no settings window, and font, size, cursor, columns and rows can't be customised. | P1 | M | `crates/terminal/src/profiles.rs`, `controller/lifecycle.rs` |
| TERM-04 | The window title is "**<user> — <process> — cols×rows**". | "<OSC title or job> — cols×rows". On the laptop this shows as "jacob@Jake: ~ — 80×24", the Ubuntu bashrc's title. | P2 | S | `controller/renderer/chrome.rs:46-63`, `session.rs:714-722` |
| TERM-05 | **Split Pane ⌘D / ⇧⌘D**. | Absent. | P2 | M | — |
| TERM-06 | Marks: **Automatically Mark Prompt Lines** works with any shell, because Terminal marks each line where Return was pressed. ⌘↑ / ⌘↓ jump between them. | Marks work only when the shell itself emits OSC 133. Stock Ubuntu bash doesn't, so **⌘↑ / ⌘↓ do nothing** out of the box. | P1 | S (mark lines at Return in the emulator, or ship a bash snippet) | `crates/terminal/src/shell_integration.rs:1` |
| TERM-07 | **Bell**: an audible bell or a visual flash, plus a bounce and badge on the Dock icon when the terminal is in the background. | Bell events are ignored. | P2 | S | `crates/terminal/src/session.rs:63-94` |
| TERM-08 | **⌘-click or ⌘-double-click a URL** anywhere in the text opens it. | Only explicit OSC 8 hyperlinks work. Plain URLs aren't detected. | P1 | S | `controller/pointer.rs:48-62` |
| TERM-09 | **Dragging a file** onto the window types its escaped path. | No drop handler. | P1 | S | `crates/terminal/src/controller` |
| TERM-10 | **Use Option as Meta Key** is off by default, so ⌥ types special characters (€, #). | Alt always sends an ESC prefix, so **⌥3 can't type "#"** on a UK layout. | P1 (UK and European keyboards) | S | `crates/terminal/src/keyboard.rs:148-165` |
| TERM-11 | **Export Text As… ⌘S**, **Print ⌘P**, **Show Inspector ⌘I**, **Edit Title ⇧⌘I**, **Paste Escaped Text ⌃⌘V**, **Open man Page for Selection**, **Scroll to Top / Bottom ⌘↖ / ⌘↘**, **Page Up / Down ⌘⇞ / ⌘⇟**. | Absent. The Scroll and Page keys and ⌘S are the ones Mac users use most. | P2 | S each | — |
| TERM-12 | The profile list: Basic, Clear Dark, Clear Light, Grass, Homebrew, Man Page, Novel, Ocean, Pro, Red Sands, Silver Aerogel, Solid Colors. | 9 profiles, including "Lulo OS Dark", which is not a Mac profile. Clear Dark, Clear Light, Silver Aerogel and Solid Colors are missing. | P2 | S | `crates/terminal/src/profiles.rs` |
| TERM-13 | Marks, ⇧⌘A, ⇧⌘P and ⌘, appear in the menus. | These are bound, but not in the exported menu. | P2 | S | `TERMINAL_MENUS` |

Terminal's close confirmation for a running command (B4) is now implemented
per tab ("Terminate / Cancel"); see §12.

---

## 8. Activity Monitor vs System Monitor

**Measured.**
- **Mac:**
  - Window 960×640.
  - Title "**Activity Monitor – All Processes**". The subtitle follows the
    View filter.
  - Toolbar: **Stop ⊗**, **Inspector ⓘ**, **Actions ⋯▾** (System
    diagnostics options), a 5-tab radio group (CPU / Memory / Energy / Disk
    / Network), and Search.
- **Lulo:** 960×640, the title "System Monitor" over a **hard-coded "All
  Processes"**, then the same toolbar and a bottom summary panel. ✓ for
  layout.

**Mac menus** (measured):
- **File:** Close ⌘W, Close All ⌥⌘W, Page Setup…, Print….
- **Edit:** the standard items and Find ›.
- **View:**
  - **Columns ›** (28 columns, each checked on or off)
  - **Dock Icon ›** (Show CPU Usage / CPU History / Network / Disk /
    Application Icon)
  - **Update Frequency ›** (1, 2 or 5 s)
  - **All Processes; All Processes, Hierarchically; My Processes; System
    Processes; Other Users' Processes; Active Processes; Inactive Processes;
    GPU Processes; Windowed Processes; Selected Processes; Applications in
    last 12 hours**
  - **Filter Processes ⌥⌘F**, **Inspect Process ⌘I**, **Sample Process
    ⌥⌘S**, Run Spindump ⌃⌥⌘S, Run System Diagnostics…
  - **Quit Process ⌥⌘Q**, **Send Signal to Process…**, Show Deltas for
    Process ⌥⌘J
  - **Clear CPU History ⌘K**
  - Enter Full Screen
- **Window:** standard, plus **Activity Monitor ⌘1**, **CPU Usage ⌘2**,
  **CPU History ⌘3**, **GPU History ⌘4**, and Keep CPU Windows on Top.
- **Help:** Activity Monitor Help ⌘?.

**Lulo live menus:**
- **View:** Find Process… ⌘F.
- **Process:** Quit Process… ("Delete"), Force Quit Process… ⇧⌘⌫.

| id | Mac | Lulo | Sev | Size | Where |
|---|---|---|---|---|---|
| MON-01 | The process list shows **processes**. Threads are a column; the summary says Threads is roughly 3–5× Processes. | **Bug:** sysinfo 0.33 lists every Linux **thread** as its own process. The table shows thread rows under their parent's name, the laptop's summary read "Threads 667 / Processes 667", and the energy and disk totals probably double-count. | **P0** (the process list is wrong) | S (filter with `Process::thread_kind()`, or turn off task enumeration) | `crates/activity-monitor/src/process_table.rs:259-304`, `metrics_panes.rs:319-323`, `sampling.rs:76-89` |
| MON-02 | **Each tab shows its own columns.** <br>- **CPU:** Process Name, % CPU, CPU Time, Threads, Idle Wake Ups, Kind, % GPU, GPU Time, PID, User. <br>- **Memory:** Process Name, Memory, Threads, Ports, PID, User. <br>- **Energy:** Energy Impact, 12 hr Power, App Nap, Preventing Sleep. <br>- **Disk:** Bytes Written, Bytes Read. <br>- **Network:** Sent Bytes, Rcvd Bytes, Sent Packets, Rcvd Packets. | The same six columns on every tab (PID, Process Name, % CPU, Memory, Energy, Disk I/O). Switching tab only changes the sort. **PID is the first column**, where the Mac puts Process Name first. There is no CPU Time or Threads column. | **P1** | M | `crates/activity-monitor/src/columns.rs:28-101`, `view.rs:133-149` |
| MON-03 | **View filter** (All / My / System / Other Users' / Active / Windowed / Hierarchically). The window subtitle follows it. **My Processes** is the second default, and the most common choice. | Absent. The subtitle is a fixed "All Processes". | P1 | S–M | `view/render/chrome.rs:286-293` |
| MON-04 | The **Energy** column is Energy Impact, based on CPU, wake-ups, GPU and I/O. | `cpu + disk_MiB × 0.5`, so it equals %CPU for most rows. It also shows a value on the first refresh, while %CPU shows "—". | P2 | S | `process_table.rs:295` |
| MON-05 | **Quit Process** is ⌥⌘Q, in the View menu. The Stop button opens "Are you sure you want to quit this process?" with Cancel, **Force Quit** and **Quit**. | Bound to ⌘⌫, and the menu hint says "Delete". The dialog matches. | P2 | S | `MONITOR_MENUS`, `crates/activity-monitor/src/main.rs:37-77` |
| MON-06 | **⌘Q quits the app.** | ⌘Q and ⌘W both close the window. The end result is the same. | — | — | — |
| MON-07 | **Inspect Process** is a separate window with **Memory, Statistics, Open Files and Ports** tabs, and Sample and Quit buttons. | A modal card with 10 facts and no tabs. **Open Files** is easy on Linux (`/proc/<pid>/fd`). | P2 | M | `view/render/overlays.rs:59-171` |
| MON-08 | The **Memory** tab's bottom panel shows **MEMORY PRESSURE** (a green, yellow and red graph), Physical Memory, Memory Used (App Memory, Wired, Compressed), Cached Files and Swap Used. | A "MEMORY USED" percentage graph. There is no pressure graph (Linux PSI at `/proc/pressure/memory` would supply it). | P2 | S | `metrics_panes.rs:343-382` |
| MON-09 | The **Disk** tab shows reads in / writes out (I/O counts) and data read and written (totals and per second). The **Network** tab shows packets in / out, packets per second, and data received and sent. | No I/O counts and no packets. | P2 | S | `metrics_panes.rs:397-451` |
| MON-10 | **Update Frequency** (1, 2 or 5 s), **Columns ›** in the menu, the **Dock Icon** modes (CPU history in the Dock icon), **Sample Process**, **Send Signal…**, **Clear CPU History**, and the Window menu's **CPU Usage / CPU History** floating windows. | Refresh is fixed at 2 s. The column chooser exists in the ⋯ button but not in the menu. The rest are absent. | P2 | S–M | — |
| MON-11 | The table shows every process. | Capped at 300 rows. | P2 | S | `process_table.rs:322` |

---

## 9. Clock

**Measured.**
- **Mac:**
  - Window 1024×768.
  - Toolbar: a centred 4-segment radio group (World Clock / Alarms /
    Stopwatch / Timers) and a **menu button** on the right (+, with a menu).
- **Lulo:** 1000×744 (1024×768 in code), the same 4-segment capsule, a +
  circle, a world map with a terminator, and a card for the local city. ✓
  layout.

**Mac menus:**
- **File:** **Start Recent Timer ›**, Close ⌘W, Close All ⌥⌘W.
- **Edit:** the standard items plus Spelling, Substitutions and
  Transformations.
- **View:** World Clock ⌘1, Alarms ⌘2, Stopwatch ⌘3, Timers ⌘4, **View
  Digital Stopwatch / View Analogue Stopwatch**.
- **Help:** Clock Help ⌘?.

**Lulo menus:**
- **File:** New ⌘N, Close Window ⌘W.
- **View:** the four tabs ⌘1–4, Start or Stop, Lap or Reset.

| id | Mac | Lulo | Sev | Size | Where |
|---|---|---|---|---|---|
| CLOCK-01 | The **Stopwatch** has digital and analogue views, and the lap list scrolls. | Digital only. **The lap list can't scroll** (`overflow_hidden`), so laps beyond the visible area are lost from view. | P1 (scroll), P2 (analogue) | S / M | `crates/clock/src/view.rs:1045-1169`, `:1158` |
| CLOCK-02 | **Timers** have presets and **Recent timers** (File › Start Recent Timer), a **label**, and a **sound** choice ("When Timer Ends"). | None of these. The model has an unused `label` field. | P2 | S | `view.rs:1173-1394`, `countdown.rs:94` |
| CLOCK-03 | **Alarms** have a sound picker. | Always the Alert cue. | P2 | S | `ring.rs:189` |
| CLOCK-04 | World Clock cities can be reordered (Edit) and the view can switch to a list. | Add and remove only. | P2 | S | `view.rs:525-652` |
| CLOCK-05 | Clock follows **Light** mode. | Dark only (a fixed 0x1E1E1E fill). | P2 | S | `crates/clock/src/metrics.rs:13` |
| CLOCK-06 | An idle Clock uses no CPU. | The ticker wakes every 250 ms on every tab, even when idle, which matters on a low-end PC. | P2 | S | `view.rs:35-36,134-157` |

---

## 10. Mac apps with no Lulo counterpart

These are the bundled Mac apps from `/System/Applications` and
`/System/Applications/Utilities`. Lulo has Files, Settings, Text Editor,
Notes, Calculator, Preview, Terminal, System Monitor, Clock, Weather, Player
(for QuickTime and Music), Archive Utility, Apps (for Launchpad), Spotlight,
Mission Control, a screenshot tool, and Force Quit.

| App | Recommendation | Why / how | Sev | Size |
|---|---|---|---|---|
| **App Store** | **Substitute** (already done). | The logo menu opens the Ubuntu App Center. It should also be in the Dock and Apps with a Lulo name and icon. | — | — |
| **Calendar** | **Build.** | Clicking the menu-bar clock on a Mac is followed by opening Calendar, and the desktop Calendar widget has no app behind it. Evolution Data Server (inventory #8). | P1 | L |
| **Reminders** | **Build after Calendar.** | EDS VTODO (inventory #9). | P2 | M |
| **Disk Utility** | **Build.** | The Mac answer to "format a USB stick" (Erase, Eject, First Aid). udisks2 (inventory #11). | P1 | M |
| **System Information** | **Build (small).** | Reached from About This Mac › More Info…. Lulo's About page covers the summary; a detail window listing hardware, USB, network and software (from `lshw`/`lsusb`-style data) is cheap. | P2 | S |
| **Console** | **Build (small) or substitute.** | A `journalctl` viewer with search. GNOME Logs could substitute, but it breaks the look. | P2 | M |
| **Font Book** | **Build (small).** | Install a downloaded `.ttf` by double-clicking it (inventory #12). | P2 | S |
| **Screenshot** | **Done** (`shell/bins/rmac-screenshot`). | Screen recording is still absent. | — | — |
| **Photos** | **Substitute** with Preview's multi-image mode and Files' gallery view for now. | Inventory #10. | P2 | L |
| **Music / TV / Podcasts** | **Substitute** with Player. | Player already plays local media and publishes MPRIS. | — | — |
| **QuickTime Player** | **Substitute** with Player. | Recording (⌃⌘N movie, ⌥⌘N audio, ⌃⌘N screen) is missing; screen recording could go into Screenshot. | P2 | M |
| **Stickies** | **Build (small) or skip.** | Cheap and loved by long-time Mac users. Floating yellow notes backed by `rmac-notes-store`. | P2 | S |
| **Dictionary** | **Build (small).** | Spotlight already has definitions when dictd is installed; a window app plus the ⌃⌘D lookup would reuse them. | P2 | S |
| **Grapher, Chess, Automator, Script Editor, Shortcuts** | **Skip** (Chess could use GNOME Chess). | Niche. | — | — |
| **Contacts** | **Skip** until there is an account-sync story (inventory #13). | | — | — |
| **Mail** | **Substitute** with Thunderbird as the default mail handler, chosen in Desktop & Dock (S16). | | P2 | S |
| **Messages, FaceTime, Phone, iPhone Mirroring, Find My, Home, Wallet, Passwords, Image Playground, Siri, Journal, News, Stocks, Books, Freeform, Games, Tips, Maps** | **Skip.** | Apple services. Tips: the Setup Assistant's Tips step covers it. Maps: a web shortcut to OpenStreetMap is enough. Passwords: a GNOME Keyring front end is possible later. | — | — |
| **Photo Booth** | **Skip** (Cheese is the substitute). | | — | — |
| **Image Capture** | **Skip** (Preview's Import from Scanner/Camera is P2). | | — | — |
| **Time Machine** | **Build or substitute.** | The Déjà Dup engine behind a Lulo pane (SET-10). | P1 | L |
| **Migration Assistant** | **Skip** for now. | | — | — |
| **Audio MIDI Setup, AirPort Utility, Bluetooth File Exchange, Boot Camp, ColorSync, Digital Color Meter, VoiceOver Utility, Screen Sharing, Magnifier, Print Center** | **Skip.** Two exceptions are worth doing: **Print Center** can open as a print queue from SET-07, and **Digital Color Meter** is a cheap S-size tool that designers use. | | P2 | S |

---

## 11. Top gaps, P0 and P1 first

| id | Gap | Sev | Size |
|---|---|---|---|
| FILES-10 | Column view: arrow keys clear the selection, and ←/→ do nothing. | P0 | S |
| FILES-29 | Files: ⌘W does nothing with one tab. ⌘N copies the other window's tabs. Windows overwrite each other's state. | P0 | S–M |
| MON-01 | System Monitor lists every thread as a process (Threads = Processes); totals double-count. | P0 | S |
| PREV-01 | Preview: PDF text can't be selected or copied. | P0 | M |
| TE-03 | Text Editor can't open `.rtf` on Linux ("could not be decoded"); there is no rich text. | P0 (open) / P1 (edit) | S / L |
| TERM-01 | Terminal has no ⌘N and only one window. | P0 | M |
| MENU-01 | No Window menu in any app except Files. | P1 | M |
| MENU-02 | No standard Edit menu (Undo, Cut, Copy, Paste, Select All) in Text Editor or Notes. | P1 | M |
| MENU-03 | Menu items are never disabled. | P1 | M |
| MENU-04 | No submenus or checkmarks in app menus. | P1 | M |
| MENU-05 | About is always disabled; there is no Settings… row. | P1 | S |
| MENU-08 | A second file opened in Text Editor, Notes or Preview starts a second process with no menu bar. | P1 | M |
| MENU-10 | ⌘F probably swallowed inside text fields. | P1 | S |
| TE-01 | ⇧⌘S is Save As; the Mac uses ⌥⇧⌘S, and ⇧⌘S is Duplicate. | P1 | S |
| TE-02 / PREV-08 | No Open Recent; Preview doesn't record recent documents. | P1 | S |
| TE-04 / NOTES-10 | Export as PDF… is implemented but missing from the live menus. | P1 | S |
| TE-06 | Find is case-sensitive, with no Ignore Case option. | P1 | S |
| TE-07 | No spell check. | P1 | M |
| NOTES-01…03 | Notes is a Markdown source editor: no paragraph styles, no ⌘B/⌘I/⌘U, no clickable checklists. | P1 | M–L |
| NOTES-06 / 07 / 09 | Notes: no ⌘D, ⌫ doesn't delete a note, no in-note Find. | P1 | S–M |
| CALC-01 / 02 | Calculator's toolbar buttons are dead; no Scientific mode. | P1 | S / M |
| PREV-02 / 03 / 04 / 05 / 09 | Preview: no Print, Markup or signature, Save/Export, Go to Page or links, or HEIC. | P1 | S–L |
| TERM-02 / 03 / 06 / 08 / 09 / 10 | Terminal: no Reset, no Settings window, marks need OSC 133, plain URLs aren't clickable, no file drop, ⌥ can't type characters. | P1 | S–M |
| MON-02 / 03 | System Monitor: the same columns on every tab; no My Processes filter. | P1 | M |
| CLOCK-01 | The stopwatch lap list can't scroll. | P1 | S |
| FILES-04 / 05 / 20 | Files: no editable favourites and no tags on Linux. | P1 | M–L |
| FILES-08 / 13 / 15 / 16 / 18 | Files: no inline folder expansion, no ⌘J View Options, Get Info is a modal with no folder size, no Open with section and no editable permissions. | P1 | S–M |
| FILES-27 / 31 / 32 / 34 / 35 / 37 | Files: Quick Look can't browse the folder; missing File, Edit and Go items (Open, Find ⌘F, Redo, ⌥⌘C, ⌥⌘V, ⇧⌘D, ⇧⌘O, ⇧⌘F); no Settings; no search scope. | P1 | S–M |
| SET-01 | Settings: no ⌘F. | P1 | S |
| SET-05 / 06 / 07 / 08 | Settings: no Login Password, Users & Groups or Printers & Scanners panes; Lock Screen has no require-password delay. | P1 | M–L |
| SET-12 / 14 | Wi-Fi: no Details… on the current network; no live scan. | P1 | S |
| SET-16 | Displays: no Brightness slider. | P1 | S |
| SET-30 / 32 | Desktop & Dock: no Dock Size slider; indicator and recents toggles missing. | P1 | S |
| SET-40 / 41 | Keyboard Shortcuts sheet has 2 categories and is read-only; no Keyboard navigation. | P1 | M |
| SET-50 / 51 | Trackpad: no More Gestures tab; no secondary-click choice (two-finger vs corner). | P1 | M / S |
| APP: Calendar, Disk Utility | Missing apps (already in the inventory; still missing). | P1 | L / M |

---

## 12. Status corrections to the existing lists

| Item | Listed as | Actually |
|---|---|---|
| beta-gap-list **S5** (Files has no Empty Trash) | Missing | **Fixed.** The Files app menu has Empty Trash… ⇧⌘⌫, and it is bound (`FILES_MENUS`, `startup/shortcuts.rs`). |
| beta-gap-list **S8** (Notes has no ⌘W) | Missing | **Fixed.** ⌘W is bound and goes through the close review (`crates/notes/src/startup_controller.rs`). |
| beta-gap-list **S9** (Files ⌘N and ⇧⌘G) | Missing | **Present, but ⌘N is buggy** (see FILES-29). ⇧⌘G works. |
| beta-gap-list **S7** (printing) | Notes and Preview have no print path | **Notes now prints** (⌘P, `print_controller.rs`). **Preview still has none** (PREV-02). |
| beta-gap-list **B4** (Terminal quits without asking) | Missing | **Partly fixed.** A running foreground job prompts "Terminate / Cancel" when its tab or window is closed (`tab_lifecycle.rs:88-168`). This was not re-checked for ⌘Q, the Dock or log-out on the laptop. |
| beta-gap-list **S15** (no app binds ⌘,) | Missing | Terminal binds ⌘, to its profile picker. No app shows **Settings…** in the app menu (MENU-05). |
| gap inventory **Quick Look** "p: images and text only" | p | Now also PDF, video posters and audio waveforms (with ffmpeg), ⌥Space full screen, and Open with. The remaining gaps are FILES-27 and FILES-28. |
| gap inventory **§2 Preview, Calculator, Clock, Archive Utility, Player** "no crate exists" | Absent | All five now exist. Their gaps are listed above. |
