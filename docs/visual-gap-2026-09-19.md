# Visual gap baseline — rmac vs macOS, 2026-09-19

This is the Task 1 baseline from the Ubuntu reference PC at commit `77f38ef`. Every comparison
below uses the side-by-side image named in the heading: rmac is on the left and the owner's macOS
capture is on the right.

## Measurement method and limits

- Both source captures are 1920 × 1080 PNGs. The Mac reference is 1920 × 1080 at 1×. The Ubuntu
  output is 1920 × 1080 physical / 1536 × 864 logical at scale 1.25, as reported by
  `niri msg outputs`. All values below are captured **physical pixels**, not inferred logical
  pixels. This 25% scale mismatch is itself a system-wide visual gap.
- Bounds and pitches were measured from the source PNGs with
  `scripts/measure-reference.py sample`, `edges-row`, and `edges-col`. The exact reference values
  already recorded in `docs/reference-captures-2026-09-18.md` are used where noted.
- A positive delta means rmac is larger/lower/farther than macOS; a negative delta means it is
  smaller/higher/closer. Values prefixed with `≈` inherit that qualification from the measured
  reference document.
- The current desktop reference is not a clean desktop: its Mac half contains open windows and
  Spotlight. Only the menu bar and Dock, which remain measurable, are compared for that pair.
- Banner and OSD were captured as `banner.png` and `osd.png`, but the playbook script defines no
  macOS reference mapping for either one. They therefore have no pair and no claimed visual pass;
  inventing a comparison would violate the no-invented-numbers rule.

## Pair-by-pair gaps

### Desktop

Pair: `target/evidence/rmac-2026-09-19/pairs/desktop-vs-mac.png`

- Menu bar: **36 px vs 29 px**, so rmac is **7 px too tall**.
- Dock shelf: **92 px vs ≈72 px**, so rmac is **≈20 px too tall**.
- Dock bottom inset: **22 px vs ≈18 px**, so rmac sits **≈4 px too high**.
- Dock shelf width in these captures: **678 px vs 1,238 px**, a **560 px shortfall** caused by the
  much smaller default pinned/running set.

### Menu open

Pair: `target/evidence/rmac-2026-09-19/pairs/menu-open-vs-mac.png`

- Panel width: **310 px vs 269 px**, so rmac is **41 px too wide**.
- Repeated item pitch: **45 px vs 24 px**, so rows are **21 px too tall**.
- Panel top: **y = 39 px vs y ≈ 30 px**, so the panel begins **≈9 px too low**.
- Menu bar under the panel: **36 px vs 29 px**, retaining the **7 px** global height error.

### Search while typing

Pair: `target/evidence/rmac-2026-09-19/pairs/search-typing-vs-mac.png`

- Captured outer width: **885 px vs 642 px**, so rmac is **243 px too wide**.
- Captured visible height: **212 px vs 59 px**, so rmac is **153 px too tall**; the rmac panel is
  also clipped by the top edge.
- Top edge: **y = 36 px vs y = 251 px**, so rmac begins **215 px too high**.
- Target pill radius is **≈29.5 px**; rmac presents a large rectangular result surface instead of
  the measured 59 px pill, leaving a **153 px height excess** before radius can be compared.

### Control Center

Pair: `target/evidence/rmac-2026-09-19/pairs/control-center-vs-mac.png`

- Panel width: **380 px vs 287 px**, so rmac is **93 px too wide**.
- Top edge: **y = 51 px vs y ≈ 63 px**, so rmac begins **≈12 px too high**.
- Right inset: **15 px vs ≈20 px**, so rmac is **≈5 px too close** to the output edge.
- Menu-bar-to-panel gap: **15 px vs ≈34 px**, so the gap is **≈19 px too small**.

### Notification Center

Pair: `target/evidence/rmac-2026-09-19/pairs/notification-center-vs-mac.png`

- Occupied width: **450 px vs 346 px**, so rmac is **104 px too wide**.
- Top edge: **y = 51 px vs y = 37 px**, so rmac begins **14 px too low**.
- Occupied height: **900 px (51–951) vs 418 px (37–455)**, so rmac is **482 px too tall**.
- Bottom edge: **y = 951 px vs y = 455 px**, so the opaque rmac rail extends **496 px too far
  downward** instead of ending with the floating card stack.

### Apps

Pair: `target/evidence/rmac-2026-09-19/pairs/apps-vs-mac.png`

- Panel width: **1,058 px vs 845 px**, so rmac is **213 px too wide**.
- Panel height: **900 px vs 578 px**, so rmac is **322 px too tall**.
- Top edge: **y = 52 px vs y = 251 px**, so rmac begins **199 px too high**.
- Bottom edge: **y = 952 px vs y = 829 px**, so rmac ends **123 px too low**.

### Files — Icon view

Pair: `target/evidence/rmac-2026-09-19/pairs/files-icons-vs-mac.png`

