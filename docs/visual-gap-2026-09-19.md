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

## Live-surface X1 — window radius and shadows

Fresh reference-PC captures are in `target/evidence/live-surfaces-x1/`. The two direct macOS
comparisons are:

- `target/evidence/live-surfaces-x1/pairs/app-files-vs-mac.png`
- `target/evidence/live-surfaces-x1/pairs/app-settings-vs-mac.png`

The capture metadata beside every PNG reports the compositor app IDs. The six normal applications
were `org.rmac.Files`, `org.rmac.SystemSettings`, `org.rmac.Terminal`, `org.rmac.Notes`,
`org.rmac.SystemMonitor`, and `org.rmac.TextEditor`. The window rule is deliberately unscoped, so
all six IDs and third-party IDs receive the same silhouette.

### X1 measurements

- Terminal's captured top-left curve begins **16 px** inside both the top and left bounds; the
  design-lab window radius is **16 px**, for a **0 px radius difference**.
- Active shadow offset is **12 px** vs the design-lab **12 px**, a **0 px difference**.
- Active shadow softness is **32 px** vs the design-lab **32 px**, a **0 px difference**.
- Inactive shadow offset is **4 px** vs the design-lab **4 px**, a **0 px difference**.
- Inactive shadow softness is **12 px** vs the design-lab **12 px**, a **0 px difference**.
- The dark-window top inner edge remains absent: **0 px vs 0.5 px**, a **0.5 px shortfall** to be
  handled with the shared window material rather than a compositor shadow.

The earlier `target/evidence/live-surfaces/` audit was made while niri had loaded the Ubuntu
recovery configuration (`~/.config/niri/config.kdl`), so it could not exercise the package-owned
rmac rule. This recapture temporarily loaded the candidate policy, restored the recovery file
after capture, and verified every app ID in the emitted `*-windows.json` files.

## Live-surface X2 — overlays are layer-shell surfaces

Fresh reference-PC captures and compositor inventories are in
`target/evidence/live-surfaces-x2/`. The direct comparisons are:

- `target/evidence/live-surfaces-x2/pairs/overlay-launcher-vs-mac.png`
- `target/evidence/live-surfaces-x2/pairs/overlay-quick-settings-vs-mac.png`
- `target/evidence/live-surfaces-x2/pairs/overlay-notification-center-vs-mac.png`
- `target/evidence/live-surfaces-x2/pairs/overlay-app-drawer-vs-mac.png`

The `*-windows.json` and `*-layers.json` files were captured while each surface was visible. They
show the user-visible X2 contract directly:

| Surface | Matching normal windows | Matching layers | Layer | Keyboard | Native title-bar height | Overlay Dock tiles |
|---|---:|---:|---|---|---:|---:|
| Search | 0 | 1 | Overlay | Exclusive | 0 px | 0 |
| Control Centre | 0 | 1 | Overlay | OnDemand | 0 px | 0 |
| Notification Centre | 0 | 1 | Overlay | OnDemand | 0 px | 0 |
| Apps | 0 | 1 | Overlay | OnDemand | 0 px | 0 |

Because the Dock and menu-bar identity are derived from Niri's normal-window inventory, the zero
normal-window count also removes the four transient Dock tiles and the internal Launcher,
QuickSettings, NotificationCenter, and Apps menu-bar identities. Notification Centre's old close
button occupied its native title bar; that bar is now **0 px**, matching the **0 px** macOS target.

### Remaining measured visual differences

X2 changes surface ownership, not the overlay designs. These bounds were measured from the fresh
1920 × 1080 PNGs with `scripts/measure-reference.py`; reference values are the captured values
already recorded above in this document.

- Search: **806 × 76 px** vs **642 × 59 px**, so it remains **164 px too wide** and **17 px too
  tall**. Its top is **y = 479 px** vs **y = 251 px**, so it is **228 px too low**.
- Control Centre: **380 px** wide vs **287 px**, so it remains **93 px too wide**. Its top is
  **y = 106 px** vs **y ≈ 63 px**, so it is **≈43 px too low**.
