# rmac agent playbook — the twelve tasks that create the Mac feeling

> **Read this before `COMPLETION_SPEC.md`.** That file is the full backlog. This file is the order
> the work must happen in, and the rules that stop the result from feeling like a Linux theme.
> **Written 2026-09-19** against commit `8b00902` (1309 commits; Phase 0 done, Phases 1–4 partly
> done, cursor theme generated, window easing landed).

## Why it still doesn't feel like a Mac — the diagnosis

Three causes, in order of damage:

1. **Nobody has ever looked at rmac.** On 2026-09-19 the repository contained **24 screenshots of
   macOS and zero screenshots of rmac**. Tasks were ticked from log lines, unit tests and prose.
   A visual product built without looking at pixels cannot converge. **Task 1 fixes this and
   everything else depends on it.**
2. **The colours were invented.** `rmac-design` shipped my from-memory guesses. The real accent is
   `#1372F9`, window surfaces are **wallpaper-tinted**, and light/dark materials have very different
   opacity. See `docs/measured-colors-2026-09-19.md`.
3. **The feel systems don't exist yet.** No sound, no scrolling physics, no motion continuity, no
   rmac file picker, no polkit dialog. These are listed in `FEEL_SPEC.md` but sit behind Phases 5–7,
   so they keep being postponed while Dock details get polished. **They move ahead of everything.**

## Rules (they override your habits)

1. **No tick without a picture.** A task is done when `target/evidence/rmac-<date>/pairs/<name>-vs-mac.png`
   exists and you have written what still differs. Text-only "verified" lines are not evidence.
2. **No invented numbers.** Every colour, size, radius, duration comes from
   `docs/measured-colors-2026-09-19.md`, `docs/reference-captures-2026-09-18.md`, or a new
   measurement made with `scripts/measure-reference.py`. If you cannot measure it, write `S` beside
   it and say so in the commit message.
3. **One task per commit**, message = the user-visible outcome.
4. **Run on the reference PC before ticking.** Compiling is not running.
5. **When a capture disagrees with a spec, the capture wins** — fix the spec in the same commit.
6. **Never fake.** No placeholder data, no control that only changes local state.
7. **Stop and ask the owner only** for the four reasons in `COMPLETION_SPEC.md` §0.6.

---

## Task 1 — Build the visual feedback loop (do this first, today)

**Why:** you cannot fix what you never see. Everything below is judged through this loop.

**Do:**
```sh
# on the Ubuntu reference PC, inside the rmac session
bash scripts/linux/capture-shell-evidence.sh
```
It captures desktop, menu open, Search, Control Center, banner, Notification Center, Apps, Files in
four views, OSD, Mission Control and the Dock menu, then pairs each with the macOS capture in
`target/evidence/reference-mac/`.

**Then**, for every pair, write one line per visible difference into
`docs/visual-gap-2026-09-19.md`, in this exact shape:

```
menu-open: panel 12 px too wide (281 vs 269) · rows 28 px vs 24 · separator too bright
           (#55595F vs #3E424B) · no select-blink on activation
```

**Done when:** `docs/visual-gap-2026-09-19.md` lists every surface with at least three concrete,
numeric differences, and the pair images exist. Commit both.

---

## Task 2 — Put the measured colours into `rmac-design`

**Read:** `docs/measured-colors-2026-09-19.md`.

**Do:**
- `accent` = `#1372F9` (dark). Replace `0A84FF`/`007AFF` everywhere.
- Opaque surfaces from the table: content `222025`, sidebar `29252E`, grouped background `222026`,
  grouped row `29272D`, sheet `262227`, field fill `181818`, status bar `29272C`,
  secondary button `363237`, destructive button `812E25`.
- Menu-bar text: `FFFFFF` on dark wallpapers, `010206` on light — full opacity, not 85%.
- Split every material alpha into **light** and **dark** values; light materials are near-opaque
  (Spotlight light measures `FCFCFC`), dark ones are genuinely translucent.

**Verify:** `cargo test -p rmac-design`; re-run Task 1's capture; the colour differences in
`visual-gap` must shrink to zero for flat surfaces.

**Done when:** no surface colour in the gap list differs by more than 2 per channel.

---

## Task 3 — Wallpaper tinting (the invisible one that matters most)

macOS mixes the wallpaper's hue into window surfaces. rmac paints flat grey, which reads as dead
against a colourful wallpaper.

**Do:**
- The wallpaper service already computes a per-output luminance for the menu bar. Extend it to
  publish a **dominant hue** (average of the image downsampled to 8×8, in sRGB) on the same channel.
- `rmac-design` gains `tint(base: Rgba, tint: Rgba, strength: f32) -> Rgba` (linear mix in sRGB).
- Apply strengths: content **0.06**, sidebar **0.10**, grouped rows **0.07**, sheets **0.05**,
  toolbars **0.08**. Materials (menus, Control Center) already sample the wallpaper through blur —
  do **not** tint them twice.
- Setting: Appearance ▸ "Allow wallpaper tinting in windows", default **on**.

**Verify:** set a strongly coloured wallpaper; Finder's sidebar must visibly take its hue; set a grey
wallpaper; surfaces must go neutral. Capture both.

---

## Task 4 — Make text look like macOS text

**Do:**
- Install `packaging/fontconfig/60-rmac.conf` to `/etc/fonts/conf.d/` from the `rmac-session`
  package (add it to `scripts/linux/native_package_contract.py`).
- In `rmac-design`, apply the tracking table from `FEEL_SPEC.md` §D.5 per text role, and enable
  `tnum` for the clock, Dock badges, System Monitor tables and every Settings value.