- Window width: **938 px vs 1,577 px**, so the rmac window is **639 px too narrow**.
- Sidebar width: **237 px vs 203 px**, so rmac is **34 px too wide**.
- Horizontal item pitch: **155 px vs 134 px**, so the grid is **21 px too loose**.
- Menu bar: **36 px vs 29 px**, retaining the **7 px** global height error.

### Files — List view

Pair: `target/evidence/rmac-2026-09-19/pairs/files-list-vs-mac.png`

- Window width: **1,890 px vs 1,577 px**, so rmac is **313 px too wide**.
- Sidebar width: **237 px vs 203 px**, so rmac is **34 px too wide**.
- Body row pitch: **30 px vs 20 px**, so every row is **10 px too tall**.
- Menu bar: **36 px vs 29 px**, retaining the **7 px** global height error.

### Files — Column view

Pair: `target/evidence/rmac-2026-09-19/pairs/files-columns-vs-mac.png`

- Window width: **1,890 px vs 1,577 px**, so rmac is **313 px too wide**.
- Sidebar width: **237 px vs 203 px**, so rmac is **34 px too wide**.
- First content-column width: **290 px vs 242 px**, so rmac is **48 px too wide**.
- Menu bar: **36 px vs 29 px**, retaining the **7 px** global height error.

### Files — Gallery view

Pair: `target/evidence/rmac-2026-09-19/pairs/files-gallery-vs-mac.png`

- Window width: **1,890 px vs 1,577 px**, so rmac is **313 px too wide**.
- Sidebar width: **237 px vs 203 px**, so rmac is **34 px too wide**.
- Filmstrip height: **130 px vs 70 px**, so rmac is **60 px too tall**.
- Inspector width: **0 px vs 250 px**, so the selected-item inspector is **250 px missing**.

### Mission Control

Pair: `target/evidence/rmac-2026-09-19/pairs/mission-control-vs-mac.png`

- Main preview top: **y = 296 px vs y = 40 px**, so the rmac preview begins **256 px too low**.
- Main preview height: **450 px vs 957 px**, so it is **507 px too short**.
- Main preview width: **945 px vs 1,033 px**, so it is **88 px too narrow**.
- Spaces title pill: **0 × 0 px vs 75 × 24 px**, so rmac is missing **75 px of width and 24 px
  of height** at the top centre.

### Dock context menu

Pair: `target/evidence/rmac-2026-09-19/pairs/dock-menu-vs-mac.png`

- Menu width: **309 px vs ≈165 px**, so rmac is **≈144 px too wide**.
- Menu item pitch: **≈45 px vs 24 px**, so rows are **≈21 px too tall**.
- Pointer height: **0 px vs 13 px**, so the triangular Dock-tile pointer is **13 px missing**.
- Menu-to-shelf gap without the pointer: **8 px vs 2 px at the target pointer tip**, leaving a
  **6 px larger visual disconnect**.

## Worst three pairs at this baseline

1. **Apps** — **322 px** too tall, **213 px** too wide, and **199 px** too high.
2. **Search while typing** — **243 px** too wide, **153 px** too tall, and **215 px** too high.
3. **Notification Center** — **482 px** too tall, **104 px** too wide, and its bottom extends
   **496 px** too far.

These are observations only. No Tasks 2–12 work is included in this baseline.

## Task 2 measured-colour recapture

Task 2 was re-captured from the Ubuntu reference PC in
`target/evidence/rmac-2026-09-19-task2/pairs/`. The rmac image remains on the left and the same
owner-Mac reference remains on the right. Finder was maximised before its final four captures; an
earlier capture containing a tiled stock Nautilus window was rejected and is not used in a pair.

### Rendered flat-surface measurements

These pixels were sampled with `scripts/measure-reference.py`; coordinates are physical pixels in
the two 1920 × 1080 source PNGs. The largest rendered flat-surface channel error is **1**, within
Task 2's maximum of 2.

| Surface | rmac sample | macOS sample | Per-channel absolute delta | Result |
|---|---:|---:|---:|---|
| Finder content | `222025` at `1000,700` | `222025` at `1000,700` | `0,0,0` | exact |
| Finder sidebar material over the captured desktop | `28242D` at `100,700` | `29252E` at `100,700` | `1,1,1` | pass |
| Finder status bar | `29272C` at `1000,940` | `29272C` at `1000,960` | `0,0,0` | exact |
| Finder search field | `181818` at `1750,84` | measured reference token `181818` | `0,0,0` | exact |

The other measured opaque values are asserted directly in `rmac-design` as `222026` grouped
background, `29272D` grouped row, `262227` sheet, `363237` secondary button, `812E25` destructive
button, and `FFFFFF` dark menu-bar text. Task 1's capture set does not expose clean pixels for all
of those roles, so this recapture does **not** claim an additional rendered comparison for them.
The light colours and all material tint alphas without black/white same-surface captures remain
explicitly marked **S**. In particular, the sidebar's `E0` alpha is S even though its captured
composite is within one channel step.