- Notification Centre: **450 × 877 px** vs **346 × 418 px**, so it remains **104 px too wide** and
  **459 px too tall**. Its top is **y = 106 px** vs **y = 37 px**, so it is **69 px too low**.
- Apps: **951 × 651 px** vs **845 × 578 px**, so it remains **106 px too wide** and **73 px too
  tall**. Its top is **y = 191 px** vs **y = 251 px**, so it is **60 px too high**.

The flat opaque materials, purple accent, and internal layout differences visible in the pairs are
not claimed by X2; they remain assigned to X3, X4, and the later per-surface work.

## Live-surface X3 — measured default accent

Fresh reference-PC captures are in `target/evidence/live-surfaces-x3/`. The direct comparisons are:

- `target/evidence/live-surfaces-x3/pairs/app-settings-vs-mac.png`
- `target/evidence/live-surfaces-x3/pairs/overlay-quick-settings-vs-mac.png`
- `target/evidence/live-surfaces-x3/pairs/overlay-app-drawer-vs-mac.png`

The automatic rmac preference now resolves to the measured macOS default `#1372F9` instead of
importing GNOME's host accent. Components continue to consume the shared `rmac-design` theme
authority; the Notes list's former Notes-specific yellow selection was the one exception and now
uses that authority with `on_accent` text.

### X3 measurements

The following pixels were sampled from the fresh 1920 x 1080 PNGs with
`scripts/measure-reference.py`:

| Surface | Coordinates | Captured | Measured reference | Channel difference |
|---|---|---:|---:|---:|
| Settings selected sidebar row | `50,250`, `200,275`, `300,250` | `#1372F9` | `#1372F9` | `0, 0, 0` |
| Settings Wi-Fi hero icon | `675,160` | `#1372F9` | `#1372F9` | `0, 0, 0` |
| Notes selected row | `600,170`, `600,210`, `300,220` | `#1372F9` | `#1372F9` | `0, 0, 0` |
| Apps active-chip border | `641,298`, `665,280`, `691,298` | `#1952A4` composite | shared accent at 50% opacity | n/a (composite) |

An exact-pixel scan found **21,264** `#1372F9` pixels in the clean Settings capture and
**25,825** in the clean Notes capture. There is no Notes reference capture in
`target/evidence/reference-mac/`, so no direct Notes pair is claimed; the measured colour authority
is the reference for that surface.

### Remaining measured visual differences

X3 changes the accent authority, not geometry or materials. The clean Settings compositor inventory
records a **750 logical px** rmac window; the macOS capture has no compositor metadata, so its
corresponding logical-width difference remains **S** rather than being invented from the screenshot.
The overlay bounds remain the measured X2 values: Control Centre is **93 px too wide**, and Apps is
**106 px too wide** and **73 px too tall**. Their opaque material difference remains assigned to X4.

## Live-surface X5 — one window-control language

Fresh reference-PC captures are in `target/evidence/live-surfaces-x5/`. Because the reference set
does not contain Terminal or Activity Monitor, the direct evidence pairs show the audited build
beside the corrected build rather than pretending an unrelated macOS window is a reference:

- `target/evidence/live-surfaces-x5/pairs/app-terminal-before-after.png`
- `target/evidence/live-surfaces-x5/pairs/app-monitor-before-after.png`

The shared gpui-component title bar unconditionally appended three Linux controls after its
application content. rmac now owns that drag region, preserving the existing left traffic lights
without rendering a second control language.

### X5 measurements

- The removed right-hand cluster was **3 × 34 = 102 logical px** in both Terminal and System
  Monitor. The fresh captures contain **0** right-hand window controls, matching the macOS target;
  the difference is **0 px**.
- The left cluster remains **3** traffic lights with the measured **12 logical px** diameter; its
  count and diameter changed by **0 px**.
- The replacement title bar deliberately retains the prior **34 logical px** height, a **0 px**
  regression within X5. `rmac-design` maps the eventual title-bar target to **38 logical px**, so a
  **4 logical px shortfall** remains for the later density pass rather than being hidden in this
  control-only commit.

The remaining System Monitor card/table redesign and Terminal content padding are unchanged and
remain in their per-surface audit lists. The fresh full-surface run also confirms the shared change
did not add right-side controls to any other rmac window.

