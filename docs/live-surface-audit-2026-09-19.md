# Live surface audit — every app and overlay, 2026-09-19

Captured on the reference PC by launching each binary and each overlay and screenshotting the result.
Images: `target/evidence/live-surfaces/*.png`. This is the companion to `docs/live-audit-2026-09-19.md`
(session health) and covers what the user actually sees.

## Ten cross-cutting defects — fix these once and every surface improves

| # | Defect | Where it shows | Why it kills the Mac feeling |
|---|---|---|---|
| **X1** | **Every window has square corners and no shadow** | Files, Settings, Terminal, Notes, Monitor, Text Editor | macOS windows are radius 16 with a soft shadow. Square, shadowless windows look pasted onto the wallpaper. Biggest single tell after icons. |
| **X2** | **Overlays are ordinary windows** | Search, Control Centre, Notification Centre, Apps | Each one appears **as a tile in the Dock**, takes over the menu-bar app name ("Launcher", "QuickSettings", "NotificationCenter"), and Notification Centre even has a title bar with a close button. They must be layer-shell surfaces. |
| **X3** | **Wrong accent colour** | Settings toggle, Control Centre icons, Apps chips, Notes selection | Purple/magenta everywhere. The measured macOS accent is **`#1372F9`**. Nothing in rmac should be purple by default. |
| **X4** | **No translucency or blur anywhere** | every overlay, the Dock, menus | All panels are flat opaque dark grey. niri 26.04 supports blur; the surfaces never ask for it. |
| **X5** | **Mixed window controls** | Terminal, System Monitor | Traffic lights on the left **and** Windows-style `− □ ✕` on the right. Pick one: traffic lights only. |
| **X6** | **Raw errors shown to users, including the word "niri"** | Settings ("NIRI_SOCKET is not set, are you running this within niri?"), Files ("Trash recovery data could not be verified") | Users must never read a socket name, a stack cause, or the compositor's name. |
| **X7** | **Type scale is too large** | sidebars, list rows, labels everywhere | Sidebar labels look 14–15 px; macOS uses 13 px body with 28 px rows. Everything reads oversized and "Linux-y". |
| **X8** | **Third-party icons are raw system icons** | Apps grid | Firefox, Disks, IBus and friends appear as GNOME/Yaru artwork with no squircle plate, so the grid looks like GNOME. Apply the plate rule from `FEEL_SPEC.md` §D.6. |
| **X9** | **Naming is inconsistent** | everywhere | The Files app's menu says **"Finder"**; the Apps grid lists **both** "Files" and "Finder" plus an "Apps" app; overlays use internal names in the menu bar. |
| **X10** | **The shell burns CPU while idle** | top-bar 10 %, dock 6.6 %, wallpaper 4.3 %, niri 15.2 % | Measured in rmac's own System Monitor. ~21 % of a core for a desktop doing nothing. It will never feel calm. |

## Per-surface findings

### Files (`app-files.png`)
- Error banner on launch: "Trash recovery data could not be verified; Trash actions are disabled".
- Shows **"No Matching Items / Try a different search"** in a folder that was never searched.
- Toolbar order is wrong: traffic lights → sidebar toggle → back/forward → `⋯` → a wide centred search field.
  macOS: back/forward · **title** · **view switcher (icon/list/column/gallery)** · group-by · share · tags · action · search at the right.
- **No view switcher at all** — the four views cannot be reached from the toolbar.
- Sidebar sections read "Recents / Shared" above "Favorites"; macOS puts Favourites first and Recents inside it. No Tags section.
- Selected sidebar row is grey, not accent. Rows ≈ 36 px instead of 28.
- Status bar uses "0 items • 92.86 GB available"; macOS uses a comma, not a bullet.
- Menu bar shows only **Finder · File · Edit · View** — missing Go, Window, Help, and the app is called Files.

### System Settings (`app-settings.png`)
- **There is no sidebar.** macOS Settings is sidebar + detail; this is a single pane with a giant centred icon, title and description — an elementary/GNOME shape.
- Red error block with raw text and a "Caused by:" chain.
- Toggle is purple and oversized; rows are tall cards instead of 36 px grouped rows.
- Two yellow explanatory callouts with long sentences — macOS puts that text under the row in 11 px secondary, if at all.
- Window: square corners, no shadow, no traffic-light/toolbar alignment.