### Pair paths and remaining non-colour differences

Colour work did not change geometry. These are the largest still-visible pixel differences in the
new pairs; they remain assigned to later playbook tasks.

| Surface | Task 2 pair | Remaining measured difference |
|---|---|---|
| Desktop | `target/evidence/rmac-2026-09-19-task2/pairs/desktop-vs-mac.png` | menu bar **36 px vs 29 px**, rmac **7 px too tall** |
| Menu open | `target/evidence/rmac-2026-09-19-task2/pairs/menu-open-vs-mac.png` | panel **310 px vs 269 px**, rmac **41 px too wide** |
| Search typing | `target/evidence/rmac-2026-09-19-task2/pairs/search-typing-vs-mac.png` | outer width **885 px vs 642 px**, rmac **243 px too wide** |
| Control Center | `target/evidence/rmac-2026-09-19-task2/pairs/control-center-vs-mac.png` | panel **380 px vs 287 px**, rmac **93 px too wide** |
| Notification Center | `target/evidence/rmac-2026-09-19-task2/pairs/notification-center-vs-mac.png` | occupied width **450 px vs 346 px**, rmac **104 px too wide** |
| Apps | `target/evidence/rmac-2026-09-19-task2/pairs/apps-vs-mac.png` | panel **1,058 px vs 845 px**, rmac **213 px too wide** |
| Files — Icon | `target/evidence/rmac-2026-09-19-task2/pairs/files-icons-vs-mac.png` | maximised window **1,890 px vs 1,577 px**, rmac **313 px too wide** |
| Files — List | `target/evidence/rmac-2026-09-19-task2/pairs/files-list-vs-mac.png` | row pitch **30 px vs 20 px**, rmac rows **10 px too tall** |
| Files — Column | `target/evidence/rmac-2026-09-19-task2/pairs/files-columns-vs-mac.png` | first content column **290 px vs 242 px**, rmac **48 px too wide** |
| Files — Gallery | `target/evidence/rmac-2026-09-19-task2/pairs/files-gallery-vs-mac.png` | filmstrip **130 px vs 70 px**, rmac **60 px too tall** |
| Mission Control | `target/evidence/rmac-2026-09-19-task2/pairs/mission-control-vs-mac.png` | main preview **450 px vs 957 px**, rmac **507 px too short** |
| Dock menu | `target/evidence/rmac-2026-09-19-task2/pairs/dock-menu-vs-mac.png` | menu **309 px vs ≈165 px**, rmac **≈144 px too wide** |

Task 2 therefore closes the measured flat-colour gap only. It does not close the recorded layout,
typography, material-alpha, wallpaper-tint, or interaction gaps.

## Task 3 wallpaper-tinting recapture

The reference-PC capture pair is
`target/evidence/rmac-2026-09-19-task3/pairs/strong-vs-gray.png`. Both halves are the same maximised
Finder window and the same state; only the wallpaper source changes. The left source is a strong
purple image and the right source is neutral gray.

The live wallpaper authority published these measured values before the captures:

| Source | Published 8×8 sRGB average | Relative luminance |
|---|---:|---:|
| Strong purple | `[190, 62, 221]` | `0.1961286` |
| Neutral gray | `[128, 128, 128]` | `0.21586052` |

Pixels below were sampled with `scripts/measure-reference.py` from the two 1920 × 1080 source
images. The gray side's chroma is at most 3 channel levels, while all three required Finder
surfaces visibly take the strong wallpaper's hue.

| Surface and coordinate | Strong wallpaper | Gray wallpaper | Strong − gray | Gray chroma |
|---|---:|---:|---:|---:|
| Finder content `1000,700` | `28202B` | `242426` | `+4,-4,+5` | 2 |
| Finder sidebar `100,700` | `322636` | `2C2C2E` | `+6,-6,+8` | 2 |
| Finder toolbar `1000,85` | `352B3A` | `303033` | `+5,-5,+7` | 3 |

The sidebar material initially retained Task 2's purple reference composite over the gray source.
That rejected capture is not used. The final implementation keeps a neutral sidebar base and applies
the explicit 0.10 sidebar role once; menus, Control Center, Dock, and other floating glass tints are
unchanged. The preference `Allow wallpaper tinting in windows` defaults on and persists through the
existing theme authority.

### Remaining measured differences

Task 3 changes colour response, not layout. The current capture therefore retains these measured
macOS geometry gaps from the immediately preceding Finder pair:

- Menu bar: **36 px vs 29 px**, so rmac remains **7 px too tall**.
- Maximised Finder window: **1,890 px vs 1,577 px**, so rmac remains **313 px wider** in the
  available reference captures.
- Finder gallery filmstrip: **130 px vs 70 px**, so rmac remains **60 px too tall**.

The reference PC has one output, so the per-output JSON publication is exercised for `eDP-1` only.
Multi-output window-local selection remains unclaimed by this evidence.
