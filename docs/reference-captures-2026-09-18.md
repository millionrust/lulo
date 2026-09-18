# Reference captures — macOS 27.0 Golden Gate, 2026-09-18

Captured on the owner's reference Mac (macOS 27.0, build 26A428, 1920×1080 @ 1×, **Dark**
appearance, accent Multicolour, en-GB with India region) and measured pixel by pixel. These values
are **measured (`M`)** and outrank every `S` guess in `COMPLETION_SPEC.md` §4.

Image files live in `target/evidence/reference-mac/` (git-ignored):
`menu-file-open-dark.png`, `menu-submenu-dark.png`, `control-center-dark.png`,
`notification-center-dark.png`, `spotlight-empty-dark.png`, `spotlight-typing-dark.png`,
`finder-view1..4-dark.png`, `sheet-save-dark.png`, `open-panel-dark.png`,
`alert-unsaved-dark.png`, `settings-{appearance,desktop,wifi,notifications}-dark.png`.

## Measured geometry

| Surface | Measurement | Value |
|---|---|---|
| Menu bar | height (wallpaper rows 0–28; maximized window starts at row 29) | **29 px**, fully transparent — no tint row exists |
| Menu (Finder ▸ File) | panel width | **269 px** |
| Menu | item pitch | **24 px** |
| Menu | panel top below menu bar | ~1 px (top edge at y ≈ 30) |
| Spotlight | bar size | **642 × 59 px**, pill radius ≈ 29.5 |
| Spotlight | position | horizontally centred; top edge at y = 251 → **23.2 % of output height** |
| Control Center | panel width | **287 px**; top edge y ≈ 63 (≈ 34 px below the menu bar), right inset ≈ 20 px |
| Dock | rendered tile | **64 px** (the `tilesize` preference of 78 renders as 64 on this display) |
| Dock | pitch / gap | **76 px / 12 px** |
| Dock | shelf height / bottom margin | **≈ 72 px / ≈ 18 px** |
| Dock | running indicator | 4 px dot, centred below the shelf |

## Layout corrections (what the captures show that the spec got wrong)

1. **Spotlight has no results card in the common case.** Typing shows a single pill with an **inline
   completion** ("terminal.app — Open") and the target's glyph in a rounded square at the right end
   of the bar. A result list only appears for multi-result queries. Build the inline completion
   first; the list is secondary.
2. **Control Center is a single column of mixed-size modules**, not a 2×2 grid:
   Wi-Fi pill (icon circle + name + subtitle), Bluetooth pill, AirDrop pill, Now Playing card to
   their right, then a row of **circular** buttons (appearance toggle, screen mirroring, Focus),
   then full-width **Display** and **Sound** slider modules, then an **Edit Controls** text button at
   the bottom right. Pills use a large corner radius (≈ 26) and a filled circular glyph badge.
3. **Appearance pane (macOS 27)** contains, in order: Appearance (Light/Dark/Auto picture tiles) ·
   **Liquid Glass** (live preview window + a slider) · Theme → **Colour** (Multicolour + 8 dots) ·
   **Text highlight colour** (pop-up, "Automatic") · **Icon & widget style** (Default / Dark / Clear /
   Tinted picture tiles). Settings sidebar also lists **Menu Bar**, **Siri**, **Spotlight**, and
   **Energy** as panes.
4. **Save sheet layout confirmed**: attached under the title bar, "Save As:" field, Tags, File
   Format, Where; **Delete** (red) bottom-left, Cancel and **Save** bottom-right — matches the spec.
5. **Finder sidebar is edge-to-edge** with sections Favourites / Locations / Tags, and the window
   uses a status bar ("70 items, 74.97 GB available"). The owner's locale means UK spellings
   ("Favourites", "Colour") and **"Bin" instead of "Trash"** — rmac must follow the locale, not
   hardcode US strings.
6. **Menu items carry no icons** in ordinary menus (macOS 27 removed them); the tag colour row and
   Share/Manage Shared File rows are the exceptions.


## Second capture pass (same day): Apps, Dock menu, Mission Control, light mode

| Surface | What the capture shows |
|---|---|
| **Apps** (`apps-grid-dark.png`) | **Not a full-screen Launchpad.** It is a floating glass panel ≈ **840 × 570** centred on screen: a search field at the top with a grid glyph and the placeholder **"Applications"**, a `⋯` menu at the top right, a row of **category chips** (Productivity & Finance · Utilities · Developer Tools · Creativity · Social · Entertainment · Other), then a scrolling **7-column** grid of ≈ 56 px icons with one-line labels. No pages, no page dots, no full-screen blur. |
| **Dock context menu** (`dock-context-menu-dark.png`) | ≈ 165 px wide, radius ≈ 10, with a small **triangular pointer** at the bottom aimed at the tile. Finder's menu: window list with ✓ · ─ · New Finder Window · New Smart Folder · Find… · ─ · Go to Folder… · Connect to Server… · ─ · favourite/recent folders · ─ · Show All Windows · Hide. **No icons on any row.** |
| **Mission Control** (`mission-control-dark.png`) | Windows scale down and spread without overlapping, each labelled with its app name; the Spaces strip collapses to a single **"Desktop" pill** at top centre with a **+** at the far right; the Dock stays visible; the focused window keeps an accent outline. |
| **Light mode** (`menu-file-open-light.png`, `control-center-light.png`, `spotlight-typing-light.png`, `desktop-light.png`) | Same geometry as dark; materials become light tints. Use these as the light-theme colour source. |
| **Lock screen** | Still missing — it cannot be captured from inside the session. Apple's material says only that the new wallpapers "animate when your Mac unlocks" ([9to5Mac](https://9to5mac.com/2026/07/06/macos-27-golden-gate-adds-these-new-wallpapers-and-screen-savers-to-your-mac/)); no published source gives the clock size, avatar size, or field geometry. **Photograph it with a phone** and measure from that, or accept the values in `COMPLETION_SPEC.md` §6.1 as approximations and mark them `S`. |

### Corrections this pass forces in `COMPLETION_SPEC.md`

1. **§5.5 Apps** — rebuilt as the floating panel above. The category chips that task 5.1 told you to delete from `launcher-app` actually belong **here**; only Spotlight loses them.
2. **§4.8 Dock context menu** — add the triangular pointer and confirm no row icons.
3. **§2.7 Mission Control** — the Spaces strip is a single "Desktop" pill plus **+**, not a filmstrip of numbered Spaces.

## Still to capture

Only three things remain: the **lock screen** (photograph it), **Dock magnification** (turn it on in
Desktop & Dock, hover, capture, turn it off), and light-appearance versions of the Finder views and
Settings panes.