- Menu-bar app name and dialog titles: **Semibold (600)**, not Bold.

**Verify:** `fc-match Inter` resolves; capture the menu bar and a Settings pane; overlay with the
macOS capture at 400% — stem weights and word widths within 3%.

---

## Task 5 — Sound (the fastest large win: half a day)

The sounds already exist: `assets/sounds/*.wav`, generated by
`assets/sounds/gen/make_sounds.py` (14 original cues, no Apple samples).

**Do:**
1. New GPUI-free crate `crates/rmac-sound`: `play(cue: Cue)`, loads from
   `/usr/share/rmac/sounds/`, one PipeWire stream with `media.role = Notification`, work off the UI
   thread, rate-limited to one play per cue per 50 ms, silent when muted or in Do Not Disturb
   (except `alert`).
2. Package `assets/sounds/*.wav` → `/usr/share/rmac/sounds/`.
3. Wire the first six cues: `alert` (alert sheets), `trash` (move to Trash), `empty-trash`,
   `screenshot`, `volume-tick` (volume keys), `notification` (banner arrival).
4. Then `mount`/`unmount`, `lock`/`unlock`, `power-plug`, `drag-drop`, `error`.
5. Settings ▸ Sound: Alert sound picker with preview, Alert volume, "Play user interface sound
   effects" (on), "Play feedback when volume is changed" (on), "Play sound on startup" (off).

**Verify:** each action produces exactly one sound, none while muted, no audible click at the tail,
`pw-top` shows no stream held open while idle.

---

## Task 6 — Cursor

`scripts/build-cursors.py` and `assets/cursors/rmac/` already exist. Finish the job:

- Install the theme to `/usr/share/icons/rmac/` from the package; set
  `cursor { xcursor-theme "rmac"; xcursor-size 24; }` in `packaging/rmac-session/config.kdl`; export
  `XCURSOR_THEME=rmac` and set the matching GTK/Qt settings so third-party apps use it too.
- Make every interactive region set its pointer: text fields → `text`, links → `pointer`, window
  edges → the eight resize shapes, column dividers → `col-resize`, drag in progress → `grabbing`,
  disallowed drop → `not-allowed`, busy → `wait`.

**Verify:** screenshot the pointer over each region (grim captures the cursor with `-c`); compare
shapes with the macOS captures.

---

## Task 7 — Motion continuity

**Read:** `FEEL_SPEC.md` §D.3 (fourteen choreographies with origins, curves, durations).

**Order:** login sequence → window open/close → minimize → menus (instant open, blink + fade on
select) → Search → Control Center / Notification Center → banners → Mission Control → Apps.

**Verify each:** `wf-recorder -f out.mp4`, step frame by frame, confirm the origin, the duration
±30 ms, and that nothing jumps. Reduce Motion turns each into a ≤ 100 ms fade.

---

## Task 8 — Scrolling physics

**Read:** `FEEL_SPEC.md` §D.4. One shared `rmac_ui::scroll::Momentum`, used by every scrollable:
exponential decay τ = 325 ms, rubber-band `limit × (1 − 1/(1 + |over|/limit))` with
`limit = 0.25 × viewport`, spring back over ~350 ms. Overlay scrollbars 7 px, 11 px on hover, fading
700 ms after the scroll stops. Remove `natural-scroll` from `shell.kdl` (the owner's Mac has it off).

**Verify:** flick a long Files list — it must coast and settle; hit the end — it must bounce.

---

## Task 9 — The owner's real defaults

From `docs/reference-captures-2026-09-18.md` and `FEEL_SPEC.md` §C: Dark by default · Dock tile 64,
pitch 76, gap 12, bottom margin 18, magnification off · menu bar 29 px · Files opens in **List** view
with the status bar on · natural scrolling off · locale `en_GB` with India region · no browser pinned
· bottom-right hot corner = New Note.

**Also:** rmac must follow the locale for words. On this profile macOS says **"Bin"**, **"Favourites"**,
**"Colour"**. Hardcoded US strings will feel foreign every single day.

---

## Task 10 — The three layouts the captures proved wrong

1. **Apps** is a floating panel ≈ 840 × 570 with a search field, a `⋯` menu, category chips and a
   7-column scrolling grid — **not** a full-screen Launchpad with pages.
2. **Search** shows an **inline completion inside the bar** ("terminal.app — Open") with the target's
   glyph at the right end; the result list is the exception, not the rule.
3. **Control Center** is a single column of mixed-size modules (Wi-Fi/Bluetooth pills, Now Playing
   card, a row of circular buttons, full-width Display and Sound sliders, "Edit Controls") — not a
   2 × 2 grid.

Build all three from the captures in `target/evidence/reference-mac/`, not from memory.

---

## Task 11 — Kill every foreign dialog

**Read:** `FEEL_SPEC.md` §D.7. In priority order: the **rmac file picker** (portal FileChooser
backend — it is the dialog users see most often), the **polkit agent**, then GTK/Qt/Electron/Firefox
theming.

**Verify:** open a file from GIMP, VLC, VS Code and Firefox; mount a disk; add a printer. No GNOME
or GTK dialog may appear anywhere in those flows.

---

## Task 12 — Words, then the ten-minute test

Apply the wording table in `FEEL_SPEC.md` §D.10 across every string, add the string test that fails
on "Error:", "Failed to", `/home/`, `dbus`. Then run the 35-step script in `FEEL_SPEC.md` §E and
report which lines fail. Repeat the script before every release.

---

## The loop, forever

```
capture → list numeric differences → fix the top five → capture again
```

Until you cannot tell which half of the pair is rmac.