## Live-surface X6 — user-safe error presentation

Fresh reference-PC captures are in `target/evidence/live-surfaces-x6/`. The reference-Mac set has
no capture of an equivalent inline failure, so these pairs compare the audited failure with the
corrected failure instead of presenting an unrelated macOS alert as evidence:

- `target/evidence/live-surfaces-x6/pairs/app-files-before-after.png`
- `target/evidence/live-surfaces-x6/pairs/app-settings-before-after.png`

Visible and accessibility error projections now pass through one shared boundary while the full
domain error remains available to diagnostics. Files and Settings show one bounded sentence and
one recovery action; compositor names, socket names, cause chains, and backtraces are not eligible
for display.

### X6 measurements

- Files' error strip remains **41 physical px** high, a **0 px** geometry change. Its action has a
  **22 physical px** right inset and remains one action before and after the change. The visible
  `verified` and `disabled` diagnostics fall from **1 each to 0**.
- Settings' error surface falls from **100 physical px** high to **55 physical px**, a reduction of
  **45 px**, because the cause chain no longer expands the toast. Its action has a **22 physical
  px** right inset.
- Settings' visible error falls from **4 case-insensitive `niri` occurrences** and **1 `Caused by:`
  line** to **0** of each. Both fresh captures retain exactly **1** visible action.

### Remaining measured visual differences

X6 corrects error content and recovery structure; it does not claim a new macOS error-surface
style. There is no equivalent inline-error capture in `target/evidence/reference-mac/` or
`design-lab/`, so the reference difference for the **41 px** Files strip and **55 px** Settings
toast is **S** rather than invented. Their opaque material remains assigned to X4, while the type
scale remains assigned to X7.

## Live-surface X10 — event-driven idle shell

Fresh reference-PC captures are in `target/evidence/live-surfaces-x10/`. The direct macOS
comparisons are:

- `target/evidence/live-surfaces-x10/pairs/app-settings-vs-mac.png`
- `target/evidence/live-surfaces-x10/pairs/overlay-launcher-vs-mac.png`
- `target/evidence/live-surfaces-x10/pairs/overlay-quick-settings-vs-mac.png`
- `target/evidence/live-surfaces-x10/pairs/overlay-notification-center-vs-mac.png`
- `target/evidence/live-surfaces-x10/pairs/overlay-app-drawer-vs-mac.png`

The capture harness was given explicit current-checkout binary paths so it could launch, capture,
and close each application instead of silently reopening the older installed package. All four
overlay inventories contain **0** transient normal windows and exactly **1** matching layer-shell
surface. The fresh Settings capture uses `#1372F9`, so the set does not regress X2 or X3.

PipeWire Client churn was the top-bar feedback loop. Every authoritative audio read starts
`pw-dump` and `wpctl`; `pw-mon` reported those inspection clients as changes, which caused another
read indefinitely. The watcher now retains Node, Device, and Metadata identities and ignores
Client-only add/remove events. Muting the real default sink still produced **1** render within
**3 seconds**, and the test restored the original mute state.

### X10 measurements

- The unpatched current release consumed **542 scheduler ticks in 30 seconds** and rendered
  **5** times while untouched. After warm-up, the patched release consumed **40 ticks in 30
  seconds** and rendered **0** times: **502 fewer ticks (92.6%)** and **5 fewer renders**.
- During the clean `22:24:06`–`22:24:26` reference-PC window, top bar stayed **12 → 12**, Dock
  stayed **88 → 88**, and wallpaper stayed **23 → 23**: **0 redraws on all three surfaces**.
- A process trace after warm-up recorded **0** `pw-dump`/`wpctl` executions. The subprocess burst
  ends **2.7 seconds** after startup instead of repeating for the lifetime of the shell.
- The package migration now archives an exact `dev.conf` override for any known rmac unit. This
  removes the debug-binary overrides found by the live audit while continuing to refuse symlinks,
  extra files, and unknown rmac units.

