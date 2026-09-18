# rmac feel spec: making it feel like a Mac, not look like one

> **Companion to `COMPLETION_SPEC.md`.** That file says *what to build*. This file says
> *why it still won't feel right* and exactly how to fix that.
> **Written:** 2026-09-18, against the owner's reference Mac running **macOS 27.0 (Golden Gate,
> build 26A428)** and repository commit `7efcb2b` (149 commits after `COMPLETION_SPEC.md` was
> written: Phase 0 complete, Phase 1 partly done and blocked on reference captures, Phases 2–4 in
> progress).
> **Rule:** every task here is as mandatory as a feature task. A desktop that has every feature and
> none of this feels like a Linux theme of macOS. That is the exact failure the owner is describing.

---

## A. The honest review: what using rmac feels like to a Mac user today

Judged from the code, not from a running machine. A Mac user sits down and, in the first 60 seconds,
notices these things — in this order:

1. **Silence.** Nothing makes a sound. No startup, no alert, no Trash, no screenshot shutter, no
   volume tick, no error beep. macOS is quiet but *never silent*; the sounds are how the machine
   confirms it heard you. rmac ships zero audio files. This is the single biggest "it's not a Mac"
   signal after icons, and nobody lists it because it isn't visible in a screenshot.
2. **The pointer.** The moment the cursor moves, the illusion breaks: it is the distro's arrow, not
   the macOS-shaped one, and the resize/I-beam/hand variants are wrong. rmac has no cursor theme.
3. **Things appear instead of arriving.** Windows pop into existence, menus blink on, minimize does
   nothing visual. On a Mac, everything *comes from somewhere*: a sheet slides out of the title bar,
   a window scales up from the Dock icon, Launchpad blooms out of its tile. Motion continuity is
   most of the "calm" that the owner says they want.
4. **Scrolling and the trackpad.** Without momentum, rubber-band overscroll and the right
   acceleration curve, every list feels stiff and cheap even when it is drawn perfectly.
5. **Text is nearly right but not quite.** The fonts now install correctly (task 0.6 is done), but
   Inter untuned reads wider and flatter than SF: wrong tracking at small sizes, proportional instead
   of tabular numerals in the clock, and menu titles a weight too heavy.
6. **The first foreign dialog.** Open a file in Text Editor, and a GNOME file chooser appears.
   Mount a disk and polkit asks for a password in a GTK box. One foreign dialog undoes ten
   beautiful surfaces. rmac has no portal backend or polkit agent of its own.
7. **Icons.** In macOS 27, app icons gained *more* Liquid Glass layers. Flat single-color SVGs in the
   Dock read as "Linux" instantly, even at small sizes.
8. **Wording.** "Are you sure you want to permanently erase the items in the Trash?" vs "Delete
   items?" — Apple's phrasing, Title Case in menus, sentence case in messages, the ellipsis rule, the
   button order (destructive on the right, Cancel left) are all part of the feeling.
9. **The desktop never rests.** A Mac at idle is *completely* still: no blinking, no spinner, no
   ticking clock seconds by default, no CPU. Any always-animating element reads as a Linux widget.
10. **Nothing remembers.** macOS reopens the windows you had, the folder you were in, the scroll
    position, the selection, the window size per app, even the tab. Without state restoration the
    desktop feels amnesiac.

**On the progress so far:** it is real and it is unusually disciplined — one token crate, a promoted
shell workspace, a design-token CI gate, minimize implemented against a compositor that has no
minimize, evidence recorded per task. Most projects like this are a theme and a screenshot. This one
has architecture. The gap is not effort or engineering; it is that everything above is unbuilt and
that the design system is still running on *my guessed numbers* because the reference captures
(task 1.1) were never taken.

**If you handed me rmac as-is:** I would say "this is the most serious macOS-like Linux desktop I've
seen" and then, within two minutes, "…but it isn't my Mac." Not because of a missing feature. Because
of silence, the pointer, the lack of motion continuity, foreign dialogs, and stiff scrolling.

**What I would expect that is not yet planned anywhere in the repo:**
sound set · cursor theme · portal backend (file chooser/print/permissions) · polkit agent ·
GTK/Qt/Electron theming so third-party apps look native · scrolling physics · state restoration ·
text-editing key bindings · drag-image rendering · spring-loaded folders · proxy icons ·
window resume after login · "Edited" title state · consistent wording · original wallpapers ·
login/boot choreography (the fade from GDM into the desktop) · the emoji picker · screenshot flow
polish · Dock genie · Mission Control choreography · rubber-band overscroll.

Every one of them is specified below.

---

## B. macOS 27 "Golden Gate" delta (the repo currently targets Tahoe 26.6)

The owner's Mac is the authority and it now runs 27.0. Update
`docs/macos-parity-spec.md` and `COMPLETION_SPEC.md` §4 with these changes, then
re-capture every reference screenshot.

