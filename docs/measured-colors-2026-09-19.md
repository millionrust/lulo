# Measured colours — macOS 27.0, reference Mac, 2026-09-19

Sampled pixel by pixel from `target/evidence/reference-mac/*.png` (1920×1080 @ 1×, Dark appearance,
accent Multicolour, wallpaper: purple/blue photo). Re-run with
`python3 scripts/measure-reference.py` after any new capture.

**These replace the guessed values in `crates/rmac-design`.** Where a value here differs from
`COMPLETION_SPEC.md` §4, this file wins.

## Three findings that change how the whole desktop looks

### 1. The accent is `#1372F9`, not `#0A84FF`
Sampled from two independent places: the selected sidebar row in System Settings and the default
**Save** button in a sheet. Both give exactly `1372F9`. Every accent-coloured pixel in rmac — sidebar
selection, default buttons, switches, focus rings, Control Center active toggles — must use it.

### 2. Window backgrounds are **wallpaper-tinted**, not neutral grey
macOS mixes a little of the desktop wallpaper's hue into window surfaces ("Allow wallpaper tinting
in windows", on by default). Measured against a purple wallpaper:

| Surface | Measured | Neutral equivalent | Difference |
|---|---|---|---|
| Finder content | `222025` | `1E1E20` | +4 red, +5 blue: visibly warmer/purpler |
| Finder sidebar | `29252E` | `242426` | strongly purple |
| Settings sidebar | `29252F` | `242426` | same |
| Settings pane background | `222026` | `1C1C1E` | same |
| Sheet fill | `262227` | `2C2C2E` | darker and tinted |

rmac currently paints flat neutral greys. Against a colourful wallpaper that reads as *dead* —
it is one of the strongest "this is not a Mac" signals and nobody can name it.

**Implement:** `rmac-design` gains `wallpaper_tint: Rgba` (the dominant hue of the current
wallpaper, published by the wallpaper service, already computed there for menu-bar luminance) and a
`tint(base, strength)` helper. Strengths measured here: window/content **0.06**, sidebar **0.10**,
grouped rows **0.07**, sheets **0.05**. Off when the user disables tinting or picks a solid colour.

### 3. Light-mode materials are almost opaque; dark-mode materials are not
Spotlight's bar in Light measures `FCFCFC` — effectively opaque white. The same bar in Dark measures
`3E4958` over a mid-dark wallpaper, i.e. genuinely translucent. Menus behave the same way
(`F2F2F4`–`F8F9FB` light vs `3E3C44`–`4B4953` dark). Using one alpha for both themes, as
`rmac-design` does today, makes light mode look smeary and dark mode look flat.

## Measured values

### Opaque surfaces (exact; use directly)

| Token | Dark | Light | Source |
|---|---|---|---|
| `accent` | `1372F9` | (capture Light Settings to confirm) | Settings sidebar selection, Save button |
| `surface.window` (content) | `222025` (tinted) | — | Finder content area |
| `surface.sidebar` | `29252E` (tinted) | — | Finder + Settings sidebars |
| `surface.grouped.background` | `222026` | — | Settings detail pane |
| `surface.grouped.row` | `29272D` | — | Settings grouped rows |
| `surface.sheet` | `262227` | — | TextEdit save sheet |
| `field.fill` (inside sheet) | `181818` | — | "Save As" text field |
| `button.secondary` (Cancel) | `363237` | — | sheet Cancel |
| `button.destructive` (Delete) | `812E25` | — | sheet Delete |
| `statusbar` | `29272C` | — | Finder status bar |
| menu-bar text | `FFFFFF` | `010206` | brightest/darkest pixel in the app-name area |

### Translucent materials (composite over the test wallpaper; solve alpha per §"How to finish this")

| Material | Dark composite | Light composite |
|---|---|---|
| menu panel | `3E3C44` … `4B4953` | `F2F2F4` … `F8F9FB` |
| menu separator | `3E424B` | `F2F4F7` |
| Spotlight bar | `3E4958` | `FCFCFC` |
| Control Center module | `36414E` | (recapture: the Light capture was taken mid-transition) |
| Control Center slider track | `4F5E76` | — |
| Dock shelf | see `desktop-light.png` | `3A3C3E`-ish over dark wallpaper areas |

## How to finish this (task for the agent, 30 minutes)

A composite tells you `c = α·tint + (1−α)·background`. Two unknowns, so sample the **same material
over two different backgrounds** and solve:

1. Set a solid **black** desktop wallpaper on the Mac. Capture the menu, Spotlight, Control Center.
2. Set a solid **white** wallpaper. Capture the same three.
3. For each pixel pair: `α = 1 − (c_white − c_black) / 255`, `tint = (c_black − (1−α)·0) / α`.
4. Write the resulting `(tint, α)` pairs into `rmac-design` as the real material tokens, and delete
   the guessed alphas.

That sequence takes one person ten minutes at the Mac and removes the last guessed numbers from the
visual system.