### Terminal (`app-terminal.png`)
- **Both** traffic lights and Windows-style `− □ ✕`.
- Profile name "rmac Dark ▼" sits in the title bar as a dropdown; macOS keeps profile switching in Settings, and the title shows the working directory and process.
- Square corners; content starts flush against the chrome with no padding.

### Notes (`app-notes.png`)
- Selected note row is **dark olive/yellow** — replace with the accent (focused) or `selection.unfocused`.
- Section header "ON THIS COMPUTER" in all caps; macOS uses sentence case ("On My Computer").
- Date reads `8/9/26`; on this locale it should be `08/09/2026` or `9 August 2026`.
- Editor pane has **Edit / Preview** buttons (a Markdown editor idiom, not Notes) and a "Tags" field mid-pane.
- Toolbar glyphs are thin outline icons, unevenly spaced, not grouped.

### System Monitor (`app-monitor.png`)
- Windows-style window buttons again.
- Big boxy stat cards with coloured numerals and a single giant blue bar — Activity Monitor uses a compact table plus a small graph strip at the bottom.
- Tabs are a centred pill row; macOS puts them as a segmented control in the toolbar.
- Useful side effect: the table itself is what proved X10.

### Search / Spotlight (`overlay-launcher.png`)
- A black rectangle with an inner rounded field and four circular buttons — not the measured **642 × 59 pill at 23.2 % height** with blur.
- Placeholder reads **"Spotlight Search"** — an Apple name; use "Search".
- No inline completion (`terminal.app — Open`), which is the behaviour that makes macOS search feel instant.
- Opens as a window: a green "L" tile appears in the Dock.

### Control Centre (`overlay-quick-settings.png`)
- Opaque panel ~380 px wide; measured target is **287** with blur.
- Purple icon circles; a Sound **switch** plus a thin slider with a dark knob; Power Mode as three text pills.
- Layout does not match the measured single-column module set (pills → Now Playing → circular buttons → Display/Sound sliders → Edit Controls).
- Opens as a window: a "Q" tile appears in the Dock.

### Notification Centre (`overlay-notification-center.png`)
- Has a **title bar reading "Notification Center" and a close button** — macOS has neither; cards float freely.
- Every card shows the app as a generic **"A" / "Application"** instead of the real icon and name.
- Test notifications are duplicated; grouping shows "2 notifications" but still lists both.
- Opaque, square, no blur; opens as a window ("N" tile in the Dock).

### Apps (`overlay-app-drawer.png`) — closest to correct
- Right shape: floating panel, search field, category chips, icon grid.
- Wrong: opaque and square (no glass), panel ≈ 1050 × 900 instead of **840 × 570**, 8 columns instead of 7, purple active chip, chips named All/Productivity/Internet/Media/Developer/Utilities/System instead of the macOS 27 set, a grid/list toggle macOS does not have, placeholder "Search Apps" instead of "Applications", and third-party icons unplated (X8).

## Fix order (each item is one commit)

1. **X1 window chrome** — radius 16 + active/inactive shadows for every rmac app; confirm the niri window rules actually match the running app-ids.
2. **X2 overlays → layer-shell** — Search, Control Centre, Notification Centre, Apps stop being windows: no Dock tile, no menu-bar identity, no close button.
3. **X3 accent** — replace every purple with `#1372F9` from `rmac-design`; no component may hardcode a colour.
4. **X5 window controls** — delete the Windows-style buttons from Terminal and System Monitor.
5. **X6 error presentation** — no raw text, no "niri", no "Caused by"; one sentence plus one action.
6. **X10 idle CPU** — top bar, Dock and wallpaper must redraw only on change (see `docs/live-audit-2026-09-19.md` §2).
7. **X4 materials** — request blur for Dock, menus and all four overlays; apply the measured composites.
8. **X7 type scale** — 13 px body, 28 px sidebar rows, 24 px list rows everywhere.
9. **X9 naming** — one name per app; the Files app says Files; overlays never appear in the menu bar.
10. **X8 icon plates** for third-party apps in the Apps grid.
11. Then the per-app lists above, app by app, starting with Files' toolbar and Settings' sidebar.

## Re-run

```sh
ssh -i ~/.ssh/rmac-reference-pc jacob@192.168.18.52 'bash -s' < scripts/linux/capture-live-surfaces.sh
scp -i ~/.ssh/rmac-reference-pc 'jacob@192.168.18.52:/tmp/caps/*.png' target/evidence/live-surfaces/
```
(The remote script used for this audit is inlined in the session log; promote it to
`scripts/linux/capture-live-surfaces.sh` so the next run is one command.)