| Area | macOS 26 (what the repo assumes) | macOS 27 Golden Gate (what to build) |
|---|---|---|
| Liquid Glass | fixed material set | **User-adjustable**: Settings ▸ Appearance ▸ *Liquid Glass* slider from "ultra-clear" to "fully tinted". rmac: one `glass_intensity` 0.0–1.0 token multiplier applied to every material's tint alpha; 0.0 = clear (alpha ×0.4), 1.0 = tinted (alpha ×1.6, capped opaque). Default mid. Reduce Transparency still forces opaque. |
| Contrast & shadows | — | Improved contrast and **window shadows**; active window must be clearly distinguishable. Raise `elev.window.active` and lower inactive further (see §D.8). |
| Window corners | dynamic radius depending on toolbar | **Standardized**: one corner radius for all system windows. Set `radius.window = 16` everywhere (verify by measurement) and delete the "16 for unified toolbar, 12 otherwise" rule from `COMPLETION_SPEC.md` §4.7 and the per-app-id niri rules. |
| Traffic lights | flat circles | **Liquid Glass look** + a **playful bounce when clicked and dragged before release**. Implement: press scales the button to 0.88 and, if the pointer drags while held, the button follows with a spring (stiffness 220, damping 18, max offset 3 px) and springs back on release. |
| Toolbars | per-app variation | **Standardized across apps**. One toolbar component, same height/insets everywhere (§5.14 of the build spec becomes mandatory for all six apps). |
| Sidebars | inset | **Edge-to-edge**: sidebars extend to the window edges; content area floats. Update Files/Settings/Notes layouts. |
| Menus | Tahoe added icons to menu items | **Icons removed** from most menus for a cleaner look. rmac: menus show **no icon column** by default; keep icons only where the item is an app/file (Open Recent, Dock windows list). Update build spec §5.9. |
| App icons | layered | **More Liquid Glass layers**, sharper edges, four appearances: light, dark, **clear**, **tinted**. rmac icons need all four variants + Settings ▸ Appearance ▸ "Icon & widget style". |
| Wallpapers | static | Golden Gate ships Sunset/Night wallpapers that **animate when the Mac unlocks**, usable as screen savers. rmac: original "Aurora" set with an unlock animation (§D.6). |
| Mission Control / Spaces | — | **Smoother animations**; scrolling improved system-wide. Treat Mission Control choreography as a first-class task (§D.3.7). |
| Spotlight | search | Now a **"Search or Ask"** bar with Siri AI built in. **rmac must not fake this.** Keep the bar visually current (single field, same geometry) but label it "Search", and never show an assistant, chat, or AI answer row. Optionally, after 1.0, a clearly-labelled local provider — never called Siri and never implied as built-in intelligence. |

