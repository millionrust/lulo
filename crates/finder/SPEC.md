# Finder fidelity spec (from Apple docs + screenshot)

Authoritative values (points). Sources: AppKit/NSColor, HIG, measured on light mode.

## Chrome
- Unified toolbar height: **52 pt**. Bg ≈ `#f6f6f6` (vibrancy), bottom hairline `#e5e5e5`.
- Traffic lights: 12 pt glyphs, 20 pt center spacing, vertically centered (center.y ≈ 26).
  → `traffic_light_position ≈ (19, 19)`, content left gutter ≈ 80 pt.
- Toolbar is **draggable** (start_window_move on drag).

## List view
- Row height **24 pt**; body text **13 pt**; selected row text → white.
- Column header **11 pt**, `secondaryLabel`; header height ~26 pt; bottom hairline.
- Disclosure indent per level **16 pt**; chevron ~10 pt, leading the icon. Icon→text gap **6 pt**.
- Alternating row stripes: `#ffffff` / `#f4f5f5`.

## Colors (light)
| token | hex |
|---|---|
| list bg | `#ffffff` |
| toolbar | `#f6f6f6` |
| sidebar | `#e9e9ed` |
| alt row | `#f4f5f5` |
| selected row (focused) | `#0063e1` (white text) |
| accent / systemBlue (folder tint) | `#007aff` |
| separator | `#e5e5e5` |
| label / secondary / tertiary | `#272727` / `#808080` / `#bfbfbf` |
| drive icon tint | `#808080` (gray) |

## Sidebar
- Sections: **Favorites**, **iCloud**, **Locations** (+ Tags). 11 pt semibold gray headers.
- Row ~28 pt, icon 18 px, text 13 pt. Selected = gray `#d8d8dc` rounded (unfocused) / accent (focused).
- Icon tint rule: folders/locations = **blue**; physical drives/hardware = **gray**.
- SF Symbol → bundled SVG: folder→folder-fill, Applications→layout-grid, Downloads→download,
  Recents→clock, Macintosh HD→hard-drive(gray), iCloud→cloud, home→house, drive→hard-drive(gray).

## Kind column
- Real Finder uses `UTType.localizedDescription`. Approximate by extension:
  Folder, Plain Text Document, PDF document, PNG image, JSON document, Markdown Document,
  Application, ZIP archive, Document (generic).

## Date
- `medium` date + `short` time + relative: "Today at 11:12 AM", "Yesterday at 1:30 PM",
  "18 Apr 2026 at 2:42 PM" (day-first, abbreviated month).

## Toolbar controls
- Left: back/forward chevrons. Title (left, 13 pt semibold) after nav.
- Right: view segmented control (grid/list[active]/columns/gallery), share, tag, more, Search field.

## Accessibility text size
- Finder-owned labels and chrome follow rmac's bounded 100%, 115%, and 130% text preference.
- Standard row and toolbar metrics stay faithful to the values above; their existing vertical room fits 130% glyphs.
- Tab-close and new-tab hit boxes are enlarged to avoid clipping their scaled symbols.