There is a measured residual outside the rmac render authorities: the pinned GPUI Wayland backend
requests a compositor frame callback at 60 Hz even while the rmac render counters remain flat.
Top bar, Dock, and wallpaper each consumed **38–40 scheduler ticks in 30 seconds** (about
**1.3% of one core**) at that platform baseline. The newer upstream backend has a parked frame
loop, but changing the pinned GPUI platform is not folded into this cross-cutting application fix.
No rmac surface redraws in response to those callbacks.

### Remaining measured visual differences

X10 changes scheduling, not geometry or colour; the fresh pairs therefore retain the already
measured gaps rather than inventing new values:

- Search remains **164 px too wide**, **17 px too tall**, and **228 px too low**.
- Control Centre remains **93 px too wide**.
- Notification Centre remains **104 px too wide** and **459 px too tall**.
- Apps remains **106 px too wide** and **73 px too tall**.

Those differences belong to X4/X7 and the later per-surface passes. Pixel geometry introduced by
X10 itself is **0 px** on every captured surface.

## Live-surface X4 — bounded glass materials

Fresh reference-PC captures are in `target/evidence/live-surfaces-x4/`. The direct comparisons are:

- `target/evidence/live-surfaces-x4/pairs/menu-open-vs-mac.png`
- `target/evidence/live-surfaces-x4/pairs/launcher-vs-mac.png`
- `target/evidence/live-surfaces-x4/pairs/quick-settings-vs-mac.png`
- `target/evidence/live-surfaces-x4/pairs/notification-center-vs-mac.png`
- `target/evidence/live-surfaces-x4/pairs/app-drawer-vs-mac.png`

The full-output Dock and menu-bar interaction hosts remain transparent. Each now owns a separate,
bounded material surface, so requesting blur cannot soften the entire desktop. Search, Control
Centre, Notification Centre, and Apps request the compositor blur protocol on their already bounded
layer-shell surfaces. The scoped niri rule sets `xray false`, making those materials sample the real
window below instead of only the wallpaper; it does not force a background effect or change their
Overlay-layer ordering.

### X4 measurements

The tint values are copied from `design-lab/tokens.css` into `rmac-design`; the mapping difference is
**0 channels** and **0 alpha steps** for every role below:

| Role | Dark token | Light token | Design-lab difference |
|---|---:|---:|---:|
| Menu | `rgba(44,44,50,.80)` (`#2C2C32CC`) | `rgba(246,246,248,.86)` (`#F6F6F8DB`) | `0, 0, 0, 0` |
| Popover | `rgba(40,42,48,.78)` (`#282A30C7`) | `rgba(244,244,246,.84)` (`#F4F4F6D6`) | `0, 0, 0, 0` |
| HUD | `rgba(28,28,32,.72)` (`#1C1C20B8`) | `rgba(255,255,255,.80)` (`#FFFFFFCC`) | `0, 0, 0, 0` |
| Dock | `rgba(255,255,255,.14)` (`#FFFFFF24`) | `rgba(255,255,255,.40)` (`#FFFFFF66`) | `0, 0, 0, 0` |
| Dock border | `rgba(255,255,255,.35)` (`#FFFFFF59`) | same | `0, 0, 0, 0` |

The menu material capture is **248 logical px** wide and its rows are **28 logical px**, matching the
checked shell mapping. The compositor blur kernel is expressed by niri as **3 passes × 3 px offset**,
not a CSS radius, so a numerical comparison with design-lab's `blur(40px)` remains **S** rather than
inventing an equivalence.

### Remaining measured visual differences

X4 changes material ownership and colour response, not surface layout. The fresh pairs therefore
retain the measured geometry gaps:

- Search remains **164 px too wide**, **17 px too tall**, and **228 px too low**.
- Control Centre remains **93 px too wide** and about **43 px too low**.
- Notification Centre remains **104 px too wide**, **459 px too tall**, and **69 px too low**.
- Apps remains **106 px too wide**, **73 px too tall**, and **60 px too high**.
- The Dock shelf remains **92 px vs ≈72 px**, or **≈20 px too tall**, and its bottom inset remains
  **22 px vs ≈18 px**, or **≈4 px too high**.

The menu and Dock glass are now bounded, but their content geometry, overlay sizing, and the
Notification Centre's unused panel area remain visible differences for the later per-surface pass.