Sources: [MacRumors — 50 new macOS Golden Gate features](https://www.macrumors.com/2026/09/17/new-features-changes-macos-27/), [9to5Mac — macOS 27 Golden Gate now available](https://9to5mac.com/2026/09/14/macos-27-golden-gate-now-available-here-is-everything-new/), [Macworld — macOS 27 Golden Gate](https://www.macworld.com/article/3139330/macos-27-mac-features-siri-apple-intelligence-release-date-compatibility.html).

---

## C. The owner's real Mac profile (measured 2026-09-18 — these are rmac's shipping defaults)

Read from the reference Mac with `defaults read`. Per the parity rule "the owner's Mac defines the
default profile", **change `ShellSettings::default()` to match this**, and record the date.

| Setting | Measured value | rmac default to ship |
|---|---|---|
| Appearance | `AppleInterfaceStyle = Dark` | **Dark** (not "Auto"): first login is dark |
| Accent / highlight | unset = **Multicolor** (blue) | accent blue, highlight accent |
| Dock tile size | `tilesize = 78` | **78** (not 56 as `COMPLETION_SPEC.md` FD-3 says — fix it; slider range 32–128) |
| Dock magnification | unset = **off**, `largesize = 128` | off; when enabled, magnified tile 128 |
| Dock autohide | `0` | off |
| Dock position | unset = bottom | bottom |
| Dock contents (in order) | **Apps, Notes, GitHub Desktop, Zed, Mail, System Settings, Terminal, Tapo, ChatGPT Classic** (Finder is implicit and always first; no browser pinned) | Files (implicit first), **Apps**, Notes, Text Editor, Terminal, System Settings — i.e. keep Files+Apps leading like the owner, drop Firefox from the default pins (the owner does not pin a browser) |
| Hot corners | bottom-right = `14` (**Quick Note**) | bottom-right = "New Note" (Notes); others unset |
| Scroll direction | `com.apple.swipescrolldirection = 0` → **natural scrolling OFF** | **off** — and `shell.kdl` currently sets `natural-scroll` for the touchpad: **remove it** |
| Language / region | `en-GB`, locale `en_GB@rg=inzzzz` (English UK, **India** region) | `en_GB` + India region: dd/mm/yyyy, 12-hour clock with am/pm, ₹, week starts Sunday-or-Monday per ICU, metric |
| Finder default view | `FXPreferredViewStyle = Nlsv` (**List view**) | Files opens in **List view** |
| Finder status bar | `ShowStatusBar = 1`, path bar unset (off) | status bar **on**, path bar off |
| Spelling autocorrect | `NSAutomaticSpellingCorrectionEnabled = 1` | on in text apps |
| Keyboard repeat | unset = system default | macOS defaults: initial delay 225 ms, repeat 90 ms → set libinput/xkb to match |

### C.2 Measured from the reference Mac's screen, 2026-09-18 (1920×1080 @ 1×, Dark, macOS 27.0)

Taken with `screencapture` of the live desktop and decoded pixel by pixel. **These override the
matching `S` guesses in `COMPLETION_SPEC.md` §4.7 — they are now `R` values.**

| Token | Measured | Was in the build spec | Notes |
|---|---|---|---|
| `menubar.height` | **29 px** (wallpaper occupies rows 0–28; a maximized window's top edge starts at row 29) | 26 | The menu bar is **fully transparent**: no tint row exists anywhere in the strip, wallpaper pixels run edge to edge behind the text. Confirms `material.menubar` alpha 0. |
| `dock.tile` | **64 px** (icon bounding box 62–64 px tall, y 994–1057) | 78 (pref) / 56 (old) | The `tilesize = 78` preference is the slider position; the rendered tile on this display is 64. Ship **64** as the default rendered size and keep the slider mapping so 78-on-the-slider renders 64. |
| `dock.pitch` | **≈76 px** centre to centre (Mail x0 769 → Phone x0 1380 over 8 tiles) | tile+8 | → **gap ≈ 12 px**, not 8. |
| `dock.shelf` | top ≈ y 990, bottom ≈ y 1062 → **height ≈ 72 px** | tile + 2×8 | → vertical padding ≈ 4–6 px, tighter than the spec assumed. |
| `dock.bottom.margin` | **≈ 18 px** between shelf bottom and screen edge | 6 | |
| `dock.indicator` | small bright dot ≈ 4 px, centred, **below the shelf glass** at y ≈ 1063–1067 | 4 px, 3 px below tile | Confirms size; the dot sits under the shelf, not under the icon inside it. |
| Dock badge | red circle, white numeral, icon's top-right corner, overlapping the icon edge | as specified | Seen on System Settings ("1"). |
| Dock separators | thin vertical lines at two places: before the unpinned/recent group and before Trash | as specified | Confirms the group model is right. |

**Do this with them:** update `crates/rmac-design` and §4.7, then re-run every Dock and menu bar
screenshot comparison. A 3 px menu bar error and a 12 px Dock pitch error are exactly the kind of
mismatch that reads as "close but not Mac" without anyone being able to say why.

**Update 2026-09-18, later the same day:** most of that capture set is now **done** — see
`docs/reference-captures-2026-09-18.md` and `target/evidence/reference-mac/`. Menu geometry
(269 px wide, 24 px item pitch), Spotlight (642 × 59 pill at 23.2 % height, with **inline
completion** rather than a result list), Control Center (287 px wide single column of mixed-size
modules), the save sheet, open panel, unsaved alert, Finder's four views and four Settings panes are
all captured and measured. Two layout assumptions in `COMPLETION_SPEC.md` were wrong and have been
corrected there: Spotlight's results card and Control Center's 2×2 grid. Also note the owner's
locale produces **"Bin", "Favourites", "Colour"** — rmac must follow the locale rather than hardcode
US spellings.

**Still to capture** (needs a human at the Mac, 10 minutes): an open menu with a submenu, Control
Center, Notification Center, Spotlight, a Finder window in each view, an alert, a sheet, the lock
screen. Lock screen cannot be screenshotted from inside the session — photograph it. Mission Control, the
Apps grid, a Dock context menu, magnification, and light-appearance versions of everything still
need a human at the Mac. Task 1.1 stays `[~]` until those land.

**Task C.1** — apply every row above to `crates/rmac-shell-settings/src/model.rs`,
`packaging/rmac-session/{shell.kdl,config.kdl}`, Files defaults, and the locale defaults, then record
the measurement date in `docs/macos-parity-spec.md`. Verify: fresh user account → first login shows a
dark desktop, a 78 px Dock with those items, list-view Files with a status bar, and non-natural
scrolling.

---

## D. The twelve feel systems (each is a build task)

### D.1 Sound — `crates/rmac-sound` (new)

macOS plays sound for: alerts, Trash, screenshot, volume change, drag-to-Dock, mount/unmount,
new mail (apps), screen lock, plug-in power, and boot. rmac plays none. **All sounds must be
original** — synthesize them, do not sample Apple's.

**Engine:** new GPUI-free crate `rmac-sound`: loads OGG/FLAC from `/usr/share/rmac/sounds/`, plays
through PipeWire on a dedicated low-latency stream with `media.role = Notification` (so it ducks
correctly and respects Do Not Disturb), max 8 concurrent, hard cap 1 play per 50 ms per cue, all
off the UI thread, latency from call to audible ≤ 30 ms.

**The sound set** (generate with the recipes; keep every generator script in `assets/sounds/gen/`
so the sounds stay reproducible and provably original):

| Cue | When | Character | Recipe (starting point) |
|---|---|---|---|
| `alert` | alert sheet appears, error | soft, neutral, not alarming | 2 sine partials 880 + 1320 Hz, 90 ms, exp decay τ=45 ms, −12 dBFS |
| `error` | operation refused (wrong password, disallowed drop) | two-note descending | 660 → 495 Hz, 70 ms each, 20 ms gap |
| `trash` | move to Trash | short paper-crumple noise burst | filtered white noise 2–6 kHz, 140 ms, fast attack, −18 dBFS |
| `empty-trash` | Empty Trash completes | longer crumple + low thud | above + 120 Hz sine 120 ms |
| `screenshot` | capture taken | camera-shutter click | 2 noise clicks 12 ms apart, band 1.5–8 kHz, 60 ms total |
| `volume-tick` | each volume key step (unless muted) | tiny pop | 1 kHz sine 18 ms, 8 ms attack, −20 dBFS |
| `mount` / `unmount` | volume appears / ejects | rising / falling two-tone | 523→784 Hz / 784→523 Hz, 110 ms |
| `power-plug` | AC connected | warm two-note rise | 392→587 Hz, 140 ms, soft attack |
| `lock` / `unlock` | session lock / unlock | short low click / soft rise | 200 Hz click 40 ms / 300→600 Hz 180 ms |
| `notification` | banner arrives (per-app setting) | gentle bell | 1046 + 1568 Hz, 220 ms, slow decay, −15 dBFS |
| `drag-drop` | item accepted by Dock/folder | soft tick | 1.4 kHz, 25 ms |
| `boot` | first frame of the desktop after login (setting, default **off**) | single warm chord | 220/330/440 Hz, 900 ms, slow attack 120 ms |

**Settings:** Sound pane gets *Alert sound* (list of the above alert candidates with preview),
*Alert volume*, *Play user interface sound effects* (default on), *Play feedback when volume is
changed* (default on), *Play sound on login* (default off).
**Verify:** every cue audible on the reference PC, none plays while muted or in Do Not Disturb
except `alert`; no cue plays twice for one action; idle sound process uses 0% CPU and holds no
PipeWire stream open when silent.

### D.2 Cursor — `assets/cursors/` + `rmac-cursor` theme

Build one original X11/Wayland cursor theme named `rmac` at sizes 24, 32, 48, 64 (hidpi via
`xcursorgen` from 2× SVG renders), installed to `/usr/share/icons/rmac/`, set as the default in
`packaging/rmac-session/config.kdl` (`cursor { xcursor-theme "rmac"; xcursor-size 24; }`) and in
`gtk-settings`/`XCURSOR_THEME` for third-party apps.

Shapes required (freedesktop names in brackets): arrow [`default`], I-beam [`text`], I-beam
horizontal [`vertical-text`], pointing hand [`pointer`], open hand [`grab`], closed hand
[`grabbing`], crosshair [`crosshair`], resize N/S/E/W/NE/NW/SE/SW [`ns-resize` etc.], resize row/col
[`row-resize`,`col-resize`], busy spinner [`wait`, animated 12 frames at 60 ms], arrow+spinner
[`progress`], not-allowed [`not-allowed`], copy [`copy`], alias [`alias`], move [`move`], zoom in/out
[`zoom-in`,`zoom-out`], help [`help`], context-menu [`context-menu`], all-scroll [`all-scroll`].

Design rules: arrow 12×19 logical, black fill with 1 px white outline and a 1 px `00000040` contact
shadow offset 1,1; hotspots exactly at the visual tip; I-beam has the small horizontal serifs and a
white halo so it stays visible over text; busy is a smooth rotating segmented ring, not a beach ball.

**Also:** pointer must change on hover for every interactive region (text fields → `text`, links →
`pointer`, window edges → resize, Dock separator → `col-resize`, Files column divider →
`col-resize`). Add a hover-cursor test to the component gallery.

### D.3 Motion continuity — everything comes from somewhere

Rule: **no element may simply appear.** Each transition names an origin, a curve, and a duration.
All use the tokens from `COMPLETION_SPEC.md` §4.9; add these specific choreographies:

1. **App launch:** Dock tile bounces (existing spec) **and** the new window scales up from the tile's
   screen position: from 8% size at the tile centre to full rect, 320 ms, ease-out, opacity 0→1 in the
   first 120 ms. Implement with a shell-owned overlay surface drawn during the map, then hand off to
   the real window (niri has no window-open animation API rmac can shape directly; if the overlay
   proves janky, use niri's own `window-open` animation configured in `shell.kdl` with a matching
   curve and drop the overlay).
2. **App quit / window close:** reverse, 200 ms, ease-in; the Dock dot fades out after the window is gone.
3. **Minimize:** Scale effect — the window rect interpolates to the Dock tile rect with a subtle
   perspective shear, 350 ms, ease-in-out, plus the window's own thumbnail fading to 0.6 opacity.
   Genie stays a Phase 10 item.
4. **Sheets:** slide down from directly under the title bar, 250 ms ease-out with a 6% overshoot;
   the parent window dims to 88% brightness at the same time; closing reverses at 200 ms.
5. **Menus:** open instantly (0 ms) — macOS does not animate menu open; close with the select-blink
   then 150 ms fade. Getting this *wrong way round* (fade in, snap out) is a classic tell.
6. **Spotlight:** field fades in over 140 ms while dropping 4 px; the results card expands from
   height 0 with the field as anchor, 180 ms, ease-out; each subsequent query re-lays out with a
   120 ms height animation, never a jump.
7. **Mission Control:** all windows scale and move to their overview slots simultaneously, 350 ms,
   ease-in-out, with the Spaces strip sliding down from the top edge 250 ms; exiting reverses and
   the chosen window ends exactly at its real rect. macOS 27 made this smoother — budget ≥ 99% of
   frames ≤ 16.67 ms during it; if niri's overview cannot hit that, tune niri's animation block in
   `shell.kdl` rather than adding a second animation on top.
8. **Apps (Launchpad):** blooms from the Dock's Apps tile: background blur ramps 0→full over 200 ms
   while the grid scales 0.92→1 with a 250 ms spring; closing reverses into the tile.
9. **Space switch:** horizontal slide of the whole desktop, 320 ms, ease-in-out; the wallpaper moves
   with it (macOS moves wallpaper too when wallpapers differ per Space).
10. **Full screen:** window expands to fill while the menu bar and Dock slide away, 400 ms
    ease-in-out — one coordinated motion, not three independent ones.
11. **Control Center / Notification Center:** scale 0.96→1 + fade anchored at the clicked menu bar
    item (transform origin = the item's centre), 250 ms.
12. **Banners:** slide in from the right edge (+24 px) with fade, 350 ms spring; stacking cards shift
    down 200 ms; dismiss slides right and fades 200 ms.
13. **Wallpaper on unlock (macOS 27 behavior):** the wallpaper animates on unlock — for rmac's
    original "Aurora" set, run a 1.2 s gentle gradient drift once, then rest.
14. **Login:** GDM fades to black 150 ms → wallpaper fades in 400 ms → menu bar and Dock slide in
    from their edges 300 ms with 80 ms stagger. **Never** show a bare compositor background, a
    half-drawn bar, or windows arriving one by one.

**Verify each:** record at 60 fps with `wf-recorder`, step through frames, confirm origin, duration
(±30 ms) and that the element never jumps. Under Reduce Motion every one of these becomes a ≤ 100 ms
opacity fade with no movement.

### D.4 Scrolling and input physics

- **Momentum scrolling** for every rmac list/grid/text view: on touchpad flick, velocity decays
  exponentially (τ = 325 ms), stops below 0.5 px/frame. GPUI gives raw axis events; implement one
  shared `rmac_ui::scroll::Momentum` used by every scrollable (Files, Settings, Notes, NC, Apps).
- **Rubber-band overscroll:** past the edge, offset = `limit × (1 − 1/(1 + |overscroll|/limit))` with
  `limit = 0.25 × viewport`; release springs back (stiffness 200, damping 26, ~350 ms). This single
  behavior contributes more "Mac feel" per line of code than any visual polish.
- **Scroll direction:** the owner's Mac has natural scrolling **off** (§C) — remove `natural-scroll`
  from `shell.kdl` and expose it in Trackpad settings.
- **Pointer acceleration:** libinput's default profile feels wrong to Mac users. Set
  `accel-profile "adaptive"` with `accel-speed 0.3` for touchpads and expose the Tracking speed
  slider mapped to −0.6…+0.9. Document the mapping in `docs/input.md`.
- **Two-finger scroll** must be pixel-precise (no line stepping), and **scrollbars** follow the
  macOS rule: hidden while not scrolling, overlay style, 7 px wide, expanding to 11 px with a
  visible track on hover, `00000059` thumb light / `FFFFFF59` dark, radius 3.5, fade out 700 ms
  after scrolling stops.
- **Key repeat** 225 ms initial / 90 ms interval; press-and-hold on a letter key does *not* open an
  accent popup (that is an Apple behavior most Linux users find odd — keep repeat).
- **Gestures** (libinput): 3-finger swipe left/right = Space switch (follows the finger, not a
  discrete jump — implement as niri gesture pass-through), 3-finger up = Mission Control, 3-finger
  down = App Exposé, 4-finger pinch in = Apps, 4-finger spread = Show Desktop, 2-finger swipe from
  right edge = Notification Center. Each must track the finger continuously and complete or cancel
  on release based on a 40% threshold.

### D.5 Typography that reads as SF

Inter is the right licensed choice, but it must be tuned or it reads "Linux-y":

- OpenType features: turn on `tnum` (tabular numerals) everywhere numbers change in place — menu bar
  clock, Dock badges, System Monitor tables, Settings values, timers. Turn on Inter's `cv08` (upright
  6 and 9) and `ss03` (rounder `@`), which read closer to SF. Turn **off** ligatures in Terminal.
  Check each feature tag against the shipped Inter version with `hb-shape` before enabling it, and
  keep only the ones that visibly help.
- Tracking table (letter-spacing) — apply per role, not globally: 10 px → +0.12 px; 11 px → +0.10;
  12 px → +0.06; 13 px (body/menus) → +0.02; 15 px → 0; 17 px → −0.10; 22 px → −0.25; 26 px → −0.35;
  96 px (lock clock) → −1.5.
- Weights: menu bar app name and dialog titles **Semibold (600)**, not Bold (700) — Apple's "bold"
  in the menu bar is optically closer to 600 at these sizes.
- Rendering: grayscale antialiasing (no subpixel RGB), hinting **off** (`hintnone`), gamma-correct
  blending. Ship `99-rmac.conf` (installed to `/etc/fonts/conf.d/`, numbered 99 so it is processed
  after Ubuntu's `6x`/`99-language-selector` generic-family rules, which otherwise override the
  aliases) setting exactly that for Inter and JetBrains Mono, and make `Inter` the fontconfig alias
  target for `-apple-system`, `system-ui`, `Helvetica Neue`, `SF Pro Text`, `SF Pro Display` so
  third-party apps and websites fall in line.
- Line heights: body 1.23×, list rows use fixed row heights (never font-driven), paragraph text in
  Notes/Text Editor 1.45×.
- **Verify:** render the same sentence in rmac and on the Mac at 13 px, overlay the two screenshots
  at 400% — stem weights and word widths should differ by < 3%.

### D.6 Icons, wallpapers, and the desktop's first impression

- **Icons (macOS 27 rule):** every first-party icon is built from ≥ 4 layers — background gradient
  plate, inner shadow, glyph, specular highlight — rendered to light / dark / **clear** / **tinted**
  variants. Provide a single `assets/icons/<app>.icon.toml` describing layers so all four variants
  are generated by one script (`scripts/build-icons.py`) into 16–1024 px PNGs. Gradient plates use
  two-stop diagonal gradients with a 6% inner top highlight and a 1 px `00000026` inner border.
- **Third-party icons:** the squircle plate rule from the build spec, plus: never stretch a
  non-square icon; centre it at 76% and let the plate carry the shape.
- **Wallpapers:** ship an original set of 6: "Aurora Dawn", "Aurora Night" (dynamic pair that follows
  light/dark), "Tide", "Basalt", "Monsoon", "Paper" — generated procedurally by
  `rmac-wallpaper-image` at the output's exact resolution so there is never a scaling blur. The
  dynamic pair cross-fades over 2 s when the appearance changes, and plays the 1.2 s unlock drift
  (§D.3.13). Default = "Aurora Night" (owner is in Dark).
- **The desktop at rest** must contain only: wallpaper, menu bar, Dock. No icons, no widgets, no
  panel, no niri artifacts. Take a first-boot screenshot and compare with the Mac's — this single
  image decides the sponsor demo's first impression.

### D.7 No foreign surfaces (the illusion-breakers)

This is the biggest missing block in the current plan.

1. **`xdg-desktop-portal-rmac`** (new crate `crates/rmac-portal-backend` + bin): implement the
   `org.freedesktop.impl.portal.*` backends for **FileChooser** (Open/Save/Directory), **Print**,
   **Screenshot/ScreenCast** (delegate to the Phase 5 overlay), **Settings** (already partly there),
   **Wallpaper**, **Secret**, **Inhibit**, **Access** (permission dialogs), **Notification**,
   **GlobalShortcuts**, **Lockdown**. Register with
   `UseIn=rmac` and install `/usr/share/xdg-desktop-portal/rmac-portals.conf` naming rmac's backend
   first, GNOME's only for what rmac does not implement.
   The **Open/Save panel** is the highest-value piece: sidebar (Favorites/Locations/Tags), the same
   four views as Files, the Go-to-Folder sheet (⇧⌘G), New Folder button, Tags field, file-type
   pop-up, "Hide extension" checkbox, keyboard behavior identical to Files. Every GTK, Qt, Electron,
   Flatpak and Snap app then gets a Mac-shaped file dialog.
2. **Polkit agent** (`rmac-polkit-agent`, layer-shell): the alert design of §5.11 with the padlock
   glyph, "rmac wants to make changes." / "Enter your password to allow this.", user avatar, password
   field, Cancel / **OK**, wrong-password shake. Register as the session's polkit agent in the
   session units so no GTK authentication dialog can ever appear.
3. **GTK 3/4 + libadwaita theming** via the existing `rmac-gtk-settings`: ship an rmac GTK theme
   (colors from `rmac-design`, 8 px control radius, Inter as the UI font, macOS-style header bars
   with traffic lights on the left and 52 px height), plus `gtk-decoration-layout=close,minimize,maximize:`
   so GTK apps put their buttons on the **left** in the right order.
4. **Qt** via `qt6ct`/`QT_QPA_PLATFORMTHEME=gtk3` and a matching palette; **Electron** via
   `--enable-features=WaylandWindowDecorations` and a documented `~/.config/electron-flags.conf`.
5. **Firefox**: ship `/usr/lib/firefox/browser/defaults/preferences/rmac.js` setting
   `widget.gtk.non-native-titlebar-buttons`-equivalent prefs so tab bar + traffic lights match, plus
   `ui.prefersReducedMotion` following rmac, and Inter as the default sans.
6. **App-not-responding**: when a window stops responding to ping for 5 s, the pointer becomes
   `wait` over that window and its title gets " (Not Responding)"; Force Quit lists it in red.
7. **XWayland apps** must be scaled crisply (niri `xwayland-satellite` with integer scale) — a blurry
   legacy app ruins the impression; document the limit honestly if it cannot be fixed.

**Verify:** open a file from GIMP (GTK), VLC (Qt), VS Code (Electron) and Firefox; every dialog is
the rmac panel; mount an internal disk → rmac polkit dialog; no GNOME dialog appears anywhere.

### D.8 Window feel

- **Drag latency:** a dragged window must track the pointer with ≤ 1 frame lag. Measure with a
  high-speed capture: pointer-to-window offset drift < 4 px at 800 px/s.
- **Magnetic edges:** while dragging, snap to screen edges and to other windows' edges within 8 px
  (disable with ⌘ held).
- **Shadows** per macOS 27's "more distinct active windows": active `0 12 32 00000059`, inactive
  `0 4 12 00000033`, and a 0.5 px `FFFFFF1A` top inner edge on dark windows.
- **Resume:** after login, reopen the apps and windows that were open at logout (if "Reopen windows
  when logging back in" was checked) with their exact rects, and restore each app's last document,
  selection, scroll offset and sidebar width. This is `rmac-window-state` plus a per-app
  `restore_state()` contract — make it part of the app contract in `COMPLETION_SPEC.md` §10.
- **Zoom (green) must be smart:** first click fills the screen keeping the current Space; second
  click returns to the previous rect — never a jump to full screen without memory.
- **Window titles:** proxy icon + name; " — Edited" while dirty; ⌘-click the title shows the path
  menu; drag the proxy icon to a Terminal window pastes the path.

### D.9 Text editing and keyboard behaviors (everywhere text can be typed)

Implement once in `rmac-editor`/`rmac-ui` and use in every field, including Spotlight and password
fields where safe:

⌥← / ⌥→ word jump · ⌘← / ⌘→ line start/end · ⌘↑ / ⌘↓ document start/end · ⌥⌫ delete word ·
⌘⌫ delete to line start · ⌃A / ⌃E / ⌃K / ⌃D / ⌃T / ⌃Y (emacs bindings macOS keeps) ·
⇧ + any of the above extends selection · double-click selects word, triple-click selects paragraph ·
double-click-drag extends by word · drag selected text to move it, ⌥-drag to copy ·
⌘Z/⇧⌘Z undo with coalescing by typing burst (500 ms) · ⌃⌘Space emoji & symbols picker (build a real
one: search field, categories, recents, skin tone, 8 columns, Return inserts) · Smart quotes/dashes/
links substitutions with per-app toggles · spell check underline (hunspell if installed) with
right-click suggestions · autoscroll when dragging a selection past the edge · caret blink 530 ms
on / 530 ms off, stops while typing.

### D.10 Words: how rmac speaks

| Rule | Right | Wrong |
|---|---|---|
| Menu items | Title Case: "Move to Trash", "Get Info" | "Move to trash" |
| Buttons | Title Case verbs: "Empty Trash", "Don't Save" | "OK" for a destructive action |
| Messages | Sentence case, full sentences, end with a period | "Error: failed" |
| Destructive confirm | "Are you sure you want to permanently erase the items in the Trash?" + "You can't undo this action." | "Delete files?" |
| Ellipsis | Only when more input follows: "Save As…", "Connect to Server…" | "Get Info…" |
| Button order | [Cancel] [**Primary**] right-aligned; destructive primary is red-tinted text, never a red filled default | Primary on the left |
| Errors | What happened + why + what to do: "The disk “Backup” wasn't ejected because one or more programs may be using it." | "Operation failed (EBUSY)" |
| Never show | errno, D-Bus names, paths in user messages, stack traces, "Linux", "niri", "Wayland", "systemd" in ordinary UI | — |
| Quotes | curly “ ” around names | "straight" |
| Units | 1 KB = 1000 bytes; show "12.4 MB", "1.2 GB"; storage uses the same base as the Mac | KiB |

**Task:** one pass over every user-visible string in the repo (`rg '"[A-Z][a-z].*"' crates/*/src`)
against this table; add the rules to `CONTRIBUTING.md`; add a test that fails on the words
"Error:", "Failed to", "Warning:", and on any message containing "/home/" or "dbus".

### D.11 Micro-interactions that people feel but can't name

- Hover: rows highlight after **0 ms**, tooltips after 700 ms first time then 0 ms while moving
  between neighbors; hover highlight fades out over 120 ms, never in.
- Press: every control darkens on mouse-down and commits on mouse-**up inside** — dragging out and
  releasing cancels (macOS rule); re-entering while held re-arms it.
- Drag image: dragging files shows a 50%-opacity stack of the icons with a blue count badge; the
  origin item dims to 40%; illegal targets show `not-allowed` and no highlight.
- Spring-loaded folders: hovering a folder for 600 ms while dragging opens it (with a subtle "pop"),
  and dragging back out returns; ⌘ cancels the spring.
- Selection: rubber-band rectangle has a 1 px accent border and 10% accent fill; selecting with
  ⌘ toggles, ⇧ extends, and the *anchor* never moves — a common bug that feels wrong immediately.
- Focus ring: 3 px accent at 60% opacity, animated in over 100 ms, only for keyboard focus, never
  for mouse clicks.
- Menu bar clock never blinks its separator unless the setting is on; seconds off by default.
- Progress: indeterminate spinners only after 500 ms of waiting (below that, show nothing — the
  operation finishes and the flash of a spinner feels slower than no spinner).
- Empty Trash, eject, and delete confirmations remember "don't ask again" where macOS does.
- Badge counts animate with a 150 ms scale 1.3→1 pop when they increase, nothing when they decrease.

### D.12 Idle silence and latency

| Thing | Budget |
|---|---|
| Key/pointer → first visible change | ≤ 50 ms p95 (menus ≤ 30 ms) |
| Shortcut → Spotlight visible | ≤ 80 ms p95 |
| Click → Control Center visible | ≤ 100 ms p95 |
| Dock hover → magnification frame | next frame |
| App icon click → bounce starts | ≤ 16 ms |
| Whole shell idle CPU (all processes) | ≤ 0.3% on the reference PC |
| Wakeups at idle | ≤ 3/s total for the shell |
| Frames during any animation listed in D.3 | ≥ 99% within the refresh interval |

A single background timer that ticks every second anywhere in the shell is a bug. Clock uses one
timer aligned to the next minute; battery/network/audio are event-driven; the Dock redraws only on
pointer/state change.

---

## E. The first ten minutes (the acceptance script that decides "does it feel like a Mac")

Run this exactly, on the reference PC, with a stopwatch and a screen recorder. Every line is a
pass/fail. This is the demo the owner will show, and the review a Mac user performs without thinking.

| # | Action | What must happen (all of it) |
|---|---|---|
| 1 | Power on | GDM appears with the rmac background; "rmac" is preselected |
| 2 | Type password, Return | GDM fades to black (150 ms) → wallpaper fades in → menu bar drops in, Dock rises, 80 ms apart; desktop is complete and still within 3 s; no flashes, no bare compositor, no window arriving late |
| 3 | Move the pointer | rmac arrow, correct shape; over the Dock the tooltip appears after the first hover; over the menu bar nothing highlights until hover |
| 4 | Click the rmac mark | System menu opens instantly, no fade; arrow-key through it; Escape closes |
| 5 | Press ⌘Space, type "term" | Search bar appears in under 80 ms; "Terminal" is the top hit before you finish typing; Return launches |
| 6 | Watch the launch | Dock tile bounces; window scales up from the tile; Terminal shows a prompt in under 900 ms; the bounce stops the moment the window appears |
| 7 | Type `ls`, drag-select output, ⌘C | Selection is accent-colored; copy makes no sound; no flicker |
| 8 | ⌘N twice, then ⌘` | Two more windows; ⌘` cycles within Terminal only; Dock never reorders |
| 9 | Click the green button | Full screen expands in one coordinated 400 ms motion; menu bar and Dock slide away; pointer to the top edge reveals the bar over the window |
| 10 | Escape full screen, click yellow | Window scales into its Dock tile in 350 ms; a minimized tile appears right of the separator |
| 11 | Click that tile | Window returns to its exact previous rect |
| 12 | Open Files, press ⌘2/⌘3/⌘1 | List/Columns/Icons views switch instantly; list view is the default (owner's Mac); status bar shows "12 items, 180 GB available" |
| 13 | Scroll a long folder with the trackpad | Momentum carries after the fingers lift; hitting the end rubber-bands and springs back; scrollbar appears then fades after 700 ms |
| 14 | Select 3 files, drag them | A 50%-opacity stack with a blue "3" badge follows the pointer; the originals dim; hovering a folder for 600 ms springs it open |
| 15 | Drop them on Trash in the Dock | Crumple sound; Trash icon becomes full; Files shows no dialog |
| 16 | ⌘Z | The move is undone, the Trash empties, the files return selected |
| 17 | Right-click a file → Get Info | Info window with the right sections; ⌘W closes it |
| 18 | Press Space on an image | Quick Look opens over everything, arrow keys move between files, Space closes |
| 19 | Open Text Editor → ⌘O | **The rmac open panel appears** — not a GNOME dialog — with the same sidebar and views as Files |
| 20 | Type text, ⌥← ⌥→ ⌘← ⌃A | Word/line motion behaves exactly like macOS |
| 21 | ⌘W without saving | A **sheet** slides from the title bar: "Do you want to save the changes made to the document “Untitled”?" [Delete] [Cancel] [**Save**] |
| 22 | Press the volume keys | OSD appears under the menu bar, ticks on each step, merges repeats, fades after 1.6 s |
| 23 | Click Control Center | Panel scales in from the icon; Wi-Fi circle toggles instantly and reflects real state; clicking the Wi-Fi label expands the network list in place |
| 24 | Run `notify-send "Build finished" "3 warnings"` | A banner slides in from the right with a gentle sound, auto-dismisses after 5 s |
| 25 | Click the clock | Notification Center slides in with that notification grouped; clicking the clock again closes it |
| 26 | Press ⌃↑ | Mission Control: every window flies to its slot in one smooth 350 ms motion; Spaces strip drops from the top |
| 27 | Three-finger swipe left/right | Spaces follow the fingers continuously and settle |
| 28 | ⌘Tab, hold | App switcher panel; Tab advances; release activates; the Dock does not reorder |
| 29 | Open System Settings → Appearance | Switch Light/Dark: everything — shell, apps, wallpaper pair, cursor theme contrast — changes within one frame, no relaunch |
| 30 | Mount a USB stick | It appears in the Files sidebar and on the Dock's right group with a mount sound; ejecting plays the unmount sound |
| 31 | An operation needing admin (e.g. add a printer) | The **rmac** authentication panel appears, not a GTK polkit dialog |
| 32 | ⌃⌘Q | Lock screen with blurred wallpaper, big clock, avatar, password field; a wrong password shakes; the right one returns you to the exact desktop |
| 33 | Close the lid, open it | One password prompt, same surface, no second lock, no flicker |
| 34 | Leave the machine idle for 2 minutes | Absolute stillness: no animation, no blinking, fan silent, `top` shows the shell under 0.3% CPU |
| 35 | Log out and back in | The apps and windows you left open come back where they were |

**Scoring:** any failed line means "not yet". The owner should run this list every two weeks during
the build; it takes 10 minutes and is worth more than any code review.

---

## F. The vibe rubric (score honestly, 0–2 each; ship at 58+/60)

Visual: 1 wallpaper quality · 2 icon quality (four appearances) · 3 menu bar proportions ·
4 Dock proportions and glass · 5 window corners and shadows · 6 typography weight/tracking ·
7 color accuracy in both themes · 8 material/blur layering · 9 no Linux artifact anywhere ·
10 consistent empty and error states.

Motion: 11 every surface arrives from its origin · 12 minimize/restore · 13 Mission Control ·
14 Spotlight/Control Center open · 15 banner in/out · 16 login choreography · 17 frame pacing ·
18 reduce-motion fallbacks.

Feel: 19 pointer shapes · 20 scrolling momentum and rubber-band · 21 gesture tracking ·
22 drag-and-drop with ghosts and springs · 23 sound set · 24 keyboard/text bindings ·
25 latency budget met · 26 state restoration · 27 wording and casing · 28 no foreign dialogs ·
29 idle silence · 30 nothing fake anywhere.

---

## G. Where this work goes in the plan (insert into `COMPLETION_SPEC.md`)

Add these as numbered tasks; the phase order stays the same.

| New task | Phase | Why there |
|---|---|---|
| **C.1 owner-profile defaults** (§C) | 0, right after 0.7 | Cheap, and every screenshot afterwards is judged against it |
| **macOS 27 delta + rename parity doc** (§B) | 0, new task 0.9 | The whole spec's reference target must be current before measuring |
| **Cursor theme** (§D.2) | 1, new task 1.10 | Visible in every screenshot from then on |
| **Typography tuning + fontconfig** (§D.5) | 1, folded into 0.6/1.1 | Affects every measurement |
| **Icons four appearances + wallpapers** (§D.6) | 1, extends 1.8 | Same |
| **Scrolling physics + input** (§D.4) | 1, new task 1.11 | Shared component; everything scrolls |
| **Text editing bindings + emoji picker** (§D.9) | 1, new task 1.12 | Shared before apps use it |
| **Login/boot choreography + window animations** (§D.3.1–3, 14) | 2, new tasks 2.10–2.11 | Needs the session and compositor actions |
| **Window feel: drag, snap, shadows, resume** (§D.8) | 2, new task 2.12 | Same |
| **Sound engine + set** (§D.1) | 3, new task 3.11 | First surfaces that need cues exist by then |
| **Overlay choreography** (§D.3.6–12) | 5, folded into each overlay task | Build the animation with the surface |
| **Portal backend + open/save panel** (§D.7.1) | 5, new task 5.10 | It is a shell surface, and Files' views are ready |
| **Polkit agent** (§D.7.2) | 6, new task 6.6 | Sits with the authentication work |
| **GTK/Qt/Electron/Firefox theming** (§D.7.3–5) | 7, new task 7.7 | After rmac's own look is settled |
| **Wording pass + string test** (§D.10) | 7, new task 7.8 | After all strings exist |
| **Micro-interactions audit** (§D.11) | 8, new task 8.9 | Polish pass with everything present |
| **First-ten-minutes script + rubric** (§E, §F) | 8, new task 8.10, then repeat before every release | The acceptance gate |

**If you can only do ten things,** do them in this order — they buy the most "Mac" per hour:

1. Fonts actually installed and tuned (§D.5) — everything is made of text.
2. Cursor theme (§D.2).
3. Owner-profile defaults (§C) — dark, 78 px Dock, list view, no natural scrolling.
4. Scrolling momentum + rubber-band (§D.4).
5. Login choreography and window open/close/minimize animations (§D.3).
6. The sound set (§D.1).
7. The rmac open/save panel and polkit agent (§D.7.1–2).
8. Icons with four appearances + the wallpaper set (§D.6).
9. Wording and button-order pass (§D.10).
10. Idle silence and the latency budget (§D.12).

---

*This file is a companion, not a replacement. `COMPLETION_SPEC.md` remains the task list; every task
here is inserted into it per §G.*
